"""
SOMA Instance -- POW 3: Synaptic Protocol.

Each SOMA runs a synapse server for receiving signals from peers.
The mind generates programs that SEND data to other SOMAs.
"""

import argparse
import json
import os
import platform

import torch

from pow.pow3.tokenizer import Tokenizer, NULL_IDX
from pow.pow3.discovery import discover_body, STOP_ID, EMIT_ID
from pow.pow3.bridge import GenericBridge
from pow.pow3.mind import SomaMind
from pow.pow3.synapse import SynapseServer


class Soma:

    def __init__(self, name="soma-a", host="localhost", port=9001,
                 artifacts="pow/pow3/artifacts"):
        self.name = name
        self.catalog, self.libc = discover_body()
        self.tokenizer = Tokenizer()
        self.tokenizer.load(os.path.join(artifacts, "vocab.json"))

        with open(os.path.join(artifacts, "meta.json")) as f:
            meta = json.load(f)
        self.mind = SomaMind(meta["vocab_size"], meta["num_conventions"])
        self.mind.load_state_dict(torch.load(
            os.path.join(artifacts, "soma_mind.pt"),
            map_location="cpu", weights_only=True))
        self.mind.eval()

        self.synapse = SynapseServer(
            name=name, host=host, port=port,
            on_signal=self._on_signal)
        self.bridge = GenericBridge(self.catalog, self.libc, synapse_server=self.synapse)

    def _on_signal(self, signal):
        if signal.type == "data":
            data = signal.payload.get("data", "")
            print(f"\n  [Synapse IN] Data from {signal.sender}:")
            if isinstance(data, list):
                for item in data[:10]:
                    print(f"    {item}")
            elif isinstance(data, dict):
                for k, v in data.items():
                    print(f"    {k}: {v}")
            else:
                print(f"    {data}")
        elif signal.type == "intent":
            intent = signal.payload.get("intent", "")
            print(f"\n  [Synapse IN] Intent from {signal.sender}: {intent}")
            self.process(intent)
        elif signal.type == "discover":
            print(f"\n  [Synapse IN] Discovered: {signal.sender}")

    def process(self, text):
        text = text.strip()
        if not text:
            return

        tokens = self.tokenizer.tokenize(text)
        ids = [NULL_IDX] + self.tokenizer.encode(text)
        steps, conf = self.mind.predict(
            torch.tensor([ids], dtype=torch.long),
            torch.tensor([len(ids)], dtype=torch.long),
            tokens, self.catalog)

        real = [s for s in steps if s.conv_id != STOP_ID]
        print(f"\n  [Mind] Program ({len(real)} steps, {conf:.0%}):")
        for i, s in enumerate(steps):
            print(f"    {s.format(i, self.catalog)}")
            if s.conv_id == STOP_ID:
                break
        print()

        def on_step(idx, op, summary):
            if op not in ("STOP", "EMIT"):
                print(f"    [{idx}] {op} ... {summary}")

        r = self.bridge.execute_program(steps, on_step=on_step)

        if r["success"]:
            out = r["output"]
            if out is None:
                print("  [Body] Done.")
            elif isinstance(out, list):
                print(f"  [Body] ({len(out)} items):")
                for item in out[:10]:
                    print(f"    {item}")
            elif isinstance(out, dict):
                print("  [Body]")
                for k, v in out.items():
                    print(f"    {k}: {v}")
            elif isinstance(out, bool):
                print("  [Body] Sent via Synaptic Protocol." if out else "  [Body] Send failed.")
            else:
                print(f"  [Body] {out}")
        else:
            print(f"  [Body] Error: {r['error']}")

    def repl(self):
        params = sum(p.numel() for p in self.mind.parameters())
        self.synapse.start()
        print("=" * 64)
        print(f"  SOMA POW 3 -- Synaptic Protocol [{self.name}]")
        print("=" * 64)
        print(f"  Name:     {self.name}")
        print(f"  Synapse:  {self.synapse.host}:{self.synapse.port}")
        print(f"  Peers:    {list(self.synapse.peers.keys()) or 'none'}")
        print(f"  Mind:     {params:,} params")
        print(f"  Catalog:  {len(self.catalog)} conventions (incl. send_signal)")
        print("=" * 64)
        print("  The SOMA can SEND results to peer SOMAs.")
        print("  :peers  :discover  quit")
        print()

        try:
            while True:
                text = input("  intent> ").strip()
                if not text:
                    continue
                if text.lower() in ("quit", "exit", "q"):
                    break
                if text == ":peers":
                    print(f"\n  Peers: {list(self.synapse.peers.keys())}")
                    print(f"  Received: {len(self.synapse.received)} signals")
                elif text == ":discover":
                    self.synapse.broadcast_discover()
                    print("\n  [Synapse] Discovery broadcast sent.")
                else:
                    self.process(text)
                print()
        except KeyboardInterrupt:
            pass
        finally:
            self.synapse.stop()
            print("\n  SOMA shutting down.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--name", default="soma-a")
    parser.add_argument("--port", type=int, default=9001)
    parser.add_argument("--peer", action="append", default=[],
                        help="name:host:port")
    args = parser.parse_args()

    soma = Soma(name=args.name, port=args.port)
    for p in args.peer:
        parts = p.split(":")
        if len(parts) == 3:
            soma.synapse.register_peer(parts[0], parts[1], int(parts[2]))
    soma.repl()
