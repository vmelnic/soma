"""
Predictive coding loss — the brain's training objective.

Bidirectional prediction across the layer hierarchy:
  - Top-down: higher layers predict lower layers' activity
  - Bottom-up: lower layers predict higher layers' activity

Prediction error = free energy. Minimizing it forces the reasoning
blocks to produce representations that are coherent across the
hierarchy in both directions.

Gradient flow:
  - Source layer (input to predictor): NOT detached. Gradient flows
    back to the reasoning block, teaching it to produce predictable
    (low-surprise) representations.
  - Target layer (prediction target): detached. The target is fixed —
    you learn to predict it, not to move it.

Better SDM retrieval → more coherent layer activations → lower
prediction error. The core learns to retrieve well because retrieval
quality directly reduces free energy.

Based on: Friston, "A free energy principle for the brain" (2006);
Rao & Ballard, "Predictive coding in the visual cortex" (1999).
"""

import torch
import torch.nn as nn
import torch.nn.functional as F


class PredictiveCodingLoss(nn.Module):

    def __init__(self, hidden_size: int, num_layers: int):
        super().__init__()
        self.top_down = nn.ModuleList(
            [nn.Linear(hidden_size, hidden_size) for _ in range(num_layers)]
        )
        self.bottom_up = nn.ModuleList(
            [nn.Linear(hidden_size, hidden_size) for _ in range(num_layers)]
        )

    def forward(self, layer_outputs: list[torch.Tensor]) -> tuple[torch.Tensor, torch.Tensor]:
        """
        layer_outputs[0] = input projection
        layer_outputs[i+1] = output of reasoning block i

        Returns (top_down_fe, bottom_up_fe) — both scalar free energies.
        """
        td_total = torch.tensor(0.0, device=layer_outputs[0].device)
        bu_total = torch.tensor(0.0, device=layer_outputs[0].device)
        n = len(self.top_down)

        for i in range(n):
            # Top-down: layer i+1 predicts layer i
            td_pred = self.top_down[i](layer_outputs[i + 1])
            td_target = layer_outputs[i].detach()
            td_total = td_total + F.mse_loss(td_pred, td_target)

            # Bottom-up: layer i predicts layer i+1
            bu_pred = self.bottom_up[i](layer_outputs[i])
            bu_target = layer_outputs[i + 1].detach()
            bu_total = bu_total + F.mse_loss(bu_pred, bu_target)

        return td_total / n, bu_total / n
