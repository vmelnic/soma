# SOMA Project — Claude Code Instructions

## What This Is

SOMA (from Greek "body") is a computational paradigm where a neural architecture IS the program. No application source code. A trained neural Mind receives structured intents, generates execution programs, and orchestrates plugins that interface with hardware, databases, networks, and other systems.

Two deliverables:
- **soma-core/** — Rust runtime (single binary, 14MB). THE production deliverable.
- **soma-synthesizer/** — Python build tool (PyTorch). Trains the Mind, exports ONNX models.

## Architecture (6 core components)

```
Mind Engine (ONNX inference) + Plugin Manager + Memory System
+ Synaptic Protocol (SOMA-to-SOMA) + MCP Server (LLM-to-SOMA) + Proprioception
```

## Repository Structure

```
soma/
  SOMA_Whitepaper.md          # Master specification (v0.3)
  01_CORE_REFACTORING.md      # Core binary spec (20 sections)
  02_SYNAPTIC_PROTOCOL.md     # Binary wire protocol spec (23 sections)
  03_PLUGINS.md               # Plugin system spec (20 sections)
  04_HELPERBOOK.md            # First application spec
  05_PLUGIN_CATALOG.md        # 40 plugin catalog
  06_INTERFACE_SOMA.md        # Frontend renderer spec
  07_SYNTHESIZER.md           # Training pipeline spec (14 sections)
  08_DEVELOPER_GUIDE.md       # Plugin development guide
  09_CONVERSATIONAL_INTERACTION.md
  00_ROADMAP.md               # Milestone ordering

  soma-core/                  # Rust runtime
    Cargo.toml                # 17 crate dependencies
    soma.toml.example         # All config fields documented
    README.md                 # Comprehensive usage + architecture docs
    src/
      main.rs                 # Entry point, CLI, REPL, startup/shutdown
      config/mod.rs           # TOML config (8 sections), env var overrides (SOMA_*)
      errors.rs               # SomaError: Inference, Plugin, Protocol, Mcp, State, Auth
      mind/
        mod.rs                # MindEngine trait, Program, ProgramStep, MindConfig
        onnx_engine.rs        # tract-onnx inference with LoRA + temperature
        tokenizer.rs          # Word-level vocab
        lora.rs               # LoRALayer (forward/merge/reset), LoRAWeights, LoRACheckpoint
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
        signal.rs             # 24 SignalTypes, 6 flags, Signal struct
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
cargo test                     # 57 tests

# Run
cargo run --bin soma -- --model ../models --intent "list files in /tmp"
cargo run --bin soma -- --model ../models --mcp   # MCP server mode
cargo run --bin soma-dump -- 127.0.0.1:9999       # Signal capture

# Python synthesizer
cd soma-synthesizer
pip install -e .
soma-synthesize train --plugins ./plugins --output ./models
soma-synthesize validate --plugins ./plugins
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

## Config

All config in `soma.toml` (see `soma.toml.example`). Override order: defaults < TOML < env vars (SOMA_*) < CLI flags.

Key env vars: `SOMA_MCP_ADMIN_TOKEN`, `SOMA_MCP_BUILDER_TOKEN`, `SOMA_MCP_VIEWER_TOKEN`, `SOMA_LOG_JSON=1`, `SOMA_MIND_TEMPERATURE`, `SOMA_PROTOCOL_BIND`.

## Spec Compliance

Implementation validated against specs with 10-agent parallel audits:

| Spec | Items | Pass |
|------|-------|------|
| `01_CORE_REFACTORING.md` | 30 | 30 |
| `02_SYNAPTIC_PROTOCOL.md` | 85 | 85 |
| `03_PLUGINS.md` | 63 | 63 |
| `05_PLUGIN_CATALOG.md` | 61 | 61 |
| `07_SYNTHESIZER.md` | 89 | 87 (+2 documented-future) |
| **Total** | **328** | **326** |

## Rules

- **NEVER GUESS.** If you don't know the answer, read the code. If the code doesn't have the answer, read the spec. If the spec doesn't cover it, ask the user. Do not speculate, assume, or fabricate answers. Always verify by reading the actual source before responding.

## When Editing

### Rust (soma-core)
- Run `cargo test` after changes — must stay at 57+ tests passing.
- Run `cargo build` — 0 errors required, warnings OK.
- The spec documents (01-09) are the source of truth. Code should match specs.
- Don't remove "unused" code that implements spec features not yet wired.
- Plugin interface changes ripple to: builtin.rs, manager.rs, mcp/server.rs, mcp/tools.rs, main.rs.
- Convention struct and Value enum are core types — changes affect almost everything.
- PluginManager is behind Arc<RwLock<>> — use `.read().unwrap()` for reads, `.write().unwrap()` only for registration.

### Python (soma-synthesizer)
- Self-contained — zero imports from poc/ or pow/.
- Requires PyTorch 2.0+ (build-time only, not runtime).
- `soma-synthesize validate` should always pass before training.
- model.py and trainer.py are the core — they define the neural architecture and loss function.

## What's Deferred (roadmap, not current scope)

- EmbeddedMindEngine for ESP32 (no_std)
- TransformerMind architecture (stub exists, not implemented)
- WASM sandbox for untrusted plugins (wasmtime)
- LoRA MoE gating network
- soma-replay and soma-mock tools
- Plugin registry (download/cache)
- Diffuse memory tier (peer queries)
- Actual catalog plugins (postgres, redis, mcp-bridge — infrastructure ready, plugins not yet built)
