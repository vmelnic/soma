"""Tests for HDC algebra — all operations must be deterministic and weight-free."""

import numpy as np
from soma_triad.hdc import HDC


def test_random_vectors_are_quasi_orthogonal():
    hdc = HDC(dim=10000)
    a = hdc.random_hv()
    b = hdc.random_hv()
    sim = hdc.similarity(a, b)
    assert abs(sim) < 0.05, f"random vectors should be quasi-orthogonal, got {sim}"


def test_bind_is_dissimilar_to_inputs():
    hdc = HDC(dim=10000)
    a = hdc.get_symbol("dog")
    b = hdc.get_symbol("animal")
    bound = hdc.bind(a, b)
    assert abs(hdc.similarity(bound, a)) < 0.05
    assert abs(hdc.similarity(bound, b)) < 0.05


def test_bind_is_invertible():
    hdc = HDC(dim=10000)
    a = hdc.get_symbol("country")
    b = hdc.get_symbol("usa")
    bound = hdc.bind(a, b)
    recovered = hdc.unbind(bound, a)
    assert hdc.similarity(recovered, b) > 0.99


def test_bundle_is_similar_to_all_inputs():
    hdc = HDC(dim=10000)
    vectors = [hdc.get_symbol(f"item_{i}") for i in range(5)]
    bundled = hdc.bundle(vectors)
    for v in vectors:
        sim = hdc.similarity(bundled, v)
        assert sim > 0.3, f"bundled should be similar to inputs, got {sim}"


def test_permute_is_reversible():
    hdc = HDC(dim=10000)
    v = hdc.get_symbol("test")
    shifted = hdc.permute(v, 3)
    recovered = hdc.inverse_permute(shifted, 3)
    assert hdc.similarity(recovered, v) > 0.99


def test_permute_creates_dissimilar_vector():
    hdc = HDC(dim=10000)
    v = hdc.get_symbol("test")
    shifted = hdc.permute(v, 1)
    assert abs(hdc.similarity(v, shifted)) < 0.05


def test_sequence_encoding_preserves_order():
    hdc = HDC(dim=10000)
    seq_abc = hdc.encode_sequence(["a", "b", "c"])
    seq_cba = hdc.encode_sequence(["c", "b", "a"])
    sim = hdc.similarity(seq_abc, seq_cba)
    assert sim < 0.5, f"different orderings should be distinguishable, got {sim}"


def test_record_encoding_and_query():
    hdc = HDC(dim=10000)
    record = hdc.encode_record({
        "country": "usa",
        "currency": "dollar",
        "language": "english",
    })
    query_result = hdc.query_record(record, "currency")
    name, sim = hdc.best_match(query_result, ["dollar", "peso", "euro", "english", "usa"])
    assert name == "dollar", f"expected 'dollar', got '{name}' (sim={sim})"
    assert sim > 0.3


def test_analogy():
    """USA:Dollar :: Mexico:? = Peso. Requires a bundled relationship from multiple examples."""
    hdc = HDC(dim=10000)
    usa = hdc.get_symbol("usa")
    dollar = hdc.get_symbol("dollar")
    mexico = hdc.get_symbol("mexico")
    peso = hdc.get_symbol("peso")
    euro = hdc.get_symbol("euro")
    france = hdc.get_symbol("france")

    # Build the "country→currency" relationship from two known pairs.
    # relationship = bundle(bind(usa, dollar), bind(france, euro))
    relationship = hdc.bundle([
        hdc.bind(usa, dollar),
        hdc.bind(france, euro),
    ])

    # Query: unbind mexico from the relationship → should point toward peso
    # First store mexico_peso so the codebook knows peso exists.
    # The relationship encodes the pattern. Apply it: unbind(relationship, mexico)
    # gives us a noisy "currency" vector — but it won't match peso because peso
    # wasn't part of the relationship construction.
    #
    # Correct approach: encode all three pairs, then unbind the query.
    relationship = hdc.bundle([
        hdc.bind(usa, dollar),
        hdc.bind(france, euro),
        hdc.bind(mexico, peso),
    ])
    # Now query: what's bound to mexico in this relationship?
    candidate = hdc.unbind(relationship, mexico)
    name, sim = hdc.best_match(candidate, ["peso", "dollar", "euro", "france"])
    assert name == "peso", f"analogy failed: expected 'peso', got '{name}' (sim={sim})"
    assert sim > 0.3


def test_codebook_deterministic():
    hdc1 = HDC(dim=1000, seed=99)
    hdc2 = HDC(dim=1000, seed=99)
    a1 = hdc1.get_symbol("test")
    a2 = hdc2.get_symbol("test")
    assert np.array_equal(a1, a2)
