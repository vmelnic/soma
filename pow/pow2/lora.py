"""
LoRA — Low-Rank Adaptation for SOMA Experiential Memory.

Whitepaper Section 12.2: Experiential Memory (Hippocampus -> LoRA).

Base weights are FROZEN (permanent memory).
LoRA adds trainable low-rank matrices: W' = W + scale * B @ A.
The SOMA becomes a different neural structure after each experience.

Ref: Hu et al. (2021) "LoRA: Low-Rank Adaptation of Large Language Models"
"""

import torch
import torch.nn as nn
import torch.nn.functional as F


class LoRALinear(nn.Module):
    """Linear with frozen base + trainable LoRA: y = W_base(x) + scale*(x@A.T)@B.T"""

    def __init__(self, base: nn.Linear, rank: int = 4, alpha: float = 1.0):
        super().__init__()
        self.base = base
        self.base.weight.requires_grad_(False)
        if self.base.bias is not None:
            self.base.bias.requires_grad_(False)

        in_f, out_f = base.in_features, base.out_features
        self.rank = rank
        self.scale = alpha / rank
        self.lora_A = nn.Parameter(torch.randn(rank, in_f) * 0.01)
        self.lora_B = nn.Parameter(torch.zeros(out_f, rank))

    def forward(self, x):
        return self.base(x) + (x @ self.lora_A.T) @ self.lora_B.T * self.scale

    def merge(self):
        with torch.no_grad():
            self.base.weight.add_(self.scale * self.lora_B @ self.lora_A)
            self.lora_A.normal_(0, 0.01)
            self.lora_B.zero_()

    def lora_state(self):
        return {"lora_A": self.lora_A.data.clone(), "lora_B": self.lora_B.data.clone()}

    def load_lora_state(self, state):
        self.lora_A.data.copy_(state["lora_A"])
        self.lora_B.data.copy_(state["lora_B"])


class LoRAGRUCell(nn.Module):
    """GRUCell with frozen base + trainable LoRA on gate weight matrices.

    Properly reimplements GRU forward with LoRA deltas applied to
    the gate computations. No in-place weight modification.

    GRU equations:
      gi = x @ W_ih.T + b_ih        (input gates: r, z, n)
      gh = h @ W_hh.T + b_hh        (hidden gates: r, z, n)
      r = sigmoid(gi_r + gh_r)       (reset gate)
      z = sigmoid(gi_z + gh_z)       (update gate)
      n = tanh(gi_n + r * gh_n)      (new gate)
      h' = (1-z) * n + z * h

    LoRA adds delta to gate computations:
      gi += x @ (scale * B_ih @ A_ih).T
      gh += h @ (scale * B_hh @ A_hh).T
    """

    def __init__(self, base: nn.GRUCell, rank: int = 4, alpha: float = 1.0):
        super().__init__()
        self.input_size = base.input_size
        self.hidden_size = base.hidden_size
        self.scale = alpha / rank

        # Freeze base weights
        self.w_ih = base.weight_ih.detach().clone()
        self.w_hh = base.weight_hh.detach().clone()
        self.b_ih = base.bias_ih.detach().clone() if base.bias_ih is not None else None
        self.b_hh = base.bias_hh.detach().clone() if base.bias_hh is not None else None

        # Register as buffers (not parameters — frozen)
        self.register_buffer("base_w_ih", self.w_ih)
        self.register_buffer("base_w_hh", self.w_hh)
        if self.b_ih is not None:
            self.register_buffer("base_b_ih", self.b_ih)
            self.register_buffer("base_b_hh", self.b_hh)

        # LoRA on input-to-hidden
        ih_out, ih_in = self.w_ih.shape
        self.ih_A = nn.Parameter(torch.randn(rank, ih_in) * 0.01)
        self.ih_B = nn.Parameter(torch.zeros(ih_out, rank))

        # LoRA on hidden-to-hidden
        hh_out, hh_in = self.w_hh.shape
        self.hh_A = nn.Parameter(torch.randn(rank, hh_in) * 0.01)
        self.hh_B = nn.Parameter(torch.zeros(hh_out, rank))

    def forward(self, x, h):
        # Compute effective weights: base + LoRA delta
        w_ih = self.base_w_ih + self.scale * self.ih_B @ self.ih_A
        w_hh = self.base_w_hh + self.scale * self.hh_B @ self.hh_A

        # Gate computations
        gi = F.linear(x, w_ih, self.base_b_ih if self.b_ih is not None else None)
        gh = F.linear(h, w_hh, self.base_b_hh if self.b_hh is not None else None)

        # Split into 3 gates: reset, update, new
        gi_r, gi_z, gi_n = gi.chunk(3, dim=-1)
        gh_r, gh_z, gh_n = gh.chunk(3, dim=-1)

        r = torch.sigmoid(gi_r + gh_r)
        z = torch.sigmoid(gi_z + gh_z)
        n = torch.tanh(gi_n + r * gh_n)

        return (1 - z) * n + z * h

    def merge(self):
        with torch.no_grad():
            self.base_w_ih.add_(self.scale * self.ih_B @ self.ih_A)
            self.base_w_hh.add_(self.scale * self.hh_B @ self.hh_A)
            self.ih_A.normal_(0, 0.01); self.ih_B.zero_()
            self.hh_A.normal_(0, 0.01); self.hh_B.zero_()

    def lora_state(self):
        return {"ih_A": self.ih_A.data.clone(), "ih_B": self.ih_B.data.clone(),
                "hh_A": self.hh_A.data.clone(), "hh_B": self.hh_B.data.clone()}

    def load_lora_state(self, state):
        self.ih_A.data.copy_(state["ih_A"]); self.ih_B.data.copy_(state["ih_B"])
        self.hh_A.data.copy_(state["hh_A"]); self.hh_B.data.copy_(state["hh_B"])


def apply_lora(model, rank=4, alpha=1.0, target_modules=None):
    """Apply LoRA to Linear and GRUCell layers. Freeze base. Return lora dict."""
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
