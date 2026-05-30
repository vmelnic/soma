"""
Proof: Triad selects correct Axis constructs for a backend spec.

Scenario: store several known backend patterns in SDM (CRUD user, CRUD order,
auth flow, etc.), then query with a new spec and verify the triad retrieves
the structurally correct construct sequence.

This proves: SDM generalizes from stored examples, HDC encodes domain structure,
and the system selects appropriate constructs without any LLM or transformer.
"""

import numpy as np
from soma_triad.triad import Triad


def make_axis_triad() -> Triad:
    """Build a triad pre-loaded with Axis backend patterns."""
    triad = Triad(dim=10000)

    # --- Store known Axis patterns as episodes ---

    # Pattern: CRUD entity (the most common backend pattern)
    triad.store_episode(
        context={"intent": "crud", "entity": "user", "fields": "id_email_name", "auth": "session"},
        outcome={"constructs": "shape_source_flow_list_flow_get_flow_create_flow_update_flow_delete", "realm": "yes", "auth": "session"},
        label="crud_user",
    )
    triad.store_episode(
        context={"intent": "crud", "entity": "product", "fields": "id_name_price_stock", "auth": "session"},
        outcome={"constructs": "shape_source_flow_list_flow_get_flow_create_flow_update_flow_delete", "realm": "yes", "auth": "session"},
        label="crud_product",
    )
    triad.store_episode(
        context={"intent": "crud", "entity": "order", "fields": "id_user_id_items_total_status", "auth": "session"},
        outcome={"constructs": "shape_source_flow_list_flow_get_flow_create_flow_update", "realm": "yes", "auth": "session"},
        label="crud_order",
    )

    # Pattern: Auth setup
    triad.store_episode(
        context={"intent": "auth", "method": "session", "entity": "user"},
        outcome={"constructs": "flow_login_flow_register_flow_logout", "realm": "yes", "auth": "none_for_register"},
        label="auth_session",
    )
    triad.store_episode(
        context={"intent": "auth", "method": "apikey", "entity": "service"},
        outcome={"constructs": "flow_authenticate_flow_revoke", "realm": "yes", "auth": "apikey"},
        label="auth_apikey",
    )

    # Pattern: Real-time stream
    triad.store_episode(
        context={"intent": "realtime", "entity": "notification", "transport": "websocket"},
        outcome={"constructs": "stream_notifications", "realm": "yes", "auth": "session"},
        label="stream_notifications",
    )
    triad.store_episode(
        context={"intent": "realtime", "entity": "order_status", "transport": "sse"},
        outcome={"constructs": "stream_order_updates", "realm": "yes", "auth": "session"},
        label="stream_orders",
    )

    # Pattern: External service integration
    triad.store_episode(
        context={"intent": "service", "provider": "stripe", "operations": "charge_refund"},
        outcome={"constructs": "service_payments_saga_checkout", "realm": "yes", "auth": "bearer"},
        label="service_stripe",
    )
    triad.store_episode(
        context={"intent": "service", "provider": "sendgrid", "operations": "send_email"},
        outcome={"constructs": "service_email", "realm": "yes", "auth": "bearer"},
        label="service_email",
    )

    # Pattern: Policy
    triad.store_episode(
        context={"intent": "policy", "rule": "require_auth", "applies_to": "flow"},
        outcome={"constructs": "policy_require_auth", "realm": "no", "auth": "na"},
        label="policy_auth",
    )
    triad.store_episode(
        context={"intent": "policy", "rule": "rate_limit_writes", "applies_to": "flow_write"},
        outcome={"constructs": "policy_rate_limit", "realm": "no", "auth": "na"},
        label="policy_ratelimit",
    )

    return triad


def test_crud_retrieval_for_new_entity():
    """Query with a new CRUD entity (never seen) -- should match CRUD pattern."""
    triad = make_axis_triad()

    # New entity: "article" -- never stored, but structurally similar to crud_user/product/order
    query = triad.hdc.encode_record({
        "intent": "crud", "entity": "article", "fields": "id_title_body_author", "auth": "session"
    })
    matches = triad.sdm.read(query, top_k=3)
    labels = [m[2] for m in matches]

    # All top matches should be CRUD patterns
    assert all("crud_" in label for label in labels), f"expected CRUD matches, got {labels}"
    # Top match should have constructs indicating shape+source+flows
    top_data = matches[0][0]
    # Verify the data vector is most similar to a CRUD outcome
    crud_outcome = triad.hdc.encode_record({
        "constructs": "shape_source_flow_list_flow_get_flow_create_flow_update_flow_delete",
        "realm": "yes", "auth": "session"
    })
    sim = triad.hdc.similarity(top_data, crud_outcome)
    assert sim > 0.3, f"top match data should be similar to CRUD outcome, got sim={sim}"


def test_auth_retrieval():
    """Query for auth setup -- should match auth patterns, not CRUD."""
    triad = make_axis_triad()

    query = triad.hdc.encode_record({
        "intent": "auth", "method": "session", "entity": "admin"
    })
    matches = triad.sdm.read(query, top_k=2)
    labels = [m[2] for m in matches]
    assert "auth_session" in labels, f"expected auth_session in top-2, got {labels}"


def test_realtime_retrieval():
    """Query for real-time -- should match stream patterns."""
    triad = make_axis_triad()

    query = triad.hdc.encode_record({
        "intent": "realtime", "entity": "chat_message", "transport": "websocket"
    })
    matches = triad.sdm.read(query, top_k=2)
    labels = [m[2] for m in matches]
    assert any("stream_" in label for label in labels), f"expected stream match, got {labels}"


def test_service_retrieval():
    """Query for payment service -- should match service patterns."""
    triad = make_axis_triad()

    query = triad.hdc.encode_record({
        "intent": "service", "provider": "paypal", "operations": "charge_refund"
    })
    matches = triad.sdm.read(query, top_k=2)
    labels = [m[2] for m in matches]
    assert "service_stripe" in labels, f"expected service_stripe (similar ops), got {labels}"


def test_policy_retrieval():
    """Query for policy -- should match policy patterns, not CRUD."""
    triad = make_axis_triad()

    query = triad.hdc.encode_record({
        "intent": "policy", "rule": "require_auth", "applies_to": "flow"
    })
    matches = triad.sdm.read(query, top_k=2)
    labels = [m[2] for m in matches]
    assert "policy_auth" in labels, f"expected policy_auth, got {labels}"


def test_full_backend_composition():
    """
    Compose a full backend: given a spec with multiple intents,
    retrieve the right pattern for each one.

    Spec: "booking system with users, bookings, auth, and real-time updates"
    Expected: CRUD(user) + CRUD(booking) + AUTH(session) + STREAM(updates)
    """
    triad = make_axis_triad()

    # Decomposed spec (what a structured input would look like)
    intents = [
        {"intent": "crud", "entity": "user", "fields": "id_email_name_role", "auth": "session"},
        {"intent": "crud", "entity": "booking", "fields": "id_user_id_date_status", "auth": "session"},
        {"intent": "auth", "method": "session", "entity": "user"},
        {"intent": "realtime", "entity": "booking_status", "transport": "sse"},
        {"intent": "policy", "rule": "require_auth", "applies_to": "flow"},
    ]

    results = []
    for intent_spec in intents:
        query = triad.hdc.encode_record(intent_spec)
        matches = triad.sdm.read(query, top_k=1)
        if matches:
            results.append((intent_spec["intent"], matches[0][2], matches[0][1]))

    # Verify each intent mapped to the correct pattern type
    assert len(results) == 5
    assert "crud_" in results[0][1]       # user -> CRUD
    assert "crud_" in results[1][1]       # booking -> CRUD
    assert "auth_" in results[2][1]       # auth -> auth pattern
    assert "stream_" in results[3][1]     # realtime -> stream
    assert "policy_" in results[4][1]     # policy -> policy

    # Verify similarity scores are meaningful (not random noise)
    for intent, label, sim in results:
        assert sim > 0.3, f"{intent} -> {label} has low similarity {sim}"


def test_generalization_unseen_fields():
    """
    HDC+SDM should generalize even when the field composition is novel.
    A 'crud' intent with completely new fields should still match CRUD patterns.
    """
    triad = make_axis_triad()

    # Fields never seen: latitude, longitude, radius, description
    query = triad.hdc.encode_record({
        "intent": "crud", "entity": "location", "fields": "id_lat_lng_radius_description", "auth": "session"
    })
    matches = triad.sdm.read(query, top_k=3)
    labels = [m[2] for m in matches]
    assert all("crud_" in label for label in labels), f"novel fields should still match CRUD, got {labels}"
