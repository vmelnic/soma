"""
SDMStore — frozen MLP-derived associative memory (extracted from a transformer).
EpisodicSDM — growing (key, value) memory written during use.

Both are queried sparsely (top-k) by the LiquidCore.
"""

import torch
import torch.nn as nn
import torch.nn.functional as F


class SDMStore(nn.Module):
    """Frozen extracted MLPs as associative memory: (gate, up, down) per layer."""

    def __init__(self, gate_all, up_all, down_all):
        super().__init__()
        self.register_buffer("gate", gate_all, persistent=False)
        self.register_buffer("up", up_all, persistent=False)
        self.register_buffer("down", down_all, persistent=False)

    @property
    def num_layers(self):
        return self.gate.shape[0]

    def query(self, x, layer_idx, top_k):
        gate = self.gate[layer_idx]
        up = self.up[layer_idx]
        down = self.down[layer_idx]
        I = gate.shape[0]

        if top_k is None or top_k >= I:
            g = F.silu(x @ gate.T)
            u = x @ up.T
            return (g * u) @ down.T

        g_full = F.silu(x @ gate.T)
        idx = g_full.abs().topk(top_k, dim=-1).indices
        flat_idx = idx.reshape(-1, top_k)
        x_flat = x.reshape(-1, x.shape[-1])
        g_sel = g_full.gather(-1, idx).reshape(-1, top_k)
        u_sel = (up[flat_idx] * x_flat.unsqueeze(1)).sum(-1)
        gu = g_sel * u_sel
        out_flat = (down.T[flat_idx] * gu.unsqueeze(-1)).sum(1)
        return out_flat.view(x.shape)


class EpisodicSDM(nn.Module):
    """Growing key-value memory. Cosine similarity retrieval, FIFO eviction."""

    def __init__(self, d, max_entries=100_000, dtype=torch.bfloat16):
        super().__init__()
        self.d = d
        self.max_entries = max_entries
        self.dtype = dtype
        self.register_buffer("keys", torch.zeros(max_entries, d, dtype=dtype))
        self.register_buffer("values", torch.zeros(max_entries, d, dtype=dtype))
        self.register_buffer("count", torch.zeros((), dtype=torch.long))
        self.register_buffer("head", torch.zeros((), dtype=torch.long))

    def write(self, k, v):
        N = k.shape[0]
        if N == 0:
            return
        head = int(self.head.item())
        end = head + N
        if end <= self.max_entries:
            self.keys[head:end] = k.to(self.dtype)
            self.values[head:end] = v.to(self.dtype)
        else:
            split = self.max_entries - head
            self.keys[head:] = k[:split].to(self.dtype)
            self.values[head:] = v[:split].to(self.dtype)
            self.keys[:N - split] = k[split:].to(self.dtype)
            self.values[:N - split] = v[split:].to(self.dtype)
        self.head.fill_((head + N) % self.max_entries)
        self.count.fill_(min(int(self.count.item()) + N, self.max_entries))

    def query(self, x, top_k=8):
        n = int(self.count.item())
        if n == 0:
            return torch.zeros_like(x)
        keys = self.keys[:n]
        values = self.values[:n]
        x_norm = F.normalize(x.float(), dim=-1)
        k_norm = F.normalize(keys.float(), dim=-1)
        scores = x_norm @ k_norm.T
        k = min(top_k, n)
        topk = scores.topk(k, dim=-1)
        weights = F.softmax(topk.values, dim=-1)
        idx = topk.indices
        retrieved = values[idx]
        return (retrieved.float() * weights.unsqueeze(-1)).sum(dim=-2).to(x.dtype)
