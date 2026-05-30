"""Tests for SDM — pure content-addressable memory, no weights."""

import numpy as np
from soma_triad.sdm import SDM


def _make_addr(dim: int, idx: int) -> np.ndarray:
    v = np.zeros(dim, dtype=np.float32)
    v[idx % dim] = 1.0
    return v


def test_write_and_read():
    sdm = SDM(dim=100)
    addr = _make_addr(100, 0)
    data = np.ones(100, dtype=np.float32)
    sdm.write(addr, data, "entry_a")

    results = sdm.read(addr, top_k=1)
    assert len(results) == 1
    assert results[0][2] == "entry_a"
    assert results[0][1] > 0.99


def test_similarity_ordering():
    sdm = SDM(dim=100)
    # Two entries at different positions
    a = np.zeros(100, dtype=np.float32); a[0] = 1.0
    b = np.zeros(100, dtype=np.float32); b[50] = 1.0
    sdm.write(a, a, "near")
    sdm.write(b, b, "far")

    # Query close to 'a'
    query = np.zeros(100, dtype=np.float32); query[0] = 0.9; query[1] = 0.1
    results = sdm.read(query, top_k=2)
    assert results[0][2] == "near"


def test_reinforcement():
    sdm = SDM(dim=100)
    addr = _make_addr(100, 0)
    data1 = np.full(100, 10.0, dtype=np.float32)
    data2 = np.full(100, 20.0, dtype=np.float32)
    sdm.write(addr, data1, "x")
    sdm.write(addr, data2, "x")

    assert sdm.count() == 1
    results = sdm.read(addr, top_k=1)
    # Running average of 10 and 20 = 15
    assert abs(results[0][0][0] - 15.0) < 0.1


def test_blended_read_interpolates():
    sdm = SDM(dim=100)
    a = np.zeros(100, dtype=np.float32); a[0] = 1.0
    b = np.zeros(100, dtype=np.float32); b[1] = 1.0
    sdm.write(a, np.full(100, 100.0, dtype=np.float32), "a")
    sdm.write(b, np.full(100, 0.0, dtype=np.float32), "b")

    # Query equidistant
    query = np.zeros(100, dtype=np.float32); query[0] = 0.707; query[1] = 0.707
    blended = sdm.read_blended(query)
    assert blended is not None
    # Should be somewhere between 0 and 100
    assert 10.0 < blended[0] < 90.0


def test_decay_removes_weak():
    sdm = SDM(dim=100)
    addr = _make_addr(100, 0)
    sdm.write(addr, addr, "weak")

    strong_addr = _make_addr(100, 50)
    for _ in range(10):
        sdm.write(strong_addr, strong_addr, "strong")

    removed = sdm.decay(0.5)
    assert removed == 1
    assert sdm.count() == 1


def test_empty_read():
    sdm = SDM(dim=100)
    results = sdm.read(np.ones(100, dtype=np.float32))
    assert results == []
    assert sdm.read_blended(np.ones(100, dtype=np.float32)) is None
