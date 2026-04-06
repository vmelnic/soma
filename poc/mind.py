"""
SOMA Mind — v0.3: Seq2Seq with Dynamic Arg Types.

The Mind IS the program. It generates multi-step programs with data
dependencies, including cross-operation data flow.

v0.3 changes from v0.2:
  - Model PREDICTS arg types (none/span/ref) instead of looking up from schema
  - Enables cross-operation data flow (FILE_WRITE can take REF for data arg)
  - Returns confidence for ambiguity detection
"""

import torch
import torch.nn as nn
import torch.nn.functional as F
from torch import Tensor

from poc.body import NUM_PRIMITIVES, MAX_PROGRAM_STEPS, START_TOKEN, Prim, ProgramStep

# Arg type constants
ARG_NONE = 0
ARG_SPAN = 1
ARG_REF = 2
NUM_ARG_TYPES = 3


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
        encoder_out_dim = hidden_dim * 2  # 256

        # ========== ENCODER ==========
        self.embedding = nn.Embedding(vocab_size, embed_dim, padding_idx=0)
        self.encoder = nn.LSTM(
            input_size=embed_dim,
            hidden_size=hidden_dim,
            num_layers=num_layers,
            bidirectional=True,
            batch_first=True,
            dropout=dropout if num_layers > 1 else 0.0,
        )

        # ========== DECODER ==========
        self.opcode_embedding = nn.Embedding(NUM_PRIMITIVES + 1, opcode_embed_dim)
        self.init_hidden = nn.Linear(encoder_out_dim, decoder_dim)
        self.context_query = nn.Linear(decoder_dim, encoder_out_dim)
        self.decoder_gru = nn.GRUCell(
            input_size=opcode_embed_dim + encoder_out_dim,
            hidden_size=decoder_dim,
        )

        # --- Output heads ---
        self.opcode_head = nn.Linear(decoder_dim, NUM_PRIMITIVES)

        # Arg type prediction (v0.3: model decides type, not schema)
        self.arg0_type_head = nn.Linear(decoder_dim, NUM_ARG_TYPES)
        self.arg1_type_head = nn.Linear(decoder_dim, NUM_ARG_TYPES)

        # Span extraction
        self.span_s0_q = nn.Linear(decoder_dim, encoder_out_dim)
        self.span_e0_q = nn.Linear(decoder_dim, encoder_out_dim)
        self.span_s1_q = nn.Linear(decoder_dim, encoder_out_dim)
        self.span_e1_q = nn.Linear(decoder_dim, encoder_out_dim)

        # Ref prediction
        ref_dim = 64
        self.ref0_q = nn.Linear(decoder_dim, ref_dim)
        self.ref0_k = nn.Linear(decoder_dim, ref_dim)
        self.ref1_q = nn.Linear(decoder_dim, ref_dim)
        self.ref1_k = nn.Linear(decoder_dim, ref_dim)

    def encode(self, input_ids: Tensor, lengths: Tensor):
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
        mask_f = mask.unsqueeze(-1).float()
        pooled = (encoder_out * mask_f).sum(dim=1) / lengths.unsqueeze(1).float()
        return encoder_out, mask, pooled

    def _compute_context(self, h, encoder_out, enc_mask):
        query = self.context_query(h)
        scores = torch.bmm(query.unsqueeze(1), encoder_out.transpose(1, 2)).squeeze(1)
        scores = scores.masked_fill(~enc_mask, -1e9)
        weights = F.softmax(scores, dim=-1)
        return torch.bmm(weights.unsqueeze(1), encoder_out).squeeze(1)

    def _compute_span_logits(self, h, encoder_out, enc_mask, query_layer):
        q = query_layer(h)
        logits = torch.bmm(q.unsqueeze(1), encoder_out.transpose(1, 2)).squeeze(1)
        return logits.masked_fill(~enc_mask, -1e9)

    def _compute_ref_logits(self, h, prev_hiddens, q_layer, k_layer, max_steps):
        batch, device = h.size(0), h.device
        t = len(prev_hiddens)
        if t == 0:
            return torch.full((batch, max_steps), -1e9, device=device)
        keys = torch.stack(prev_hiddens, dim=1)
        q = q_layer(h).unsqueeze(1)
        k = k_layer(keys)
        scores = torch.bmm(q, k.transpose(1, 2)).squeeze(1)
        if t < max_steps:
            pad = torch.full((batch, max_steps - t), -1e9, device=device)
            scores = torch.cat([scores, pad], dim=1)
        return scores

    def forward(self, input_ids, lengths, target_opcodes) -> dict:
        """Forward with teacher forcing. Returns all logits for loss computation."""
        batch = input_ids.size(0)
        device = input_ids.device
        max_steps = MAX_PROGRAM_STEPS

        encoder_out, enc_mask, pooled = self.encode(input_ids, lengths)
        h = torch.tanh(self.init_hidden(pooled))

        all_op, all_a0t, all_a1t = [], [], []
        all_s0s, all_s0e, all_s1s, all_s1e = [], [], [], []
        all_r0, all_r1 = [], []
        prev_hiddens = []

        for t in range(max_steps):
            if t == 0:
                prev_op = torch.full((batch,), START_TOKEN, dtype=torch.long, device=device)
            else:
                prev_op = target_opcodes[:, t - 1]

            prev_emb = self.opcode_embedding(prev_op)
            context = self._compute_context(h, encoder_out, enc_mask)
            h = self.decoder_gru(torch.cat([prev_emb, context], dim=-1), h)
            prev_hiddens.append(h)

            all_op.append(self.opcode_head(h))
            all_a0t.append(self.arg0_type_head(h))
            all_a1t.append(self.arg1_type_head(h))

            all_s0s.append(self._compute_span_logits(h, encoder_out, enc_mask, self.span_s0_q))
            all_s0e.append(self._compute_span_logits(h, encoder_out, enc_mask, self.span_e0_q))
            all_s1s.append(self._compute_span_logits(h, encoder_out, enc_mask, self.span_s1_q))
            all_s1e.append(self._compute_span_logits(h, encoder_out, enc_mask, self.span_e1_q))

            all_r0.append(self._compute_ref_logits(h, prev_hiddens[:-1], self.ref0_q, self.ref0_k, max_steps))
            all_r1.append(self._compute_ref_logits(h, prev_hiddens[:-1], self.ref1_q, self.ref1_k, max_steps))

        return {
            "op_logits": torch.stack(all_op, dim=1),
            "a0t": torch.stack(all_a0t, dim=1),   # (batch, steps, 3)
            "a1t": torch.stack(all_a1t, dim=1),
            "s0s": torch.stack(all_s0s, dim=1),
            "s0e": torch.stack(all_s0e, dim=1),
            "s1s": torch.stack(all_s1s, dim=1),
            "s1e": torch.stack(all_s1e, dim=1),
            "r0": torch.stack(all_r0, dim=1),
            "r1": torch.stack(all_r1, dim=1),
        }

    @torch.no_grad()
    def predict(self, input_ids, lengths, tokens):
        """Autoregressive inference. Returns (steps, confidence)."""
        self.eval()
        device = input_ids.device

        encoder_out, enc_mask, pooled = self.encode(input_ids, lengths)
        h = torch.tanh(self.init_hidden(pooled))

        prev_op = torch.tensor([START_TOKEN], dtype=torch.long, device=device)
        prev_hiddens = []
        steps = []
        first_confidence = 0.0

        for t in range(MAX_PROGRAM_STEPS):
            prev_emb = self.opcode_embedding(prev_op)
            context = self._compute_context(h, encoder_out, enc_mask)
            h = self.decoder_gru(torch.cat([prev_emb, context], dim=-1), h)
            prev_hiddens.append(h)

            op_logits = self.opcode_head(h)
            op_probs = F.softmax(op_logits, dim=-1)
            predicted_op = op_logits.argmax(dim=-1).item()

            if t == 0:
                first_confidence = op_probs.max().item()

            if predicted_op == Prim.STOP:
                steps.append(ProgramStep(Prim.STOP, "none", None, "none", None))
                break

            # Predict arg types (v0.3: dynamic, not schema-based)
            a0_type_pred = self.arg0_type_head(h).argmax(dim=-1).item()
            a1_type_pred = self.arg1_type_head(h).argmax(dim=-1).item()

            a0_type, a0_val = self._resolve_predicted_arg(
                a0_type_pred, h, encoder_out, enc_mask, prev_hiddens[:-1],
                self.span_s0_q, self.span_e0_q, self.ref0_q, self.ref0_k, tokens
            )
            a1_type, a1_val = self._resolve_predicted_arg(
                a1_type_pred, h, encoder_out, enc_mask, prev_hiddens[:-1],
                self.span_s1_q, self.span_e1_q, self.ref1_q, self.ref1_k, tokens
            )

            steps.append(ProgramStep(predicted_op, a0_type, a0_val, a1_type, a1_val))
            prev_op = torch.tensor([predicted_op], dtype=torch.long, device=device)

        return steps, first_confidence

    def _resolve_predicted_arg(self, type_id, h, encoder_out, enc_mask,
                                prev_hiddens, span_s_q, span_e_q,
                                ref_q, ref_k, tokens):
        """Resolve argument using model-predicted type."""
        if type_id == ARG_NONE:
            return ("none", None)

        elif type_id == ARG_SPAN:
            s = self._compute_span_logits(h, encoder_out, enc_mask, span_s_q).argmax(dim=-1).item()
            e = self._compute_span_logits(h, encoder_out, enc_mask, span_e_q).argmax(dim=-1).item()
            e = max(e, s)
            if s == 0 and e == 0:
                return ("span", "")
            param_tokens = tokens[s - 1: e]
            return ("span", " ".join(param_tokens))

        elif type_id == ARG_REF:
            ref_logits = self._compute_ref_logits(h, prev_hiddens, ref_q, ref_k, MAX_PROGRAM_STEPS)
            return ("ref", ref_logits.argmax(dim=-1).item())

        return ("none", None)
