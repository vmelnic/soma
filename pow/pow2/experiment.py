"""
POW 2 Experiment: Does adaptation actually improve performance?

Measures confidence before and after experience on repeated patterns.
Proves the memory loop works end-to-end.

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

LORA_TARGETS = ["op_head", "a0t_head", "a1t_head",
                "s0s_q", "s0e_q", "s1s_q", "s1e_q", "r0q", "r1q"]


def measure(mind, tok, catalog, intents):
    results = []
    for text in intents:
        tokens = tok.tokenize(text)
        ids = [NULL_IDX] + tok.encode(text)
        steps, conf = mind.predict(
            torch.tensor([ids], dtype=torch.long),
            torch.tensor([len(ids)], dtype=torch.long),
            tokens, catalog)
        results.append({"intent": text, "confidence": conf, "steps": steps})
    return results


def record(tok, exp, text, steps, catalog):
    tokens = tok.tokenize(text)
    ids = [NULL_IDX] + tok.encode(text)
    ms, nc = 8, len(catalog)
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
            a0r.append(s.arg_values[0] if s.arg_values else 0); a1r.append(-1)
            for lst in [a0ss, a0se, a1ss, a1se]: lst.append(-1)
        else:
            opcodes.append(s.conv_id)
            for si, (tl, sl, el, rl) in enumerate([
                (a0t, a0ss, a0se, a0r), (a1t, a1ss, a1se, a1r)
            ]):
                if si < len(s.arg_types):
                    at, av = s.arg_types[si], s.arg_values[si]
                    if at == "span":
                        tl.append(ARG_SPAN)
                        vt = tok.tokenize(av) if av else []
                        sp = find_span(tokens, vt) if vt else None
                        sl.append(sp[0]+1 if sp else -1); el.append(sp[1]+1 if sp else -1)
                        rl.append(-1)
                    elif at == "ref":
                        tl.append(ARG_REF); sl.append(-1); el.append(-1)
                        rl.append(av if isinstance(av, int) else 0)
                    else:
                        tl.append(ARG_NONE); sl.append(-1); el.append(-1); rl.append(-1)
                else:
                    tl.append(ARG_NONE); sl.append(-1); el.append(-1); rl.append(-1)

    while len(opcodes) < ms:
        opcodes.append(stop_out)
        for lst in [a0t, a1t]: lst.append(ARG_NONE)
        for lst in [a0ss, a0se, a1ss, a1se, a0r, a1r]: lst.append(-1)

    exp.add(ids, len(ids), opcodes[:ms], a0t[:ms], a1t[:ms],
            a0ss[:ms], a0se[:ms], a1ss[:ms], a1se[:ms], a0r[:ms], a1r[:ms])


def run():
    print("=" * 70)
    print("  POW 2 Experiment: Does Adaptation Improve Performance?")
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
        mind, rank=4, alpha=1.0, target_modules=LORA_TARGETS)
    mind.eval()
    print(f"\n  Base: {total - trainable:,} frozen, LoRA: {trainable:,} trainable")
    print(f"  Targets: {list(lora_layers.keys())}")

    # Test intents that the base model may NOT be 100% confident on:
    # novel phrasings, unseen filenames, edge cases
    test_intents = [
        # Known patterns (baseline high)
        "list files in /tmp",
        "what time is it",
        # Novel phrasings (baseline may be lower)
        "show me everything in /var/log",
        "check out the contents of ~/Downloads",
        "display all items in /etc",
        "give me the current time right now",
        "tell me what directory i am in",
        "read the file mystery.dat",
        "is there a file called phantom.cfg",
        "what is running on this machine",
    ]

    # Training: execute the NOVEL phrasings repeatedly so the SOMA
    # learns those patterns through experience
    training = ([
        "show me everything in /tmp",
        "show me everything in /var/log",
        "show me everything in ~/Downloads",
        "check out the contents of /etc",
        "check out the contents of ~/Documents",
        "display all items in /tmp",
        "display all items in /opt",
        "give me the current time right now",
        "tell me what directory i am in",
        "read the file mystery.dat",
    ] * 4 + [
        "list files in /tmp",
        "what time is it",
        "where am i",
    ])

    # Phase 1: Baseline
    print("\n--- Phase 1: Baseline Confidence ---")
    baseline = measure(mind, tok, catalog, test_intents)
    for r in baseline:
        print(f"  {r['confidence']:6.2%}  {r['intent']}")
    avg_b = sum(r["confidence"] for r in baseline) / len(baseline)

    # Phase 2: Execute + record
    print(f"\n--- Phase 2: Execute {len(training)} intents ---")
    exp = ExperienceBuffer(max_size=200)
    ok = 0
    for text in training:
        tokens = tok.tokenize(text)
        ids = [NULL_IDX] + tok.encode(text)
        steps, _ = mind.predict(
            torch.tensor([ids], dtype=torch.long),
            torch.tensor([len(ids)], dtype=torch.long),
            tokens, catalog)
        r = bridge.execute_program(steps)
        if r["success"]:
            record(tok, exp, text, steps, catalog)
            ok += 1
    print(f"  {ok}/{len(training)} successful, buffer: {len(exp)}")

    # Phase 3: Adapt
    print("\n--- Phase 3: Adaptation (20 replay cycles) ---")
    for cycle in range(20):
        batch = exp.sample(min(16, len(exp)))
        loss = adapt_from_experience(mind, batch, lr=1e-3)
        if (cycle + 1) % 5 == 0:
            print(f"  Cycle {cycle+1:2d}: loss={loss:.6f}")

    # Phase 4: Post-adaptation
    print("\n--- Phase 4: Post-Adaptation Confidence ---")
    adapted = measure(mind, tok, catalog, test_intents)

    print(f"\n  {'Intent':<35s} {'Before':>8s} {'After':>8s} {'Delta':>8s}")
    print(f"  {'-'*35} {'-'*8} {'-'*8} {'-'*8}")
    for b, a in zip(baseline, adapted):
        d = a["confidence"] - b["confidence"]
        m = "+" if d > 0.001 else (" " if d > -0.001 else "-")
        print(f"  {b['intent']:<35s} {b['confidence']:7.2%} {a['confidence']:7.2%} "
              f"{d:+7.2%} {m}")

    avg_a = sum(r["confidence"] for r in adapted) / len(adapted)
    avg_d = avg_a - avg_b

    # Phase 5: Rollback
    print("\n--- Phase 5: Rollback ---")
    reset_lora(lora_layers)
    rolled = measure(mind, tok, catalog, test_intents[:3])
    avg_r = sum(r["confidence"] for r in rolled) / len(rolled)
    avg_b3 = sum(r["confidence"] for r in baseline[:3]) / 3
    print(f"  Rollback avg: {avg_r:.4%} (baseline was: {avg_b3:.4%})")

    print("\n" + "=" * 70)
    print(f"  Baseline avg:     {avg_b:.4%}")
    print(f"  Adapted avg:      {avg_a:.4%}")
    print(f"  Delta:            {avg_d:+.4%}")
    if avg_d > 0:
        print(f"  RESULT: Adaptation IMPROVED confidence by {avg_d:+.4%}")
    else:
        print(f"  RESULT: No measurable improvement ({avg_d:+.4%})")
    print(f"  Rollback correct: {abs(avg_r - avg_b3) < 0.01}")
    print("=" * 70)


if __name__ == "__main__":
    run()
