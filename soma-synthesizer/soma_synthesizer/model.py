"""
SOMA Mind — BiLSTM encoder + GRU decoder with attention.

Architecture per whitepaper Section 4.1:

    Encoder:  BiLSTM (2-layer, bidirectional)
    Decoder:  GRU autoregressive with additive attention over encoder states
    Heads:    opcode, arg0_type, arg1_type,
              span_s0/e0/s1/e1, ref0/ref1,
              lit0, lit1

Self-contained — depends only on torch.
"""

import torch
import torch.nn as nn
import torch.nn.functional as F

# ---------------------------------------------------------------------------
# Argument-type constants
# ---------------------------------------------------------------------------
ARG_NONE = 0
ARG_SPAN = 1
ARG_REF = 2
ARG_LITERAL = 3


class SomaMind(nn.Module):
    """Sequence-to-program model.

    Parameters
    ----------
    vocab_size : int
        Number of tokens in the input vocabulary (including specials).
    num_opcodes : int
        Number of opcode classes the model can predict (excluding internal
        start token — that is added automatically).
    max_steps : int
        Maximum decoder steps (program length).
    embed_dim : int
        Token embedding dimensionality.
    hidden_dim : int
        Uni-directional LSTM hidden size (encoder output is 2*hidden_dim).
    decoder_dim : int
        GRU hidden size.
    opcode_embed_dim : int
        Embedding size for previously predicted opcode fed back into the
        decoder.
    num_layers : int
        Number of BiLSTM layers in the encoder.
    dropout : float
        Dropout applied inside the encoder (between layers) and to
        embeddings.
    ref_dim : int
        Projection dimensionality for reference-pointer heads.
    """

    def __init__(
        self,
        vocab_size: int,
        num_opcodes: int,
        max_steps: int = 8,
        embed_dim: int = 64,
        hidden_dim: int = 128,
        decoder_dim: int = 256,
        opcode_embed_dim: int = 32,
        num_layers: int = 2,
        dropout: float = 0.3,
        ref_dim: int = 64,
    ):
        super().__init__()

        self.vocab_size = vocab_size
        self.num_opcodes = num_opcodes
        self.max_steps = max_steps
        self.decoder_dim = decoder_dim

        # The opcode embedding table needs room for all real opcodes plus
        # one extra entry used as the <START> token at t=0.
        self.start_tok = num_opcodes  # index for the synthetic start token

        enc_dim = hidden_dim * 2  # bidirectional

        # ----- Encoder -----
        self.embedding = nn.Embedding(vocab_size, embed_dim, padding_idx=0)
        self.encoder = nn.LSTM(
            embed_dim,
            hidden_dim,
            num_layers,
            bidirectional=True,
            batch_first=True,
            dropout=dropout if num_layers > 1 else 0.0,
        )

        # ----- Decoder -----
        self.op_emb = nn.Embedding(num_opcodes + 1, opcode_embed_dim)  # +1 for start
        self.init_h = nn.Linear(enc_dim, decoder_dim)
        self.ctx_q = nn.Linear(decoder_dim, enc_dim)
        self.gru = nn.GRUCell(opcode_embed_dim + enc_dim, decoder_dim)

        # ----- Output heads -----
        self.op_head = nn.Linear(decoder_dim, num_opcodes)

        # Argument-type heads: 4-way (NONE / SPAN / REF / LITERAL)
        self.a0t_head = nn.Linear(decoder_dim, 4)
        self.a1t_head = nn.Linear(decoder_dim, 4)

        # Span pointer heads (dot-product attention over encoder outputs)
        self.s0s_q = nn.Linear(decoder_dim, enc_dim)
        self.s0e_q = nn.Linear(decoder_dim, enc_dim)
        self.s1s_q = nn.Linear(decoder_dim, enc_dim)
        self.s1e_q = nn.Linear(decoder_dim, enc_dim)

        # Reference pointer heads (dot-product over previous decoder states)
        self.r0q = nn.Linear(decoder_dim, ref_dim)
        self.r0k = nn.Linear(decoder_dim, ref_dim)
        self.r1q = nn.Linear(decoder_dim, ref_dim)
        self.r1k = nn.Linear(decoder_dim, ref_dim)

        # Literal value heads (predict a token index from the vocabulary)
        self.lit0_head = nn.Linear(decoder_dim, vocab_size)
        self.lit1_head = nn.Linear(decoder_dim, vocab_size)

    # ------------------------------------------------------------------
    # Encoder
    # ------------------------------------------------------------------

    def encode(
        self,
        ids: torch.Tensor,
        lens: torch.Tensor,
    ) -> tuple[torch.Tensor, torch.Tensor, torch.Tensor]:
        """Encode a padded batch of token-index sequences.

        Parameters
        ----------
        ids : (B, L) LongTensor
        lens : (B,) LongTensor — true lengths before padding

        Returns
        -------
        enc_out : (B, L, enc_dim) — per-position encoder states
        enc_mask : (B, L) BoolTensor — True for real positions
        pooled : (B, enc_dim) — mean-pooled encoder summary
        """
        B, L = ids.shape
        emb = self.embedding(ids)
        packed = nn.utils.rnn.pack_padded_sequence(
            emb, lens.cpu(), batch_first=True, enforce_sorted=False,
        )
        out, _ = self.encoder(packed)
        out, _ = nn.utils.rnn.pad_packed_sequence(
            out, batch_first=True, total_length=L,
        )

        pos = torch.arange(L, device=ids.device).unsqueeze(0)
        mask = pos < lens.unsqueeze(1)  # (B, L)

        mf = mask.unsqueeze(-1).float()
        pooled = (out * mf).sum(1) / lens.unsqueeze(1).float()

        return out, mask, pooled

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------

    def _ctx(
        self,
        h: torch.Tensor,
        enc_out: torch.Tensor,
        enc_mask: torch.Tensor,
    ) -> torch.Tensor:
        """Compute attention context vector over encoder outputs."""
        q = self.ctx_q(h)  # (B, enc_dim)
        scores = torch.bmm(
            q.unsqueeze(1), enc_out.transpose(1, 2),
        ).squeeze(1)  # (B, L)
        scores = scores.masked_fill(~enc_mask, -1e9)
        attn = F.softmax(scores, dim=-1)
        ctx = torch.bmm(attn.unsqueeze(1), enc_out).squeeze(1)  # (B, enc_dim)
        return ctx

    def _span(
        self,
        h: torch.Tensor,
        enc_out: torch.Tensor,
        enc_mask: torch.Tensor,
        query_layer: nn.Linear,
    ) -> torch.Tensor:
        """Span pointer logits (B, L) via dot-product attention."""
        logits = torch.bmm(
            query_layer(h).unsqueeze(1), enc_out.transpose(1, 2),
        ).squeeze(1)
        return logits.masked_fill(~enc_mask, -1e9)

    def _ref(
        self,
        h: torch.Tensor,
        prev_h: list[torch.Tensor],
        query_layer: nn.Linear,
        key_layer: nn.Linear,
    ) -> torch.Tensor:
        """Reference pointer logits (B, max_steps) over previous decoder states."""
        B = h.size(0)
        dev = h.device
        ms = self.max_steps
        t = len(prev_h)
        if t == 0:
            return torch.full((B, ms), -1e9, device=dev)
        keys = torch.stack(prev_h, dim=1)  # (B, t, decoder_dim)
        scores = torch.bmm(
            query_layer(h).unsqueeze(1), key_layer(keys).transpose(1, 2),
        ).squeeze(1)  # (B, t)
        if t < ms:
            pad = torch.full((B, ms - t), -1e9, device=dev)
            scores = torch.cat([scores, pad], dim=1)
        return scores

    # ------------------------------------------------------------------
    # Forward (teacher-forced training)
    # ------------------------------------------------------------------

    def forward(
        self,
        input_ids: torch.Tensor,
        lengths: torch.Tensor,
        targets: torch.Tensor,
    ) -> dict[str, torch.Tensor]:
        """Teacher-forced forward pass.

        Parameters
        ----------
        input_ids : (B, L) LongTensor — padded encoder input
        lengths : (B,) LongTensor — true lengths
        targets : (B, max_steps) LongTensor — ground-truth opcode ids for
            teacher forcing (shifted by one inside this method; at t the
            decoder receives targets[:, t-1]).

        Returns
        -------
        dict mapping head name to (B, max_steps, *) logit tensors:
            op       (B, T, num_opcodes)
            a0t      (B, T, 4)
            a1t      (B, T, 4)
            s0s      (B, T, L)   span start arg0
            s0e      (B, T, L)   span end   arg0
            s1s      (B, T, L)   span start arg1
            s1e      (B, T, L)   span end   arg1
            r0       (B, T, max_steps) ref arg0
            r1       (B, T, max_steps) ref arg1
            lit0     (B, T, vocab_size) literal arg0
            lit1     (B, T, vocab_size) literal arg1
        """
        B = input_ids.size(0)
        dev = input_ids.device

        enc_out, enc_mask, pooled = self.encode(input_ids, lengths)
        h = torch.tanh(self.init_h(pooled))  # (B, decoder_dim)

        prev_h: list[torch.Tensor] = []
        outs: dict[str, list[torch.Tensor]] = {
            k: []
            for k in [
                "op", "a0t", "a1t",
                "s0s", "s0e", "s1s", "s1e",
                "r0", "r1",
                "lit0", "lit1",
            ]
        }

        for t in range(self.max_steps):
            # Previous opcode: start token at t=0, else ground-truth
            if t == 0:
                prev_op = torch.full(
                    (B,), self.start_tok, dtype=torch.long, device=dev,
                )
            else:
                prev_op = targets[:, t - 1]

            ctx = self._ctx(h, enc_out, enc_mask)
            gru_in = torch.cat([self.op_emb(prev_op), ctx], dim=-1)
            h = self.gru(gru_in, h)
            prev_h.append(h)

            outs["op"].append(self.op_head(h))
            outs["a0t"].append(self.a0t_head(h))
            outs["a1t"].append(self.a1t_head(h))

            outs["s0s"].append(self._span(h, enc_out, enc_mask, self.s0s_q))
            outs["s0e"].append(self._span(h, enc_out, enc_mask, self.s0e_q))
            outs["s1s"].append(self._span(h, enc_out, enc_mask, self.s1s_q))
            outs["s1e"].append(self._span(h, enc_out, enc_mask, self.s1e_q))

            outs["r0"].append(self._ref(h, prev_h[:-1], self.r0q, self.r0k))
            outs["r1"].append(self._ref(h, prev_h[:-1], self.r1q, self.r1k))

            outs["lit0"].append(self.lit0_head(h))
            outs["lit1"].append(self.lit1_head(h))

        return {k: torch.stack(v, dim=1) for k, v in outs.items()}

    # ------------------------------------------------------------------
    # Autoregressive prediction
    # ------------------------------------------------------------------

    @torch.no_grad()
    def predict(
        self,
        input_ids: torch.Tensor,
        lengths: torch.Tensor,
        stop_opcode: int | None = None,
    ) -> dict[str, torch.Tensor]:
        """Autoregressive (greedy) decoding.

        Parameters
        ----------
        input_ids : (1, L) LongTensor — single example, padded
        lengths : (1,) LongTensor
        stop_opcode : int | None
            If the predicted opcode equals *stop_opcode*, decoding halts
            early.  Pass ``None`` to always run for ``max_steps``.

        Returns
        -------
        dict with:
            ops       (1, T') LongTensor — predicted opcode indices
            a0t       (1, T') LongTensor — predicted arg0 types
            a1t       (1, T') LongTensor — predicted arg1 types
            s0s       (1, T') LongTensor — span-start arg0
            s0e       (1, T') LongTensor — span-end   arg0
            s1s       (1, T') LongTensor — span-start arg1
            s1e       (1, T') LongTensor — span-end   arg1
            r0        (1, T') LongTensor — ref arg0
            r1        (1, T') LongTensor — ref arg1
            lit0      (1, T') LongTensor — literal arg0 token index
            lit1      (1, T') LongTensor — literal arg1 token index
            op_probs  (1, T') FloatTensor — opcode softmax max confidence
        """
        was_training = self.training
        self.train(False)
        dev = input_ids.device

        enc_out, enc_mask, pooled = self.encode(input_ids, lengths)
        h = torch.tanh(self.init_h(pooled))

        prev_op = torch.tensor([self.start_tok], dtype=torch.long, device=dev)
        prev_h: list[torch.Tensor] = []

        results: dict[str, list[torch.Tensor]] = {
            k: []
            for k in [
                "ops", "a0t", "a1t",
                "s0s", "s0e", "s1s", "s1e",
                "r0", "r1",
                "lit0", "lit1",
                "op_probs",
            ]
        }

        for _t in range(self.max_steps):
            ctx = self._ctx(h, enc_out, enc_mask)
            gru_in = torch.cat([self.op_emb(prev_op), ctx], dim=-1)
            h = self.gru(gru_in, h)
            prev_h.append(h)

            op_logits = self.op_head(h)
            op_probs = F.softmax(op_logits, dim=-1)
            pred_op = op_logits.argmax(dim=-1)  # (1,)

            results["ops"].append(pred_op)
            results["op_probs"].append(op_probs.max(dim=-1).values)

            results["a0t"].append(self.a0t_head(h).argmax(dim=-1))
            results["a1t"].append(self.a1t_head(h).argmax(dim=-1))

            results["s0s"].append(
                self._span(h, enc_out, enc_mask, self.s0s_q).argmax(dim=-1),
            )
            results["s0e"].append(
                self._span(h, enc_out, enc_mask, self.s0e_q).argmax(dim=-1),
            )
            results["s1s"].append(
                self._span(h, enc_out, enc_mask, self.s1s_q).argmax(dim=-1),
            )
            results["s1e"].append(
                self._span(h, enc_out, enc_mask, self.s1e_q).argmax(dim=-1),
            )

            results["r0"].append(
                self._ref(h, prev_h[:-1], self.r0q, self.r0k).argmax(dim=-1),
            )
            results["r1"].append(
                self._ref(h, prev_h[:-1], self.r1q, self.r1k).argmax(dim=-1),
            )

            results["lit0"].append(self.lit0_head(h).argmax(dim=-1))
            results["lit1"].append(self.lit1_head(h).argmax(dim=-1))

            # Early stop
            if stop_opcode is not None and pred_op.item() == stop_opcode:
                break

            prev_op = pred_op

        self.train(was_training)
        return {k: torch.stack(v, dim=1) for k, v in results.items()}


class TransformerMind(nn.Module):
    """Transformer-based Mind architecture for large SOMAs (Spec Section 4.2).

    For SOMAs with 100+ conventions (web applications with many plugins).
    Uses Transformer encoder (4-8 layers, 4-8 heads) and Transformer decoder
    with cross-attention to encoder output.

    Advantages over BiLSTM+GRU:
    - Better at capturing long-range dependencies in intents (>50 tokens)
    - Better at generating longer programs (>8 steps)
    - Requires more parameters (~10-50M vs ~1M for BiLSTM)

    Status: Architecture designed, implementation pending.
    The MindEngine trait in soma-core supports this transparently —
    a TransformerMind would produce the same output format (Program).
    """

    def __init__(self, vocab_size, num_conventions, max_steps=16,
                 embed_dim=256, num_heads=8, num_encoder_layers=6,
                 num_decoder_layers=6, ff_dim=1024, dropout=0.1,
                 opcode_embed_dim=64):
        super().__init__()
        raise NotImplementedError(
            "TransformerMind is designed but not yet implemented. "
            "Use SomaMind (BiLSTM+GRU) for current synthesis. "
            "See 07_SYNTHESIZER.md Section 4.2 for architecture details."
        )
