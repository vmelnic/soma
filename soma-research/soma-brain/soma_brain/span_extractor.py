"""
Span extractor — predicts answer start/end positions in a passage.

For extractive QA: given a passage and question context from the reasoning
core, predict which byte span in the passage is the answer.

Structured output, not text generation.
"""

import torch
import torch.nn as nn
import torch.nn.functional as F


class CrossAttentionBlock(nn.Module):

    def __init__(self, hidden_size: int, num_heads: int):
        super().__init__()
        self.cross_attn = nn.MultiheadAttention(
            hidden_size, num_heads, batch_first=True,
        )
        self.cross_norm = nn.LayerNorm(hidden_size)
        self.ffn = nn.Sequential(
            nn.Linear(hidden_size, hidden_size * 4),
            nn.GELU(),
            nn.Linear(hidden_size * 4, hidden_size),
        )
        self.ffn_norm = nn.LayerNorm(hidden_size)

    def forward(self, x: torch.Tensor, cond: torch.Tensor) -> torch.Tensor:
        h = self.cross_norm(x)
        attn_out = self.cross_attn(h, cond, cond, need_weights=False)[0]
        x = x + attn_out
        x = x + self.ffn(self.ffn_norm(x))
        return x


class SpanExtractor(nn.Module):

    def __init__(
        self,
        hidden_size: int = 512,
        cond_size: int = 1024,
        num_heads: int = 8,
        num_encoder_layers: int = 2,
        num_cross_layers: int = 3,
        max_ctx_len: int = 2048,
    ):
        super().__init__()
        self.hidden_size = hidden_size
        self.max_ctx_len = max_ctx_len

        self.byte_embed = nn.Embedding(256, hidden_size)
        self.pos_embed = nn.Embedding(max_ctx_len, hidden_size)
        self.cond_proj = nn.Linear(cond_size, hidden_size)

        self.encoder = nn.GRU(
            hidden_size, hidden_size, num_encoder_layers,
            batch_first=True, bidirectional=True,
        )
        self.enc_proj = nn.Linear(hidden_size * 2, hidden_size)

        self.cross_layers = nn.ModuleList([
            CrossAttentionBlock(hidden_size, num_heads)
            for _ in range(num_cross_layers)
        ])
        self.out_norm = nn.LayerNorm(hidden_size)

        self.start_proj = nn.Linear(hidden_size, 1)
        self.end_proj = nn.Linear(hidden_size, 1)

    def forward(
        self,
        context_ids: torch.Tensor,
        context_lengths: torch.Tensor,
        conditioning: torch.Tensor,
    ) -> tuple[torch.Tensor, torch.Tensor]:
        """
        context_ids:     (B, ctx_len) byte IDs of passage
        context_lengths: (B,) actual lengths (before padding)
        conditioning:    (B, cond_seq, cond_dim) from reasoning core

        Returns: (start_logits, end_logits) each (B, ctx_len)
        """
        B, L = context_ids.shape
        pos = torch.arange(L, device=context_ids.device).unsqueeze(0)
        x = self.byte_embed(context_ids) + self.pos_embed(pos)

        enc, _ = self.encoder(x)
        enc = self.enc_proj(enc)

        cond = self.cond_proj(conditioning)
        for layer in self.cross_layers:
            enc = layer(enc, cond)
        enc = self.out_norm(enc)

        start_logits = self.start_proj(enc).squeeze(-1)
        end_logits = self.end_proj(enc).squeeze(-1)

        mask = pos.expand(B, -1) >= context_lengths.unsqueeze(1)
        start_logits = start_logits.masked_fill(mask, -1e9)
        end_logits = end_logits.masked_fill(mask, -1e9)

        return start_logits, end_logits

    @torch.no_grad()
    def extract(
        self,
        context_ids: torch.Tensor,
        context_lengths: torch.Tensor,
        conditioning: torch.Tensor,
        max_answer_len: int = 64,
    ) -> list[tuple[int, int]]:
        """Returns list of (start, end) byte positions for each batch element."""
        start_logits, end_logits = self.forward(
            context_ids, context_lengths, conditioning,
        )
        B, L = context_ids.shape
        spans = []

        for b in range(B):
            length = context_lengths[b].item()
            s_log = start_logits[b, :length]
            e_log = end_logits[b, :length]

            s_probs = F.softmax(s_log, dim=-1)
            e_probs = F.softmax(e_log, dim=-1)

            best_score = -1e9
            best_start, best_end = 0, 0
            top_starts = torch.topk(s_probs, min(20, length)).indices

            for s in top_starts:
                s = s.item()
                end_max = min(s + max_answer_len, length)
                if s >= end_max:
                    continue
                e_slice = e_probs[s:end_max]
                e_idx = e_slice.argmax().item()
                score = s_probs[s].item() * e_probs[s + e_idx].item()
                if score > best_score:
                    best_score = score
                    best_start = s
                    best_end = s + e_idx

            spans.append((best_start, best_end))

        return spans
