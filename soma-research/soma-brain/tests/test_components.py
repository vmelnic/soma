"""Unit tests for SOMA Brain components."""

import torch
import pytest

from soma_brain import (
    BrainConfig, SomaBrain, LiquidLayer, LTCCell,
    SparseDistributedMemory, TTTLayer, MemoryAttention, ReasoningBlock,
    DiffusionDecoder, SpanExtractor, EpisodeLoader, generate_synthetic_episodes,
    ConsolidationLoop, BrainPort,
)


@pytest.fixture
def config():
    return BrainConfig.tiny()


@pytest.fixture
def device():
    return torch.device("cpu")


class TestLTCCell:
    def test_forward_shape(self):
        cell = LTCCell(64, 128)
        x = torch.randn(4, 64)
        h = torch.zeros(4, 128)
        h_new = cell(x, h)
        assert h_new.shape == (4, 128)

    def test_state_changes(self):
        cell = LTCCell(64, 128)
        x = torch.randn(4, 64)
        h = torch.zeros(4, 128)
        h1 = cell(x, h)
        h2 = cell(x, h1)
        assert not torch.allclose(h1, h2)

    def test_time_constant_varies(self):
        cell = LTCCell(64, 128)
        x1 = torch.randn(1, 64)
        x2 = torch.randn(1, 64) * 5
        h = torch.zeros(1, 128)
        h1 = cell(x1, h)
        h2 = cell(x2, h)
        assert not torch.allclose(h1, h2)


class TestLiquidLayer:
    def test_output_shape(self):
        layer = LiquidLayer(64, 128, ode_steps=2)
        x = torch.randn(4, 10, 64)
        out, h = layer(x)
        assert out.shape == (4, 10, 128)
        assert h.shape == (4, 128)

    def test_gradient_flows(self):
        layer = LiquidLayer(64, 64, ode_steps=2)
        x = torch.randn(4, 5, 64, requires_grad=True)
        out, _ = layer(x)
        out.sum().backward()
        assert x.grad is not None
        assert x.grad.abs().sum() > 0


class TestSDM:
    def test_empty_read(self):
        sdm = SparseDistributedMemory(64, 128, top_k=4)
        q = torch.randn(4, 64)
        result = sdm.read(q)
        assert result.shape == (4, 128)
        assert result.abs().sum() == 0

    def test_read_topk_shape(self):
        sdm = SparseDistributedMemory(64, 128, top_k=4)
        sdm.write(torch.randn(10, 64), torch.randn(10, 128))
        q = torch.randn(4, 64)
        entries, scores = sdm.read_topk(q)
        assert entries.shape == (4, 4, 128)
        assert scores.shape == (4, 4)

    def test_write_then_read(self):
        sdm = SparseDistributedMemory(64, 128, top_k=4)
        addr = torch.randn(1, 64)
        data = torch.ones(1, 128) * 42.0
        sdm.write(addr, data)
        retrieved = sdm.read(addr)
        assert retrieved.abs().sum() > 0

    def test_dynamic_growth(self):
        sdm = SparseDistributedMemory(64, 128, top_k=4)
        assert sdm.num_locations == 0
        sdm.write(torch.randn(5, 64), torch.randn(5, 128))
        assert sdm.num_locations == 5
        sdm.write(torch.randn(3, 64), torch.randn(3, 128))
        assert sdm.num_locations == 8

    def test_clear(self):
        sdm = SparseDistributedMemory(64, 128, top_k=4)
        sdm.write(torch.randn(5, 64), torch.randn(5, 128))
        sdm.clear()
        assert sdm.num_locations == 0

    def test_similar_addresses_retrieve_similar(self):
        torch.manual_seed(42)
        sdm = SparseDistributedMemory(64, 128, top_k=4)
        addr = torch.randn(1, 64)
        data = torch.ones(1, 128) * 10.0
        sdm.write(addr, data)
        sdm.write(torch.randn(20, 64), torch.randn(20, 128))
        near = addr + torch.randn(1, 64) * 0.01
        _, scores_near = sdm.read_topk(near)
        _, scores_far = sdm.read_topk(torch.randn(1, 64))
        assert scores_near.max() > scores_far.max()


class TestMemoryAttention:
    def test_output_shape(self):
        ma = MemoryAttention(128, num_heads=4)
        state = torch.randn(2, 10, 128)
        memories = torch.randn(2, 8, 128)
        out = ma(state, memories)
        assert out.shape == (2, 10, 128)

    def test_gradient_flows(self):
        ma = MemoryAttention(128, num_heads=4)
        state = torch.randn(2, 5, 128, requires_grad=True)
        memories = torch.randn(2, 8, 128, requires_grad=True)
        out = ma(state, memories)
        out.sum().backward()
        assert state.grad is not None
        assert memories.grad is not None

    def test_different_memories_give_different_output(self):
        ma = MemoryAttention(128, num_heads=4)
        state = torch.randn(1, 5, 128)
        m1 = torch.randn(1, 4, 128)
        m2 = torch.randn(1, 4, 128) * 3
        out1 = ma(state, m1)
        out2 = ma(state, m2)
        assert not torch.allclose(out1, out2)


class TestReasoningBlock:
    def test_output_shape(self, config):
        block = ReasoningBlock(config.hidden_size, config.ode_steps, config.num_attn_heads)
        sdm = SparseDistributedMemory(config.hidden_size, config.hidden_size, config.sdm_top_k)
        sdm.write(torch.randn(10, config.hidden_size), torch.randn(10, config.hidden_size))
        x = torch.randn(2, 10, config.hidden_size)
        out, h = block(x, sdm)
        assert out.shape == (2, 10, config.hidden_size)

    def test_multi_hop_changes_output(self, config):
        sdm = SparseDistributedMemory(config.hidden_size, config.hidden_size, config.sdm_top_k)
        sdm.write(torch.randn(10, config.hidden_size), torch.randn(10, config.hidden_size))

        b1 = ReasoningBlock(config.hidden_size, config.ode_steps, config.num_attn_heads)
        b2 = ReasoningBlock(config.hidden_size, config.ode_steps, config.num_attn_heads)

        x = torch.randn(1, 8, config.hidden_size)
        out1, _ = b1(x, sdm)
        out2, _ = b2(out1, sdm)
        assert not torch.allclose(out1, out2)


class TestTTT:
    def test_output_shape(self):
        ttt = TTTLayer(128, 64, lr=0.01)
        x = torch.randn(4, 10, 128)
        out = ttt(x)
        assert out.shape == (4, 10, 128)

    def test_update_changes_output(self):
        ttt = TTTLayer(128, 64, lr=0.1)
        x = torch.randn(1, 20, 128)
        out_with = ttt(x, update=True)
        out_without = ttt(x, update=False)
        assert not torch.allclose(out_with, out_without, atol=1e-5)

    def test_gradient_flows(self):
        ttt = TTTLayer(128, 64, lr=0.01)
        x = torch.randn(4, 5, 128, requires_grad=True)
        out = ttt(x)
        out.sum().backward()
        assert x.grad is not None


class TestFullBrain:
    def test_reason_shape(self, config, device):
        model = SomaBrain(config).to(device)
        emb = torch.randn(2, config.embed_dim, device=device)
        result = model.reason(emb)
        assert result.hidden.shape == (2, config.hidden_size)
        assert result.retrieved_entries.shape == (2, config.sdm_top_k, config.hidden_size)
        assert result.retrieved_scores.shape == (2, config.sdm_top_k)
        assert result.skill_scores.shape == (2, config.max_skills)
        assert result.bindings.shape == (2, config.binding_dim)
        assert result.confidence.shape == (2, 1)
        assert (result.confidence >= 0).all() and (result.confidence <= 1).all()

    def test_ingest(self, config, device):
        model = SomaBrain(config).to(device)
        emb = torch.randn(3, config.embed_dim, device=device)
        assert model.sdm.num_locations == 0
        n = model.ingest(emb)
        assert n == 3
        assert model.sdm.num_locations == 3

    def test_ingest_then_reason(self, config, device):
        model = SomaBrain(config).to(device)
        emb = torch.randn(5, config.embed_dim, device=device)
        model.ingest(emb)
        result = model.reason(emb[:1])
        assert result.retrieved_entries.abs().sum() > 0
        assert result.retrieved_scores.abs().sum() > 0

    def test_ingest_with_source_text(self, config, device):
        model = SomaBrain(config).to(device)
        emb = torch.randn(1, config.embed_dim, device=device)
        model.ingest(emb, text="hello world")
        assert len(model.source_texts) == 1
        assert model.source_texts[0] == "hello world"

    def test_different_queries_different_retrieval(self, config, device):
        model = SomaBrain(config).to(device)
        emb = torch.randn(10, config.embed_dim, device=device)
        model.ingest(emb)
        r1 = model.reason(emb[:1])
        r2 = model.reason(emb[5:6])
        assert not torch.allclose(r1.retrieved_scores, r2.retrieved_scores)

    def test_forward_train_outputs(self, config, device):
        model = SomaBrain(config).to(device)
        emb = torch.randn(4, config.embed_dim, device=device)
        model.ingest(emb)
        out = model.forward_train(emb)
        assert out["reconstructed"].shape == (4, config.embed_dim)
        assert len(out["layer_outputs"]) == config.num_layers + 1
        assert out["skill_logits"].shape == (4, config.max_skills)
        assert out["bindings"].shape == (4, config.binding_dim)
        assert out["confidence"].shape == (4, 1)

    def test_heads_gradient_flow(self, config, device):
        model = SomaBrain(config).to(device)
        emb = torch.randn(2, config.embed_dim, device=device)
        model.ingest(emb)
        out = model.forward_train(emb)
        loss = out["skill_logits"].sum() + out["bindings"].sum() + out["confidence"].sum()
        loss.backward()
        for name, p in model.named_parameters():
            if "skill_head" in name or "binding_head" in name or "confidence_head" in name:
                assert p.grad is not None, f"no gradient for {name}"

    def test_skill_registry(self, config, device):
        model = SomaBrain(config).to(device)
        skills = ["soma.ports.git.status", "soma.ports.git.diff", "soma.ports.fs.read"]
        model.register_skills(skills)
        emb = torch.randn(1, config.embed_dim, device=device)
        result = model.reason(emb)
        decoded = model.decode_skill(result.skill_scores, top_k=3)
        assert len(decoded) == 3
        assert all(isinstance(s, str) and isinstance(v, float) for s, v in decoded)
        assert all(s in skills for s, _ in decoded)

    def test_generate_text(self, config, device):
        model = SomaBrain(config).to(device)
        emb = torch.randn(5, config.embed_dim, device=device)
        model.ingest(emb)
        texts = model.generate(emb[:1], max_len=32, steps=4)
        assert len(texts) == 1
        assert isinstance(texts[0], str)
        assert len(texts[0]) > 0

    def test_parameter_count(self, config):
        model = SomaBrain(config)
        params = model.count_parameters()
        assert params["total_unique"] > 0
        assert params["reasoning_blocks"] > 0
        assert params["ttt"] > 0


class TestSpanExtractor:
    def test_forward_shape(self):
        ext = SpanExtractor(hidden_size=128, cond_size=256, num_heads=4, num_encoder_layers=2)
        ctx_ids = torch.randint(0, 256, (2, 64))
        ctx_len = torch.tensor([50, 64])
        cond = torch.randn(2, 5, 256)
        start_logits, end_logits = ext(ctx_ids, ctx_len, cond)
        assert start_logits.shape == (2, 64)
        assert end_logits.shape == (2, 64)

    def test_padding_masked(self):
        ext = SpanExtractor(hidden_size=128, cond_size=256, num_heads=4, num_encoder_layers=2)
        ctx_ids = torch.randint(0, 256, (1, 32))
        ctx_len = torch.tensor([10])
        cond = torch.randn(1, 3, 256)
        start_logits, end_logits = ext(ctx_ids, ctx_len, cond)
        assert start_logits[0, 10:].max() < -1e8
        assert end_logits[0, 10:].max() < -1e8

    def test_extract_returns_spans(self):
        ext = SpanExtractor(hidden_size=128, cond_size=256, num_heads=4, num_encoder_layers=2)
        ctx_ids = torch.randint(0, 256, (2, 64))
        ctx_len = torch.tensor([50, 64])
        cond = torch.randn(2, 5, 256)
        spans = ext.extract(ctx_ids, ctx_len, cond)
        assert len(spans) == 2
        for s, e in spans:
            assert 0 <= s <= e

    def test_gradient_flows(self):
        ext = SpanExtractor(hidden_size=128, cond_size=256, num_heads=4, num_encoder_layers=2)
        ctx_ids = torch.randint(0, 256, (2, 32))
        ctx_len = torch.tensor([32, 32])
        cond = torch.randn(2, 3, 256, requires_grad=True)
        start_logits, end_logits = ext(ctx_ids, ctx_len, cond)
        loss = start_logits.sum() + end_logits.sum()
        loss.backward()
        assert cond.grad is not None


class TestDiffusionDecoder:
    def test_forward_shape(self):
        dec = DiffusionDecoder(vocab_size=1000, hidden_size=128, num_layers=2, num_heads=4, cond_size=256, max_seq_len=64)
        token_ids = torch.randint(0, 1000, (2, 32))
        t = torch.full((2, 1), 0.5)
        cond = torch.randn(2, 1, 256)
        logits = dec(token_ids, t, cond)
        assert logits.shape == (2, 32, 1000)

    def test_compute_loss(self):
        dec = DiffusionDecoder(vocab_size=1000, hidden_size=128, num_layers=2, num_heads=4, cond_size=256, max_seq_len=64)
        clean_ids = torch.randint(0, 1000, (2, 32))
        cond = torch.randn(2, 1, 256)
        loss = dec.compute_loss(clean_ids, cond)
        assert loss.shape == ()
        assert loss.item() > 0

    def test_generate_shape(self):
        dec = DiffusionDecoder(vocab_size=1000, hidden_size=128, num_layers=2, num_heads=4, cond_size=256, max_seq_len=64)
        cond = torch.randn(2, 1, 256)
        ids = dec.generate(cond, seq_len=32, steps=4)
        assert ids.shape == (2, 32)
        assert (ids >= 0).all() and (ids < 1000).all()

    def test_gradient_flows(self):
        dec = DiffusionDecoder(vocab_size=1000, hidden_size=128, num_layers=2, num_heads=4, cond_size=256, max_seq_len=64)
        clean_ids = torch.randint(0, 1000, (2, 32))
        cond = torch.randn(2, 1, 256, requires_grad=True)
        loss = dec.compute_loss(clean_ids, cond)
        loss.backward()
        assert cond.grad is not None

    def test_compute_loss_with_lengths(self):
        dec = DiffusionDecoder(vocab_size=1000, hidden_size=128, num_layers=2, num_heads=4, cond_size=256, max_seq_len=64)
        clean_ids = torch.randint(0, 1000, (2, 32))
        clean_ids[0, 5:] = 0
        clean_ids[1, 10:] = 0
        lengths = torch.tensor([5, 10])
        cond = torch.randn(2, 1, 256)
        loss = dec.compute_loss(clean_ids, cond, lengths=lengths)
        assert loss.shape == ()
        assert loss.item() > 0

    def test_compute_loss_lengths_none_is_backward_compatible(self):
        dec = DiffusionDecoder(vocab_size=1000, hidden_size=128, num_layers=2, num_heads=4, cond_size=256, max_seq_len=64)
        clean_ids = torch.randint(0, 1000, (2, 32))
        cond = torch.randn(2, 1, 256)
        loss = dec.compute_loss(clean_ids, cond, lengths=None)
        assert loss.shape == ()
        assert loss.item() > 0


class TestEpisodes:
    def test_synthetic_generation(self):
        skills = ["soma.ports.git.status", "soma.ports.fs.read", "soma.ports.http.get"]
        episodes = generate_synthetic_episodes(skills, n_episodes=10)
        assert len(episodes) == 10
        for ep in episodes:
            assert "success" in ep
            assert "steps" in ep
            assert len(ep["steps"]) >= 2

    def test_episode_loader(self, tmp_path):
        import json
        skills = ["skill_a", "skill_b"]
        episodes = generate_synthetic_episodes(skills, n_episodes=5)
        ep_file = tmp_path / "episodes.json"
        ep_file.write_text(json.dumps(episodes))

        loader = EpisodeLoader(skills)
        loaded = loader.load_episodes(ep_file)
        assert len(loaded) == 5

    def test_collate(self):
        from soma_brain import DistillationSample
        samples = [
            DistillationSample(query_embedding=torch.randn(128), skill_index=0, outcome_score=1.0, step_context="a"),
            DistillationSample(query_embedding=torch.randn(128), skill_index=1, outcome_score=-0.5, step_context="b"),
        ]
        batch = EpisodeLoader.collate(samples)
        assert batch["embeddings"].shape == (2, 128)
        assert batch["skill_targets"].shape == (2,)
        assert batch["outcome_scores"].shape == (2,)


class TestConsolidation:
    def test_session_complete_writes_sdm(self, config, device):
        model = SomaBrain(config).to(device)
        initial = model.sdm.num_locations
        loop = ConsolidationLoop(model)
        episode = {"success": True, "steps": [
            {"belief_summary": {"action": "read_file"}},
            {"belief_summary": {"action": "parse_data"}},
        ]}
        # Without embedder, no writebacks
        stats = loop.on_session_complete(episode)
        assert model.sdm.num_locations == initial

    def test_failed_session_no_writeback(self, config, device):
        model = SomaBrain(config).to(device)
        loop = ConsolidationLoop(model)
        episode = {"success": False, "steps": [{"belief_summary": {}}]}
        stats = loop.on_session_complete(episode)
        assert stats["sdm_new_entries"] == 0

    def test_confidence_measurement(self, config, device):
        model = SomaBrain(config).to(device)
        emb = torch.randn(5, config.embed_dim, device=device)
        model.ingest(emb)
        loop = ConsolidationLoop(model)
        conf = loop.measure_confidence(emb)
        assert 0.0 <= conf <= 1.0

    def test_stats(self, config, device):
        model = SomaBrain(config).to(device)
        loop = ConsolidationLoop(model)
        stats = loop.get_stats()
        assert stats["sessions_observed"] == 0

    def test_ttt_consolidation_writes_novel_patterns(self, config, device):
        model = SomaBrain(config).to(device)
        emb = torch.randn(2, config.embed_dim, device=device)
        model.ingest(emb)
        initial_sdm = model.sdm.num_locations

        x = torch.randn(1, 1, config.hidden_size, device=device)
        model.ttt(x, update=True)
        assert len(model.ttt.get_session_inputs()) > 0

        loop = ConsolidationLoop(model, ttt_consolidation_threshold=0.5)
        stats = loop.consolidate_ttt()
        assert stats["inputs_seen"] > 0
        assert stats["entries_written"] + stats["skipped_redundant"] == stats["inputs_seen"]
        assert model.sdm.num_locations >= initial_sdm

    def test_ttt_consolidation_resets_state(self, config, device):
        model = SomaBrain(config).to(device)
        x = torch.randn(1, 1, config.hidden_size, device=device)
        model.ttt(x, update=True)
        assert len(model.ttt.get_session_inputs()) > 0

        loop = ConsolidationLoop(model)
        loop.consolidate_ttt()
        assert len(model.ttt.get_session_inputs()) == 0
        assert model.ttt._session_W is None

    def test_ttt_consolidation_skips_redundant(self, config, device):
        model = SomaBrain(config).to(device)
        emb = torch.randn(1, config.embed_dim, device=device)
        model.ingest(emb)

        h = model.input_proj(emb).unsqueeze(1)
        model.ttt(h, update=True)

        loop = ConsolidationLoop(model, ttt_consolidation_threshold=0.99)
        stats = loop.consolidate_ttt()
        assert stats["skipped_redundant"] >= 0

    def test_session_complete_includes_ttt(self, config, device):
        model = SomaBrain(config).to(device)
        x = torch.randn(1, 1, config.hidden_size, device=device)
        model.ttt(x, update=True)

        loop = ConsolidationLoop(model)
        episode = {"success": True, "steps": []}
        stats = loop.on_session_complete(episode)
        assert "ttt_consolidated" in stats


class TestBrainPort:
    def test_manifest(self, config, device):
        model = SomaBrain(config).to(device)
        from soma_brain import Embedder
        # Use a mock embedder for tests
        class MockEmbedder:
            embed_dim = config.embed_dim
            def embed_one(self, text):
                return torch.randn(config.embed_dim)
        port = BrainPort(model, MockEmbedder())
        m = port.manifest()
        assert m["port_id"] == "brain"
        cap_ids = [c["capability_id"] for c in m["capabilities"]]
        assert "reason" in cap_ids
        assert "generate" in cap_ids
        assert "ingest" in cap_ids
        assert "status" in cap_ids

    def test_invoke_status(self, config, device):
        model = SomaBrain(config).to(device)
        class MockEmbedder:
            embed_dim = config.embed_dim
            def embed_one(self, text):
                return torch.randn(config.embed_dim)
        port = BrainPort(model, MockEmbedder())
        result = port.invoke("status", {})
        assert result["success"] is True
        assert "sdm_entries" in result
        assert "parameters" in result

    def test_invoke_ingest(self, config, device):
        model = SomaBrain(config).to(device)
        class MockEmbedder:
            embed_dim = config.embed_dim
            def embed_one(self, text):
                return torch.randn(config.embed_dim)
        port = BrainPort(model, MockEmbedder())
        result = port.invoke("ingest", {"text": "hello world"})
        assert result["success"] is True
        assert result["entries_added"] == 1

    def test_invoke_reason(self, config, device):
        model = SomaBrain(config).to(device)
        class MockEmbedder:
            embed_dim = config.embed_dim
            def embed_one(self, text):
                return torch.randn(config.embed_dim)
        port = BrainPort(model, MockEmbedder())
        port.invoke("ingest", {"text": "some knowledge"})
        result = port.invoke("reason", {"query": "test query"})
        assert result["success"] is True
        assert "confidence" in result
        assert "authoritative" in result

    def test_invoke_unknown(self, config, device):
        model = SomaBrain(config).to(device)
        class MockEmbedder:
            embed_dim = config.embed_dim
            def embed_one(self, text):
                return torch.randn(config.embed_dim)
        port = BrainPort(model, MockEmbedder())
        result = port.invoke("nonexistent", {})
        assert result["success"] is False
