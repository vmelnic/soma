"""
Bridge — small trainable module that connects Qwen-3B's hidden states
to the bigger SDM (extracted from Qwen-32B).

At a chosen layer N of the small model:
  hidden state h ∈ R^{D_small}
    → project to D_big with bridge.proj_in
    → query SDM (sparse top-k MLP lookup at SDM layer L)
    → project back to D_small with bridge.proj_out
    → blend (gated residual) into h

Only the bridge is trainable. Qwen frozen, SDM frozen.
"""

import torch
import torch.nn as nn
import torch.nn.functional as F


class Bridge(nn.Module):
    def __init__(self, d_small, d_big, sdm_path=None,
                 inject_layers=None, top_k=128, n_layers_small=36,
                 dtype=torch.float16):
        """
        d_small: hidden_size of the small chat model (Qwen-3B = 2048)
        d_big:   hidden_size of the SDM source (Qwen-32B = 5120)
        inject_layers: list of small-model layer indices where we
            inject SDM augmentation (e.g., [12, 18, 24] for Qwen-3B's 36)
        top_k: sparse SDM top-k per query
        """
        super().__init__()
        self.d_small = d_small
        self.d_big = d_big
        self.top_k = top_k
        self.inject_layers = inject_layers or []
        self.n_layers_small = n_layers_small

        # One projection pair per injection point (each can specialize).
        self.proj_in = nn.ModuleList([
            nn.Linear(d_small, d_big, bias=False)
            for _ in self.inject_layers
        ])
        self.proj_out = nn.ModuleList([
            nn.Linear(d_big, d_small, bias=False)
            for _ in self.inject_layers
        ])
        # Per-layer learnable gate (starts near 0 so init = unmodified Qwen)
        self.gate = nn.ParameterList([
            nn.Parameter(torch.zeros(d_small))
            for _ in self.inject_layers
        ])
        # Norm before the cross-domain projection
        self.norm = nn.ModuleList([
            nn.LayerNorm(d_small)
            for _ in self.inject_layers
        ])

        # Init projections small (random would dwarf Qwen's hidden states)
        for m in self.proj_in:
            nn.init.normal_(m.weight, std=0.02)
        for m in self.proj_out:
            nn.init.normal_(m.weight, std=0.02)

        # SDM tensors loaded lazily from disk
        self.sdm_path = sdm_path
        self._sdm = None
        self._sdm_layers_total = None
        self.dtype = dtype

    def load_sdm(self, device):
        """Memory-map the SDM file. Stays on disk; pages in on access."""
        if self._sdm is not None:
            return
        if self.sdm_path is None:
            raise ValueError("sdm_path not set")
        sdm = torch.load(self.sdm_path, map_location="cpu",
                         mmap=True, weights_only=False)
        self._sdm = sdm
        self._sdm_layers_total = sdm["gate_all"].shape[0]
        # Move SDM to GPU only if it fits; otherwise keep on CPU and copy
        # per-query. For 32B SDM (~57GB) we keep on CPU.
        # The per-layer slice (~900MB) is small enough to copy per query.

    def query_sdm(self, x_big, sdm_layer):
        """Sparse top-k MLP lookup at the given SDM layer."""
        gate = self._sdm["gate_all"][sdm_layer].to(x_big.device, x_big.dtype)
        up = self._sdm["up_all"][sdm_layer].to(x_big.device, x_big.dtype)
        down = self._sdm["down_all"][sdm_layer].to(x_big.device, x_big.dtype)
        I = gate.shape[0]

        if self.top_k is None or self.top_k >= I:
            g = F.silu(x_big @ gate.T)
            u = x_big @ up.T
            return (g * u) @ down.T

        g_full = F.silu(x_big @ gate.T)
        idx = g_full.abs().topk(self.top_k, dim=-1).indices
        flat_idx = idx.reshape(-1, self.top_k)
        x_flat = x_big.reshape(-1, self.d_big)
        g_sel = g_full.gather(-1, idx).reshape(-1, self.top_k)
        u_sel = (up[flat_idx] * x_flat.unsqueeze(1)).sum(-1)
        gu = g_sel * u_sel
        out_flat = (down.T[flat_idx] * gu.unsqueeze(-1)).sum(1)
        return out_flat.view(x_big.shape)

    def augment(self, h, layer_idx):
        """Apply bridge augmentation to hidden state h at small-model layer_idx."""
        if layer_idx not in self.inject_layers:
            return h
        i = self.inject_layers.index(layer_idx)
        sdm_layer = min(
            int(layer_idx * self._sdm_layers_total / self.n_layers_small),
            self._sdm_layers_total - 1,
        )

        x = self.norm[i](h)
        x_big = self.proj_in[i](x)
        retrieved_big = self.query_sdm(x_big, sdm_layer)
        retrieved_small = self.proj_out[i](retrieved_big)
        gated = retrieved_small * torch.tanh(self.gate[i])
        return h + gated


class GraftedQwen(nn.Module):
    """Frozen Qwen + Bridge-augmented forward at chosen layers.

    Hooks into the source Qwen model's forward; at injection points,
    runs the bridge on the residual-stream hidden state.
    """

    def __init__(self, qwen_model, bridge):
        super().__init__()
        self.qwen = qwen_model
        for p in self.qwen.parameters():
            p.requires_grad = False
        self.bridge = bridge
        self._hooks = []

    def attach_hooks(self):
        """Register forward hooks at the bridge's injection layers."""
        self.detach_hooks()
        for layer_idx in self.bridge.inject_layers:
            block = self.qwen.model.layers[layer_idx]
            def make_hook(li):
                def hook(module, inp, out):
                    h = out[0] if isinstance(out, tuple) else out
                    h_aug = self.bridge.augment(h, li)
                    if isinstance(out, tuple):
                        return (h_aug,) + out[1:]
                    return h_aug
                return hook
            self._hooks.append(block.register_forward_hook(make_hook(layer_idx)))

    def detach_hooks(self):
        for h in self._hooks:
            h.remove()
        self._hooks = []

    def forward(self, *args, **kwargs):
        return self.qwen(*args, **kwargs)
