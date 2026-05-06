"""
Standalone LiquidCore: CfC sequence + depth recurrence + sparse SDM queries.

NO transformer at runtime. Token in, logits out. Knowledge in static SDM
(extracted Qwen MLPs); the small CfC core navigates it.
"""

import torch
import torch.nn as nn
import torch.nn.functional as F


def rms_norm(x, weight, eps):
    in_dtype = x.dtype
    x32 = x.to(torch.float32)
    var = x32.pow(2).mean(dim=-1, keepdim=True)
    x32 = x32 * torch.rsqrt(var + eps)
    return x32.to(in_dtype) * weight


class CfCCell(nn.Module):
    """Closed-form continuous-time cell (Hasani 2022). No attention.

    h_new = rate * update + (1 - rate) * (h + target)
    rate, update, target are learned functions of (h, x).
    Adaptive time constant via the sigmoid gate.
    """

    def __init__(self, d, hidden=None):
        super().__init__()
        h = hidden or d
        self.f = nn.Sequential(nn.Linear(d * 2, h), nn.SiLU(), nn.Linear(h, d))
        self.g = nn.Sequential(nn.Linear(d * 2, h), nn.SiLU(), nn.Linear(h, d))
        self.a = nn.Sequential(nn.Linear(d * 2, h), nn.SiLU(), nn.Linear(h, d))
        for layer in (self.f[-1], self.g[-1], self.a[-1]):
            nn.init.zeros_(layer.weight)
            nn.init.zeros_(layer.bias)

    def forward(self, h, x):
        z = torch.cat([h, x], dim=-1)
        rate = torch.sigmoid(-self.f(z))
        target = self.a(z)
        update = self.g(z)
        return rate * update + (1.0 - rate) * (h + target)


class LiquidCore(nn.Module):
    """Standalone reasoning core.

    Inference path:
      input_ids → embed → CfC sweep over T (recurrent) → depth integration
      (SDM query → CfC step) × n_steps → final norm → logits.

    Trainable: CfC cell + query_norm.
    Frozen: embed_tokens, lm_head, final_norm (borrowed from extraction).
    Frozen substrate: SDM (gate, up, down).
    """

    def __init__(self, d, vocab_size, sdm, n_steps=8, eps=1e-6,
                 top_k=128, tie_embeddings=True):
        super().__init__()
        self.d = d
        self.n_steps = n_steps
        self.eps = eps
        self.top_k = top_k
        self.tie = tie_embeddings

        self.embed_tokens = nn.Parameter(torch.empty(vocab_size, d))
        self.final_norm = nn.Parameter(torch.ones(d))
        if not tie_embeddings:
            self.lm_head = nn.Parameter(torch.empty(vocab_size, d))
        else:
            self.lm_head = None

        self.cell = CfCCell(d)
        self.query_norm = nn.Parameter(torch.ones(d))
        self.sdm = sdm

        nn.init.normal_(self.embed_tokens, std=0.02)
        if not tie_embeddings:
            nn.init.normal_(self.lm_head, std=0.02)

    def forward(self, input_ids, top_k=None, return_hidden=False):
        B, T = input_ids.shape
        D = self.d
        k = top_k or self.top_k

        x = F.embedding(input_ids, self.embed_tokens)

        # Sequence-recurrent CfC sweep (causal)
        h_t = torch.zeros(B, D, device=x.device, dtype=x.dtype)
        states = []
        for t in range(T):
            h_t = self.cell(h_t.unsqueeze(1), x[:, t:t+1, :]).squeeze(1)
            states.append(h_t)
        state = torch.stack(states, dim=1)

        # Depth integration with SDM queries
        L = self.sdm.num_layers
        for step_idx in range(self.n_steps):
            sdm_layer = int(step_idx * L / self.n_steps)
            q = rms_norm(state, self.query_norm, self.eps)
            retrieval = self.sdm.query(q, sdm_layer, k)
            state = self.cell(state, retrieval)

        state = rms_norm(state, self.final_norm, self.eps)
        weight = self.embed_tokens if self.tie else self.lm_head
        logits = state @ weight.T
        if return_hidden:
            return logits, state
        return logits

    def count_params(self, trainable_only=True):
        return sum(p.numel() for p in self.parameters()
                   if (not trainable_only) or p.requires_grad)


def bootstrap_from_extraction(extracted, n_steps=8, top_k=128, dtype=torch.bfloat16):
    """Build a LiquidCore from extract.py output.

    Borrowed (FROZEN — knowledge): embed, final_norm, lm_head.
    Substrate (FROZEN — knowledge): static SDM from extracted MLPs.
    Trainable: CfC cell weights + query_norm.
    """
    from .sdm import SDMStore

    cfg = extracted["config"]
    H = cfg["hidden_size"]

    gate_all = torch.stack([w["gate"] for w in extracted["layers"]]).to(dtype)
    up_all = torch.stack([w["up"] for w in extracted["layers"]]).to(dtype)
    down_all = torch.stack([w["down"] for w in extracted["layers"]]).to(dtype)
    sdm = SDMStore(gate_all, up_all, down_all)

    core = LiquidCore(
        d=H,
        vocab_size=cfg["vocab_size"],
        sdm=sdm,
        n_steps=n_steps,
        eps=cfg["rms_norm_eps"],
        top_k=top_k,
        tie_embeddings=cfg["tie_word_embeddings"],
    )

    # Borrow knowledge components — FREEZE them
    core.embed_tokens.data.copy_(extracted["embed_tokens"].to(dtype))
    core.embed_tokens.requires_grad = False
    core.final_norm.data.copy_(extracted["final_norm"].to(dtype))
    core.final_norm.requires_grad = False
    if core.lm_head is not None:
        core.lm_head.data.copy_(extracted["lm_head"].to(dtype))
        core.lm_head.requires_grad = False

    return core.to(dtype)
