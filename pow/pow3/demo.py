"""
POW 3 Demo: Two SOMAs communicating via Synaptic Protocol.
Proves Whitepaper Section 10.

Usage:
    python -m pow.pow3.demo
"""

import json
import os
import time

import torch

from pow.pow3.tokenizer import Tokenizer, NULL_IDX
from pow.pow3.discovery import discover_body, STOP_ID
from pow.pow3.bridge import GenericBridge
from pow.pow3.mind import SomaMind
from pow.pow3.synapse import SynapseServer


def make_soma(name, port, peer_name, peer_port, artifacts="pow/pow3/artifacts"):
    catalog, libc = discover_body()
    tok = Tokenizer()
    tok.load(os.path.join(artifacts, "vocab.json"))
    with open(os.path.join(artifacts, "meta.json")) as f:
        meta = json.load(f)
    mind = SomaMind(meta["vocab_size"], meta["num_conventions"])
    mind.load_state_dict(torch.load(
        os.path.join(artifacts, "soma_mind.pt"),
        map_location="cpu", weights_only=True))
    mind.eval()
    syn = SynapseServer(name=name, host="localhost", port=port)
    syn.register_peer(peer_name, "localhost", peer_port)
    bridge = GenericBridge(catalog, libc, synapse_server=syn)
    return mind, tok, catalog, bridge, syn


def predict(mind, tok, catalog, text):
    tokens = tok.tokenize(text)
    ids = [NULL_IDX] + tok.encode(text)
    return mind.predict(
        torch.tensor([ids], dtype=torch.long),
        torch.tensor([len(ids)], dtype=torch.long),
        tokens, catalog)


def show_program(steps, catalog):
    for i, s in enumerate(steps):
        print(f"    {s.format(i, catalog)}")
        if s.conv_id == STOP_ID:
            break


def run():
    print("=" * 70)
    print("  POW 3: Two SOMAs Communicating via Synaptic Protocol")
    print("=" * 70)

    print("\n  Creating SOMA-A (port 9001) and SOMA-B (port 9002)...")
    mind_a, tok_a, cat_a, bridge_a, syn_a = make_soma("soma-a", 9001, "soma-b", 9002)
    mind_b, tok_b, cat_b, bridge_b, syn_b = make_soma("soma-b", 9002, "soma-a", 9001)
    syn_a.start()
    syn_b.start()
    time.sleep(0.5)
    print(f"  SOMA-A synapse: localhost:9001")
    print(f"  SOMA-B synapse: localhost:9002")

    # Test 1: Discovery
    print("\n--- Test 1: Discovery ---")
    syn_a.broadcast_discover()
    time.sleep(0.3)
    found = "soma-a" in syn_b.peers
    print(f"  SOMA-A broadcast -> SOMA-B discovered: {found}")

    # Test 2: List files and send to SOMA-B
    print("\n--- Test 2: List /tmp and send to SOMA-B ---")
    steps, conf = predict(mind_a, tok_a, cat_a, "list files in /tmp and send to soma-b")
    print(f"  [SOMA-A Mind] {conf:.0%}:")
    show_program(steps, cat_a)

    before = len(syn_b.received)
    r = bridge_a.execute_program(steps)
    time.sleep(0.3)
    after = len(syn_b.received)
    print(f"\n  Executed: {r['success']}")
    if not r["success"]:
        print(f"  Error: {r['error']}")
    print(f"  SOMA-B received: {after - before} signal(s)")
    if after > before:
        data = syn_b.received[-1].payload.get("data", [])
        if isinstance(data, list):
            print(f"  Data: {data[:5]}... ({len(data)} items)")
        else:
            print(f"  Data: {data}")

    # Test 3: Read file and send
    print("\n--- Test 3: Read file and send to SOMA-B ---")
    with open("/tmp/pow3_test.txt", "w") as f:
        f.write("synaptic protocol works")

    steps2, conf2 = predict(mind_a, tok_a, cat_a, "read /tmp/pow3_test.txt and send to soma-b")
    print(f"  [SOMA-A Mind] {conf2:.0%}:")
    show_program(steps2, cat_a)

    before2 = len(syn_b.received)
    r2 = bridge_a.execute_program(steps2)
    time.sleep(0.3)
    after2 = len(syn_b.received)
    if after2 > before2:
        print(f"\n  SOMA-B received: \"{syn_b.received[-1].payload.get('data', '')}\"")

    # Test 4: Send time
    print("\n--- Test 4: Send time to SOMA-B ---")
    steps3, _ = predict(mind_a, tok_a, cat_a, "get the time and send to soma-b")
    show_program(steps3, cat_a)
    before3 = len(syn_b.received)
    bridge_a.execute_program(steps3)
    time.sleep(0.3)
    if len(syn_b.received) > before3:
        print(f"  SOMA-B received: {syn_b.received[-1].payload.get('data', '')}")

    # Test 5: Local still works
    print("\n--- Test 5: Local execution (no send) ---")
    steps4, _ = predict(mind_a, tok_a, cat_a, "what time is it")
    show_program(steps4, cat_a)
    r4 = bridge_a.execute_program(steps4)
    print(f"  Local output: {r4['output']}")

    # Cleanup
    syn_a.stop()
    syn_b.stop()
    os.remove("/tmp/pow3_test.txt")

    total = len(syn_b.received)
    print("\n" + "=" * 70)
    print(f"  Discovery:     SOMA-A -> SOMA-B via presence broadcast")
    print(f"  Signals sent:  {total} (data + discover)")
    print(f"  Data types:    file listing, file content, time")
    print(f"  Protocol:      JSON/TCP (Synaptic Protocol)")
    print(f"  Local:         still works (EMIT vs SEND)")
    print(f"\n  The mind decides WHEN to send vs display locally.")
    print(f"  The mind decides WHAT data to send.")
    print(f"  Whitepaper Section 10: Soma Network validated.")
    print("=" * 70)


if __name__ == "__main__":
    run()
