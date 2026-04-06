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


def apply_lora(model: nn.Module, rank: int = 4, alpha: float = 1.0,
               target_modules: list[str] | None = None) -> dict[str, LoRALinear]:
    """Apply LoRA adapters to a model's Linear layers.

    Freezes all base parameters. Only LoRA A/B matrices are trainable.
    Returns dict of {name: LoRALinear} for checkpoint/restore.

    Args:
        model: The base model (from synthesis)
        rank: LoRA rank (lower = fewer params, less capacity)
        target_modules: List of attribute names to adapt. If None, adapts all Linear layers.
    """
    lora_layers = {}

    # Freeze everything first (permanent memory)
    for param in model.parameters():
        param.requires_grad_(False)

    # Find and wrap target Linear layers with LoRA
    for name, module in list(model.named_modules()):
        if not isinstance(module, nn.Linear):
            continue
        if target_modules is not None and name not in target_modules:
            continue

        # Replace the Linear with LoRALinear
        lora = LoRALinear(module, rank=rank, alpha=alpha)
        lora_layers[name] = lora

        # Set the LoRA module on the parent
        parts = name.split(".")
        parent = model
        for part in parts[:-1]:
            parent = getattr(parent, part)
        setattr(parent, parts[-1], lora)

    trainable = sum(p.numel() for p in model.parameters() if p.requires_grad)
    total = sum(p.numel() for p in model.parameters())

    return lora_layers, trainable, total
