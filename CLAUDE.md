# SOMA Project — Claude Code Instructions

## What This Is

SOMA (Greek "soma" = body) — the runtime IS the program. Two production paths:

- **LLM-driven**: LLM calls `invoke_port` via MCP → SOMA executes against external systems. SOMA is the body, LLM is the brain. Proven with HelperBook (3 ports, 32 capabilities, frontend).
- **Autonomous**: SOMA receives goals → selects skills → invokes ports → learns from episodes → compiles routines → plan-following mode for known patterns. Proven with reference pack (filesystem skills, episode→schema→routine cycle).

Active deliverables:
- **soma-next/** — Rust runtime. 1177 tests, zero warnings.
- **soma-ports/** — 11 dynamically loaded port adapters + SDK.
- **soma-helperbook/** — Service marketplace app (postgres + redis + auth, Express frontend).
- **soma-project-smtp/** — Email delivery proof.
- **soma-project-s3/** — AWS S3 proof.
- **soma-project-postgres/** — PostgreSQL proof.
- **soma-project-llm/** — Ollama + SOMA proof. LLM generates SQL from natural language, SOMA executes via postgres port.
- **soma-project-mcp/** — Claude Code MCP integration. SOMA as MCP server for Claude (.mcp.json at repo root).

Legacy (in repo but not active): soma-core/, soma-plugins/, soma-synthesizer/, poc/, pow/

## Two Execution Paths

### LLM-driven (HelperBook pattern)
```
LLM → invoke_port("postgres", "query", {sql: "..."}) → SOMA → PostgreSQL → result → LLM
```
- LLM decides what to do (writes SQL, chooses operations)
- SOMA executes, returns PortCallRecord with tracing
- `dump_state` gives LLM complete runtime context in one call (~5KB)
- No skills needed, no goals, no episodes — direct port invocation

### Autonomous (Reference pack pattern)
```
create_goal("list files in /tmp") → selector picks readdir → filesystem port → observation → episode
```
- Control loop: goal → belief → select skill → execute via port → observe → critic → repeat
- Episodes stored with embeddings (HashEmbedder, 128-dim FNV-1a feature hash)
- PrefixSpan extracts frequent skill subsequences → schemas induced
- High-confidence schemas → routines compiled
- Plan-following mode: routine found → skip deliberation → walk compiled_skill_path
- Memory ring buffer (1024 episodes), consolidation evicts after schema extraction

## Memory System

Brain-like three-tier consolidation:
```
Episodes (ring buffer 1024) → PrefixSpan → Schemas → compile → Routines → plan-following
```

- **HashEmbedder** (memory/embedder.rs): FNV-1a feature hashing, 128-dim, deterministic, works on ESP32
- **PrefixSpan** (memory/sequence_mining.rs): frequent subsequence mining, min_support threshold
- **Schema induction** (memory/schemas.rs): cluster episodes by embedding similarity (cosine 0.8), run PrefixSpan per cluster
- **Routine compilation** (memory/routines.rs): high-confidence schema → fixed skill path
- **Plan-following** (runtime/session.rs): when routine matches, working_memory.active_plan set, control loop walks the plan without fresh selection each step

## Repository Structure

```
soma/
  soma-next/                  # Rust runtime
    src/
      main.rs                 # Entry point, CLI, MCP, REPL
      bootstrap.rs            # Runtime assembly, GoalEmbedder wiring
      config.rs               # TOML config, env var overrides (SOMA_*)
      errors.rs               # SomaError enum
      runtime/
        session.rs            # 16-step control loop + plan-following mode
        policy.rs             # Safety policy enforcement
        port.rs               # Port trait, DefaultPortRuntime, invoke pipeline
        pack.rs, skill.rs, selector.rs, belief.rs, critic.rs,
        predictor.rs, resource.rs, goal.rs, trace.rs, metrics.rs,
        proprioception.rs, dynamic_port.rs, port_verify.rs
      adapters.rs             # SkillRegistryAdapter (routine/schema-aware), EpisodeMemoryAdapter (embedding-aware), PolicyEngineAdapter, PortBackedSkillExecutor
      interfaces/
        cli.rs                # 11 commands, build_episode_from_session, attempt_learning
        mcp.rs                # 16 MCP tools, episode storage after create_goal
      memory/
        episodes.rs           # Ring buffer (VecDeque, 1024 cap), retrieve_by_embedding
        schemas.rs            # PrefixSpan-based induction with embedding clustering
        routines.rs           # Compilation from schemas, invalidation
        embedder.rs           # GoalEmbedder trait + HashEmbedder (FNV-1a)
        sequence_mining.rs    # PrefixSpan algorithm
        persistence.rs, checkpoint.rs, working.rs, world.rs
      distributed/            # TCP/TLS, WebSocket, Unix socket, delegation, sync
      types/                  # Domain model (session.rs has active_plan/plan_step in WorkingMemory)
      ports/                  # Built-in: filesystem, http
    packs/reference/          # 7 filesystem skills for autonomous testing

  soma-ports/                 # Port workspace
    sdk/                      # soma-port-sdk: Port trait, types
    auth/, crypto/, geo/, image/, postgres/, push/, redis/, s3/, smtp/, timer/

  soma-helperbook/            # Service marketplace app
    docker-compose.yml        # PostgreSQL 17, Redis 7, Mailcatcher
    schema.sql, seed.sql      # 19 tables, test data
    frontend/                 # Express + plain JS + Tailwind
    packs/                    # postgres, redis, auth manifests + .dylibs
    scripts/                  # setup-db, seed-db, clean-db, start, start-mcp, dump-state, show-memory
    capabilities-checklist/   # run.mjs (44 tests), persistence.mjs (3-process test)

  soma-project-smtp/          # Email delivery proof
  soma-project-s3/            # AWS S3 proof
  soma-project-postgres/      # PostgreSQL proof
  soma-project-llm/           # Ollama LLM + SOMA proof (ollama.js CLI, docker-compose)
  soma-project-mcp/           # Claude Code MCP integration (SOMA as MCP server)
  .mcp.json                   # Root MCP config — registers SOMA for Claude Code
  docs/                       # 7 docs: vision, architecture, mcp, ports, distributed, building-projects, helperbook
```

## Build and Test

```bash
# Runtime
cd soma-next
cargo build --release        # ~10MB binary
cargo test                   # 1177+ tests, must all pass
cargo clippy                 # Must be zero warnings

# Ports
cd soma-ports
cargo build --workspace --release
cargo build --release --manifest-path redis/Cargo.toml  # redis built separately

# HelperBook
cd soma-helperbook
docker compose up -d --wait
scripts/setup-db.sh && scripts/seed-db.sh
node capabilities-checklist/run.mjs          # 44 runtime capability tests
node capabilities-checklist/persistence.mjs  # memory persistence across restarts

# Autonomous goal test
cd soma-next
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"create_goal","arguments":{"objective":"list files in /tmp"}}}' \
| cargo run --release -- --mcp --pack packs/reference/manifest.json
```

## Key Design Decisions

- **soma-port-sdk dependency**: soma-next depends on SDK for correct vtable. SdkPortAdapter bridges via JSON serialization.
- **Library naming**: `libsoma_port_{port_id}.dylib` — manifest port_id determines filename.
- **SOMA_PORTS_PLUGIN_PATH**: colon-separated. Applied even without soma.toml (env override fix).
- **observable_fields**: must be empty `[]` or only output schema fields. Port-level, checked per invocation.
- **output_schema**: use `{"schema": {"description": "any"}}` for ports returning non-objects (redis, crypto).
- **Policy**: read-only skills skip rule evaluation. Destructive/irreversible require confirmation or host override.
- **Episode ring buffer**: VecDeque with capacity 1024. Evicts oldest, returns to caller for consolidation.
- **Plan-following**: WorkingMemory.active_plan + plan_step. Set from matching routine's compiled_skill_path. Critic advances/clears plan.
- **MCP episode storage**: create_goal in MCP handler stores episodes + triggers learning (was missing, added).
- **MCP protocol compliance**: tools/list must return `inputSchema` (camelCase, not snake_case). tools/call results must be wrapped in `{"content": [{"type": "text", "text": "..."}]}`. Both were bugs, both fixed.
- **macOS binary copy**: copied binaries may need `xattr -d com.apple.quarantine` + `codesign -fs -` to run.
- **Failure recovery spec**: BindingFailure → SwitchCandidate, not Continue. The architecture.md failure recovery table is the source of truth for critic behavior.
- **Predictor calibration**: SimpleCandidatePredictor must penalize skills that fail repeatedly within a session. Score decay prevents infinite retry loops.

## Core Analogy

SOMA = body. The runtime is an organism's body — it executes, senses, adapts. It does NOT interpret intent. Understanding these roles is critical:

| Role | Responsibility | Example |
|---|---|---|
| **Brain** (LLM or caller) | Intent interpretation, decision-making, composing inputs | Decides `table="users"`, writes SQL, chooses which port to call |
| **Body** (SOMA runtime) | Execution, observation, adaptation, proprioception | Invokes the port, records the result, updates belief, learns from episodes |

The body does not think. It acts. An organism's hand doesn't decide where to reach — the brain does. The hand provides proprioception (where it is, what it's touching), and the brain uses that to decide the next action.

**This means for soma-next:**
- The runtime MUST be domain-agnostic. It knows about skills, ports, observations, episodes — never about SQL, table names, HTTP verbs, or any port-specific semantics.
- Input binding comes from the caller (brain), belief state, working memory, or goal fields — never from hardcoded domain extraction in the runtime.
- The autonomous path works when skills have self-contained input schemas and the caller provides bindings via `GoalSpec.objective.structured` or prior observations populate working memory.
- `goal_utils.rs` extracts filesystem paths because `/tmp` is syntactically recognizable (starts with `/`). This is pattern recognition, not domain knowledge. Do NOT add SQL parsing, table name extraction, or any port-specific logic here.

**When editing soma-next, ask:** "Would this code change if the port were different?" If yes, it doesn't belong in the runtime.

## Rules

- **NEVER GUESS.** Read the code. Read the spec. If neither answers, ask the user.
- **NO SPEC CITATIONS IN COMMENTS.** Comments explain what and why, not where the requirement came from.
- **BODY ≠ BRAIN.** Never add port-specific or domain-specific logic to soma-next. The runtime is universal. Domain knowledge lives in pack manifests, skill declarations, and the caller (LLM).
- **READ THE ARCHITECTURE.** Before changing session.rs, adapters.rs, or any runtime component, read docs/architecture.md. The 16-step control loop, failure recovery table, and skill lifecycle are specified there. Follow the spec — don't invent new behavior.

## When Editing

### soma-next
- `cargo test` after changes — 1177+ tests passing.
- `cargo clippy` — zero warnings.
- MCP tool changes: update build_tools(), add handler, add routing (tools/call AND direct dispatch), update tool count in tests (currently 16). The `McpTool` struct uses `#[serde(rename = "inputSchema")]` — MCP spec requires camelCase. tools/call responses are wrapped via `tool_success_response()` into MCP content array format.
- Episode/learning changes: update both cli.rs AND mcp.rs (both paths store episodes).
- Memory system: embedder.rs (GoalEmbedder trait), sequence_mining.rs (PrefixSpan), schemas.rs (induction), routines.rs (compilation).
- Plan-following: session.rs (active_plan logic after step 6), adapters.rs (SkillRegistryAdapter, SimpleSessionCritic).

### soma-ports
- Each port: cdylib crate, depends on soma-port-sdk, exports `soma_port_init`.
- `cargo build --release` builds all (except redis — separate manifest).
- After rebuilding ports, re-copy .dylib to project directories.

### Pack Manifests
- Full PackSpec format. Use packs/reference/manifest.json as template.
- port_id matches library name. observable_fields = [].
- Skills need all fields (see reference pack for exact format).

### Projects
- Copy binary: `cp soma-next/target/release/soma <project>/bin/soma`
- Copy port: `cp soma-ports/target/release/libsoma_port_*.dylib <project>/packs/*/`
- macOS: `xattr -d com.apple.quarantine bin/soma && codesign -fs - bin/soma`

## Current State (April 2026)

**What works:**
- LLM-driven path: invoke_port, list_ports, dump_state — all ports, all projects
- Autonomous path: create_goal → skill selection → port execution → episode → schema → routine → plan-following
- Memory: ring buffer, HashEmbedder, PrefixSpan, consolidation, disk persistence
- 3 proof projects (SMTP, S3, Postgres) + HelperBook app
- 44/44 capabilities checklist, 1177 unit tests

**What's next:**
- Multi-step autonomous goals (current routines are single-step; architecture supports multi-step but no multi-step episodes exist yet)
- Composite skills (skill sequences as first-class entities)
- ONNX embedding model (semantic similarity beyond token overlap)
- SOMA-to-SOMA schema/routine transfer
- ESP32 target (no_std, pre-loaded routines from capable peer)

## What's Deferred

- TransformerMind architecture
- WASM sandbox for untrusted plugins
- Plugin registry (download/cache)
- Diffuse memory tier (peer queries)
