# soma-next — Runtime Editing Guide

## Editing hotspots

- **MCP tool change**: update `build_tools()` in `src/interfaces/mcp.rs`, add handler, add routing in tools/call AND direct dispatch, update `test_list_tools_count` / `test_default_impl`. `McpTool` uses `#[serde(rename = "inputSchema")]` (MCP is camelCase). Wrap responses via `tool_success_response()`.
- **Episode/learning change**: both `interfaces/cli.rs` AND `interfaces/mcp.rs` store episodes via `runtime/goal_executor.rs`. Update the shared path, not either interface in isolation.
- **Memory pipeline**: `memory/embedder.rs` (GoalEmbedder) -> `memory/sequence_mining.rs` (PrefixSpan) -> `memory/schemas.rs` (induction) -> `memory/routines.rs` (compilation) -> `memory/checkpoint.rs` (disk).
- **Plan-following**: `runtime/session.rs::active_plan`, `adapters.rs::{SkillRegistryAdapter, SimpleSessionCritic}`. Branching via `NextStep::{Continue,Goto,CallRoutine,Complete,Abandon}`, sub-routine composition via `plan_stack` (max depth 16).
- **Async goals**: `runtime/goal_registry.rs` (background thread + cancel flag + `TraceNotifier` push) + `runtime/goal_executor.rs` (shared `run_loop` / `finalize_episode`). Push notifications via `RuntimeHandle.trace_notifier`.
- **Remote skill invocation**: `distributed/transport.rs::LocalDispatchHandler::handle(InvokeSkill)`. Injects target skill as single-step plan, seeds `objective.structured` from input. Does NOT go through predictor/selector — uses plan-following directly.
- **mDNS discovery**: `distributed/discovery.rs`. Browses `_soma._tcp.local.`, assigns `lan-<instance>` peer IDs. Wired via `--discover-lan` in `main.rs`.
- **Belief projection**: `runtime/belief_projection.rs` (BeliefProjector — JMESPath + TOON). Used at brain fallback call site in `session.rs`, exposed via `inspect_belief_projection` MCP tool. Dependencies: `jmespath` (with `sync` feature) and `toon-format`.

## Build

```bash
cargo test && cargo clippy
```

Both must pass after every change.
