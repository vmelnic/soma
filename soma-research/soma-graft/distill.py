"""
Train the bridge via self-supervised LM loss.

Frozen Qwen-3B + bridge + frozen 32B-SDM → minimize next-token cross-entropy
on a text corpus. Qwen-3B alone has some baseline LM loss. The bridge,
gated to start at zero contribution, learns to inject SDM retrievals only
where they help reduce loss.

No big-teacher inference needed at train time — saves 64GB RAM and slow CPU forward.
"""

import argparse
import os
import time

import torch
import torch.nn.functional as F
from transformers import AutoModelForCausalLM, AutoTokenizer

from bridge import Bridge, GraftedQwen


def get_corpus(n=1000, max_len=128):
    from datasets import load_dataset
    ds = load_dataset("wikimedia/wikipedia", "20231101.en",
                      streaming=True, split="train")
    out = []
    for ex in ds:
        if len(ex["text"]) > 32:
            out.append(ex["text"][:max_len * 6])
        if len(out) >= n:
            break
    return out


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--small-model", default="Qwen/Qwen2.5-3B")
    parser.add_argument("--sdm", default="checkpoints/qwen32b_sdm.pt")
    parser.add_argument("--save", default="checkpoints/bridge.pt")
    parser.add_argument("--inject-layers", type=int, nargs="+",
                        default=[12, 18, 24, 30])
    parser.add_argument("--top-k", type=int, default=128)
    parser.add_argument("--steps", type=int, default=2000)
    parser.add_argument("--batch-size", type=int, default=1)
    parser.add_argument("--lr", type=float, default=3e-4)
    parser.add_argument("--max-len", type=int, default=128)
    parser.add_argument("--corpus-size", type=int, default=1000)
    parser.add_argument("--warmup", type=int, default=100)
    parser.add_argument("--log-every", type=int, default=20)
    parser.add_argument("--save-every", type=int, default=200)
    args = parser.parse_args()

    device = torch.device("cuda")
    dtype = torch.bfloat16

    print(f"loading small model ({args.small_model}) frozen on GPU...")
    small = AutoModelForCausalLM.from_pretrained(
        args.small_model, dtype=dtype,
    ).to(device)
    small.train(False)
    H_small = small.config.hidden_size
    N_layers_small = small.config.num_hidden_layers

    # SDM-side hidden size from the SDM file's config
    sdm_cfg = torch.load(args.sdm, map_location="cpu", mmap=True,
                         weights_only=False)["config"]
    H_big = sdm_cfg["hidden_size"]
    print(f"  small d={H_small}  big d={H_big}  small layers={N_layers_small}")

    print(f"building bridge: {H_small} <-> {H_big}, inject at {args.inject_layers}")
    bridge = Bridge(
        d_small=H_small, d_big=H_big,
        sdm_path=args.sdm,
        inject_layers=args.inject_layers,
        top_k=args.top_k,
        n_layers_small=N_layers_small,
        dtype=dtype,
    ).to(device).to(dtype)
    bridge.load_sdm(device)
    print(f"  SDM layers: {bridge._sdm_layers_total}")

    grafted = GraftedQwen(small, bridge)
    grafted.attach_hooks()

    n_train = sum(p.numel() for p in bridge.parameters() if p.requires_grad)
    print(f"  trainable bridge params: {n_train:,}")

    tok = AutoTokenizer.from_pretrained(args.small_model)
    print(f"\npreparing corpus ({args.corpus_size} samples)...")
    texts = get_corpus(n=args.corpus_size, max_len=args.max_len)
    cached = []
    for t in texts:
        ids = tok(t, return_tensors="pt", truncation=True,
                  max_length=args.max_len).input_ids[0]
        if ids.shape[0] >= 8:
            cached.append(ids)
    print(f"  ready: {len(cached)}")

    optimizer = torch.optim.AdamW(
        [p for p in bridge.parameters() if p.requires_grad],
        lr=args.lr, weight_decay=0.01, eps=1e-6,
    )
    scheduler = torch.optim.lr_scheduler.LambdaLR(
        optimizer, lambda s: min(1.0, (s + 1) / max(args.warmup, 1)),
    )

    print(f"\ntraining {args.steps} steps...\n")
    t0 = time.time()
    losses = []

    for step in range(1, args.steps + 1):
        idx = torch.randint(0, len(cached), (args.batch_size,)).tolist()
        max_T = max(cached[i].shape[0] for i in idx)
        ids_batch = torch.zeros(args.batch_size, max_T, dtype=torch.long, device=device)
        for b, i in enumerate(idx):
            T = cached[i].shape[0]
            ids_batch[b, :T] = cached[i].to(device)

        # Self-supervised LM loss: predict next token
        student_logits = grafted(ids_batch).logits  # (B, T, V)
        # Shift for causal LM
        logits_shift = student_logits[:, :-1, :].contiguous()
        targets_shift = ids_batch[:, 1:].contiguous()
        loss = F.cross_entropy(
            logits_shift.float().reshape(-1, logits_shift.shape[-1]),
            targets_shift.reshape(-1),
        )

        optimizer.zero_grad()
        loss.backward()
        torch.nn.utils.clip_grad_norm_(bridge.parameters(), 0.5)
        optimizer.step()
        scheduler.step()

        losses.append(loss.item())

        if step % args.log_every == 0 or step == 1:
            avg = sum(losses[-args.log_every:]) / min(len(losses), args.log_every)
            elapsed = time.time() - t0
            print(f"  step {step}/{args.steps} | ce {avg:.4f} | "
                  f"{step/elapsed:.2f} step/s")

        if step % args.save_every == 0 or step == args.steps:
            os.makedirs(os.path.dirname(args.save) or ".", exist_ok=True)
            torch.save({
                "bridge_state": bridge.state_dict(),
                "config": {
                    "d_small": H_small, "d_big": H_big,
                    "inject_layers": args.inject_layers,
                    "top_k": args.top_k,
                    "n_layers_small": N_layers_small,
                },
                "step": step,
            }, args.save)

    print(f"\ndone: {args.steps} steps in {time.time() - t0:.0f}s")
    print(f"saved: {args.save}")


if __name__ == "__main__":
    main()
