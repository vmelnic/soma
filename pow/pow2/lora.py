"""
LoRA — Low-Rank Adaptation for SOMA Experiential Memory.

Implements Whitepaper Section 12.2:
  Experiential Memory (Hippocampus -> LoRA Adaptation Layers)

The base model weights (from synthesis) are FROZEN — permanent memory.
LoRA adds small trainable matrices (A, B) on top: W' = W + B @ A.
Only LoRA parameters update during experience. The SOMA literally
becomes a slightly different neural structure after each experience.

Reference: Hu et al. (2021) "LoRA: Low-Rank Adaptation of Large Language Models"
"""

import torch
import torch.nn as nn


class LoRALinear(nn.Module):
    """A Linear layer with frozen base weights + trainable LoRA adaptation.

    Forward: y = W_frozen @ x + (B @ A) @ x
    Only A and B are trainable. Base W is permanent memory.
    """

    def __init__(self, base: nn.Linear, rank: int = 4, alpha: float = 1.0):
        super().__init__()
        self.base = base
        self.base.weight.requires_grad_(False)
        if self.base.bias is not None:
            self.base.bias.requires_grad_(False)

        in_f = base.in_features
        out_f = base.out_features
        self.rank = rank
        self.scale = alpha / rank

        # LoRA matrices: W' = W + scale * B @ A
        # A initialized with small random, B initialized to zero
        # So initially LoRA has no effect (B @ A = 0)
        self.lora_A = nn.Parameter(torch.randn(rank, in_f) * 0.01)
        self.lora_B = nn.Parameter(torch.zeros(out_f, rank))

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        base_out = self.base(x)
        # LoRA adaptation: low-rank delta
        lora_out = (x @ self.lora_A.T) @ self.lora_B.T * self.scale
        return base_out + lora_out

    def merge(self):
        """Consolidation: merge LoRA into base weights (sleep cycle).
        After merge, LoRA is reset to zero — ready for new learning."""
        with torch.no_grad():
            self.base.weight.add_(self.scale * self.lora_B @ self.lora_A)
            self.lora_A.zero_()
            self.lora_B.zero_()
            # Re-initialize A with small random for next learning cycle
            self.lora_A.normal_(0, 0.01)

    def lora_state(self) -> dict:
        """Serialize LoRA state (for checkpointing)."""
        return {
            "A": self.lora_A.data.clone(),
            "B": self.lora_B.data.clone(),
        }

    def load_lora_state(self, state: dict):
        """Restore LoRA state (from checkpoint)."""
        self.lora_A.data.copy_(state["A"])
        self.lora_B.data.copy_(state["B"])


class LoRAGRUCell(nn.Module):
    """A GRUCell with frozen base weights + trainable LoRA on gate matrices.

    GRU has two weight matrices:
      weight_ih: (3*hidden, input)  — input-to-gates [r, z, n]
      weight_hh: (3*hidden, hidden) — hidden-to-gates [r, z, n]

    LoRA adapts BOTH, giving the decoder the ability to change
    how it processes sequences — not just output heads.
    """

    def __init__(self, base: nn.GRUCell, rank: int = 4, alpha: float = 1.0):
        super().__init__()
        self.base = base
        self.base.weight_ih.requires_grad_(False)
        self.base.weight_hh.requires_grad_(False)
        if self.base.bias_ih is not None:
            self.base.bias_ih.requires_grad_(False)
        if self.base.bias_hh is not None:
            self.base.bias_hh.requires_grad_(False)

        self.scale = alpha / rank

        # LoRA on input-to-hidden
        ih_out, ih_in = base.weight_ih.shape
        self.ih_A = nn.Parameter(torch.randn(rank, ih_in) * 0.01)
        self.ih_B = nn.Parameter(torch.zeros(ih_out, rank))

        # LoRA on hidden-to-hidden
        hh_out, hh_in = base.weight_hh.shape
        self.hh_A = nn.Parameter(torch.randn(rank, hh_in) * 0.01)
        self.hh_B = nn.Parameter(torch.zeros(hh_out, rank))

    def forward(self, x: torch.Tensor, h: torch.Tensor) -> torch.Tensor:
        # Base GRU computation + LoRA delta on gate computations
        # GRU: gates = x @ W_ih.T + h @ W_hh.T + bias
        # LoRA adds: x @ (B_ih @ A_ih).T + h @ (B_hh @ A_hh).T
        #
        # We apply LoRA by temporarily modifying the weights, running
        # the base GRU, then restoring. This is cleaner than reimplementing GRU.
        with torch.no_grad():
            ih_delta = self.scale * self.ih_B @ self.ih_A
            hh_delta = self.scale * self.hh_B @ self.hh_A
            self.base.weight_ih.add_(ih_delta)
            self.base.weight_hh.add_(hh_delta)

        result = self.base(x, h)

        with torch.no_grad():
            self.base.weight_ih.sub_(ih_delta)
            self.base.weight_hh.sub_(hh_delta)

        return result

    def merge(self):
        with torch.no_grad():
            self.base.weight_ih.add_(self.scale * self.ih_B @ self.ih_A)
            self.base.weight_hh.add_(self.scale * self.hh_B @ self.hh_A)
            for p in [self.ih_A, self.ih_B, self.hh_A, self.hh_B]:
                p.zero_()
            self.ih_A.normal_(0, 0.01)
            self.hh_A.normal_(0, 0.01)

    def lora_state(self):
        return {"ih_A": self.ih_A.data.clone(), "ih_B": self.ih_B.data.clone(),
                "hh_A": self.hh_A.data.clone(), "hh_B": self.hh_B.data.clone()}

    def load_lora_state(self, state):
        self.ih_A.data.copy_(state["ih_A"])
        self.ih_B.data.copy_(state["ih_B"])
        self.hh_A.data.copy_(state["hh_A"])
        self.hh_B.data.copy_(state["hh_B"])


def apply_lora(model: nn.Module, rank: int = 4, alpha: float = 1.0,
               target_modules: list[str] | None = None):
    """Apply LoRA adapters to Linear layers AND GRUCell.

    Freezes all base parameters. Only LoRA A/B matrices are trainable.
    Returns (lora_layers, trainable_count, total_count).
    """
    lora_layers = {}

    for param in model.parameters():
        param.requires_grad_(False)

    for name, module in list(model.named_modules()):
        if target_modules is not None and name not in target_modules:
            continue

        if isinstance(module, nn.Linear):
            lora = LoRALinear(module, rank=rank, alpha=alpha)
        elif isinstance(module, nn.GRUCell):
            lora = LoRAGRUCell(module, rank=rank, alpha=alpha)
        else:
            continue

        lora_layers[name] = lora

        parts = name.split(".")
        parent = model
        for part in parts[:-1]:
            parent = getattr(parent, part)
        setattr(parent, parts[-1], lora)

    trainable = sum(p.numel() for p in model.parameters() if p.requires_grad)
    total = sum(p.numel() for p in model.parameters())

    return lora_layers, trainable, total
