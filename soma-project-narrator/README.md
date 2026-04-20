# soma-project-narrator

An interpreter LLM for SOMA. Attaches over MCP (stdio) and narrates the body's activity in natural-language, first-person-of-body voice.

## Why

SOMA is a body. Bodies emit signals; they don't render interfaces. This process is the analogue of the brain's interoceptive / narrative-self layer — it reads SOMA's typed event stream and produces a running commentary for a human operator (or another LLM).

## Run

```bash
# 1. pick any soma MCP server you already have (the bridge's hello pack has no DB deps):
SERVER=../soma-project-mcp-bridge/scripts/run-mcp.sh

# 2. export a key if you want LLM narration (else --raw mode prints structured events):
export ANTHROPIC_API_KEY=sk-ant-...

# 3. attach:
./scripts/run.sh "$SERVER"            # narrated
./scripts/run.sh "$SERVER" --raw      # structured events only
./scripts/run.sh "$SERVER" --speak    # + macOS `say` TTS
```

In a second terminal, drive the body — submit goals or invoke ports against the same SOMA instance — and watch this process speak.

## What it polls

- `list_sessions` — detect new/terminal goals.
- `stream_goal_observations(goal_id, after_step)` — per-step events: selected skill, port calls, critic decisions, failures, rollbacks.
- `dump_state(sections=routines)` — detect newly compiled routines (muscle memory).

## Status

**Proven end-to-end:**
- Attaches to a live SOMA MCP server via stdio (tested against `soma-project-mcp-bridge`).
- Polls `list_sessions`, `stream_goal_observations`, `dump_state(routines)` and emits one event per delta.
- `--raw` mode prints structured signal; without it, the narrator asks Claude Haiku to produce one-sentence first-person narration per event.

**Not yet proven (future slices):**
- Full goal-lifecycle narration with organic multi-step episodes (requires a pack that exercises `create_goal_async` and the control loop).
- TTS path via macOS `say` — code path exists but not exercised in CI.
- Operator-model: narrator adapting its verbosity/register to the human reader.

## Scope

This is the first slice of a research direction: treating the interface as a generative model over the runtime's own signal. Future work: operator-model (theory-of-mind over the human), multi-register narrators (terse/debug/alarm), rate–distortion tuning of what crosses the reporting threshold.
