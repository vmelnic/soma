"""Query SOMA Brain — retrieve, reason, generate, and test port/consolidation."""

import argparse
import os
import time

import torch

from soma_brain import SomaBrain, Embedder, BrainPort
from soma_brain.episodes import generate_synthetic_episodes


def load_brain(checkpoint, size, device):
    if checkpoint and os.path.exists(checkpoint):
        data = torch.load(checkpoint, map_location=device, weights_only=False)
        config = data["config"]
        model = SomaBrain(config).to(device)
        model.load_state_dict(data["state_dict"], strict=False)
        if "sdm_addresses" in data:
            addrs = data["sdm_addresses"]
            entries = data["sdm_entries"]
            if isinstance(addrs, list):
                model.sdm._addr_buf = torch.stack(addrs)
                model.sdm._data_buf = torch.stack(entries)
                model.sdm._count = len(addrs)
            else:
                model.sdm._addr_buf = addrs
                model.sdm._data_buf = entries
                model.sdm._count = addrs.shape[0]
        if "source_texts" in data:
            model.source_texts = data["source_texts"]
        if "source_embeddings" in data:
            se = data["source_embeddings"]
            if isinstance(se, torch.Tensor) and se.dim() == 2:
                model.source_embeddings = list(se)
            elif isinstance(se, list):
                model.source_embeddings = se
        print(f"loaded: {checkpoint} (SDM {model.sdm.num_locations} entries, {len(model.source_texts)} sources)")
    else:
        from soma_brain import BrainConfig
        config = getattr(BrainConfig, size)()
        model = SomaBrain(config).to(device)
        print(f"fresh {size} brain (SDM empty)")
    model.eval()
    return model


def main():
    parser = argparse.ArgumentParser(description="Query SOMA Brain")
    parser.add_argument("--checkpoint", type=str, default="checkpoints/brain.pt")
    parser.add_argument("--size", choices=["tiny", "medium", "small", "base", "large"], default="medium")
    args = parser.parse_args()

    device = torch.device("mps" if torch.backends.mps.is_available() else "cpu")

    print("loading embedder...")
    embedder = Embedder()

    model = load_brain(args.checkpoint, args.size, device)
    port = BrainPort(model, embedder)

    print("commands: /quit /sdm /params /gen <text> /port <cap> /consolidate\n")

    while True:
        try:
            query = input("query> ").strip()
        except (EOFError, KeyboardInterrupt):
            break

        if not query:
            continue
        if query == "/quit":
            break
        if query == "/params":
            for k, v in model.count_parameters().items():
                print(f"  {k}: {v:,}")
            continue
        if query == "/sdm":
            print(f"  SDM: {model.sdm.num_locations} entries")
            print(f"  sources: {len(model.source_texts)}")
            continue

        if query.startswith("/gen "):
            prompt = query[5:].strip()
            if not prompt:
                print("  usage: /gen <prompt>")
                continue
            print("  generating...", flush=True)
            t0 = time.time()
            emb = embedder.embed_one(prompt).unsqueeze(0).to(device)
            texts = model.generate(emb, max_len=64, steps=16)
            elapsed = time.time() - t0
            print(f"  [{elapsed:.1f}s] {texts[0]}\n")
            continue

        if query.startswith("/port "):
            parts = query[6:].strip().split(" ", 1)
            cap = parts[0]
            param_text = parts[1] if len(parts) > 1 else ""
            params = {"query": param_text} if param_text else {}
            result = port.invoke(cap, params)
            for k, v in result.items():
                if isinstance(v, list):
                    print(f"  {k}:")
                    for item in v[:5]:
                        print(f"    {item}")
                else:
                    print(f"  {k}: {v}")
            print()
            continue

        if query == "/consolidate":
            print("  testing consolidation with synthetic episode...")
            episodes = generate_synthetic_episodes(
                ["skill_" + str(i) for i in range(10)], n_episodes=2,
            )
            for ep in episodes:
                result = port.invoke("consolidate_episode", {"episode": ep})
                print(f"  episode: {result}")
            status = port.invoke("status", {})
            print(f"  status: {status}")
            print()
            continue

        if query == "/manifest":
            import json
            print(json.dumps(port.manifest(), indent=2))
            print()
            continue

        emb = embedder.embed_one(query).unsqueeze(0).to(device)
        result = model.reason(emb)

        print()
        if result.sources:
            for text, score in result.sources:
                preview = text[:150].replace("\n", " ").strip()
                print(f"  [{score:.4f}] {preview}")
        else:
            print("  (no sources)")

        conf = result.confidence.item()
        print(f"  confidence: {conf:.4f} ({'authoritative' if conf >= 0.7 else 'defer to LLM'})")
        print()


if __name__ == "__main__":
    main()
