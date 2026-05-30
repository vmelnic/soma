"""
LTC Controller — tiny ODE-based network that sequences operations.

The ONLY trainable component. Learns control flow: when to retrieve from SDM,
which HDC operations to apply, when to emit a result. Does NOT store knowledge
or perform composition — only decides the next operation.

Based on: Hasani et al., "Liquid Time-constant Networks" (AAAI 2021).
"""

import torch
import torch.nn as nn
import torch.nn.functional as F


class LTCCell(nn.Module):
    """Single-step Liquid Time-Constant cell with input-dependent time constants."""

    def __init__(self, input_size: int, hidden_size: int):
        super().__init__()
        self.hidden_size = hidden_size
        self.W_in = nn.Linear(input_size, hidden_size)
        self.W_rec = nn.Linear(hidden_size, hidden_size, bias=False)
        self.W_tau = nn.Linear(input_size + hidden_size, hidden_size)
        nn.init.zeros_(self.W_tau.bias)

    def forward(self, x: torch.Tensor, h: torch.Tensor) -> torch.Tensor:
        alpha = torch.sigmoid(self.W_tau(torch.cat([x, h], dim=-1)))
        f = torch.tanh(self.W_in(x) + self.W_rec(h))
        return (1 - alpha) * h + alpha * f


class Operation:
    """An operation the controller can emit."""
    SDM_READ = 0
    HDC_BIND = 1
    HDC_UNBIND = 2
    HDC_BUNDLE = 3
    HDC_ENCODE_SEQ = 4
    HDC_ENCODE_RECORD = 5
    HDC_QUERY_RECORD = 6
    HDC_BEST_MATCH = 7
    EMIT = 8
    NUM_OPS = 9


class Controller(nn.Module):
    """
    LTC-based controller that decides which operation to perform next.

    Input: current state vector (from SDM read, HDC result, or initial encoding)
    Output: operation selection + operation argument vector
    """

    def __init__(self, state_size: int, hidden_size: int = 128, ode_steps: int = 4):
        super().__init__()
        self.state_size = state_size
        self.hidden_size = hidden_size
        self.ode_steps = ode_steps

        self.cell = LTCCell(state_size, hidden_size)
        self.op_head = nn.Linear(hidden_size, Operation.NUM_OPS)
        self.arg_head = nn.Linear(hidden_size, state_size)

    def forward(
        self, state: torch.Tensor, hidden: torch.Tensor | None = None
    ) -> tuple[torch.Tensor, torch.Tensor, torch.Tensor]:
        """
        One step of the controller.

        Args:
            state: current state vector [batch, state_size]
            hidden: LTC hidden state [batch, hidden_size]

        Returns:
            op_logits: [batch, NUM_OPS] — which operation to perform
            arg_vec: [batch, state_size] — argument/query for the operation
            new_hidden: [batch, hidden_size] — updated hidden state
        """
        if hidden is None:
            hidden = torch.zeros(state.shape[0], self.hidden_size, device=state.device)

        h = hidden
        for _ in range(self.ode_steps):
            h = self.cell(state, h)

        op_logits = self.op_head(h)
        arg_vec = self.arg_head(h)

        return op_logits, arg_vec, h

    def select_op(self, op_logits: torch.Tensor, temperature: float = 1.0) -> int:
        """Select an operation from logits (greedy or sampled)."""
        if temperature <= 0:
            return int(op_logits.argmax(dim=-1).item())
        probs = F.softmax(op_logits / temperature, dim=-1)
        return int(torch.multinomial(probs, 1).item())

    def param_count(self) -> int:
        return sum(p.numel() for p in self.parameters())
