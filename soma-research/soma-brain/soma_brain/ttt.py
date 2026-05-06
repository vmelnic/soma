"""
Test-Time Training (TTT) memory layer.

The hidden state IS a set of neural network weights that update via
gradient descent during inference. Each input token produces a
self-supervised loss; one gradient step updates the inner model. The
model literally learns from what it reads.

Based on: Sun et al., "Learning to (Learn at Test Time)" (2024).
"""

import torch
import torch.nn as nn
import torch.nn.functional as F


class TTTLayer(nn.Module):

    def __init__(self, hidden_size: int, inner_size: int, lr: float = 0.01):
        super().__init__()
        self.hidden_size = hidden_size
        self.inner_size = inner_size
        self.lr = lr

        self.W_init = nn.Parameter(torch.randn(hidden_size, inner_size) * 0.02)
        self.b_init = nn.Parameter(torch.zeros(inner_size))

        self.target_proj = nn.Linear(hidden_size, inner_size)
        self.out_proj = nn.Linear(inner_size, hidden_size)
        self.gate = nn.Linear(hidden_size * 2, hidden_size)
        self.norm = nn.LayerNorm(hidden_size)

        self._session_W: torch.Tensor | None = None
        self._session_b: torch.Tensor | None = None
        self._session_inputs: list[torch.Tensor] = []

    def forward(self, x: torch.Tensor, update: bool = True) -> torch.Tensor:
        batch, seq_len, d = x.shape

        W = self.W_init.unsqueeze(0).expand(batch, -1, -1).clone()
        b = self.b_init.unsqueeze(0).expand(batch, -1).clone()

        outputs = []
        for t in range(seq_len):
            x_t = x[:, t, :]
            inner_out = torch.bmm(x_t.unsqueeze(1), W).squeeze(1) + b
            ttt_out = self.out_proj(F.silu(inner_out))

            g = torch.sigmoid(self.gate(torch.cat([x_t, ttt_out], dim=-1)))
            outputs.append(self.norm(g * ttt_out + (1 - g) * x_t))

            if update:
                target = self.target_proj(x_t).detach()
                error = inner_out - target
                d_W = torch.bmm(x_t.unsqueeze(2), error.unsqueeze(1)) / self.inner_size
                d_b = error / self.inner_size
                W = W - self.lr * d_W
                b = b - self.lr * d_b
                self._session_inputs.append(x_t.detach())

        self._session_W = W.detach()
        self._session_b = b.detach()

        return torch.stack(outputs, dim=1)

    @torch.no_grad()
    def get_session_delta(self) -> torch.Tensor | None:
        """Return the weight delta from this session as a flat vector.

        The delta captures what TTT learned during inference — patterns
        not yet in long-term memory. Returns None if no session ran.
        """
        if self._session_W is None:
            return None
        dW = self._session_W - self.W_init.unsqueeze(0)
        db = self._session_b - self.b_init.unsqueeze(0)
        return torch.cat([dW.flatten(1), db], dim=-1)

    @torch.no_grad()
    def get_session_inputs(self) -> list[torch.Tensor]:
        """Return inputs seen during the session (for consolidation context)."""
        return self._session_inputs

    def reset_state(self) -> None:
        """Clear session state for a fresh session."""
        self._session_W = None
        self._session_b = None
        self._session_inputs.clear()
