# soma-project-multistep

End-to-end proof that SOMA's multi-step autonomous routine learning works.

## What this proves

The full chain from raw episodes to autonomous multi-step plan execution, against the real `soma-next` library — no mocks, no stubs in the critical path:

1. **Multi-step episodes can be stored** in `DefaultEpisodeStore`
2. **PrefixSpan-based schema induction** produces a multi-step schema from those episodes
3. **Schema compilation** produces a multi-step routine via `RoutineStore::compile_from_schema`
4. **Plan-following loop logic** correctly walks every step (simulated against the same logic in `runtime/session.rs:1612-1913`)
5. **Real `SessionController.run_step()`** with the bootstrapped runtime and the reference pack walks all 3 skills via plan-following and reaches `Completed` status

Until this proof existed, multi-step routines were "exists in code, unit-tested in isolation, never observed working end-to-end with the real control loop." Now they are.

## Skill sequence under test

```
soma.ports.reference.stat → soma.ports.reference.readdir → soma.ports.reference.stat
```

Three skills against `/tmp` (which is a real directory on the host). The middle step is `readdir` rather than `readfile` because `/tmp` is a directory — using `readfile` causes the second step to fail and plan-following correctly abandons the plan, which would mask the test.

## Run the proof

```bash
cd soma-project-multistep
cargo run
```

Expected output (last 10 lines):

```
--- Phase 5: Real SessionController plan-following ---
  ...
    iteration 1: Continue (skill=stat,    critic=Continue, plan=Some(len=3, step=1))
    iteration 2: Continue (skill=readdir, critic=Continue, plan=Some(len=3, step=2))
    iteration 3: Completed (skill=stat,   critic=Stop,     plan=None (step=0))
  Final session status: Completed
  PASS: real SessionController activated plan-following AND walked all 3 steps

==================================================
ALL PHASES PASSED
==================================================
```

If any phase fails, the program panics with an explicit assertion message.

## Phase breakdown

| Phase | What is exercised | Component |
|-------|-------------------|-----------|
| 1 | Episode storage with multi-step traces | `memory/episodes.rs` |
| 2 | Schema induction with embedding clustering + PrefixSpan | `memory/schemas.rs`, `memory/sequence_mining.rs` |
| 3 | Routine compilation from schema | `memory/routines.rs` |
| 4 | Plan-following control logic (simulated against the real algorithm) | mirrors `runtime/session.rs:1612-1913` |
| 5 | Real `SessionController.run_step()` end-to-end | `bootstrap`, `runtime/session.rs`, `adapters.rs`, real `FilesystemPort` |

Phase 5 is the strongest claim. It uses the real `bootstrap()` function to assemble the runtime, loads the reference pack, registers a multi-step routine compiled in Phase 3, creates a session via the real `SessionController`, and calls `run_step()` until the session reaches a terminal state. The `FilesystemPort` actually executes against the local filesystem during this phase.

## What had to be injected vs. learned

To isolate the proof to "does multi-step plan-following work end-to-end" without confounding it with separate concerns, two things are injected manually:

1. **Episodes** (Phase 1) — synthetic but structurally identical to what the real control loop would produce. This is necessary because the real control loop hasn't yet been demonstrated to produce multi-step episodes from a single goal — that's a separate concern about selector/critic behavior, not about multi-step routines.

2. **Belief binding** `path=/tmp` (Phase 5) — injected directly into `session.belief.active_bindings` after `create_session()` because the default `SimpleBeliefSource` does not extract bindings from `goal.objective.structured`. This is also a separate concern (input binding from goal context) that is independent of plan-following correctness.

Everything else — schema induction, routine compilation, plan activation, plan walking, critic override, session termination — is the real production code path running unmodified.

## Implications

The CLAUDE.md note "Multi-step autonomous goals (current routines are single-step; architecture supports multi-step but no multi-step episodes exist yet)" is now obsolete in part. The corrected status is:

| Component | Status |
|---|---|
| Type system supports multi-step | Was already true |
| Schema induction from multi-step episodes | **Proven by this project** |
| Routine compilation to multi-step path | **Proven by this project** |
| Plan-following walks multi-step routines via real `SessionController` | **Proven by this project** |
| Real control loop produces multi-step episodes from a single autonomous goal | **Still unproven** (selector/critic behavior, separate concern) |
| Default belief source binds inputs from `goal.objective.structured` | **Not implemented** (separate concern) |

The two remaining gaps are independent of plan-following. Multi-step routine learning, compilation, and execution all work — given multi-step episodes and bound inputs.
