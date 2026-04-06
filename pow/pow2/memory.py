"""
SOMA Experiential Memory + Checkpoint/Restore — POW 2.

Implements Whitepaper Sections 9 and 12:
  - Section 12.2: LoRA as experiential memory (hippocampus)
  - Section 12.3: Consolidation ("sleep") -- merge LoRA into base
  - Section 9.3: CRIU-inspired mind checkpointing
"""

import random
from datetime import datetime

import torch
import torch.nn as nn

from pow.pow2.lora import LoRALinear


class ExperienceBuffer:
    """Stores recent (input, target) pairs from successful executions."""

    def __init__(self, max_size: int = 200):
        self.buffer = []
        self.max_size = max_size
        self.total_seen = 0

    def add(self, token_ids, length, target_opcodes,
            target_a0_types, target_a1_types,
            target_a0_spans_s, target_a0_spans_e,
            target_a1_spans_s, target_a1_spans_e,
            target_a0_refs, target_a1_refs):
        self.buffer.append({
            "token_ids": token_ids, "length": length,
            "opcode": target_opcodes,
            "a0_type": target_a0_types, "a1_type": target_a1_types,
            "a0_span_s": target_a0_spans_s, "a0_span_e": target_a0_spans_e,
            "a1_span_s": target_a1_spans_s, "a1_span_e": target_a1_spans_e,
            "a0_ref": target_a0_refs, "a1_ref": target_a1_refs,
        })
        self.total_seen += 1
        if len(self.buffer) > self.max_size:
            self.buffer.pop(0)

    def sample(self, n):
        if len(self.buffer) <= n:
            return list(self.buffer)
        return random.sample(self.buffer, n)

    def __len__(self):
        return len(self.buffer)


def _mce(logits, targets, ign=-1):
    m = targets != ign
    if not m.any():
        return torch.tensor(0.0, device=logits.device)
    return nn.functional.cross_entropy(logits[m], targets[m])


def adapt_from_experience(model, batch, lr=5e-4):
    """One adaptation step on a batch of experiences.
    Only LoRA parameters update. Base weights stay frozen.
    This is hippocampal learning (Section 12.2)."""
    if not batch:
        return 0.0

    ml = max(e["length"] for e in batch)
    padded = [e["token_ids"] + [0] * (ml - e["length"]) for e in batch]
    device = next(model.parameters()).device

    ids = torch.tensor(padded, dtype=torch.long, device=device)
    lens = torch.tensor([e["length"] for e in batch], dtype=torch.long, device=device)
    tgts = {}
    for key in ["opcode", "a0_type", "a1_type", "a0_span_s", "a0_span_e",
                "a1_span_s", "a1_span_e", "a0_ref", "a1_ref"]:
        tgts[key] = torch.tensor([e[key] for e in batch], dtype=torch.long, device=device)

    tgt_ops = tgts["opcode"]
    B, S = tgt_ops.shape

    model.train()
    out = model(ids, lens, tgt_ops)

    loss = nn.functional.cross_entropy(out["op"].reshape(B*S, -1), tgt_ops.reshape(B*S))
    loss = loss + nn.functional.cross_entropy(
        out["a0t"].reshape(B*S, -1), tgts["a0_type"].reshape(B*S))
    loss = loss + nn.functional.cross_entropy(
        out["a1t"].reshape(B*S, -1), tgts["a1_type"].reshape(B*S))
    for ks, ke, ts, te in [("s0s","s0e","a0_span_s","a0_span_e"),
                            ("s1s","s1e","a1_span_s","a1_span_e")]:
        loss = loss + _mce(out[ks].reshape(B*S,-1), tgts[ts].reshape(B*S))
        loss = loss + _mce(out[ke].reshape(B*S,-1), tgts[te].reshape(B*S))
    for kr, tr in [("r0","a0_ref"), ("r1","a1_ref")]:
        loss = loss + _mce(out[kr].reshape(B*S,-1), tgts[tr].reshape(B*S))

    opt = torch.optim.Adam([p for p in model.parameters() if p.requires_grad], lr=lr)
    opt.zero_grad()
    loss.backward()
    torch.nn.utils.clip_grad_norm_([p for p in model.parameters() if p.requires_grad], 1.0)
    opt.step()
    model.eval()
    return loss.item()


def save_checkpoint(model, lora_layers, experience, path, metadata=None):
    """Serialize the complete SOMA mind (Section 9.3).
    The checkpoint IS the SOMA at this moment."""
    ckpt = {
        "lora_state": {n: l.lora_state() for n, l in lora_layers.items()},
        "experience": {"total_seen": experience.total_seen, "buffer_size": len(experience)},
        "metadata": {"timestamp": datetime.now().isoformat(), "version": "pow2", **(metadata or {})},
    }
    torch.save(ckpt, path)


def restore_checkpoint(model, lora_layers, path):
    """Restore a SOMA mind from checkpoint."""
    ckpt = torch.load(path, map_location="cpu", weights_only=False)
    for name, layer in lora_layers.items():
        if name in ckpt["lora_state"]:
            layer.load_lora_state(ckpt["lora_state"][name])
    model.eval()
    return ckpt.get("metadata", {}), ckpt.get("experience", {})


def reset_lora(lora_layers):
    """Rollback -- reset all LoRA to zero (lose experience)."""
    from pow.pow2.lora import LoRALinear, LoRAGRUCell
    for layer in lora_layers.values():
        with torch.no_grad():
            if isinstance(layer, LoRALinear):
                layer.lora_A.normal_(0, 0.01)
                layer.lora_B.zero_()
            elif isinstance(layer, LoRAGRUCell):
                layer.ih_A.normal_(0, 0.01); layer.ih_B.zero_()
                layer.hh_A.normal_(0, 0.01); layer.hh_B.zero_()


def consolidate(lora_layers):
    """Sleep cycle -- merge LoRA into base weights (Section 12.3).
    Proven adaptations become permanent memory."""
    for layer in lora_layers.values():
        layer.merge()
