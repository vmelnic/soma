"""kNN-LM generation: blend base-LM logits with retrieved token distribution.

Usage:
    python infer.py --index index/rust-book --prompt "fn main() {" --max-tokens 64
"""
from __future__ import annotations

import argparse
import pathlib

import numpy as np
import torch
import torch.nn.functional as F

from config import Config, MODEL_PRESETS


def load_meta(index_dir: pathlib.Path):
    meta = {}
    for line in (index_dir / "meta.txt").read_text().splitlines():
        if "=" in line:
            key, val = line.split("=", 1)
            meta[key] = val
    return meta


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--index", type=pathlib.Path, required=True)
    ap.add_argument("--prompt", type=str, required=True)
    ap.add_argument("--max-tokens", type=int, default=64)
    ap.add_argument("--k", type=int, default=16)
    ap.add_argument("--lam", type=float, default=0.25)
    ap.add_argument("--temperature", type=float, default=0.8)
    ap.add_argument("--model", default=None)
    args = ap.parse_args()

    meta = load_meta(args.index)
    model_id = args.model or meta["model"]
    if args.model and args.model in MODEL_PRESETS:
        model_id = MODEL_PRESETS[args.model]

    import faiss
    from transformers import AutoModelForCausalLM, AutoTokenizer

    print(f"loading {model_id}", flush=True)
    tok = AutoTokenizer.from_pretrained(model_id, trust_remote_code=True)
    model = AutoModelForCausalLM.from_pretrained(
        model_id, torch_dtype=torch.float16, device_map="cuda", trust_remote_code=True
    )
    model.train(False)
    vocab_size = model.config.vocab_size

    print("loading FAISS index", flush=True)
    index = faiss.read_index(str(args.index / "index.faiss"))
    vals = np.load(args.index / "vals.npy")

    input_ids = tok.encode(args.prompt, return_tensors="pt").to("cuda")
    past = None
    cfg = Config()

    for _ in range(args.max_tokens):
        with torch.no_grad():
            out = model(
                input_ids=input_ids if past is None else input_ids[:, -1:],
                past_key_values=past,
                use_cache=True,
                output_hidden_states=True,
            )
        past = out.past_key_values
        logits = out.logits[0, -1]
        h = out.hidden_states[cfg.hidden_layer][0, -1].detach().to(torch.float32).cpu().numpy()

        D, I = index.search(h.reshape(1, -1), args.k)
        dist = D[0]
        ids = vals[I[0]]
        weights = np.exp(-dist / max(args.temperature, 1e-6))
        p_knn = np.zeros(vocab_size, dtype=np.float32)
        for token_id, w in zip(ids, weights):
            if 0 <= token_id < vocab_size:
                p_knn[token_id] += w
        s = p_knn.sum()
        if s > 0:
            p_knn /= s

        p_lm = F.softmax(logits / args.temperature, dim=-1).detach().cpu().float().numpy()
        p_final = args.lam * p_knn + (1.0 - args.lam) * p_lm
        next_id = int(np.argmax(p_final))

        input_ids = torch.cat(
            [input_ids, torch.tensor([[next_id]], device="cuda")], dim=1
        )
        if next_id == tok.eos_token_id:
            break

    print(tok.decode(input_ids[0].tolist()))


if __name__ == "__main__":
    main()
