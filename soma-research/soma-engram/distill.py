"""
Distill the standalone LiquidCore from a teacher (Qwen) using diverse text.

Teacher: HF Qwen-3B (frozen, used only to produce target hidden states).
Student: LiquidCore (CfC + SDM queries, only ~38M trainable).
Loss: MSE on final hidden states + KL on logits.

After distillation: discard teacher. Run LiquidCore standalone.
"""

import argparse
import os
import time

import torch
import torch.nn.functional as F
from transformers import AutoModelForCausalLM, AutoTokenizer

from ltc import bootstrap_from_extraction


def get_corpus(n=2000, max_len=128, source="wiki"):
    """Diverse text from HuggingFace streaming dataset."""
    from datasets import load_dataset
    if source == "wiki":
        ds = load_dataset("wikimedia/wikipedia", "20231101.en",
                          streaming=True, split="train")
    elif source == "fineweb":
        ds = load_dataset("HuggingFaceFW/fineweb-edu", "sample-10BT",
                          streaming=True, split="train")
    else:
        raise ValueError(source)
    out = []
    for example in ds:
        text = example["text"][:max_len * 6]
        if len(text) > 32:
            out.append(text)
        if len(out) >= n:
            break
    return out


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--ckpt", default="checkpoints/qwen3b_full.pt")
    parser.add_argument("--model", default="Qwen/Qwen2.5-3B")
    parser.add_argument("--save", default="checkpoints/liquid_core.pt")
    parser.add_argument("--source", default="wiki", choices=["wiki", "fineweb"])
    parser.add_argument("--n-steps", type=int, default=8)
    parser.add_argument("--top-k", type=int, default=128)
    parser.add_argument("--steps", type=int, default=20000)
    parser.add_argument("--batch-size", type=int, default=2)
    parser.add_argument("--lr", type=float, default=3e-4)
    parser.add_argument("--alpha-kl", type=float, default=1.0)
    parser.add_argument("--warmup", type=int, default=500)
    parser.add_argument("--max-len", type=int, default=128)
    parser.add_argument("--corpus-size", type=int, default=10000)
    parser.add_argument("--log-every", type=int, default=100)
    parser.add_argument("--save-every", type=int, default=1000)
    parser.add_argument("--resume", action="store_true",
                        help="Resume from existing --save checkpoint")
    args = parser.parse_args()

    device = torch.device("cuda")
    dtype = torch.bfloat16

    print(f"loading extraction from {args.ckpt}...")
    extracted = torch.load(args.ckpt, map_location="cpu", weights_only=False)
    cfg = extracted["config"]

    print("building LiquidCore...")
    core = bootstrap_from_extraction(
        extracted, n_steps=args.n_steps, top_k=args.top_k, dtype=dtype,
    ).to(device)

    if args.resume and os.path.exists(args.save):
        ckpt = torch.load(args.save, map_location=device, weights_only=False)
        core.load_state_dict(ckpt["ltc_state"], strict=False)
        print(f"  resumed from step {ckpt.get('step', 0)}")

    n_train = sum(p.numel() for p in core.parameters() if p.requires_grad)
    print(f"  trainable: {n_train:,}")

    print("loading teacher Qwen...")
    teacher = AutoModelForCausalLM.from_pretrained(args.model, dtype=dtype).to(device)
    teacher.train(False)
    for p in teacher.parameters():
        p.requires_grad = False
    tok = AutoTokenizer.from_pretrained(args.model)

    print(f"\npreparing corpus ({args.corpus_size} from {args.source})...")
    texts = get_corpus(n=args.corpus_size, max_len=args.max_len, source=args.source)
    cached_ids = []
    for t in texts:
        ids = tok(t, return_tensors="pt", truncation=True,
                  max_length=args.max_len).input_ids[0]
        if ids.shape[0] >= 8:
            cached_ids.append(ids)
    print(f"  ready: {len(cached_ids)} samples")

    optimizer = torch.optim.AdamW(
        [p for p in core.parameters() if p.requires_grad],
        lr=args.lr, weight_decay=0.01, eps=1e-6,
    )
    scheduler = torch.optim.lr_scheduler.LambdaLR(
        optimizer,
        lambda s: min(1.0, (s + 1) / max(args.warmup, 1)),
    )

    free_gb = torch.cuda.mem_get_info()[0] / 1e9
    print(f"GPU free: {free_gb:.1f}GB\ntraining {args.steps} steps, batch={args.batch_size}\n")

    t0 = time.time()
    losses_h, losses_kl = [], []

    for step in range(1, args.steps + 1):
        idx = torch.randint(0, len(cached_ids), (args.batch_size,)).tolist()
        max_T = max(cached_ids[i].shape[0] for i in idx)
        ids_batch = torch.zeros(args.batch_size, max_T, dtype=torch.long, device=device)
        masks = torch.zeros(args.batch_size, max_T, dtype=torch.bool, device=device)
        for b, i in enumerate(idx):
            T = cached_ids[i].shape[0]
            ids_batch[b, :T] = cached_ids[i].to(device)
            masks[b, :T] = True

        with torch.no_grad():
            t_out = teacher(ids_batch, output_hidden_states=True)
            target_h = t_out.hidden_states[-1]
            target_logits = t_out.logits

        ltc_logits, ltc_h = core(ids_batch, return_hidden=True)

        diff_h = (ltc_h.float() - target_h.float())
        loss_h = (diff_h.pow(2).sum(-1) * masks).sum() / masks.sum() / cfg["hidden_size"]

        log_p_ltc = F.log_softmax(ltc_logits.float(), dim=-1)
        p_target = F.softmax(target_logits.float(), dim=-1)
        kl = F.kl_div(log_p_ltc, p_target, reduction="none").sum(-1)
        loss_kl = (kl * masks).sum() / masks.sum()

        loss = loss_h + args.alpha_kl * loss_kl

        optimizer.zero_grad()
        loss.backward()
        torch.nn.utils.clip_grad_norm_(
            [p for p in core.parameters() if p.requires_grad], 0.5,
        )
        optimizer.step()
        scheduler.step()

        losses_h.append(loss_h.item())
        losses_kl.append(loss_kl.item())

        if step % args.log_every == 0 or step == 1:
            elapsed = time.time() - t0
            n = min(len(losses_h), args.log_every)
            avg_h = sum(losses_h[-n:]) / n
            avg_kl = sum(losses_kl[-n:]) / n
            mem = torch.cuda.max_memory_allocated() / 1e9
            print(f"  step {step}/{args.steps} | h={avg_h:.4f} kl={avg_kl:.4f} "
                  f"| {step/elapsed:.2f} step/s | mem {mem:.1f}GB")

        if step % args.save_every == 0 or step == args.steps:
            os.makedirs(os.path.dirname(args.save) or ".", exist_ok=True)
            state = {k: v for k, v in core.state_dict().items() if "sdm" not in k}
            torch.save({
                "ltc_state": state,
                "config": cfg,
                "n_steps": args.n_steps,
                "top_k": args.top_k,
                "step": step,
            }, args.save)

    print(f"\ndone: {args.steps} steps in {time.time() - t0:.0f}s")
    print(f"saved: {args.save}")


if __name__ == "__main__":
    main()
