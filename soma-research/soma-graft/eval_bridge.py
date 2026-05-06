"""
Compare LM loss: vanilla Qwen-3B vs Qwen-3B + Bridge + 32B-SDM on same text.
If bridge < vanilla, the SDM augmentation actually helps.
"""

import argparse
import torch
import torch.nn.functional as F
from transformers import AutoModelForCausalLM, AutoTokenizer

from bridge import Bridge, GraftedQwen


def get_eval_corpus(n=100, max_len=128):
    from datasets import load_dataset
    ds = load_dataset("wikimedia/wikipedia", "20231101.en",
                      streaming=True, split="train")
    out = []
    skip = 50000  # skip past distillation samples
    for i, ex in enumerate(ds):
        if i < skip:
            continue
        if len(ex["text"]) > 32:
            out.append(ex["text"][:max_len * 6])
        if len(out) >= n:
            break
    return out


def lm_loss(model, ids):
    out = model(ids)
    logits = out.logits[:, :-1, :].contiguous()
    targets = ids[:, 1:].contiguous()
    return F.cross_entropy(
        logits.float().reshape(-1, logits.shape[-1]),
        targets.reshape(-1),
    ).item()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--small-model", default="Qwen/Qwen2.5-3B")
    parser.add_argument("--sdm", default="checkpoints/qwen32b_sdm.pt")
    parser.add_argument("--bridge", default="checkpoints/bridge.pt")
    parser.add_argument("--n", type=int, default=100)
    parser.add_argument("--max-len", type=int, default=128)
    args = parser.parse_args()

    device = torch.device("cuda")
    dtype = torch.bfloat16

    small = AutoModelForCausalLM.from_pretrained(args.small_model, dtype=dtype).to(device)
    small.train(False)
    tok = AutoTokenizer.from_pretrained(args.small_model)

    # Load bridge config and weights
    bridge_ckpt = torch.load(args.bridge, map_location="cpu", weights_only=False)
    bcfg = bridge_ckpt["config"]
    bridge = Bridge(
        d_small=bcfg["d_small"], d_big=bcfg["d_big"],
        sdm_path=args.sdm,
        inject_layers=bcfg["inject_layers"],
        top_k=bcfg["top_k"],
        n_layers_small=bcfg.get("n_layers_small", small.config.num_hidden_layers),
    ).to(device).to(dtype)
    bridge.load_state_dict(bridge_ckpt["bridge_state"])
    bridge.load_sdm(device)

    grafted = GraftedQwen(small, bridge)

    # Eval corpus (held-out text)
    print(f"loading {args.n} eval samples...")
    texts = get_eval_corpus(n=args.n, max_len=args.max_len)
    cached = []
    for t in texts:
        ids = tok(t, return_tensors="pt", truncation=True,
                  max_length=args.max_len).input_ids
        if ids.shape[1] >= 8:
            cached.append(ids.to(device))

    print(f"\nMeasuring loss on {len(cached)} samples...")
    vanilla_losses = []
    bridge_losses = []
    with torch.no_grad():
        for ids in cached:
            # Vanilla (no bridge hooks)
            grafted.detach_hooks()
            vanilla_losses.append(lm_loss(small, ids))
            # With bridge
            grafted.attach_hooks()
            bridge_losses.append(lm_loss(small, ids))
            grafted.detach_hooks()

    v_avg = sum(vanilla_losses) / len(vanilla_losses)
    b_avg = sum(bridge_losses) / len(bridge_losses)
    delta = b_avg - v_avg

    print(f"\n=== Results ===")
    print(f"  vanilla Qwen-3B:     ce = {v_avg:.4f}")
    print(f"  Qwen-3B + bridge:    ce = {b_avg:.4f}")
    print(f"  delta (b - v):       {delta:+.4f}  ({'bridge HELPS' if delta < 0 else 'bridge HURTS'})")
    n_better = sum(1 for v, b in zip(vanilla_losses, bridge_losses) if b < v)
    print(f"  per-sample wins:     {n_better}/{len(cached)} samples improved with bridge")


if __name__ == "__main__":
    main()
