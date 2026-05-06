"""Diagnose the LTC + SDM pipeline independent of the decoder."""

import torch
import torch.nn.functional as F
from soma_brain import SomaBrain, BrainConfig


def get_device():
    if torch.backends.mps.is_available():
        return torch.device("mps")
    if torch.cuda.is_available():
        return torch.device("cuda")
    return torch.device("cpu")


def main():
    device = get_device()
    print(f"device: {device}")

    data = torch.load("checkpoints/brain_base.pt", map_location=device, weights_only=False)
    config = data["config"]
    model = SomaBrain(config).to(device)
    model.load_state_dict(data["state_dict"], strict=False)

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

    addr_dim = model.sdm._addr_buf.shape[-1]
    print(f"SDM: {model.sdm.num_locations} entries, {addr_dim}-dim addresses")

    val_data = torch.load("data/val_qa.pt", map_location="cpu", weights_only=False)
    print(f"val: {len(val_data['answers'])} pairs")
    print(f"question embeddings: {val_data['embeddings'].shape[-1]}-dim")
    print(f"hidden_size: {config.hidden_size}, embed_dim: {config.embed_dim}\n")

    model.train(False)
    n = min(200, len(val_data["answers"]))

    addr_matrix = model.sdm._addr_buf[:model.sdm._count].to(device)
    addr_norm = F.normalize(addr_matrix, dim=-1)

    # Test 1: Direct SDM cosine sim (bypass all projections)
    print("=== Test 1: Direct cosine sim on SDM addresses ===")
    hits_raw = {1: 0, 3: 0, 5: 0, 8: 0}
    with torch.no_grad():
        for i in range(n):
            emb = val_data["embeddings"][i:i+1].to(device)
            projected = model.input_proj(emb)
            q = F.normalize(projected, dim=-1)
            sim = torch.matmul(q, addr_norm.T)
            topk_idx = torch.topk(sim, 8, dim=-1).indices[0]

            gold = val_data["answers"][i].lower()
            for rank, idx in enumerate(topk_idx):
                text = model.source_texts[idx.item()]
                if gold in text.lower():
                    for k in hits_raw:
                        if rank < k:
                            hits_raw[k] += 1
                    break

            if i < 3:
                print(f"  Q: {val_data['questions'][i][:70]}")
                print(f"  A: {val_data['answers'][i]}")
                idx0 = topk_idx[0].item()
                contains = "YES" if gold in model.source_texts[idx0].lower() else "no"
                print(f"  top1: sim={sim[0,idx0]:.3f} answer={contains} '{model.source_texts[idx0][:70]}...'")
                print()

    for k in sorted(hits_raw):
        print(f"  hit@{k}: {hits_raw[k]}/{n} = {hits_raw[k]/n:.1%}")

    # Test 2: Through SDM.query_proj (as model uses it)
    print(f"\n=== Test 2: Through SDM.query_proj ===")
    hits_proj = {1: 0, 3: 0, 5: 0, 8: 0}
    with torch.no_grad():
        for i in range(n):
            emb = val_data["embeddings"][i:i+1].to(device)
            projected = model.input_proj(emb)
            q = F.normalize(model.sdm.query_proj(projected), dim=-1)
            sim = torch.matmul(q, addr_norm.T)
            topk_idx = torch.topk(sim, 8, dim=-1).indices[0]
            gold = val_data["answers"][i].lower()
            for rank, idx in enumerate(topk_idx):
                if gold in model.source_texts[idx.item()].lower():
                    for k in hits_proj:
                        if rank < k:
                            hits_proj[k] += 1
                    break
    for k in sorted(hits_proj):
        print(f"  hit@{k}: {hits_proj[k]}/{n} = {hits_proj[k]/n:.1%}")

    # Test 3: Full forward pass (input_proj → liquid → SDM query)
    print(f"\n=== Test 3: Full forward pass ===")
    hits_full = {1: 0, 3: 0, 5: 0, 8: 0}
    with torch.no_grad():
        for i in range(n):
            emb = val_data["embeddings"][i:i+1].to(device)
            out = model.forward_train(emb)
            h = out["layer_outputs"][-1]
            q = F.normalize(model.sdm.query_proj(h), dim=-1)
            sim = torch.matmul(q, addr_norm.T)
            topk_idx = torch.topk(sim, 8, dim=-1).indices[0]
            gold = val_data["answers"][i].lower()
            for rank, idx in enumerate(topk_idx):
                if gold in model.source_texts[idx.item()].lower():
                    for k in hits_full:
                        if rank < k:
                            hits_full[k] += 1
                    break
    for k in sorted(hits_full):
        print(f"  hit@{k}: {hits_full[k]}/{n} = {hits_full[k]/n:.1%}")

    # Test 4: Reconstruction quality
    print(f"\n=== Test 4: Reconstruction ===")
    cos_sims = []
    with torch.no_grad():
        for i in range(n):
            emb = val_data["embeddings"][i:i+1].to(device)
            out = model.forward_train(emb)
            cos = F.cosine_similarity(out["reconstructed"], emb, dim=-1).item()
            cos_sims.append(cos)
    print(f"  mean cosine sim: {sum(cos_sims)/len(cos_sims):.4f}")
    print(f"  min: {min(cos_sims):.4f}, max: {max(cos_sims):.4f}")


if __name__ == "__main__":
    main()
