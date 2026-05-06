"""Build a kNN-LM index: one forward pass over the corpus, dump (hidden, next_token) to FAISS.

No training. No gradients. Output is a FAISS-IVF-PQ index plus a side array of next-token ids.

Usage:
    python build_index.py --corpus corpus/rust-book --out index/rust-book
"""
from __future__ import annotations

import argparse
import pathlib
import sys
import time

import numpy as np
import torch
from tqdm import tqdm

from config import Config, MODEL_PRESETS


def iter_corpus_files(root: pathlib.Path):
    exts = {".md", ".txt", ".rs", ".py", ".toml", ".rst"}
    for p in sorted(root.rglob("*")):
        if p.is_file() and p.suffix.lower() in exts:
            yield p


def chunk_tokens(ids, chunk: int, stride: int):
    i = 0
    while i + 2 < len(ids):
        end = min(i + chunk, len(ids))
        yield ids[i:end]
        if end == len(ids):
            return
        i += stride


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--corpus", type=pathlib.Path, required=True, nargs="+")
    ap.add_argument("--out", type=pathlib.Path, required=True)
    ap.add_argument("--model", default="gemma4-26b-a4b")
    ap.add_argument("--chunk", type=int, default=2048)
    ap.add_argument("--stride", type=int, default=2048)
    ap.add_argument("--max-entries", type=int, default=20_000_000)
    args = ap.parse_args()

    cfg = Config(model_id=MODEL_PRESETS.get(args.model, args.model))
    args.out.mkdir(parents=True, exist_ok=True)

    from transformers import AutoModelForCausalLM, AutoTokenizer

    print(f"loading {cfg.model_id} (fp16, cuda)", flush=True)
    tok = AutoTokenizer.from_pretrained(cfg.model_id, trust_remote_code=True)
    model = AutoModelForCausalLM.from_pretrained(
        cfg.model_id,
        torch_dtype=torch.float16,
        device_map="cuda",
        trust_remote_code=True,
    )
    model.train(False)
    hidden_dim = model.config.hidden_size
    print(f"hidden_dim={hidden_dim}", flush=True)

    files = []
    for root in args.corpus:
        files.extend(iter_corpus_files(root))
    if not files:
        print(f"no files under {args.corpus}", file=sys.stderr)
        sys.exit(1)
    print(f"{len(files)} files in corpus", flush=True)

    keys = np.zeros((args.max_entries, hidden_dim), dtype=np.float16)
    vals = np.zeros((args.max_entries,), dtype=np.int32)
    n = 0
    t0 = time.time()

    for path in tqdm(files, desc="files"):
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError as e:
            print(f"skip {path}: {e}", file=sys.stderr)
            continue
        ids = tok.encode(text, add_special_tokens=False)
        if len(ids) < 8:
            continue
        for window in chunk_tokens(ids, args.chunk, args.stride):
            if n >= args.max_entries:
                break
            input_ids = torch.tensor([window], device=cfg.device)
            with torch.no_grad():
                out = model(
                    input_ids=input_ids,
                    output_hidden_states=True,
                    use_cache=False,
                )
            hs = out.hidden_states[cfg.hidden_layer][0]
            take = min(hs.shape[0] - 1, args.max_entries - n)
            keys[n : n + take] = hs[:take].detach().to(torch.float16).cpu().numpy()
            vals[n : n + take] = np.array(window[1 : 1 + take], dtype=np.int32)
            n += take
        if n >= args.max_entries:
            break

    keys = keys[:n]
    vals = vals[:n]
    elapsed = time.time() - t0
    print(f"collected {n:,} (key,token) pairs in {elapsed:.1f}s", flush=True)

    import faiss

    print("training IVF-PQ index", flush=True)
    keys_f32 = keys.astype(np.float32)
    quantizer = faiss.IndexFlatIP(hidden_dim)
    index = faiss.IndexIVFPQ(
        quantizer, hidden_dim, cfg.faiss_nlist, cfg.faiss_pq_m, cfg.faiss_pq_nbits
    )
    train_n = min(200_000, n)
    perm = np.random.default_rng(0).permutation(n)[:train_n]
    index.train(keys_f32[perm])
    index.add(keys_f32)
    index.nprobe = cfg.faiss_nprobe

    faiss.write_index(index, str(args.out / "index.faiss"))
    np.save(args.out / "vals.npy", vals)
    (args.out / "meta.txt").write_text(
        f"model={cfg.model_id}\nhidden_dim={hidden_dim}\nentries={n}\n"
    )
    print(f"wrote {args.out}/index.faiss + vals.npy", flush=True)


if __name__ == "__main__":
    main()
