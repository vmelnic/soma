# soma-project-evolution

End-to-end proof that **mutation + selection produce adaptation** in SOMA, on real
routine execution through a real port — with no human authoring the winner.

```bash
cd soma-project-evolution && cargo run --release
# exit 0 iff all phases PASS
```

## What it proves

The unit of evolution is the **routine**. Fitness is the real outcome of executing
the routine's skill through the reference filesystem port — every run yields a
`PortCallRecord` whose `success` is read straight off the session trace. The
heritable trait under selection is *which skill* a routine calls, which is exactly
what the point-mutation operator perturbs.

The environment controls fitness through one path the harness flips:

| `target` is… | `readfile` | `stat` | `readdir` |
|---|---|---|---|
| a **file** | ✓ | ✓ | ✗ |
| a **directory** | ✗ | ✓ | ✗→✓ |

so `stat` is the generalist and `readfile`/`readdir` are specialists.

| Phase | Demonstrates |
|---|---|
| 1 — Variation | a sustained-success seed breeds `Mutated`-origin offspring with traceable lineage |
| 2 — Selection-against | a routine that never succeeds is decayed and invalidated |
| 3 — Selection-for | a succeeding routine's confidence climbs and clamps at the ceiling |
| 4 — Adaptation | flip the environment file→dir: the `readfile` seed dies, and a mutated `stat` descendant survives and succeeds where the seed fails |
| 5 — Directed rescue | a doomed `readdir`-on-a-file seed is rescued before death by `guided_mutate`, which replaces the skill that failed; the rescue variant runs in its place |

## How it relates to the runtime

The selection loop here mirrors the reactive monitor's feedback block
(`soma-next/src/runtime/world_state.rs`) line for line — the same decay/reinforce
constants and the same public functions from `soma-next/src/memory/mutation.rs`
(`mutate`, `should_breed`, `reinforced_confidence`, `skill_alphabet`, `seed_for`).
The only substitution is the scheduler: an explicit, deterministic generation loop
instead of the wall-clock background thread, so the proof is reproducible.

Routines execute through the real `SessionController` exactly as the monitor runs a
fired routine (inject the routine's steps, follow the plan). Success is the real
port outcome, never a scored oracle.

See `docs/evolutionary-soma.md` for the full thesis.
