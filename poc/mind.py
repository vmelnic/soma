"""
SOMA Mind — Phase 2: Seq2Seq Program Generator.

The Mind IS the program. It takes natural language intent and outputs
a sequence of primitive instructions with data dependencies.

Architecture:
  Encoder: BiLSTM (kept from Phase 1)
  Decoder: Autoregressive GRU generating one program step per time step
    - Opcode head: which primitive to execute
    - Span heads: extract text from input (for span arguments)
    - Ref heads: reference return value of previous step (for ref arguments)
"""

import torch
import torch.nn as nn
import torch.nn.functional as F
from torch import Tensor

from poc.body import NUM_PRIMITIVES, MAX_PROGRAM_STEPS, START_TOKEN, OPCODE_SCHEMA, Prim, ProgramStep


class SomaMind(nn.Module):

    def __init__(
        self,
        vocab_size: int,
        embed_dim: int = 64,
        hidden_dim: int = 128,
        decoder_dim: int = 256,
        opcode_embed_dim: int = 32,
        num_layers: int = 2,
        dropout: float = 0.3,
    ):
        super().__init__()
        self.hidden_dim = hidden_dim
        self.decoder_dim = decoder_dim
        encoder_out_dim = hidden_dim * 2  # bidirectional = 256

        # ========== ENCODER (Layer 1: Intent Reception) ==========
        self.embedding = nn.Embedding(vocab_size, embed_dim, padding_idx=0)
        self.encoder = nn.LSTM(
            input_size=embed_dim,
            hidden_size=hidden_dim,
            num_layers=num_layers,
            bidirectional=True,
            batch_first=True,
            dropout=dropout if num_layers > 1 else 0.0,
        )

        # ========== DECODER (Layers 2+3: Planning + Execution) ==========

        # Opcode embeddings for decoder input (19 primitives + 1 START token)
        self.opcode_embedding = nn.Embedding(NUM_PRIMITIVES + 1, opcode_embed_dim)

        # Project encoder pooled output to decoder initial hidden state
        self.init_hidden = nn.Linear(encoder_out_dim, decoder_dim)

        # Attention over encoder outputs for context vector
        self.context_query = nn.Linear(decoder_dim, encoder_out_dim)

        # GRU decoder cell
        self.decoder_gru = nn.GRUCell(
            input_size=opcode_embed_dim + encoder_out_dim,  # 32 + 256 = 288
            hidden_size=decoder_dim,
        )

        # --- Output heads ---

        # Opcode prediction
        self.opcode_head = nn.Linear(decoder_dim, NUM_PRIMITIVES)

        # Span extraction (4 heads: start/end for arg0 and arg1)
        # Each projects decoder hidden to encoder space for dot-product attention
        self.span_s0_q = nn.Linear(decoder_dim, encoder_out_dim)
        self.span_e0_q = nn.Linear(decoder_dim, encoder_out_dim)
        self.span_s1_q = nn.Linear(decoder_dim, encoder_out_dim)
        self.span_e1_q = nn.Linear(decoder_dim, encoder_out_dim)

        # Ref prediction (query + key projections for attention over previous steps)
        ref_dim = 64
        self.ref0_q = nn.Linear(decoder_dim, ref_dim)
        self.ref0_k = nn.Linear(decoder_dim, ref_dim)
        self.ref1_q = nn.Linear(decoder_dim, ref_dim)
        self.ref1_k = nn.Linear(decoder_dim, ref_dim)

    def encode(self, input_ids: Tensor, lengths: Tensor):
        """Encode intent text. Returns (encoder_out, mask, pooled)."""
        batch_size, max_len = input_ids.shape
        embedded = self.embedding(input_ids)

        packed = nn.utils.rnn.pack_padded_sequence(
            embedded, lengths.cpu(), batch_first=True, enforce_sorted=False
        )
        packed_out, _ = self.encoder(packed)
        encoder_out, _ = nn.utils.rnn.pad_packed_sequence(
            packed_out, batch_first=True, total_length=max_len
        )

        positions = torch.arange(max_len, device=input_ids.device).unsqueeze(0)
        mask = positions < lengths.unsqueeze(1)

        # Mean pool for initial decoder state
        mask_f = mask.unsqueeze(-1).float()
        pooled = (encoder_out * mask_f).sum(dim=1) / lengths.unsqueeze(1).float()

        return encoder_out, mask, pooled

    def _compute_context(self, h: Tensor, encoder_out: Tensor, enc_mask: Tensor) -> Tensor:
        """Compute attention context vector over encoder outputs."""
        query = self.context_query(h)  # (batch, enc_dim)
        scores = torch.bmm(query.unsqueeze(1), encoder_out.transpose(1, 2)).squeeze(1)
        scores = scores.masked_fill(~enc_mask, -1e9)
        weights = F.softmax(scores, dim=-1)
        context = torch.bmm(weights.unsqueeze(1), encoder_out).squeeze(1)
        return context

    def _compute_span_logits(self, h: Tensor, encoder_out: Tensor, enc_mask: Tensor, query_layer: nn.Linear) -> Tensor:
        """Compute span position logits via dot-product attention."""
        q = query_layer(h)  # (batch, enc_dim)
        logits = torch.bmm(q.unsqueeze(1), encoder_out.transpose(1, 2)).squeeze(1)
        logits = logits.masked_fill(~enc_mask, -1e9)
        return logits

    def _compute_ref_logits(self, h: Tensor, prev_hiddens: list[Tensor],
                            q_layer: nn.Linear, k_layer: nn.Linear,
                            max_steps: int) -> Tensor:
        """Compute ref logits via attention over previous decoder hidden states."""
        batch = h.size(0)
        device = h.device
        t = len(prev_hiddens)

        if t == 0:
            return torch.full((batch, max_steps), -1e9, device=device)

        # Stack previous hiddens: (batch, t, decoder_dim)
        keys = torch.stack(prev_hiddens, dim=1)
        q = q_layer(h).unsqueeze(1)           # (batch, 1, ref_dim)
        k = k_layer(keys)                      # (batch, t, ref_dim)
        scores = torch.bmm(q, k.transpose(1, 2)).squeeze(1)  # (batch, t)

        # Pad to max_steps
        if t < max_steps:
            pad = torch.full((batch, max_steps - t), -1e9, device=device)
            scores = torch.cat([scores, pad], dim=1)

        return scores

    def forward(
        self,
        input_ids: Tensor,
        lengths: Tensor,
        target_opcodes: Tensor,  # (batch, max_steps)
    ) -> dict:
        """Forward pass with teacher forcing.

        Returns dict of logits tensors for loss computation.
        """
        batch = input_ids.size(0)
        device = input_ids.device
        max_steps = MAX_PROGRAM_STEPS

        # Encode
        encoder_out, enc_mask, pooled = self.encode(input_ids, lengths)

        # Init decoder
        h = torch.tanh(self.init_hidden(pooled))

        # Accumulators
        all_op = []
        all_s0s, all_s0e, all_s1s, all_s1e = [], [], [], []
        all_r0, all_r1 = [], []
        prev_hiddens = []

        for t in range(max_steps):
            # Previous opcode (teacher forcing)
            if t == 0:
                prev_op = torch.full((batch,), START_TOKEN, dtype=torch.long, device=device)
            else:
                prev_op = target_opcodes[:, t - 1]

            prev_emb = self.opcode_embedding(prev_op)
            context = self._compute_context(h, encoder_out, enc_mask)
            gru_input = torch.cat([prev_emb, context], dim=-1)
            h = self.decoder_gru(gru_input, h)
            prev_hiddens.append(h)

            # Opcode logits
            all_op.append(self.opcode_head(h))

            # Span logits
            all_s0s.append(self._compute_span_logits(h, encoder_out, enc_mask, self.span_s0_q))
            all_s0e.append(self._compute_span_logits(h, encoder_out, enc_mask, self.span_e0_q))
            all_s1s.append(self._compute_span_logits(h, encoder_out, enc_mask, self.span_s1_q))
            all_s1e.append(self._compute_span_logits(h, encoder_out, enc_mask, self.span_e1_q))

            # Ref logits
            all_r0.append(self._compute_ref_logits(h, prev_hiddens[:-1], self.ref0_q, self.ref0_k, max_steps))
            all_r1.append(self._compute_ref_logits(h, prev_hiddens[:-1], self.ref1_q, self.ref1_k, max_steps))

        return {
            "op_logits": torch.stack(all_op, dim=1),       # (batch, steps, 19)
            "s0s": torch.stack(all_s0s, dim=1),             # (batch, steps, seq_len)
            "s0e": torch.stack(all_s0e, dim=1),
            "s1s": torch.stack(all_s1s, dim=1),
            "s1e": torch.stack(all_s1e, dim=1),
            "r0": torch.stack(all_r0, dim=1),               # (batch, steps, max_steps)
            "r1": torch.stack(all_r1, dim=1),
        }

    @torch.no_grad()
    def predict(self, input_ids: Tensor, lengths: Tensor, tokens: list[str]) -> list[ProgramStep]:
        """Autoregressive inference. Returns a list of ProgramStep."""
        self.eval()
        device = input_ids.device

        encoder_out, enc_mask, pooled = self.encode(input_ids, lengths)
        h = torch.tanh(self.init_hidden(pooled))

        prev_op = torch.tensor([START_TOKEN], dtype=torch.long, device=device)
        prev_hiddens = []
        steps = []

        for t in range(MAX_PROGRAM_STEPS):
            prev_emb = self.opcode_embedding(prev_op)
            context = self._compute_context(h, encoder_out, enc_mask)
            gru_input = torch.cat([prev_emb, context], dim=-1)
            h = self.decoder_gru(gru_input, h)
            prev_hiddens.append(h)

            # Predict opcode
            op_logits = self.opcode_head(h)
            predicted_op = op_logits.argmax(dim=-1).item()

            if predicted_op == Prim.STOP:
                steps.append(ProgramStep(Prim.STOP, "none", None, "none", None))
                break

            # Get schema for this opcode
            schema = OPCODE_SCHEMA[predicted_op]

            # Resolve arg0
            a0_type, a0_val = self._resolve_arg(
                schema[0], h, encoder_out, enc_mask, prev_hiddens[:-1],
                self.span_s0_q, self.span_e0_q, self.ref0_q, self.ref0_k, tokens
            )

            # Resolve arg1
            a1_type, a1_val = self._resolve_arg(
                schema[1], h, encoder_out, enc_mask, prev_hiddens[:-1],
                self.span_s1_q, self.span_e1_q, self.ref1_q, self.ref1_k, tokens
            )

            steps.append(ProgramStep(predicted_op, a0_type, a0_val, a1_type, a1_val))
            prev_op = torch.tensor([predicted_op], dtype=torch.long, device=device)

        return steps

    def _resolve_arg(
        self, arg_type: str, h: Tensor, encoder_out: Tensor, enc_mask: Tensor,
        prev_hiddens: list[Tensor],
        span_s_q: nn.Linear, span_e_q: nn.Linear,
        ref_q: nn.Linear, ref_k: nn.Linear,
        tokens: list[str],
    ) -> tuple[str, object]:
        """Resolve one argument based on schema type."""
        if arg_type == "none":
            return ("none", None)

        elif arg_type == "span":
            s_logits = self._compute_span_logits(h, encoder_out, enc_mask, span_s_q)
            e_logits = self._compute_span_logits(h, encoder_out, enc_mask, span_e_q)
            s = s_logits.argmax(dim=-1).item()
            e = e_logits.argmax(dim=-1).item()
            e = max(e, s)
            # Positions are offset by 1 (position 0 = NULL token)
            if s == 0 and e == 0:
                return ("span", "")
            param_tokens = tokens[s - 1: e]
            return ("span", " ".join(param_tokens))

        elif arg_type == "ref":
            ref_logits = self._compute_ref_logits(
                h, prev_hiddens, ref_q, ref_k, MAX_PROGRAM_STEPS
            )
            ref_idx = ref_logits.argmax(dim=-1).item()
            return ("ref", ref_idx)

        return ("none", None)
