"""Tests for the Triad orchestrator — LTC + HDC + SDM wired together."""

import numpy as np
from soma_triad.triad import Triad


def test_store_and_retrieve_episode():
    triad = Triad(dim=10000)
    triad.store_episode(
        context={"target": "hostile", "range": "close", "iff": "confirmed"},
        outcome={"action": "intercept", "result": "success"},
        label="intercept_1",
    )
    triad.store_episode(
        context={"target": "friendly", "range": "close", "iff": "confirmed"},
        outcome={"action": "abort", "result": "success"},
        label="abort_1",
    )
    assert triad.knowledge_count() == 2


def test_reason_returns_result():
    triad = Triad(dim=10000)
    triad.store_episode(
        context={"domain": "file", "operation": "read"},
        outcome={"skill": "filesystem_read", "result": "success"},
        label="ep_1",
    )
    result = triad.reason({"domain": "file", "operation": "read"}, max_steps=5)
    assert "result" in result
    assert "best_match" in result
    assert "trace" in result
    assert result["steps"] <= 5


def test_controller_param_count_is_tiny():
    triad = Triad(dim=10000, hidden_size=128)
    params = triad.param_count()
    # Should be well under 10M
    assert params < 10_000_000, f"controller too large: {params} params"
    # For dim=10000, hidden=128: roughly 10000*128 + 128*128 + 128*9 + 128*10000 ≈ 2.6M
    assert params > 100_000, f"controller suspiciously small: {params} params"


def test_triad_with_multiple_domains():
    """Same triad, different domains — SDM retrieves domain-appropriate episodes."""
    triad = Triad(dim=10000)

    # Coder domain
    triad.store_episode(
        context={"domain": "code", "event": "file_changed"},
        outcome={"skill": "git_status", "result": "success"},
        label="code_1",
    )
    triad.store_episode(
        context={"domain": "code", "event": "test_failed"},
        outcome={"skill": "run_tests", "result": "success"},
        label="code_2",
    )

    # Kitchen domain
    triad.store_episode(
        context={"domain": "kitchen", "event": "object_moved"},
        outcome={"skill": "scan_area", "result": "success"},
        label="kitchen_1",
    )
    triad.store_episode(
        context={"domain": "kitchen", "event": "drawer_stuck"},
        outcome={"skill": "force_open", "result": "success"},
        label="kitchen_2",
    )

    # Query code domain — SDM should retrieve code episodes
    code_query = triad.hdc.encode_record({"domain": "code", "event": "file_changed"})
    matches = triad.sdm.read(code_query, top_k=2)
    labels = [m[2] for m in matches]
    assert "code_1" in labels, f"expected code_1 in top-2, got {labels}"

    # Query kitchen domain — SDM should retrieve kitchen episodes
    kitchen_query = triad.hdc.encode_record({"domain": "kitchen", "event": "object_moved"})
    matches = triad.sdm.read(kitchen_query, top_k=2)
    labels = [m[2] for m in matches]
    assert "kitchen_1" in labels, f"expected kitchen_1 in top-2, got {labels}"


def test_hdc_no_learnable_params():
    """HDC must have zero parameters."""
    triad = Triad(dim=10000)
    # HDC is pure numpy — no torch parameters
    assert not hasattr(triad.hdc, "parameters")


def test_sdm_no_learnable_params():
    """SDM must have zero parameters."""
    triad = Triad(dim=10000)
    assert not hasattr(triad.sdm, "parameters")
