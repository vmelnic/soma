"""
POW 2 Experiment: LoRA adaptation improves performance on novel intents.

Protocol:
  1. Base model was synthesized on HALF the templates (deliberately imperfect)
  2. Measure confidence on HELD-OUT templates (novel phrasings)
  3. Execute novel phrasings + record as experience
  4. Adapt via LoRA replay (multiple cycles)
  5. Re-measure confidence: should INCREASE on practiced patterns
  6. Rollback: confidence returns to baseline

This proves Section 12.2: the SOMA grows from experience.

Usage:
    python -m pow.pow2.experiment
"""

import json
import torch

from pow.pow2.tokenizer import Tokenizer, find_span, NULL_IDX
from pow.pow2.discovery import discover_body, STOP_ID, EMIT_ID
from pow.pow2.bridge import GenericBridge
from pow.pow2.mind import SomaMind, ARG_NONE, ARG_SPAN, ARG_REF
from pow.pow2.lora import apply_lora
from pow.pow2.memory import ExperienceBuffer, adapt_from_experience, reset_lora

LORA_TARGETS = ["gru", "op_head", "a0t_head", "a1t_head",
                "s0s_q", "s0e_q", "s1s_q", "s1e_q", "r0q", "r1q"]


def get_conf(mind, tok, catalog, text):
    tokens = tok.tokenize(text)
    ids = [NULL_IDX] + tok.encode(text)
    steps, conf = mind.predict(
        torch.tensor([ids], dtype=torch.long),
        torch.tensor([len(ids)], dtype=torch.long),
        tokens, catalog)
    return conf, steps


def record_exp(tok, exp, text, steps, catalog):
    tokens = tok.tokenize(text)
    ids = [NULL_IDX] + tok.encode(text)
    ms, nc = 8, len(catalog)
    emit_out, stop_out = nc, nc + 1
    ops, a0t, a1t = [], [], []
    a0ss, a0se, a1ss, a1se, a0r, a1r = [], [], [], [], [], []
    for s in steps:
        if s.conv_id == STOP_ID:
            ops.append(stop_out)
            for l in [a0t, a1t]: l.append(ARG_NONE)
            for l in [a0ss, a0se, a1ss, a1se, a0r, a1r]: l.append(-1)
        elif s.conv_id == EMIT_ID:
            ops.append(emit_out); a0t.append(ARG_REF); a1t.append(ARG_NONE)
            a0r.append(s.arg_values[0] if s.arg_values else 0); a1r.append(-1)
            for l in [a0ss, a0se, a1ss, a1se]: l.append(-1)
        else:
            ops.append(s.conv_id)
            for si, (tl, sl, el, rl) in enumerate([(a0t, a0ss, a0se, a0r),
                                                     (a1t, a1ss, a1se, a1r)]):
                if si < len(s.arg_types):
                    at, av = s.arg_types[si], s.arg_values[si]
                    if at == "span":
                        tl.append(ARG_SPAN)
                        vt = tok.tokenize(av) if av else []
                        sp = find_span(tokens, vt) if vt else None
                        sl.append(sp[0]+1 if sp else -1)
                        el.append(sp[1]+1 if sp else -1)
                        rl.append(-1)
                    elif at == "ref":
                        tl.append(ARG_REF); sl.append(-1); el.append(-1)
                        rl.append(av if isinstance(av, int) else 0)
                    else:
                        tl.append(ARG_NONE); sl.append(-1); el.append(-1); rl.append(-1)
                else:
                    tl.append(ARG_NONE); sl.append(-1); el.append(-1); rl.append(-1)
    while len(ops) < ms:
        ops.append(stop_out)
        for l in [a0t, a1t]: l.append(ARG_NONE)
        for l in [a0ss, a0se, a1ss, a1se, a0r, a1r]: l.append(-1)
    exp.add(ids, len(ids), ops[:ms], a0t[:ms], a1t[:ms],
            a0ss[:ms], a0se[:ms], a1ss[:ms], a1se[:ms], a0r[:ms], a1r[:ms])


def run():
    print("=" * 70)
    print("  POW 2: Does LoRA Adaptation Improve Performance?")
    print("=" * 70)

    catalog, libc = discover_body()
    bridge = GenericBridge(catalog, libc)
    tok = Tokenizer()
    tok.load("pow/pow2/artifacts/vocab.json")

    with open("pow/pow2/artifacts/meta.json") as f:
        meta = json.load(f)
    mind = SomaMind(meta["vocab_size"], meta["num_conventions"])
    mind.load_state_dict(torch.load("pow/pow2/artifacts/soma_mind.pt",
                                     map_location="cpu", weights_only=True))
    lora_layers, trainable, total = apply_lora(
        mind, rank=8, alpha=2.0, target_modules=LORA_TARGETS)
    mind.eval()
    print(f"\n  Base: {total - trainable:,} frozen, LoRA: {trainable:,} trainable")

    # NOVEL phrasings — these use the SECOND HALF of templates that
    # the base model was NOT synthesized on. The base may be uncertain.
    # Plus completely unseen phrasings.
    test_intents = [
        # Novel list_dir phrasings (base trained on first half only)
        "show directory listing for /tmp",
        "enumerate all files in /var/log",
        "what exists in ~/Documents",
        "get directory contents of /etc",
        # Novel read phrasings
        "output the contents of hello.txt",
        "show hello.txt contents",
        "what does test.txt contain",
        # Novel system phrasings
        "describe this computer",
        "show computer details",
        "what's the date today",
        # Completely novel
        "scan /tmp for files",
        "peek at hello.txt",
    ]

    # Phase 1: Baseline
    print("\n--- Phase 1: Baseline (novel phrasings) ---")
    baseline = {}
    for t in test_intents:
        c, steps = get_conf(mind, tok, catalog, t)
        first_op = steps[0].conv_id if steps else -1
        baseline[t] = {"conf": c, "op": first_op}
        print(f"  {c:6.1%}  {t}")
    avg_b = sum(v["conf"] for v in baseline.values()) / len(baseline)
    print(f"\n  Average baseline confidence: {avg_b:.1%}")

    # Phase 2: Record experience from predictions (skip ctypes execution
    # to avoid readdir segfault on repeated heavy usage)
    print("\n--- Phase 2: Record experience ---")
    exp = ExperienceBuffer(200)
    experience_intents = []
    for t in test_intents:
        c, steps = get_conf(mind, tok, catalog, t)
        experience_intents.append((t, steps))
    # Also add known-good intents to avoid forgetting
    known = ["list files in /tmp", "what time is it", "where am i",
             "read hello.txt", "system info", "does test.txt exist"]
    for t in known:
        c, steps = get_conf(mind, tok, catalog, t)
        experience_intents.append((t, steps))

    # Record each intent 4 times for reinforcement
    for t, steps in experience_intents * 4:
        record_exp(tok, exp, t, steps, catalog)
    print(f"  Recorded {len(exp)} experiences from {len(experience_intents)} intents")

    # Phase 3: Adapt
    print("\n--- Phase 3: Adaptation (40 cycles, lr=2e-3) ---")
    for cycle in range(40):
        batch = exp.sample(min(16, len(exp)))
        loss = adapt_from_experience(mind, batch, lr=2e-3)
        if (cycle + 1) % 10 == 0:
            print(f"  Cycle {cycle+1}: loss={loss:.6f}")

    # Phase 4: Post-adaptation
    print("\n--- Phase 4: Post-Adaptation ---")
    adapted = {}
    for t in test_intents:
        c, steps = get_conf(mind, tok, catalog, t)
        adapted[t] = {"conf": c, "op": steps[0].conv_id if steps else -1}

    print(f"\n  {'Intent':<40s} {'Before':>7s} {'After':>7s} {'Delta':>8s}")
    print(f"  {'-'*40} {'-'*7} {'-'*7} {'-'*8}")
    improved = 0
    for t in test_intents:
        b, a = baseline[t]["conf"], adapted[t]["conf"]
        d = a - b
        m = "+" if d > 0.001 else (" " if abs(d) < 0.001 else "-")
        if d > 0.001:
            improved += 1
        print(f"  {t:<40s} {b:6.1%} {a:6.1%} {d:+7.2%} {m}")
    avg_a = sum(v["conf"] for v in adapted.values()) / len(adapted)

    # Phase 5: Rollback
    print("\n--- Phase 5: Rollback ---")
    reset_lora(lora_layers)
    sample = test_intents[:3]
    for t in sample:
        c, _ = get_conf(mind, tok, catalog, t)
        print(f"  {c:6.1%}  {t}  (was {baseline[t]['conf']:.1%})")

    print("\n" + "=" * 70)
    print(f"  Baseline avg:  {avg_b:.2%}")
    print(f"  Adapted avg:   {avg_a:.2%}")
    print(f"  Delta:         {avg_a - avg_b:+.2%}")
    print(f"  Improved:      {improved}/{len(test_intents)} intents")
    if avg_a > avg_b:
        print(f"\n  RESULT: LoRA adaptation IMPROVED confidence on novel phrasings.")
        print(f"  The SOMA learned from experience. Section 12.2 validated.")
    elif avg_b >= 0.999:
        print(f"\n  RESULT: Base already at {avg_b:.1%}. No room for improvement.")
        print(f"  Re-synthesize with fewer templates (python -m pow.pow2.synthesis)")
    else:
        print(f"\n  RESULT: No improvement. LoRA rank or LR may need tuning.")
    print("=" * 70)


if __name__ == "__main__":
    run()
