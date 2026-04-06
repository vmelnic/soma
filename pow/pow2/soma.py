"""
SOMA Instance -- POW 2: Experiential Memory.

The SOMA grows from experience via LoRA adaptation.
Checkpoint serializes the mind. Restore brings it back.

Commands: :checkpoint :restore :rollback :consolidate :status :adapt
"""

import json
import os
import platform

import torch

from pow.pow2.tokenizer import Tokenizer, find_span, NULL_IDX
from pow.pow2.discovery import discover_body, STOP_ID, EMIT_ID
from pow.pow2.bridge import GenericBridge
from pow.pow2.mind import SomaMind, ARG_NONE, ARG_SPAN, ARG_REF
from pow.pow2.lora import apply_lora
from pow.pow2.memory import (
    ExperienceBuffer, adapt_from_experience,
    save_checkpoint, restore_checkpoint, reset_lora, consolidate,
)

CHECKPOINT_PATH = "pow/pow2/mind_checkpoint.pt"
ADAPT_BATCH = 8
ADAPT_EVERY = 5

LORA_TARGETS = ["op_head", "a0t_head", "a1t_head",
                "s0s_q", "s0e_q", "s1s_q", "s1e_q", "r0q", "r1q"]


class Soma:

    def __init__(self, base_artifacts="pow/pow1/artifacts"):
        self.catalog, self.libc = discover_body()
        self.bridge = GenericBridge(self.catalog, self.libc)
        self.tokenizer = Tokenizer()
        self.tokenizer.load(os.path.join(base_artifacts, "vocab.json"))

        with open(os.path.join(base_artifacts, "meta.json")) as f:
            meta = json.load(f)
        self.mind = SomaMind(meta["vocab_size"], meta["num_conventions"])
        self.mind.load_state_dict(torch.load(
            os.path.join(base_artifacts, "soma_mind.pt"),
            map_location="cpu", weights_only=True))

        self.lora_layers, self.trainable, self.total = apply_lora(
            self.mind, rank=4, alpha=1.0, target_modules=LORA_TARGETS)
        self.mind.eval()

        self.experience = ExperienceBuffer(max_size=200)
        self.successes_since_adapt = 0
        self.adapt_count = 0

    def _record(self, text, steps):
        tokens = self.tokenizer.tokenize(text)
        ids = [NULL_IDX] + self.tokenizer.encode(text)
        ms = 8
        nc = len(self.catalog)
        emit_out, stop_out = nc, nc + 1

        opcodes, a0t, a1t = [], [], []
        a0ss, a0se, a1ss, a1se, a0r, a1r = [], [], [], [], [], []

        for s in steps:
            if s.conv_id == STOP_ID:
                opcodes.append(stop_out)
                for lst in [a0t, a1t]: lst.append(ARG_NONE)
                for lst in [a0ss, a0se, a1ss, a1se, a0r, a1r]: lst.append(-1)
            elif s.conv_id == EMIT_ID:
                opcodes.append(emit_out)
                a0t.append(ARG_REF); a1t.append(ARG_NONE)
                a0r.append(s.arg_values[0] if s.arg_values else 0)
                a1r.append(-1)
                for lst in [a0ss, a0se, a1ss, a1se]: lst.append(-1)
            else:
                opcodes.append(s.conv_id)
                for slot_idx, (typ_lst, ss_lst, se_lst, r_lst) in enumerate([
                    (a0t, a0ss, a0se, a0r), (a1t, a1ss, a1se, a1r)
                ]):
                    if slot_idx < len(s.arg_types):
                        at, av = s.arg_types[slot_idx], s.arg_values[slot_idx]
                        if at == "span":
                            typ_lst.append(ARG_SPAN)
                            vt = self.tokenizer.tokenize(av) if av else []
                            sp = find_span(tokens, vt) if vt else None
                            ss_lst.append(sp[0]+1 if sp else -1)
                            se_lst.append(sp[1]+1 if sp else -1)
                            r_lst.append(-1)
                        elif at == "ref":
                            typ_lst.append(ARG_REF)
                            ss_lst.append(-1); se_lst.append(-1)
                            r_lst.append(av if isinstance(av, int) else 0)
                        else:
                            typ_lst.append(ARG_NONE)
                            ss_lst.append(-1); se_lst.append(-1); r_lst.append(-1)
                    else:
                        typ_lst.append(ARG_NONE)
                        ss_lst.append(-1); se_lst.append(-1); r_lst.append(-1)

        while len(opcodes) < ms:
            opcodes.append(stop_out)
            for lst in [a0t, a1t]: lst.append(ARG_NONE)
            for lst in [a0ss, a0se, a1ss, a1se, a0r, a1r]: lst.append(-1)

        self.experience.add(ids, len(ids), opcodes[:ms],
                            a0t[:ms], a1t[:ms], a0ss[:ms], a0se[:ms],
                            a1ss[:ms], a1se[:ms], a0r[:ms], a1r[:ms])

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
        print(f"\n  [Mind] Program ({len(real)} steps, {conf:.1%}):")
        for i, s in enumerate(steps):
            print(f"    {s.format(i, self.catalog)}")
            if s.conv_id == STOP_ID: break
        print()

        def on_step(idx, op, summary):
            if op not in ("STOP", "EMIT"):
                print(f"    [{idx}] {op} ... {summary}")

        r = self.bridge.execute_program(steps, on_step=on_step)

        if r["success"]:
            out = r["output"]
            if out is None: print("  [Body] Done.")
            elif isinstance(out, list):
                print(f"  [Body] ({len(out)} items):")
                for item in out[:10]: print(f"    {item}")
            elif isinstance(out, dict):
                print("  [Body]"); [print(f"    {k}: {v}") for k, v in out.items()]
            elif isinstance(out, bool): print(f"  [Body] {'exists' if out else 'not found'}")
            else: print(f"  [Body] {out}")

            self._record(text, steps)
            self.successes_since_adapt += 1
            if self.successes_since_adapt >= ADAPT_EVERY and len(self.experience) >= ADAPT_BATCH:
                batch = self.experience.sample(ADAPT_BATCH)
                loss = adapt_from_experience(self.mind, batch)
                self.adapt_count += 1
                self.successes_since_adapt = 0
                print(f"\n  [Memory] Adapted (loss={loss:.4f}, cycle #{self.adapt_count})")
        else:
            print(f"  [Body] Error: {r['error']}")

    def _lora_magnitude(self):
        total = 0.0
        for layer in self.lora_layers.values():
            delta = layer.lora_B @ layer.lora_A * layer.scale
            total += delta.norm().item()
        return total

    def repl(self):
        params = sum(p.numel() for p in self.mind.parameters())
        print("=" * 64)
        print("  SOMA POW 2 -- Experiential Memory")
        print("  The SOMA grows from experience.")
        print("=" * 64)
        print(f"  Body:       {platform.system()} {platform.machine()}")
        print(f"  Mind:       {params:,} params (base + LoRA)")
        print(f"  LoRA:       {self.trainable:,} trainable (rank=4)")
        print(f"  Bridge:     generic ctypes (data-driven)")
        print("=" * 64)
        print("  :checkpoint :restore :rollback :consolidate :status :adapt")
        print("  Type intent to execute. 'quit' to exit.")
        print()

        while True:
            try:
                text = input("intent> ").strip()
                if not text: continue
                if text.lower() in ("quit", "exit", "q"):
                    print("\nSOMA shutting down."); break
                elif text == ":checkpoint":
                    save_checkpoint(self.mind, self.lora_layers, self.experience,
                                    CHECKPOINT_PATH, {"adapt_count": self.adapt_count})
                    print(f"\n  [Checkpoint] Mind saved. Experiences: {self.experience.total_seen}, "
                          f"Adaptations: {self.adapt_count}")
                elif text == ":restore":
                    if os.path.exists(CHECKPOINT_PATH):
                        meta, exp = restore_checkpoint(self.mind, self.lora_layers, CHECKPOINT_PATH)
                        print(f"\n  [Restore] Mind restored from {meta.get('timestamp', '?')}")
                    else: print("\n  [Restore] No checkpoint found.")
                elif text == ":rollback":
                    reset_lora(self.lora_layers); self.adapt_count = 0
                    print("\n  [Rollback] LoRA reset. Experience lost. Back to synthesis state.")
                elif text == ":consolidate":
                    consolidate(self.lora_layers); self.adapt_count = 0
                    print("\n  [Sleep] LoRA merged into base. Permanent memory grew.")
                elif text == ":status":
                    mag = self._lora_magnitude()
                    print(f"\n  [Memory Status]")
                    print(f"    Permanent:    {self.total - self.trainable:,} params (frozen)")
                    print(f"    Experiential: {self.trainable:,} LoRA params")
                    print(f"    LoRA delta:   {mag:.6f} (0 = no adaptation)")
                    print(f"    Experiences:  {self.experience.total_seen}")
                    print(f"    Buffer:       {len(self.experience)}/{self.experience.max_size}")
                    print(f"    Adaptations:  {self.adapt_count}")
                elif text.startswith(":adapt"):
                    parts = text.split()
                    n = int(parts[1]) if len(parts) > 1 else min(len(self.experience), 20)
                    if len(self.experience) < 2:
                        print("\n  [Memory] Not enough experience.")
                    else:
                        batch = self.experience.sample(min(n, len(self.experience)))
                        loss = adapt_from_experience(self.mind, batch)
                        self.adapt_count += 1
                        print(f"\n  [Memory] Adapted on {len(batch)} experiences (loss={loss:.4f})")
                elif text.lower() in ("help", "?"):
                    print("\n  [Proprioception]")
                    for c in self.catalog:
                        args = ", ".join(a["name"] for a in c.var_args)
                        print(f"    [{c.id:2d}] libc.{c.function}({args})")
                else:
                    self.process(text)
                print()
            except KeyboardInterrupt:
                print("\n\nSOMA shutting down."); break


if __name__ == "__main__":
    Soma().repl()
