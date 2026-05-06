"""
Ingest knowledge into SOMA Brain SDM.

Chunks documents, embeds with pretrained model, writes to SDM.
No training. No gradients. Knowledge goes to RAM.
"""

import argparse
import sys
import time
from pathlib import Path


def log(msg):
    print(msg, flush=True)

import torch

from soma_brain import SomaBrain, BrainConfig, Embedder


EXTS = {".txt", ".md", ".rst", ".json", ".py", ".rs", ".toml", ".yaml", ".yml"}
SKIP = {".venv", "__pycache__", ".git", "node_modules", ".pytest_cache", "target", ".egg-info", "checkpoints"}


def get_device():
    if torch.backends.mps.is_available():
        return torch.device("mps")
    if torch.cuda.is_available():
        return torch.device("cuda")
    return torch.device("cpu")


def load_files(paths: list[str]) -> list[tuple[Path, str]]:
    files = []
    for p in paths:
        path = Path(p)
        if path.is_dir():
            for f in sorted(path.rglob("*")):
                if any(s in f.parts for s in SKIP):
                    continue
                if f.is_file() and f.suffix in EXTS:
                    files.append((f, f.read_text(errors="replace")))
        elif path.is_file():
            files.append((path, path.read_text(errors="replace")))
    return files


def main():
    parser = argparse.ArgumentParser(description="Ingest knowledge into SOMA Brain SDM")
    parser.add_argument("--size", choices=["tiny", "medium", "small", "base", "large"], default="medium")
    parser.add_argument("--data", type=str, nargs="+", required=True)
    parser.add_argument("--save", type=str, default="checkpoints/brain.pt")
    parser.add_argument("--chunk-size", type=int, default=512)
    parser.add_argument("--embed-batch", type=int, default=32)
    args = parser.parse_args()

    config = getattr(BrainConfig, args.size)()

    device = get_device()
    log(f"device: {device} | config: {args.size}")

    log("loading embedder...")
    embedder = Embedder()
    config.embed_dim = embedder.embed_dim
    log(f"embedder: {embedder.model.model_card_data.base_model or 'unknown'} ({embedder.embed_dim}-dim)")

    model = SomaBrain(config).to(device)
    params = model.count_parameters()
    log(f"reasoning core: {params['total_unique']:,} params")

    log("\n— loading files —")
    files = load_files(args.data)
    if not files:
        log("error: no files found")
        return

    all_chunks = []
    for i, (path, text) in enumerate(files):
        chunks = Embedder.chunk_text(text, args.chunk_size)
        all_chunks.extend(chunks)
        log(f"  [{i+1}/{len(files)}] {path} — {len(chunks)} chunks")
    log(f"  total: {len(files)} files, {len(all_chunks)} chunks")

    total = len(all_chunks)
    log_interval = max(1, total // 20)
    log(f"\n— embedding + ingesting into SDM ({total} chunks) —")
    t0 = time.time()
    last_log = 0
    for i in range(0, total, args.embed_batch):
        batch = all_chunks[i : i + args.embed_batch]
        embeddings = embedder.embed(batch).to(device)
        for j, text in enumerate(batch):
            model.ingest(embeddings[j:j+1], text=text)

        done = min(i + args.embed_batch, total)
        if done - last_log >= log_interval or done == total:
            elapsed = time.time() - t0
            rate = done / elapsed if elapsed > 0 else 0
            eta = (total - done) / rate if rate > 0 else 0
            pct = done * 100 // total
            log(f"  {pct:3d}% | {done}/{total} | {rate:.0f} chunks/s | {elapsed:.0f}s elapsed | eta {eta:.0f}s")
            last_log = done

    elapsed = time.time() - t0
    log(f"done: {len(all_chunks)} chunks → SDM ({model.sdm.num_locations} entries) in {elapsed:.1f}s")

    import os
    os.makedirs(os.path.dirname(args.save) or ".", exist_ok=True)
    torch.save({
        "config": config,
        "state_dict": model.state_dict(),
        "sdm_addresses": model.sdm._addr_buf[:model.sdm._count].cpu(),
        "sdm_entries": model.sdm._data_buf[:model.sdm._count].cpu(),
        "source_texts": model.source_texts,
        "source_embeddings": torch.stack(model.source_embeddings).cpu() if model.source_embeddings else [],
    }, args.save)
    log(f"saved: {args.save}")


if __name__ == "__main__":
    main()
