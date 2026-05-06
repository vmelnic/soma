"""
Autoregressive decoder — generates answer bytes by reading passage context.

GRU backbone with cross-attention to:
  1. Reasoning core's stacked layer outputs (question understanding)
  2. Byte-embedded passage text (answer source material)

The decoder learns reading comprehension: given a passage and question
context, extract and generate the answer byte-by-byte.
"""

import torch
import torch.nn as nn
import torch.nn.functional as F


class ARDecoder(nn.Module):

    def __init__(
        self,
        vocab_size: int = 256,
        hidden_size: int = 512,
        num_layers: int = 2,
        cond_size: int = 1024,
        max_seq_len: int = 256,
    ):
        super().__init__()
        self.vocab_size = vocab_size
        self.hidden_size = hidden_size
        self.num_layers = num_layers
        self.max_seq_len = max_seq_len

        self.embed = nn.Embedding(vocab_size + 1, hidden_size)
        self.ctx_embed = nn.Embedding(vocab_size, hidden_size)
        self.ctx_pos = nn.Embedding(1024, hidden_size)
        self.cond_proj = nn.Linear(cond_size, hidden_size)
        self.gru = nn.GRU(hidden_size, hidden_size, num_layers, batch_first=True)
        self.cross_attn = nn.MultiheadAttention(hidden_size, 8, batch_first=True)
        self.cross_norm = nn.LayerNorm(hidden_size)
        self.out_norm = nn.LayerNorm(hidden_size)
        self.out_proj = nn.Linear(hidden_size, vocab_size)

    def _build_context(
        self,
        conditioning: torch.Tensor,
        context_ids: torch.Tensor | None = None,
    ) -> torch.Tensor:
        cond = self.cond_proj(conditioning)
        if context_ids is None:
            return cond
        pos = torch.arange(context_ids.shape[1], device=context_ids.device).unsqueeze(0)
        ctx = self.ctx_embed(context_ids) + self.ctx_pos(pos)
        return torch.cat([cond, ctx], dim=1)

    def forward(
        self,
        target_ids: torch.Tensor,
        conditioning: torch.Tensor,
        context_ids: torch.Tensor | None = None,
    ) -> torch.Tensor:
        """
        target_ids:    (batch, seq_len) — ground truth answer bytes
        conditioning:  (batch, cond_seq, cond_dim) — stacked layer outputs
        context_ids:   (batch, ctx_len) — passage text as bytes (optional)

        Returns: (batch, seq_len, vocab_size) logits
        """
        B, S = target_ids.shape
        ctx = self._build_context(conditioning, context_ids)

        bos = torch.full((B, 1), self.vocab_size, dtype=torch.long, device=target_ids.device)
        input_ids = torch.cat([bos, target_ids[:, :-1]], dim=1)

        x = self.embed(input_ids)
        h0 = ctx.mean(dim=1).unsqueeze(0).expand(self.num_layers, -1, -1).contiguous()
        rnn_out, _ = self.gru(x, h0)

        attn_out = self.cross_attn(
            self.cross_norm(rnn_out), ctx, ctx, need_weights=False
        )[0]
        out = rnn_out + attn_out

        return self.out_proj(self.out_norm(out))

    @torch.no_grad()
    def generate(
        self,
        conditioning: torch.Tensor,
        context_ids: torch.Tensor | None = None,
        seq_len: int = 64,
        temperature: float = 0.0,
        **kwargs,
    ) -> torch.Tensor:
        B = conditioning.shape[0]
        device = conditioning.device
        ctx = self._build_context(conditioning, context_ids)
        h = ctx.mean(dim=1).unsqueeze(0).expand(self.num_layers, -1, -1).contiguous()

        token = torch.full((B, 1), self.vocab_size, dtype=torch.long, device=device)
        generated = []

        for _ in range(seq_len):
            x = self.embed(token)
            rnn_out, h = self.gru(x, h)
            attn_out = self.cross_attn(
                self.cross_norm(rnn_out), ctx, ctx, need_weights=False
            )[0]
            out = rnn_out + attn_out
            logits = self.out_proj(self.out_norm(out[:, -1]))

            if temperature > 0:
                probs = F.softmax(logits / temperature, dim=-1)
                token = torch.multinomial(probs, 1)
            else:
                token = logits.argmax(dim=-1, keepdim=True)
            generated.append(token)

        return torch.cat(generated, dim=1)
