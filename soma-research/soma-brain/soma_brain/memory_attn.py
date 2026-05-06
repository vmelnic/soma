"""
Memory attention — cross-attention from liquid core state to SDM entries.

NOT self-attention. Tokens never attend to other tokens through this module.
The liquid core's ODE dynamics handle token-to-token interaction. This module
handles core-to-memory interaction: the reasoning core reads from its
knowledge store.

Architecturally identical to a DNC read head or Memory Network output module.
Cost: O(seq_len * k) where k = number of retrieved SDM entries (small, fixed).
"""

import torch
import torch.nn as nn
import torch.nn.functional as F


class MemoryAttention(nn.Module):
    """Cross-attention: core state (query) attends to SDM entries (key/value)."""

    def __init__(self, hidden_size: int, num_heads: int = 4):
        super().__init__()
        self.num_heads = num_heads
        self.head_dim = hidden_size // num_heads
        assert hidden_size % num_heads == 0

        self.q_proj = nn.Linear(hidden_size, hidden_size)
        self.k_proj = nn.Linear(hidden_size, hidden_size)
        self.v_proj = nn.Linear(hidden_size, hidden_size)
        self.out_proj = nn.Linear(hidden_size, hidden_size)

    def forward(self, state: torch.Tensor, memories: torch.Tensor) -> torch.Tensor:
        """
        state:    (batch, seq_len, hidden) — liquid core output
        memories: (batch, k, hidden)       — retrieved SDM entries
        returns:  (batch, seq_len, hidden) — memory-informed state
        """
        B, S, D = state.shape
        K = memories.shape[1]
        h, d = self.num_heads, self.head_dim

        q = self.q_proj(state).view(B, S, h, d).transpose(1, 2)
        k = self.k_proj(memories).view(B, K, h, d).transpose(1, 2)
        v = self.v_proj(memories).view(B, K, h, d).transpose(1, 2)

        # (batch, heads, seq_len, k) — each position reads from k memories
        scores = torch.matmul(q, k.transpose(-2, -1)) / (d ** 0.5)
        attn = F.softmax(scores, dim=-1)
        out = torch.matmul(attn, v)

        return self.out_proj(out.transpose(1, 2).reshape(B, S, D))


class ReasoningBlock(nn.Module):
    """One hop: liquid dynamics → SDM retrieval → memory attention.

    Each block processes the sequence with ODE dynamics, then queries SDM
    for relevant knowledge, then lets the core read from retrieved memories.
    Stacking N blocks gives N hops of retrieval refinement.
    """

    def __init__(self, hidden_size: int, ode_steps: int, num_heads: int):
        super().__init__()
        from .liquid import LiquidLayer

        self.liquid = LiquidLayer(hidden_size, hidden_size, ode_steps)
        self.mem_attn = MemoryAttention(hidden_size, num_heads)
        self.gate = nn.Linear(hidden_size * 2, hidden_size)
        nn.init.zeros_(self.gate.weight)
        nn.init.constant_(self.gate.bias, -3.0)
        self.norm = nn.LayerNorm(hidden_size)

    def forward(
        self, x: torch.Tensor, sdm, h: torch.Tensor | None = None
    ) -> tuple[torch.Tensor, torch.Tensor]:
        x_liquid, h = self.liquid(x, h)

        query = x_liquid.mean(dim=1)
        memories, _scores = sdm.read_topk(query)

        mem_out = self.mem_attn(x_liquid, memories)

        g = torch.sigmoid(self.gate(torch.cat([x_liquid, mem_out], dim=-1)))
        out = self.norm(x + g * mem_out)

        return out, h
