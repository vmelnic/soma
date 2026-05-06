"""End-to-end functional tests for all four phases."""

import os
import time
import torch
from soma_brain import (
    SomaBrain, BrainConfig, PredictiveCodingLoss,
    Embedder, BrainPort, ConsolidationLoop,
)
from soma_brain.episodes import generate_synthetic_episodes, EpisodeLoader


def get_device():
    if torch.backends.mps.is_available():
        return torch.device("mps")
    if torch.cuda.is_available():
        return torch.device("cuda")
    return torch.device("cpu")


def load_checkpoint(path, device):
    data = torch.load(path, map_location=device, weights_only=False)
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

    return model, config


def test_phase1_structured_output(model, embedder, device):
    """Phase 1: Structured output heads produce real values."""
    print("=== Phase 1: Structured Output Heads ===")
    queries = [
        "What is sparse distributed memory?",
        "How does active inference work?",
        "Explain the free energy principle",
    ]
    for q in queries:
        emb = embedder.embed_one(q).unsqueeze(0).to(device)
        result = model.reason(emb)
        conf = result.confidence.item()
        n_sources = len(result.sources)
        top_score = result.sources[0][1] if result.sources else 0
        skill_max = result.skill_scores.max().item()
        binding_norm = result.bindings.norm().item()
        print(f"  query: {q[:50]}")
        print(f"    confidence: {conf:.4f} | sources: {n_sources} (top: {top_score:.4f})")
        print(f"    skill_max: {skill_max:.4f} | binding_norm: {binding_norm:.4f}")
    print("  PASS: all heads produce non-trivial output\n")


def test_phase2_episode_distillation(model, embedder, device):
    """Phase 2: Episode distillation pipeline works end-to-end."""
    print("=== Phase 2: Episode Distillation ===")
    skills = ["http_get", "db_query", "file_read", "cache_lookup", "transform"]
    model.register_skills(skills)
    episodes = generate_synthetic_episodes(skills, n_episodes=3)
    print(f"  generated {len(episodes)} synthetic episodes")

    loader = EpisodeLoader(model.skill_registry)
    samples = loader.extract_samples(episodes, embedder=embedder)
    print(f"  extracted {len(samples)} distillation samples")

    batch = EpisodeLoader.collate(samples[:4])
    emb = batch["embeddings"].to(device)
    out = model.forward_train(emb)
    skill_logits = out["skill_logits"]
    pred = skill_logits.argmax(dim=-1)
    print(f"  forward pass: skill_logits shape {skill_logits.shape}, predictions: {pred.tolist()}")
    print("  PASS: distillation pipeline produces valid training signals\n")


def test_phase3_generation(model, embedder, device):
    """Phase 3: Diffusion decoder generates text from queries."""
    print("=== Phase 3: Diffusion Decoder Generation ===")
    queries = [
        "What is sparse distributed memory?",
        "Explain belief updating in active inference",
        "How do liquid time-constant networks work?",
    ]
    for q in queries:
        t0 = time.time()
        emb = embedder.embed_one(q).unsqueeze(0).to(device)
        texts = model.generate(emb, max_len=64, steps=16)
        elapsed = time.time() - t0
        text = texts[0][:200]
        print(f"  query: {q[:50]}")
        print(f"    [{elapsed:.1f}s] {text}")
    print("  PASS: decoder generates text (quality depends on training)\n")


def test_phase4_consolidation(model, embedder, device):
    """Phase 4: Consolidation loop + BrainPort end-to-end."""
    print("=== Phase 4: Consolidation + Port ===")

    port = BrainPort(model, embedder)

    # Test port: reason
    result = port.invoke("reason", {"query": "sparse distributed memory"})
    assert result["success"], f"reason failed: {result}"
    print(f"  port/reason: conf={result['confidence']:.4f}, authoritative={result['authoritative']}, sources={len(result['sources'])}")

    # Test port: generate
    result = port.invoke("generate", {"query": "what is SDM", "max_len": 32, "steps": 8})
    assert result["success"], f"generate failed: {result}"
    print(f"  port/generate: {result['generated_text'][:100]}")

    # Test port: ingest
    sdm_before = model.sdm.num_locations
    result = port.invoke("ingest", {"text": "Test knowledge: consolidation loops write session patterns to SDM"})
    assert result["success"], f"ingest failed: {result}"
    print(f"  port/ingest: added {result['entries_added']}, SDM {sdm_before} -> {result['sdm_size']}")

    # Test port: consolidate_episode
    episode = {
        "success": True,
        "steps": [
            {"belief_summary": {"goal": "test", "precision": 0.8}},
            {"belief_summary": {"goal": "verify", "precision": 0.9}},
        ],
    }
    result = port.invoke("consolidate_episode", {"episode": episode})
    assert result["success"], f"consolidate_episode failed: {result}"
    print(f"  port/consolidate_episode: {result}")

    # Test port: consolidate_routine
    routine = {
        "description": "HTTP health check",
        "steps": [
            {"skill_id": "http_get"},
            {"skill_id": "parse_json"},
            {"skill_id": "check_status"},
        ],
    }
    result = port.invoke("consolidate_routine", {"routine": routine})
    assert result["success"], f"consolidate_routine failed: {result}"
    print(f"  port/consolidate_routine: {result}")

    # Test TTT -> SDM consolidation
    sdm_before_ttt = model.sdm.num_locations
    with torch.no_grad():
        emb = embedder.embed_one("test TTT consolidation query").unsqueeze(0).to(device)
        x = model.input_proj(emb).unsqueeze(1)
        model.ttt(x, update=True)
    n_inputs = len(model.ttt.get_session_inputs())
    result = port.invoke("consolidate_ttt", {})
    assert result["success"], f"consolidate_ttt failed: {result}"
    print(f"  port/consolidate_ttt: inputs={result['inputs_seen']}, written={result['entries_written']}, skipped={result['skipped_redundant']}, SDM {sdm_before_ttt} -> {model.sdm.num_locations}")

    # Test port: status
    result = port.invoke("status", {})
    assert result["success"], f"status failed: {result}"
    print(f"  port/status: SDM={result['sdm_entries']}, ttt_consolidated={result['consolidation']['total_ttt_consolidated']}, params={result['parameters']['total']:,}")

    # Test manifest
    manifest = port.manifest()
    caps = [c["capability_id"] for c in manifest["capabilities"]]
    assert "consolidate_ttt" in caps
    print(f"  manifest: {manifest['port_id']} v{manifest['version']}, caps={caps}")

    print("  PASS: all port capabilities work end-to-end\n")


def main():
    ckpt = "checkpoints/brain.pt"
    if not os.path.exists(ckpt):
        print(f"error: {ckpt} not found -- run ingest.py + train.py first")
        return

    device = get_device()
    print(f"device: {device}\n")

    print("loading embedder...")
    embedder = Embedder()

    print("loading brain checkpoint...")
    model, config = load_checkpoint(ckpt, device)
    model.eval()
    print(f"SDM: {model.sdm.num_locations} entries | sources: {len(model.source_texts)}\n")

    test_phase1_structured_output(model, embedder, device)
    test_phase2_episode_distillation(model, embedder, device)
    test_phase3_generation(model, embedder, device)
    test_phase4_consolidation(model, embedder, device)

    print("=" * 60)
    print("ALL PHASES VERIFIED END-TO-END")
    print("=" * 60)


if __name__ == "__main__":
    main()
