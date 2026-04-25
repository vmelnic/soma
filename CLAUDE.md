# SOMA — Claude Code Instructions

## What SOMA is

SOMA = body. The runtime is the application; projects are empty shells plus manifests. No application source code.

Two execution paths:

- **LLM-driven**: brain calls `invoke_port` via MCP → runtime invokes ports → returns `PortCallRecord`.
- **Autonomous**: brain calls `create_goal` → runtime runs control loop → selects skills → invokes ports → observes → learns from episodes → compiles routines → plan-following on repeat.

Brain decides. Body executes. A hand doesn't decide where to reach; it provides proprioception so the brain can.

## Repository

```
soma-next/              Rust runtime. Single binary. All core code in src/.
soma-ports/             Port adapters + SDK. cdylib crates, export soma_port_init.
soma-project-*/         Proof/demo apps. Each is manifests + bin/ + packs/.
docs/                   Architecture, design proposals, protocol specs.
.mcp.json               Root MCP config for Claude Code.
```

Legacy (not active): `soma-core/`, `soma-plugins/`, `soma-synthesizer/`, `poc/`, `pow/`.

## Authoritative sources — check these, don't guess

| Question | Ask |
|---|---|
| Which MCP tools exist? | `tools/list` at runtime, or `src/interfaces/mcp.rs::build_tools()` |
| Which ports are loaded? | `list_ports` at runtime, or `soma-ports/*/manifest.json` |
| What's shipped end-to-end? | `soma-project-*/` directories (each is a proof) |
| What are the runtime invariants? | `docs/architecture.md` + the Invariants section below |
| What's the wire protocol? | `src/distributed/transport.rs` (`TransportMessage`/`TransportResponse`) |
| Current state of a session? | `inspect_session` or `dump_state` |

If the answer lives in the code, read the code. Do not fabricate from docs or training data.

## Build and test

```bash
# Runtime — must pass after every change
cd soma-next && cargo test && cargo clippy

# Ports (redis has its own manifest)
cd soma-ports && cargo build --workspace --release
cargo build --release --manifest-path soma-ports/redis/Cargo.toml

# Proof harnesses exit 0 iff all phases PASS
cd soma-project-autonomy  && cargo run --release
cd soma-project-multistep && cargo run
cd soma-project-inference && cargo run --release
cd soma-project-minigrid && cargo run --release
```

After rebuilding a port, re-copy the `.dylib` to any `soma-project-*/packs/` that uses it. After rebuilding soma-next for a project, on macOS: `xattr -d com.apple.quarantine bin/soma && codesign -fs - bin/soma`.

## Invariants

A change that violates any of these is an architectural redirection, not a bug fix.

- **Body != brain.** Runtime is domain-agnostic. It knows skills, ports, observations, episodes — never SQL, HTTP verbs, table names, API flows. Test: "would this change if the port were different?" If yes, it belongs in a port or brain.
- **Input binding** comes from brain, belief state, working memory, or goal fields. Never hardcoded domain extraction.
- **Every external interaction** produces a typed `PortCallRecord`.
- **Sessions carry finite budgets.** Exhaustion terminates the session.
- **Learning mines structure**, not content. Belief updates minimize free energy (KL divergence + prediction error). Skill selection minimizes expected free energy (pragmatic×precision + epistemic×(1−precision)). Routine compilation applies Bayesian Model Reduction (accuracy vs complexity gate). No gradients.
- **Routines compose hierarchically.** `CompiledStep::SubRoutine` pushes the plan stack, executes a child routine, and pops back. Max nesting depth 16.
- **Interfaces are self-describing** at runtime.

## How to think

- **Think first.** Surface tradeoffs and state assumptions before writing code. Present multiple interpretations when ambiguity exists.
- **Simplicity first.** Minimum code that solves the problem. No speculative features, unnecessary abstractions, or over-engineered error handling.
- **Surgical changes.** Touch only what you must. Don't reformat unrelated code, don't remove pre-existing dead code unless asked.
- **Goal-driven.** Define success criteria. Loop until verified against real behavior, not just compilation.

## Working rules

- **NEVER GUESS.** Read the code. Read the spec. If neither answers, ask.
- **NO HEDGING.** Don't call work "hard" or "1-2 weeks". Ship the first concrete step.
- **TIMEBOX DEBUGGING.** If two patches don't fix the bug, the diagnosis is wrong — back out and re-examine. Round or absurd panic values usually indicate uninitialized memory, stack corruption, or format-string mismatch. Check heap/stack size, buffer overflow, missing init first.
- **HONEST STATUS.** "Proven" = expected user-visible behavior end-to-end on real data. "Compiles", "boots", "got further" are NOT proof. 15/16 passing is not "working" — report the failing case.
- **NO SPEC CITATIONS IN COMMENTS.** Comments say what and why; never cite RFC numbers or architecture-doc sections.
- **NO COUNT/SIZE CHURN.** Tool counts, test counts, binary sizes rot. Point readers at `tools/list` / `cargo test` / `ls -la`.
