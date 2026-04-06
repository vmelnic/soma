"""
SOMA Instance — POW 1: Neural mind drives libc directly.
No dispatch table. No per-operation code.

Usage:
    python -m pow.pow1.soma
"""

import json
import os
import platform
import sys

import torch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(__file__))))

from poc.tokenizer import Tokenizer, NULL_IDX
from pow.pow1.discovery import discover_body, STOP_ID
from pow.pow1.bridge import GenericBridge
from pow.pow1.mind import SomaMind


class Soma:
    def __init__(self, artifacts_dir="pow/pow1/artifacts"):
        self.catalog, self.libc = discover_body()
        self.bridge = GenericBridge(self.catalog, self.libc)
        self.tokenizer = Tokenizer()
        self.tokenizer.load(os.path.join(artifacts_dir, "vocab.json"))

        with open(os.path.join(artifacts_dir, "meta.json")) as f:
            meta = json.load(f)
        self.mind = SomaMind(meta["vocab_size"], meta["num_conventions"])
        self.mind.load_state_dict(torch.load(
            os.path.join(artifacts_dir, "soma_mind.pt"),
            map_location="cpu", weights_only=True))
        self.mind.eval()
        self.executions = 0
        self.errors = 0

    def process(self, text):
        text = text.strip()
        if not text:
            return None
        if text.lower() in ("help", "?") or "what can you do" in text.lower():
            print("\n  [Proprioception]")
            for c in self.catalog:
                args = ", ".join(a["name"] for a in c.var_args)
                print(f"    [{c.id:2d}] libc.{c.function}({args}) -- {c.description}")
            print(f"\n  Executions: {self.executions}, Errors: {self.errors}")
            return None

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
        self.executions += 1

        if r["success"]:
            out = r["output"]
            if out is None:
                print("  [Body] Done.")
            elif isinstance(out, list):
                print(f"  [Body] ({len(out)} items):")
                for item in out[:15]:
                    print(f"    {item}")
                if len(out) > 15:
                    print(f"    ... and {len(out)-15} more")
            elif isinstance(out, dict):
                print("  [Body]")
                for k, v in out.items():
                    print(f"    {k}: {v}")
            elif isinstance(out, bool):
                print(f"  [Body] {'exists' if out else 'not found'}")
            else:
                print(f"  [Body] {out}")
        else:
            self.errors += 1
            print(f"  [Body] Error: {r['error']}")

    def repl(self):
        params = sum(p.numel() for p in self.mind.parameters())
        print("=" * 64)
        print("  SOMA POW 1 -- Neural Mind Drives libc Directly")
        print("  No dispatch table. No per-operation code.")
        print("=" * 64)
        print(f"  Body:     {platform.system()} {platform.machine()}")
        print(f"            {len(self.catalog)} libc calling conventions (discovered)")
        print(f"  Mind:     {params:,} params (BiLSTM + GRU)")
        print(f"  Bridge:   generic ctypes (zero domain logic)")
        print("=" * 64)
        print("  Type intent. Mind generates libc calls. Bridge executes via ctypes.")
        print("  Type 'quit' to exit, 'help' for capabilities.")
        print()
        while True:
            try:
                text = input("intent> ").strip()
                if not text:
                    continue
                if text.lower() in ("quit", "exit", "q"):
                    print("\nSOMA shutting down.")
                    break
                self.process(text)
                print()
            except KeyboardInterrupt:
                print("\n\nSOMA shutting down.")
                break


if __name__ == "__main__":
    Soma().repl()
