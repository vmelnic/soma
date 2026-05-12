# soma-project-coder

Proves SOMA as a **code generation runtime** — a system that learns coding patterns from episodes and compiles them into routines that execute without LLM deliberation.

Traditional code agents call an LLM for every decision. SOMA metabolizes coding episodes into compiled routines that plan-follow deterministically.

## What it proves

Three real coding tasks (Express+SQLite CRUD APIs) executed through SOMA ports, consolidated into a compiled Tier 1 routine:

| Phase | What | Result |
|-------|------|--------|
| 1 | LLM connectivity (Qwen3.6 27B on RTX 3090) | Tier 2 verified |
| 2 | SOMA port roundtrip (filesystem read/write) | Port protocol verified |
| 3 | Plan decomposition from natural language goal | JSON plan generated |
| 4 | Plan execution with episode capture | 17/18 steps ok |
| 5 | Episode persistence and retrieval | 3 episodes stored |
| 6 | Schema induction via PrefixSpan | 1 schema discovered |
| 7 | Routine compilation via BMR gate | 1 routine compiled (confidence 1.0) |

## Three-tier cognitive architecture

- **Tier 1** — Compiled routines. No LLM. Plan-following from learned patterns.
- **Tier 2** — Qwen3.6 27B (local, RTX 3090 via Ollama). Content generation and planning.
- **Tier 3** — Claude API (fallback). Used when Tier 2 fails or for complex reasoning.

## Compiled routine

After 3 episodes of building Express+SQLite CRUD APIs, the system compiled `routine_coder_43bdd284`:

```
13 steps, confidence 1.0, BMR score 0.072
mkdir×3 → writefile(package.json) → npm_install →
writefile×4(db, routes, app, tests) → npm_test →
git init → git add → git commit
```

## Prerequisites

- Ollama running with Qwen3.6 27B (or any compatible model)
- soma-next runtime built (`bin/soma`)
- soma-ports built (`packs/` — filesystem, git, runner, patch, search)

## Setup

```bash
cp .env.example .env   # edit with your Ollama host and API keys
./build.sh             # builds soma-next + ports, copies to bin/ and packs/
```

### Windows RTX 3090 setup

On the Windows host:
```bash
ollama pull dieKeule/qwen3.6_27b:latest
```

Start Ollama with performance flags (34 tok/s on RTX 3090):
```
OLLAMA_HOST=0.0.0.0:11434 OLLAMA_FLASH_ATTENTION=1 OLLAMA_KV_CACHE_TYPE=q8_0 ollama serve
```

SSH tunnel from macOS:
```bash
ssh -f -N -L 11434:127.0.0.1:11434 user@windows-host
```

`num_ctx=4096` is sent per-request by `llm.js` — no server-side config needed.

## Usage

```bash
# Proof harness — 7 phases, exit 0 = all pass
node src/prove.js

# Plan + execute a coding task
node src/plan.js "Build an Express API with user CRUD and SQLite"
node src/execute.js

# Consolidate episodes into schemas and routines
node src/consolidate.js
```

## Architecture

```
src/
  plan.js         — LLM-driven plan decomposition (goal → JSON steps)
  execute.js      — Step executor with error recovery and episode capture
  llm.js          — Tier 2/3 LLM client (Ollama + Anthropic fallback)
  mcp.js          — MCP JSON-RPC 2.0 stdio client for SOMA runtime
  episodes.js     — Episode recorder (per-step observations + tier tracking)
  routines.js     — PrefixSpan schema induction + BMR routine compilation
  consolidate.js  — CLI entry for episode → schema → routine pipeline
  prove.js        — 7-phase proof harness
  env.js          — Environment config loader

packs/            — SOMA port dylibs + manifests (filesystem, git, runner, patch, search)
scripts/          — Ollama setup and diagnostics
```
