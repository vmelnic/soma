"""
Liquid Time-Constant (LTC) network.

Each neuron's time constant is input-dependent — the network adapts its
dynamics per token. ODE integration over multiple sub-steps per input
gives richer computation per parameter than a single matrix multiply.

Based on: Hasani et al., "Liquid Time-constant Networks" (AAAI 2021).
"""

import torch
import torch.nn as nn


class LTCCell(nn.Module):
    """Single-step Liquid Time-Constant cell."""

    def __init__(self, input_size: int, hidden_size: int):
        super().__init__()
        self.hidden_size = hidden_size
        self.W_in = nn.Linear(input_size, hidden_size)
        self.W_rec = nn.Linear(hidden_size, hidden_size, bias=False)
        self.W_tau = nn.Linear(input_size + hidden_size, hidden_size)
        nn.init.zeros_(self.W_tau.bias)

    def forward(self, x: torch.Tensor, h: torch.Tensor, dt: float = 1.0) -> torch.Tensor:
        # alpha in (0, 1) — input-dependent mixing rate, always stable
        alpha = torch.sigmoid(self.W_tau(torch.cat([x, h], dim=-1)))
        f = torch.tanh(self.W_in(x) + self.W_rec(h))
        return (1 - alpha) * h + alpha * f


class LiquidLayer(nn.Module):
    """Process a sequence through an LTC cell with multi-step ODE integration."""

    def __init__(self, input_size: int, hidden_size: int, ode_steps: int = 4):
        super().__init__()
        self.cell = LTCCell(input_size, hidden_size)
        self.ode_steps = ode_steps
        self.norm = nn.LayerNorm(hidden_size)
        self.res_proj = nn.Linear(input_size, hidden_size, bias=False) if input_size != hidden_size else None

    def forward(
        self, x: torch.Tensor, h: torch.Tensor | None = None
    ) -> tuple[torch.Tensor, torch.Tensor]:
        batch, seq_len, _ = x.shape
        if h is None:
            h = torch.zeros(batch, self.cell.hidden_size, device=x.device)

        dt = 1.0 / self.ode_steps
        outputs = []
        for t in range(seq_len):
            x_t = x[:, t, :]
            for _ in range(self.ode_steps):
                h = self.cell(x_t, h, dt)
            outputs.append(h)

        out = torch.stack(outputs, dim=1)
        res = self.res_proj(x) if self.res_proj is not None else x
        return self.norm(res + out), h
