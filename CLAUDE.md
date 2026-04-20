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
```

After rebuilding a port, re-copy the `.dylib` to any `soma-project-*/packs/` that uses it. After rebuilding soma-next for a project, on macOS: `xattr -d com.apple.quarantine bin/soma && codesign -fs - bin/soma`.

## Invariants

A change that violates any of these is an architectural redirection, not a bug fix.

- **Body ≠ brain.** Runtime is domain-agnostic. It knows skills, ports, observations, episodes — never SQL, HTTP verbs, table names, API flows. When editing soma-next: "would this change if the port were different?" If yes, it belongs in a port adapter or the brain.
- **Input binding** comes from brain, belief state, working memory, or goal fields. Never hardcoded domain extraction.
- **Every external interaction** produces a typed `PortCallRecord`.
- **Sessions carry finite budgets.** Exhaustion terminates the session.
- **Learning mines structure**, not content.
- **Interfaces are self-describing** at runtime.

## Working rules

- **NEVER GUESS.** Read the code. Read the spec. If neither answers, ask.
- **NO HEDGING.** Don't call work "hard" or "1-2 weeks". Ship the first concrete step.
- **TIMEBOX DEBUGGING.** If two patches don't fix the bug, the diagnosis is wrong — back out and re-examine. Round or absurd panic values usually indicate uninitialized memory, stack corruption, or format-string mismatch, not "zero-byte alloc" paths. Check heap/stack size, buffer overflow, missing init first.
- **HONEST STATUS.** "Proven" = expected user-visible behavior end-to-end on real data, captured in session. "Compiles", "boots", "got further than before" are NOT proof. 15/16 passing is not "working" — report the failing case.
- **NO SPEC CITATIONS IN COMMENTS.** Comments say what and why; never cite RFC numbers or architecture-doc sections.
- **NO COUNT/SIZE CHURN.** Tool counts, test counts, binary sizes rot. Point readers at `tools/list` / `cargo test` / `ls -la` instead of writing numbers.

## Editing hotspots

- **MCP tool change**: update `build_tools()` (`src/interfaces/mcp.rs`), add handler, add routing in tools/call AND direct dispatch, update `test_list_tools_count` / `test_default_impl`. `McpTool` uses `#[serde(rename = "inputSchema")]` (MCP is camelCase). Wrap tools/call responses via `tool_success_response()`.
- **Episode/learning change**: both `interfaces/cli.rs` AND `interfaces/mcp.rs` store episodes via `runtime/goal_executor.rs`. Update the shared path, not either interface in isolation.
- **Memory pipeline**: `memory/embedder.rs` (GoalEmbedder) → `memory/sequence_mining.rs` (PrefixSpan) → `memory/schemas.rs` (induction) → `memory/routines.rs` (compilation) → `memory/checkpoint.rs` (disk).
- **Plan-following**: `runtime/session.rs::active_plan`, `adapters.rs::{SkillRegistryAdapter, SimpleSessionCritic}`. Branching via `NextStep::{Continue,Goto,CallRoutine,Complete,Abandon}`, sub-routine composition via `plan_stack` (max depth 16).
- **Async goals**: `runtime/goal_registry.rs` (background thread + cancel flag) + `runtime/goal_executor.rs` (shared `run_loop` / `finalize_episode`).
- **Pack manifests**: full `PackSpec`; template in `packs/reference/manifest.json`. `port_id` must match library name. Skills need all fields.
- **Port crates**: cdylib, depend on `soma-port-sdk`, export `soma_port_init`.
- **Remote skill invocation handler**: `distributed/transport.rs::LocalDispatchHandler::handle(InvokeSkill)`. Injects the target skill as a single-step plan and seeds `objective.structured` from the input payload. The handler does NOT go through the predictor/selector — it uses plan-following directly.
- **mDNS discovery**: `distributed/discovery.rs`. Browses `_soma._tcp.local.`, assigns `lan-<instance>` peer IDs. Wired via `--discover-lan` in `main.rs`.

## Known gaps — honest status

- **Organic multi-step episode production.** Multi-step routines work *given* multi-step episodes; `soma-project-multistep` Phase 1 still injects them. Selector/critic can continue on unsatisfied success predicates (`adapters.rs::SimpleSessionCritic`), but no proof harness exercises organic chaining from a single goal.
- **No MCP-level observation streaming.** Internal infra exists (`src/distributed/streaming.rs`) and is used for remote-peer delivery tracking, but no MCP tool exposes a subscription channel — async goals remain poll-only via `get_goal_status` from the brain's perspective.
