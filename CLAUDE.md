# SOMA Project — Claude Code Instructions

## What This Is

SOMA (from Greek "body") is a computational paradigm where a neural architecture IS the program. No application source code. A trained neural Mind receives structured intents, generates execution programs, and orchestrates plugins that interface with hardware, databases, networks, and other systems.

Four deliverables:
- **soma-core/** — Rust runtime (single binary, 14MB). THE production deliverable.
- **soma-plugins/** — Rust plugin workspace (SDK + 6 plugins). External catalog plugins.
- **soma-synthesizer/** — Python build tool (PyTorch). Trains the Mind, exports ONNX models.
- **soma-helperbook/** — First real-world application. Service marketplace frontend + database.

## Architecture (6 core components)

```
Mind Engine (ONNX inference) + Plugin Manager + Memory System
+ Synaptic Protocol (SOMA-to-SOMA) + MCP Server (LLM-to-SOMA) + Proprioception
```

## Repository Structure

```
soma/
  SOMA_Whitepaper.md          # Master specification (v0.3)
  README.md                   # Project overview + quick links
  docs/
    web4.md                   # Web 4 vision: neural execution paradigm
    architecture.md           # 6 core components, design decisions, runtime behavior
    getting-started.md        # Build, install, run SOMA
    building-apps.md          # Step-by-step: new app from scratch
    mind-engine.md            # Neural inference, LoRA, tokenizer, memory system
    synaptic-protocol.md      # Binary wire protocol spec (SOMA-to-SOMA)
    mcp-interface.md          # LLM integration, all MCP tools, auth
    plugin-system.md          # Plugin architecture: trait, conventions, Value, loading
    plugin-catalog.md         # All plugins + conventions reference
    plugin-development.md     # Tutorial: build a plugin end-to-end
    synthesizer.md            # Training pipeline (PyTorch to ONNX)
    helperbook.md             # HelperBook application guide
    roadmap.md                # Status, milestones, deferred items

  soma-core/                  # Rust runtime
    Cargo.toml                # 26 crate dependencies
    soma.toml.example         # All config fields documented
    README.md                 # Comprehensive usage + architecture docs
    src/
      main.rs                 # Entry point, CLI, REPL, startup/shutdown
      config/mod.rs           # TOML config (8 sections), env var overrides (SOMA_*)
      errors.rs               # SomaError: Inference, Plugin, Protocol, Mcp, State, Auth
      mind/
        mod.rs                # MindEngine trait, Program, ProgramStep, MindConfig
        onnx_engine.rs        # tract-onnx inference with LoRA + temperature
        tokenizer.rs          # Word-level + BPE tokenizers (auto-detected from JSON)
        lora.rs               # LoRALayer (forward/merge/reset), LoRAWeights, LoRACheckpoint
        adaptation.rs         # Runtime LoRA adaptation — gradient descent on experiences
      plugin/
        interface.rs          # SomaPlugin trait (18 methods), Value (10 variants),
                              # Convention, ArgType, ReturnSpec, CleanupSpec, TrustLevel,
                              # PluginPermissions, PluginConfig
        manager.rs            # Arc<RwLock<PluginManager>>, routing, cleanup walk,
                              # ConventionStats, topological sort, name resolution
        builtin.rs            # PosixPlugin: 22 conventions (libc + high-level fs)
        dynamic.rs            # libloading, scan, manifest parsing, Ed25519 verify
        process.rs            # ProcessManager for child processes
      protocol/
        signal.rs             # 26 SignalTypes, 6 flags, Signal struct
        codec.rs              # Binary wire format, CRC32, zstd, ChaCha20-Poly1305
        connection.rs         # TCP, heartbeat, RTT, channels, session token (24h)
        server.rs             # Listener, PubSub, capability+relay enforcement, metrics
        client.rs             # Auto-reconnect, subscription replay
        router.rs             # SignalRouter, DashMap pending_requests, 30s timeout
        discovery.rs          # PeerRegistry, TTL forwarding, PEER_QUERY
        relay.rs              # Multi-hop, loop prevention, max_hops
        chunked.rs            # Resumable transfer, SHA-256
        pubsub.rs             # Wildcards, durable, catch-up, fan-out
        streaming.rs          # Stream lifecycle, frame counting
        rate_limit.rs         # Graduated response, CONTROL signal, PeerBlacklist
        offline_queue.rs      # Priority queue, expiry
        encryption.rs         # ChaCha20-Poly1305, X25519, Ed25519 (real crypto)
        websocket.rs          # WebSocket transport adapter
        unix_socket.rs        # Unix Domain Socket transport
      memory/
        checkpoint.rs         # v2 format, SHA-256 model hash, plugin state + manifest
        experience.rs         # Ring buffer (successes only)
        consolidation.rs      # LoRA merge threshold + structure
      mcp/
        server.rs             # JSON-RPC 2.0, 22+ tools, auth, audit trail
        tools.rs              # Tool definitions (state + action + plugin)
        auth.rs               # Admin/builder/viewer from env vars
      metrics/mod.rs          # 20 Prometheus counters, JSON + text exposition
      proprioception/mod.rs   # Self-model, RSS tracking
      state/
        decision_log.rs       # Permanent decisions
        execution_history.rs  # Bounded executions
      bin/
        soma_dump.rs          # Signal capture CLI tool

  soma-plugins/               # Plugin workspace (SDK + 6 catalog plugins)
    Cargo.toml                # Workspace: sdk, auth, crypto, geo, http-bridge, postgres, redis
    sdk/                      # Plugin interface types (SomaPlugin trait, Value, Convention)
    crypto/                   # 13 conventions: hash, sign, encrypt, JWT, random generation
    postgres/                 # 15 conventions: query, execute, ORM-style find/count/aggregate
    redis/                    # 14 conventions: strings, hashes, lists, pub/sub, keys
    auth/                     # 10 conventions: OTP verification, session management, TOTP
    geo/                      # 5 conventions: distance, radius filter, geocoding
    http-bridge/              # 5 conventions: HTTP client (GET/POST/PUT/DELETE)

  soma-synthesizer/           # Python build tool
    pyproject.toml            # PyTorch, ONNX deps
    README.md                 # Comprehensive docs
    soma_synthesizer/
      __init__.py
      cli.py                  # soma-synthesize: train, train-lora, export, validate, test, benchmark, export-experience
      config.py               # SynthesisConfig from TOML (architecture, training, augmentation, lora)
      tokenizer.py            # Word-level + BPE tokenizers, find_span
      model.py                # SomaMind (BiLSTM+GRU, 11 output heads), TransformerMind (stub)
      data.py                 # ConventionCatalog, training data collection, expansion, Dataset
      trainer.py              # SomaTrainer: combined loss, 7 eval metrics, early stopping
      augmentor.py            # Synonym replacement, word dropout, shuffle, typo injection
      validator.py            # Training data validation (7 checks, errors + warnings)
      exporter.py             # ONNX export, .soma-model int8, catalog.json, meta.json with SHA-256
      lora.py                 # LoRALinear, apply/remove/save/load/merge, plugin-specific training

  soma-helperbook/            # First real-world SOMA application
    docker-compose.yml        # PostgreSQL + Redis services
    schema.sql                # 19 tables (users, connections, messages, chats, appointments, etc.)
    seed.sql                  # Test data (users, chats, messages, appointments, reviews)
    soma.toml                 # HelperBook-specific SOMA config
    domain/                   # Domain-specific training data for Mind synthesis
    scripts/                  # setup-db, seed-db, clean-db, start, start-mcp, synthesize
    frontend/                 # Plain JS + Tailwind CSS, Express bridge to SOMA MCP
      server.js               # Express server bridging HTTP to SOMA MCP
      index.html              # Single-page app

  poc/                        # Python proof of concept (v0.1-v0.3)
  pow/                        # Proofs of work (POW1, POW2, POW3)
  models/                     # Exported ONNX models
  export_onnx.py              # Legacy ONNX exporter (superseded by soma-synthesizer)
```

## Build and Test

```bash
# Rust runtime
cd soma-core
cargo build --release          # 14MB binary
cargo test                     # 101 tests

# Run
cargo run --bin soma -- --model ../models --intent "list files in /tmp"
cargo run --bin soma -- --model ../models --mcp   # MCP server mode
cargo run --bin soma-dump -- 127.0.0.1:9999       # Signal capture

# Plugins
cd soma-plugins
cargo build --release          # Produces .dylib/.so per plugin

# Python synthesizer
cd soma-synthesizer
pip install -e .
soma-synthesize train --plugins ./plugins --output ./models
soma-synthesize validate --plugins ./plugins

# HelperBook (first application)
cd soma-helperbook
docker compose up -d --wait    # PostgreSQL + Redis
scripts/setup-db.sh            # Apply schema (19 tables)
scripts/seed-db.sh             # Seed test data
cd frontend && npm install && node server.js  # http://localhost:8080
```

## Key Design Decisions

- **tract-onnx over ort**: Pure Rust, no C++ deps, sufficient for <50M param models. See `mind/onnx_engine.rs` header.
- **infer(&str) not infer(tokens)**: Tokenizer is internal to engine, encapsulation over spec purity. See `mind/mod.rs` trait doc.
- **Convention IDs offset by plugin_idx*1000**: Prevents routing conflicts. See `plugin/manager.rs` register().
- **Arc<RwLock<PluginManager>>**: Enables runtime convention registration (MCP Bridge plugin). Write lock only for install_plugin.
- **Interior mutability for crashed plugins**: `RwLock<HashSet>` so `execute_step(&self)` can mark crashes.
- **Experience records successes only**: Spec Section 17.1 — don't reinforce bad programs.
- **Checkpoint version 2**: Backwards-compat with v1 via `#[serde(default)]`. Includes SHA-256 model hash.
- **Real crypto**: ChaCha20-Poly1305, X25519, Ed25519 via dalek crates. Not placeholders.
- **Synchronous postgres plugin**: Uses `block_on()` for tokio-postgres inside the sync SomaPlugin trait. Connection pooled and cached across calls.
- **Cached LoRA adaptation**: Runtime adaptation via gradient descent on frozen decoder hidden states. Teacher forcing with experience replay. No Python needed.
- **Catalog routing via plugin_idx*1000**: Each external plugin gets a unique index range. Postgres conventions at 2000+, redis at 3000+, etc. See `plugin/manager.rs`.
- **BPE tokenizer in Rust**: Auto-detected from tokenizer.json format. Handles OOV, SQL, URLs. Character-level fallback for unknown subwords.

## Config

All config in `soma.toml` (see `soma.toml.example`). Override order: defaults < TOML < env vars (SOMA_*) < CLI flags.

Key env vars: `SOMA_MCP_ADMIN_TOKEN`, `SOMA_MCP_BUILDER_TOKEN`, `SOMA_MCP_VIEWER_TOKEN`, `SOMA_LOG_JSON=1`, `SOMA_MIND_TEMPERATURE`, `SOMA_PROTOCOL_BIND`.

## Documentation

The `docs/` directory contains all consolidated documentation. The `SOMA_Whitepaper.md` is the master specification. Each doc in `docs/` covers one topic with zero redundancy:

| Doc | Covers |
|-----|--------|
| `docs/architecture.md` | 6 core components, design decisions, runtime behavior |
| `docs/mind-engine.md` | Neural inference, LoRA, tokenizer, memory system |
| `docs/synaptic-protocol.md` | Binary wire protocol (SOMA-to-SOMA) |
| `docs/mcp-interface.md` | LLM integration, all MCP tools, auth |
| `docs/plugin-system.md` | Plugin architecture, trait, conventions, Value |
| `docs/plugin-catalog.md` | All plugins + conventions reference |
| `docs/plugin-development.md` | Tutorial: build a plugin |
| `docs/synthesizer.md` | Training pipeline (PyTorch to ONNX) |
| `docs/building-apps.md` | Step-by-step app building guide |
| `docs/helperbook.md` | HelperBook application guide |
| `docs/web4.md` | Web 4 vision document |
| `docs/roadmap.md` | Status and milestones |

## Rules

- **NEVER GUESS.** If you don't know the answer, read the code. If the code doesn't have the answer, read the spec. If the spec doesn't cover it, ask the user. Do not speculate, assume, or fabricate answers. Always verify by reading the actual source before responding.
- **NO SPEC CITATIONS IN COMMENTS.** Never write comments like `// §7.1:`, `// S13.2.1:`, `// MUST`, `// Section 12`, or any reference to spec section numbers or RFC keywords. Comments explain what the code does and why — not where the requirement came from.

## When Editing

### Rust (soma-core)
- Run `cargo test` after changes — must stay at 101+ tests passing.
- Run `cargo build` — 0 errors required, warnings OK.
- The docs in `docs/` and `SOMA_Whitepaper.md` are the source of truth. Code should match docs.
- Don't remove "unused" code that implements spec features not yet wired.
- Plugin interface changes ripple to: builtin.rs, manager.rs, mcp/server.rs, mcp/tools.rs, main.rs.
- Convention struct and Value enum are core types — changes affect almost everything.
- PluginManager is behind Arc<RwLock<>> — use `.read().unwrap()` for reads, `.write().unwrap()` only for registration.

### Python (soma-synthesizer)
- Self-contained — zero imports from poc/ or pow/.
- Requires PyTorch 2.0+ (build-time only, not runtime).
- `soma-synthesize validate` should always pass before training.
- model.py and trainer.py are the core — they define the neural architecture and loss function.

### Rust (soma-plugins)
- Each plugin is a cdylib crate in the `soma-plugins/` workspace.
- All plugins depend on `soma-plugin-sdk` from `soma-plugins/sdk/`.
- Each plugin has `manifest.json` + `training/examples.json`.
- `cargo build --release` from `soma-plugins/` builds all plugins.

### HelperBook (soma-helperbook)
- Requires Docker for PostgreSQL + Redis (`docker compose up -d`).
- Schema changes go in `schema.sql`, test data in `seed.sql`.
- Frontend is plain JS (no build step). Express server bridges HTTP to SOMA MCP.

## What's Deferred (roadmap, not current scope)

- EmbeddedMindEngine for ESP32 (no_std)
- TransformerMind architecture (stub exists, not implemented)
- WASM sandbox for untrusted plugins (wasmtime)
- LoRA MoE gating network
- soma-replay and soma-mock tools
- Plugin registry (download/cache)
- Diffuse memory tier (peer queries)
