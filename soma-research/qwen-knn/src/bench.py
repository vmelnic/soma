"""Compare vanilla base LM vs Base+kNN-LM on held-out files.

Reports token-level perplexity and top-1 accuracy for both, side-by-side.

Usage:
    python bench.py --index index/rust-book --held-out bench/held_out
"""
from __future__ import annotations

import argparse
import math
import pathlib

import numpy as np
import torch
import torch.nn.functional as F

from config import Config


def load_meta(index_dir: pathlib.Path):
    meta = {}
    for line in (index_dir / "meta.txt").read_text().splitlines():
        if "=" in line:
            key, val = line.split("=", 1)
            meta[key] = val
    return meta


def iter_files(root: pathlib.Path):
    for p in sorted(root.rglob("*")):
        if p.is_file() and p.suffix.lower() in {".md", ".txt", ".rs", ".py"}:
            yield p


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--index", type=pathlib.Path, required=True)
    ap.add_argument("--held-out", type=pathlib.Path, required=True)
    ap.add_argument("--k", type=int, default=16)
    ap.add_argument("--lam", type=float, default=0.25)
    ap.add_argument("--temperature", type=float, default=1.0)
    ap.add_argument("--max-tokens", type=int, default=20_000)
    ap.add_argument("--window", type=int, default=512)
    args = ap.parse_args()

    meta = load_meta(args.index)
    model_id = meta["model"]

    import faiss
    from transformers import AutoModelForCausalLM, AutoTokenizer

    print(f"loading {model_id}", flush=True)
    tok = AutoTokenizer.from_pretrained(model_id, trust_remote_code=True)
    model = AutoModelForCausalLM.from_pretrained(
        model_id, torch_dtype=torch.float16, device_map="cuda", trust_remote_code=True
    )
    model.train(False)
    vocab_size = model.config.vocab_size

    index = faiss.read_index(str(args.index / "index.faiss"))
    vals = np.load(args.index / "vals.npy")
    cfg = Config()

    nll_lm = 0.0
    nll_knn = 0.0
    correct_lm = 0
    correct_knn = 0
    total = 0

    for path in iter_files(args.held_out):
        text = path.read_text(encoding="utf-8", errors="ignore")
        ids = tok.encode(text, add_special_tokens=False)
        if len(ids) < 8:
            continue
        ids = ids[: args.max_tokens]

        for start in range(0, len(ids) - 1, args.window):
            window = ids[start : start + args.window + 1]
            if len(window) < 2:
                continue
            input_ids = torch.tensor([window], device="cuda")
            with torch.no_grad():
                out = model(
                    input_ids=input_ids,
                    output_hidden_states=True,
                    use_cache=False,
                )
            logits = out.logits[0]
            hs = out.hidden_states[cfg.hidden_layer][0]

            targets = window[1:]
            h_q = hs[: len(targets)].detach().to(torch.float32).cpu().numpy()
            D, I = index.search(h_q, args.k)

            logits_cpu = logits[: len(targets)].detach().to(torch.float32).cpu().numpy()
            for t, target in enumerate(targets):
                lm_logits = logits_cpu[t] / args.temperature
                lm_logits -= lm_logits.max()
                exp_lm = np.exp(lm_logits)
                p_lm = exp_lm / exp_lm.sum()

                weights = np.exp(-D[t] / max(args.temperature, 1e-6))
                p_knn = np.zeros(vocab_size, dtype=np.float32)
                for tok_id, w in zip(vals[I[t]], weights):
                    if 0 <= tok_id < vocab_size:
                        p_knn[tok_id] += w
                s = p_knn.sum()
                if s > 0:
                    p_knn /= s
                p_final = args.lam * p_knn + (1.0 - args.lam) * p_lm

                nll_lm -= math.log(max(p_lm[target], 1e-12))
                nll_knn -= math.log(max(p_final[target], 1e-12))
                if int(p_lm.argmax()) == target:
                    correct_lm += 1
                if int(p_final.argmax()) == target:
                    correct_knn += 1
                total += 1
            del out, logits, hs
            torch.cuda.empty_cache()

    if total == 0:
        print("no held-out tokens scored")
        return

    print(f"tokens scored: {total}")
    print(f"vanilla   ppl={math.exp(nll_lm / total):.3f}  top1={correct_lm / total:.4f}")
    print(f"kNN-LM    ppl={math.exp(nll_knn / total):.3f}  top1={correct_knn / total:.4f}  (k={args.k}, lam={args.lam})")


if __name__ == "__main__":
    main()
