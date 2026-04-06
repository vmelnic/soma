"""
SOMA Mind — The neural architecture.

This IS the SOMA's brain. A BiLSTM encoder with two output heads:
  - Classification head: maps intent to an operation opcode (Layer 2: Planning)
  - Span extraction head: extracts parameter positions from input (Layer 3: Execution)

The Mind outputs TENSORS, not text. The opcode is an argmax of logits.
Parameter positions are argmax indices into the input sequence.
No code is generated at any point.
"""

import torch
import torch.nn as nn
from torch import Tensor

from poc.body import NUM_OPERATIONS, MAX_PARAM_SLOTS


class SomaMind(nn.Module):

    def __init__(
        self,
        vocab_size: int,
        embed_dim: int = 64,
        hidden_dim: int = 128,
        num_ops: int = NUM_OPERATIONS,
        num_layers: int = 2,
        dropout: float = 0.3,
    ):
        super().__init__()

        self.hidden_dim = hidden_dim
        self.num_ops = num_ops
        encoder_out_dim = hidden_dim * 2  # bidirectional

        # --- Layer 1: Intent Reception ---
        self.embedding = nn.Embedding(vocab_size, embed_dim, padding_idx=0)

        self.encoder = nn.LSTM(
            input_size=embed_dim,
            hidden_size=hidden_dim,
            num_layers=num_layers,
            bidirectional=True,
            batch_first=True,
            dropout=dropout if num_layers > 1 else 0.0,
        )

        # --- Layer 2: Planning (what operation?) ---
        self.op_classifier = nn.Sequential(
            nn.Linear(encoder_out_dim, hidden_dim),
            nn.ReLU(),
            nn.Dropout(dropout),
            nn.Linear(hidden_dim, num_ops),
        )

        # --- Layer 3: Execution (what parameters?) ---
        # Separate start/end scorers per parameter slot.
        # Each scores every token position independently.
        self.span_start_0 = nn.Linear(encoder_out_dim, 1)
        self.span_end_0 = nn.Linear(encoder_out_dim, 1)
        self.span_start_1 = nn.Linear(encoder_out_dim, 1)
        self.span_end_1 = nn.Linear(encoder_out_dim, 1)

    def forward(
        self, input_ids: Tensor, lengths: Tensor
    ) -> tuple[Tensor, list[tuple[Tensor, Tensor]]]:
        """
        Args:
            input_ids: (batch, max_seq_len) padded token indices
            lengths:   (batch,) actual sequence lengths

        Returns:
            op_logits:   (batch, num_ops)
            span_logits: list of 2 tuples (start_logits, end_logits),
                         each (batch, max_seq_len)
        """
        batch_size, max_len = input_ids.shape

        # Embed
        embedded = self.embedding(input_ids)  # (batch, seq, embed_dim)

        # Pack and encode
        packed = nn.utils.rnn.pack_padded_sequence(
            embedded, lengths.cpu(), batch_first=True, enforce_sorted=False
        )
        packed_out, _ = self.encoder(packed)
        encoder_out, _ = nn.utils.rnn.pad_packed_sequence(
            packed_out, batch_first=True, total_length=max_len
        )
        # encoder_out: (batch, seq, hidden*2)

        # Create mask for non-padded positions
        positions = torch.arange(max_len, device=input_ids.device).unsqueeze(0)
        mask = positions < lengths.unsqueeze(1)  # (batch, seq)

        # --- Classification: mean-pool over non-padded tokens ---
        mask_f = mask.unsqueeze(-1).float()  # (batch, seq, 1)
        pooled = (encoder_out * mask_f).sum(dim=1) / lengths.unsqueeze(1).float()
        op_logits = self.op_classifier(pooled)  # (batch, num_ops)

        # --- Span extraction: per-token scores ---
        neg_inf = (~mask).float() * -1e9

        start_0 = self.span_start_0(encoder_out).squeeze(-1) + neg_inf
        end_0 = self.span_end_0(encoder_out).squeeze(-1) + neg_inf
        start_1 = self.span_start_1(encoder_out).squeeze(-1) + neg_inf
        end_1 = self.span_end_1(encoder_out).squeeze(-1) + neg_inf

        span_logits = [(start_0, end_0), (start_1, end_1)]

        return op_logits, span_logits

    @torch.no_grad()
    def predict(
        self, input_ids: Tensor, lengths: Tensor
    ) -> tuple[Tensor, list[tuple[Tensor, Tensor]], Tensor]:
        """
        Inference: returns predicted opcode, spans, and confidence.

        Returns:
            predicted_op: (batch,) int tensor of opcodes
            spans: list of 2 (start_idx, end_idx) tuples, each (batch,)
            confidence: (batch,) float tensor of max softmax probability
        """
        self.eval()
        op_logits, span_logits = self.forward(input_ids, lengths)

        # Opcode: argmax
        op_probs = torch.softmax(op_logits, dim=-1)
        confidence = op_probs.max(dim=-1).values
        predicted_op = op_logits.argmax(dim=-1)

        # Spans: argmax of start and end
        spans = []
        for start_logits, end_logits in span_logits:
            start_idx = start_logits.argmax(dim=-1)
            end_idx = end_logits.argmax(dim=-1)
            # Enforce end >= start
            end_idx = torch.max(end_idx, start_idx)
            spans.append((start_idx, end_idx))

        return predicted_op, spans, confidence
