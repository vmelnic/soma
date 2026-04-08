# SOMA Core

The single Rust binary that IS a SOMA. Neural mind generates programs, drives plugins directly. No Python, no Node, no JVM at runtime.

```
./soma --model ../models --mcp
```

One binary. One file. Copy and run.

## What It Does

SOMA Core receives structured intents (from LLMs via MCP or from peer SOMAs via Synaptic Protocol), generates execution programs through a trained neural Mind, and orchestrates plugins that interface with hardware, databases, networks, and other systems.

```
Human ←→ LLM (Claude, ChatGPT, Ollama) ←→ MCP ←→ SOMA Core ←→ Plugins (body)
                                                        ↕
                                                  Peer SOMAs (Synaptic Protocol)
```

## Build

```bash
cargo build --release
```

Produces two binaries:
- `soma` (14MB) — the SOMA runtime
- `soma-dump` (896KB) — Synaptic Protocol signal capture tool

## Usage

### Interactive REPL (development)

```bash
./soma --model ../models

intent> list files in /tmp

  [Mind] Program (4 steps, 100%):
    $0 = libc.opendir("/tmp")
    $1 = libc.readdir($0)
    $2 = libc.closedir($0)
    $3 = EMIT($1)
    STOP

  [Body] (17 items):
    file1.txt
    file2.txt
    ...
```

REPL commands: `:status` `:inspect` `:checkpoint` `:consolidate` `:decisions` `:metrics`

### MCP Server Mode (production)

```bash
./soma --model ../models --mcp
```

Runs JSON-RPC 2.0 on stdio. Any MCP-compatible LLM connects and drives SOMA:

**State tools:** `soma.get_state`, `soma.get_plugins`, `soma.get_conventions`, `soma.get_health`, `soma.get_recent_activity`, `soma.get_peers`, `soma.get_experience`, `soma.get_checkpoints`, `soma.get_config`, `soma.get_decisions`, `soma.get_metrics`, `soma.get_schema`, `soma.get_business_rules`, `soma.get_render_state`

**Action tools:** `soma.intent`, `soma.checkpoint`, `soma.record_decision`, `soma.confirm`, `soma.install_plugin`, `soma.restore_checkpoint`, `soma.shutdown`

**Plugin tools:** Every loaded convention exposed as `soma.{plugin}.{convention}` (e.g., `soma.posix.read_file`, `soma.posix.list_dir_simple`)

### Single Intent

```bash
./soma --model ../models --intent "read hello.txt"
```

### Signal Capture

```bash
soma-dump 127.0.0.1:9999
soma-dump 127.0.0.1:9999 --signal-type intent --channel 5
soma-dump 127.0.0.1:9999 --count 100 --raw
```

### CLI Options

```
soma [OPTIONS]

  --model <dir>         Model directory (encoder.onnx, decoder.onnx, tokenizer.json, meta.json)
  --config <file>       Configuration file (default: soma.toml)
  --intent <text>       Single intent (non-interactive)
  --mcp                 Run MCP server on stdio
  --bind <addr:port>    Synaptic Protocol bind address
  --peer <name:addr>    Additional peer (repeatable)
  --checkpoint <file>   Restore specific checkpoint
  --log-level <level>   Override log level (trace/debug/info/warn/error)
```

## Architecture

Six core components:

```
┌──────────────────────────────────────────────────────┐
│  SOMA Core Binary                                     │
│                                                       │
│  ┌─────────────┐  ┌───────────────┐  ┌────────────┐ │
│  │  Mind Engine │  │  Synaptic     │  │  MCP       │ │
│  │  (ONNX)     │  │  Protocol     │  │  Server    │ │
│  └──────┬──────┘  └──────┬────────┘  └─────┬──────┘ │
│  ┌──────┴────────────────┴──────────────────┴──────┐ │
│  │  Plugin Manager                                  │ │
│  └──────┬──────────────────────────────────────────┘ │
│  ┌──────┴──────┐  ┌──────────────┐  ┌────────────┐  │
│  │  Memory     │  │ Proprioception│  │  Metrics   │  │
│  └─────────────┘  └──────────────┘  └────────────┘  │
└──────────────────────────────────────────────────────┘
```

### Mind Engine (`mind/`)

- **OnnxMindEngine** — ONNX inference via `tract-onnx` (pure Rust, CPU)
- BiLSTM encoder + GRU autoregressive decoder
- 11 output heads: opcode, arg types x2, span positions x4, ref pointers x2, literal values x2
- LoRA adaptation: active layers applied as post-hoc logit adjustment during inference
- Runtime LoRA adaptation: gradient descent on frozen decoder hidden states using successful experiences (no Python needed)
- Temperature-controlled softmax for deterministic execution
- Model integrity: SHA-256 hash verified against checkpoint on restore

### Plugin Manager (`plugin/`)

- **SomaPlugin trait** — 18 methods covering identity, execution, streaming, state, lifecycle, LoRA, config
- **Convention struct** — id, name, args (ArgType enum), returns (ReturnSpec), cleanup (CleanupSpec), side_effects, latency, determinism
- **Value enum** — Null, Bool, Int, Float, String, Bytes, List(recursive), Map(recursive), Handle, Signal
- **PosixPlugin** — 22 built-in libc conventions (open/read/write/close + high-level read_file/write_file/list_dir/copy)
- **Dynamic loading** — `libloading` for .so/.dylib at runtime, `soma_plugin_init` C ABI
- **Manifest parsing** — reads `manifest.json` for metadata, platform compat, dependencies
- **Plugin signing** — Ed25519 verification via `ed25519-dalek`
- **External plugins** — 6 catalog plugins in `soma-plugins/` workspace (crypto, postgres, redis, auth, geo, http-bridge), built as cdylib crates using the shared SDK at `soma-plugins/sdk/`
- **Crash isolation** — `catch_unwind` + `RwLock<HashSet>` marks crashed plugins, refuses re-execution
- **Cleanup conventions** — backwards walk on program failure, invoke cleanup (e.g., close_fd, rollback)
- **Per-convention stats** — call count, duration, error count tracked via `ConventionStats`
- **Runtime registration** — `Arc<RwLock<PluginManager>>` allows adding conventions after startup

### Synaptic Protocol (`protocol/`)

Binary wire protocol for SOMA-to-SOMA communication. 22-byte overhead per signal.

| Module | What |
|--------|------|
| `signal.rs` | 24 signal types, 6 flag bits, Signal struct |
| `codec.rs` | Binary encode/decode, CRC32, zstd compression, ChaCha20-Poly1305 encryption |
| `connection.rs` | TCP management, heartbeat (30s PING, 10s PONG timeout), RTT tracking, channels, session tokens (24h expiry) |
| `server.rs` | TCP listener, capability enforcement, PubSub SUBSCRIBE/UNSUBSCRIBE handling, max connection limit, per-connection metrics |
| `client.rs` | Auto-reconnect (exponential backoff 100ms→60s), subscription replay on reconnect |
| `router.rs` | SignalRouter with DashMap pending_requests, 30s request-response correlation timeout |
| `discovery.rs` | PeerRegistry, DISCOVER with TTL-based forwarding (chemical gradient), PEER_QUERY/PEER_LIST with reachable_via |
| `relay.rs` | Multi-hop forwarding, loop prevention via relay_path, max_hops=3, capability gating |
| `chunked.rs` | Resumable file transfer, SHA-256 verification, CHUNK_ACK |
| `pubsub.rs` | Topic wildcards (`notifications:*`), durable subscriptions with catch-up replay, fan-out |
| `streaming.rs` | STREAM_START/DATA/END lifecycle, frame counting |
| `rate_limit.rs` | Graduated response (Warning→Sustained→Severe), CONTROL/rate_limit signal, PeerBlacklist (5 min) |
| `offline_queue.rs` | Priority-based store-and-forward, expired signal drop |
| `encryption.rs` | ChaCha20-Poly1305 AEAD, X25519 ECDH key exchange, Ed25519 identity/signing (real crypto, not placeholders) |
| `websocket.rs` | WebSocket transport adapter for browser renderers |
| `unix_socket.rs` | Unix Domain Socket transport for same-host SOMAs |

### MCP Server (`mcp/`)

JSON-RPC 2.0 over stdio. Milestone 3: "At this point, an LLM can drive SOMA."

- 22+ tools (state queries + actions + every plugin convention)
- Auth: admin/builder/viewer levels from env vars (`SOMA_MCP_ADMIN_TOKEN`, etc.)
- Audit trail: every tool call logged via `tracing::info!`
- Destructive action confirmation: `soma.restore_checkpoint` requires explicit `confirmed: true`
- Resources: `soma://state` and `soma://metrics` via `resources/read`
- Shutdown: `soma.shutdown` tool for graceful MCP-triggered exit

### Memory (`memory/`)

- **Checkpoint** — SOMA magic header, version 2 (backwards compat with v1), SHA-256 model hash, LoRA state, plugin states, plugin manifest, decisions, execution history
- **Experience buffer** — ring buffer, records successes only (Whitepaper Section 17.1), bounded
- **Consolidation** — threshold check + merge structure (LoRA → base weight merge when integrated)

### State (`state/`)

Permanent institutional memory — survives across LLM sessions:

- **Decision log** — what was built, why, when, by which session
- **Execution history** — bounded ring buffer of recent executions with trace_id

### Metrics (`metrics/`)

20 Prometheus-compatible metrics:

```
soma_inferences_total, soma_inferences_success, soma_inferences_failed
soma_inference_duration_sum_ms, soma_inference_confidence_avg
soma_programs_executed, soma_program_steps_executed
soma_plugin_calls_total, soma_plugin_calls_failed, soma_plugin_retries, soma_plugin_duration_sum_ms
soma_experience_buffer_size, soma_adaptations_total, soma_checkpoints_saved
soma_memory_rss_bytes, soma_lora_magnitude
soma_protocol_connections_active, soma_protocol_signals_sent/received, soma_protocol_bytes_transferred
```

### Proprioception (`proprioception/`)

Self-model: uptime, inference stats, success rate, adaptations, checkpoints, consolidations, decisions, active connections, signals processed, RSS memory tracking via `libc::getrusage`.

## Configuration

All config in `soma.toml` (see `soma.toml.example` for all fields). Override order:

```
Compiled defaults < soma.toml < SOMA_* env vars < CLI flags
```

Key sections:

```toml
[soma]
id = "my-soma"
log_level = "info"
trace_verbosity = "normal"     # terse / normal / verbose
plugins_directory = "plugins"

[mind]
model_dir = "models"
max_program_steps = 32
temperature = 1.0              # lower = more deterministic
max_inference_time_secs = 5

[mind.lora]
adaptation_enabled = true
max_lora_layers = 64

[memory]
auto_checkpoint = true
max_checkpoints = 5
checkpoint_interval_secs = 3600

[memory.consolidation]
enabled = true
trigger = "experience_count"
threshold = 500

[protocol]
bind = "127.0.0.1:9999"
max_connections = 16
connection_timeout_secs = 60

[protocol.encryption]
enabled = false

[mcp]
enabled = true
transport = "stdio"

[security]
require_auth = false
admin_token_env = "SOMA_MCP_ADMIN_TOKEN"
require_confirmation = true

[resources]
max_concurrent_inferences = 4
max_plugins_loaded = 50
```

## Startup Sequence

1. Load config (TOML + env vars + CLI)
2. Initialize logging (JSON lines when `SOMA_LOG_JSON=1`)
3. Load Mind Engine (ONNX models + SHA-256 hash)
4. **Verify model** (test inference on "ping")
5. Load plugins (built-in + dynamic scan from plugins directory)
6. Initialize state system (decisions + execution history)
7. Initialize metrics (20 Prometheus counters)
8. Restore checkpoint (verify model hash, restore decisions + LoRA)
9. Start Synaptic Protocol server
10. Start MCP server (if `--mcp`)
11. Ready

Failure handling: missing model = fatal. Plugin load fail = skip + continue. Corrupt checkpoint = start fresh.

## Shutdown Sequence

1. Stop accepting new connections
2. Notify peers with CLOSE signals (joined threads)
3. Drain in-flight requests (10s timeout)
4. Flush outbound queues
5. Auto-checkpoint (includes plugin state, decisions, model hash)
6. Unload plugins (reverse order)
7. Close MCP server
8. Close Synaptic listeners
9. Final log

## Testing

```bash
cargo test                    # 101 tests
cargo build --release         # 14MB binary
```

Tests cover: codec roundtrip (9 tests), encryption (5), pub/sub (7), rate limiting (5), relay (6), streaming (6), chunked transfer (6), offline queue (4), signal router (4), unknown signal handling (1), BPE tokenizer (13), adaptation engine, LoRA layers, and more.

## Source Files

```
src/
  main.rs                     # Entry point, CLI, REPL, startup/shutdown
  config/mod.rs               # TOML config with 8 sections, env var overrides
  errors.rs                   # SomaError: Inference, Plugin, Protocol, Mcp, State, Auth
  mind/
    mod.rs                    # MindEngine trait, Program, ProgramStep, MindConfig
    onnx_engine.rs            # tract-onnx inference with LoRA + temperature
    tokenizer.rs              # Word-level + BPE tokenizers, auto-detected from JSON
    lora.rs                   # LoRALayer (forward/merge/reset), LoRAWeights, LoRACheckpoint
    adaptation.rs             # Runtime LoRA adaptation via gradient descent on experiences
  plugin/
    interface.rs              # SomaPlugin trait (18 methods), Value (10 variants),
                              # Convention, ArgType, ReturnSpec, CleanupSpec, TrustLevel,
                              # PluginPermissions, PluginConfig
    manager.rs                # Arc<RwLock<PluginManager>>, routing, cleanup walk,
                              # ConventionStats, topological sort, name resolution
    builtin.rs                # PosixPlugin: 22 libc + high-level conventions
    dynamic.rs                # libloading, scan, manifest parsing, Ed25519 verify
    process.rs                # ProcessManager for child processes (MCP Bridge)
  protocol/
    signal.rs                 # 24 SignalTypes, 6 flags, Signal struct
    codec.rs                  # Binary wire format, CRC32, zstd, ChaCha20-Poly1305
    connection.rs             # TCP, heartbeat, RTT, channels, session token (24h)
    server.rs                 # Listener, PubSub, capability enforcement, metrics
    client.rs                 # Auto-reconnect, subscription replay
    router.rs                 # SignalRouter, DashMap pending_requests, 30s timeout
    discovery.rs              # PeerRegistry, TTL forwarding, PEER_QUERY
    relay.rs                  # Multi-hop, loop prevention, max_hops
    chunked.rs                # Resumable transfer, SHA-256
    pubsub.rs                 # Wildcards, durable, catch-up, fan-out
    streaming.rs              # Stream lifecycle, frame counting
    rate_limit.rs             # Graduated response, CONTROL signal, blacklist
    offline_queue.rs          # Priority queue, expiry
    encryption.rs             # ChaCha20-Poly1305, X25519, Ed25519
    websocket.rs              # WebSocket adapter
    unix_socket.rs            # Unix Domain Socket
  memory/
    checkpoint.rs             # v2 format, SHA-256 model hash, plugin state + manifest
    experience.rs             # Ring buffer (successes only)
    consolidation.rs          # Merge threshold + structure
  mcp/
    server.rs                 # JSON-RPC 2.0, 22+ tools, auth, audit trail
    tools.rs                  # Tool definitions (state + action + plugin)
    auth.rs                   # Admin/builder/viewer from env vars
  metrics/mod.rs              # 20 Prometheus counters, JSON + text exposition
  proprioception/mod.rs       # Self-model, RSS tracking
  state/
    decision_log.rs           # Permanent decisions
    execution_history.rs      # Bounded executions
  bin/
    soma_dump.rs              # Signal capture CLI tool
```

**45 source files. ~14,700 lines of Rust. 101 tests. 14MB release binary.**

## Spec Compliance

Validated against specification documents with 10-agent parallel audits:

| Spec | Items | Pass |
|------|-------|------|
| [`architecture.md`](../docs/architecture.md) + [`mind-engine.md`](../docs/mind-engine.md) (20 sections) | 30 | 30 |
| [`synaptic-protocol.md`](../docs/synaptic-protocol.md) (23 sections) | 85 | 85 |
| [`plugin-system.md`](../docs/plugin-system.md) (20 sections) | 63 | 63 |
| [`plugin-catalog.md`](../docs/plugin-catalog.md) (40 plugins) | 61 | 61 |
| **Total** | **239** | **239** |

## Dependencies

| Crate | Purpose |
|-------|---------|
| `tract-onnx` | ONNX inference (pure Rust, no C++ deps) |
| `tokio` | Async runtime |
| `serde` / `serde_json` / `rmp-serde` | JSON + MessagePack serialization |
| `clap` | CLI argument parsing |
| `tracing` / `tracing-subscriber` | Structured logging (JSON lines) |
| `chacha20poly1305` | Per-signal AEAD encryption |
| `x25519-dalek` / `ed25519-dalek` | Key exchange + identity signing |
| `crc32fast` | Wire format checksums |
| `zstd` | Signal payload compression |
| `sha2` | Model + chunk integrity |
| `dashmap` | Lock-free concurrent maps (SignalRouter) |
| `libloading` | Dynamic .so/.dylib plugin loading |
| `tokio-tungstenite` / `futures-util` | WebSocket transport |
| `toml` | Configuration parsing |
| `uuid` | Trace ID generation |
| `bitflags` | Signal flag bits |
| `soma-plugin-sdk` | Shared plugin interface types (from `soma-plugins/sdk/`) |
