"""
Discrete diffusion decoder — generates text from the liquid core's hidden state.

Masked diffusion: at training time, randomly mask tokens and learn to predict
them conditioned on the reasoning core's hidden state. At inference, start
from all-mask and iteratively unmask over N refinement steps.

The decoder does NOT attend to itself autoregressively. All positions are
predicted in parallel, refined iteratively. Generation time scales with
refinement steps, not sequence length.

Reference: Sahoo et al., "Simple and Effective Masked Diffusion Language
Models" (MDLM, NeurIPS 2024).
"""

import math

import torch
import torch.nn as nn
import torch.nn.functional as F


class DiffusionBlock(nn.Module):

    def __init__(self, hidden_size: int, num_heads: int, cond_size: int):
        super().__init__()
        self.self_attn = nn.MultiheadAttention(hidden_size, num_heads, batch_first=True)
        self.cross_attn = nn.MultiheadAttention(hidden_size, num_heads, batch_first=True)
        self.ff = nn.Sequential(
            nn.Linear(hidden_size, hidden_size * 4),
            nn.GELU(),
            nn.Linear(hidden_size * 4, hidden_size),
        )
        self.norm1 = nn.LayerNorm(hidden_size)
        self.norm2 = nn.LayerNorm(hidden_size)
        self.norm3 = nn.LayerNorm(hidden_size)
        self.cond_proj = nn.Linear(cond_size, hidden_size) if cond_size != hidden_size else nn.Identity()

    def forward(self, x: torch.Tensor, cond: torch.Tensor) -> torch.Tensor:
        c = self.cond_proj(cond)
        h = self.norm1(x)
        h = x + self.self_attn(h, h, h, need_weights=False)[0]
        h2 = self.norm2(h)
        h = h + self.cross_attn(h2, c, c, need_weights=False)[0]
        h = h + self.ff(self.norm3(h))
        return h


class DiffusionDecoder(nn.Module):

    def __init__(
        self,
        vocab_size: int,
        hidden_size: int = 512,
        num_layers: int = 6,
        num_heads: int = 8,
        cond_size: int = 512,
        max_seq_len: int = 256,
        mask_token_id: int = 0,
    ):
        super().__init__()
        self.vocab_size = vocab_size
        self.hidden_size = hidden_size
        self.max_seq_len = max_seq_len
        self.mask_token_id = mask_token_id

        self.token_embed = nn.Embedding(vocab_size + 1, hidden_size)
        self.pos_embed = nn.Embedding(max_seq_len, hidden_size)
        self.timestep_embed = nn.Sequential(
            nn.Linear(1, hidden_size),
            nn.SiLU(),
            nn.Linear(hidden_size, hidden_size),
        )

        self.blocks = nn.ModuleList([
            DiffusionBlock(hidden_size, num_heads, cond_size)
            for _ in range(num_layers)
        ])

        self.out_norm = nn.LayerNorm(hidden_size)
        self.out_proj = nn.Linear(hidden_size, vocab_size)

    def forward(
        self,
        token_ids: torch.Tensor,
        timestep: torch.Tensor,
        conditioning: torch.Tensor,
    ) -> torch.Tensor:
        """
        token_ids:    (batch, seq_len) — partially masked token sequence
        timestep:     (batch, 1) — diffusion timestep in [0, 1]
        conditioning: (batch, cond_seq, cond_dim) — from liquid core

        Returns: (batch, seq_len, vocab_size) logits
        """
        B, S = token_ids.shape
        pos = torch.arange(S, device=token_ids.device).unsqueeze(0)

        x = self.token_embed(token_ids) + self.pos_embed(pos)
        t_emb = self.timestep_embed(timestep).unsqueeze(1)
        x = x + t_emb

        for block in self.blocks:
            x = block(x, conditioning)

        return self.out_proj(self.out_norm(x))

    def compute_loss(
        self,
        clean_ids: torch.Tensor,
        conditioning: torch.Tensor,
        lengths: torch.Tensor | None = None,
        mask_ratio: float | None = None,
    ) -> torch.Tensor:
        """Training loss: mask random tokens, predict them.

        lengths: (batch,) — real token count per sample. Positions beyond
        each length are excluded from masking so the model never trains on
        padding tokens.
        """
        B, S = clean_ids.shape
        device = clean_ids.device

        if mask_ratio is None:
            mask_ratio = torch.rand(1, device=device).item() * 0.8 + 0.1

        mask = torch.rand(B, S, device=device) < mask_ratio
        mask[:, 0] = False

        if lengths is not None:
            for b in range(B):
                mask[b, lengths[b]:] = False

        if mask.sum() == 0:
            return torch.tensor(0.0, device=device, requires_grad=True)

        noised = clean_ids.clone()
        noised[mask] = self.vocab_size

        t = torch.full((B, 1), mask_ratio, device=device)
        logits = self.forward(noised, t, conditioning)

        loss = F.cross_entropy(
            logits[mask].view(-1, self.vocab_size),
            clean_ids[mask].view(-1),
        )
        return loss

    @torch.no_grad()
    def generate(
        self,
        conditioning: torch.Tensor,
        seq_len: int = 128,
        steps: int = 16,
    ) -> torch.Tensor:
        """Iterative unmasking from all-mask to text."""
        B = conditioning.shape[0]
        device = conditioning.device

        ids = torch.full((B, seq_len), self.vocab_size, device=device, dtype=torch.long)

        for step in range(steps):
            t = 1.0 - step / steps
            t_tensor = torch.full((B, 1), t, device=device)
            logits = self.forward(ids, t_tensor, conditioning)
            probs = F.softmax(logits, dim=-1)

            is_masked = (ids == self.vocab_size)
            n_masked = is_masked.sum(dim=-1, keepdim=True).float()
            n_to_unmask = (n_masked / (steps - step)).clamp(min=1).long()

            confidence = probs.max(dim=-1).values
            confidence[~is_masked] = -1.0

            for b in range(B):
                k = min(n_to_unmask[b].item(), is_masked[b].sum().item())
                if k == 0:
                    continue
                _, top_pos = torch.topk(confidence[b], k)
                ids[b, top_pos] = probs[b, top_pos].argmax(dim=-1)

        remaining = (ids == self.vocab_size)
        if remaining.any():
            t_tensor = torch.zeros(B, 1, device=device)
            logits = self.forward(ids, t_tensor, conditioning)
            ids[remaining] = logits[remaining].argmax(dim=-1)

        return ids
