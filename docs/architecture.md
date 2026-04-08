# Architecture

## Overview

SOMA (from Greek "body") is a computational paradigm where a trained neural Mind
receives structured intents, generates execution programs, and orchestrates plugins
that interface with hardware, databases, networks, and other systems. No application
source code is written.

The runtime is a single Rust binary (~14MB) with zero runtime dependencies. Python
is used only at build time (the Synthesizer trains the Mind and exports ONNX models).

### Six Core Components

```
                     Human
                       |  natural language
                       v
                  LLM (any)
                       |  MCP (JSON-RPC 2.0)
                       v
  +--------------------------------------------+
  |  SOMA Core Binary                          |
  |                                            |
  |  +----------+  +-----------+  +----------+ |
  |  |   Mind   |  | Synaptic  |  |   MCP    | |
  |  |  Engine  |  | Protocol  |  |  Server  | |
  |  |  (ONNX)  |  | (SOMA <-> |  | (LLM <->| |
  |  |          |  |   SOMA)   |  |   SOMA)  | |
  |  +----+-----+  +-----+----+  +----+-----+ |
  |       |              |             |       |
  |  +----+--------------+-------------+-----+ |
  |  |  Plugin Manager                       | |
  |  |  (discover, load, route, compose)     | |
  |  +----+----------------------------------+ |
  |       |                                    |
  |  +----+------+  +---------------------+   |
  |  |  Memory   |  |  Proprioception     |   |
  |  |  (LoRA +  |  |  (self-model,       |   |
  |  | checkpoint)|  |   health, caps)    |   |
  |  +-----------+  +---------------------+   |
  |                                            |
  +--------------------------------------------+
        |                         |
   +----+----+              +-----+------+
   | Plugins |              |   Peer     |
   | (.so /  |              |   SOMAs    |
   | built-in)|             +------------+
   +---------+
```

| Component        | Role                                                          |
|------------------|---------------------------------------------------------------|
| Mind Engine      | Maps intents to programs via neural inference (BiLSTM + GRU)  |
| Plugin Manager   | Loads plugins, maintains convention catalog, routes execution  |
| Memory System    | LoRA adaptation, checkpoints, experience recording            |
| Synaptic Protocol| Binary wire protocol for SOMA-to-SOMA communication           |
| MCP Server       | JSON-RPC 2.0 interface for LLM-to-SOMA communication          |
| Proprioception   | Self-model: health, capabilities, resource usage              |

SOMA is a pure executor. It does not converse or generate natural language.
Conversational intelligence is provided by external LLMs (Claude, ChatGPT, Ollama)
that connect via MCP. The LLM brings understanding. SOMA brings state, memory,
and execution.

---

## Mind Engine

The Mind maps structured intents to executable programs. It consists of an encoder
and an autoregressive decoder.

**Encoder (BiLSTM):** Bidirectional LSTM, 2 layers. Input: tokenized intent
(BPE vocabulary, 2K-6K tokens). Output: contextualized encoding plus a pooled
representation used as decoder initial context.

**Decoder (GRU):** At each step, produces:
- Opcode logits: which convention to call (softmax over all conventions + EMIT + STOP)
- Argument type logits: literal, span (from intent), or ref (previous step result)
- Span position logits: start and end positions within the intent
- Ref logits: pointer attention to previous step results

The decoder runs until STOP or `max_program_steps` (default: 16).

### MindEngine Trait

```rust
pub trait MindEngine: Send + Sync {
    fn load(&mut self, model_path: &Path, config: &MindConfig) -> Result<()>;
    fn infer(&self, tokens: &[u32], length: usize) -> Result<Program>;
    fn info(&self) -> MindInfo;
    fn attach_lora(&mut self, name: &str, weights: &LoRAWeights) -> Result<()>;
    fn detach_lora(&mut self, name: &str) -> Result<()>;
    fn merge_lora(&mut self, name: &str) -> Result<()>;
    fn checkpoint_lora(&self) -> Result<LoRACheckpoint>;
    fn restore_lora(&mut self, checkpoint: &LoRACheckpoint) -> Result<()>;
}
```

### Dual Backends

| Backend           | Target              | Model Format    | RAM         | LoRA        |
|-------------------|---------------------|-----------------|-------------|-------------|
| OnnxMindEngine    | Server, desktop, Pi | .onnx (f32/f16) | 50MB-8GB+   | Rank 4-64   |
| EmbeddedMindEngine| ESP32, MCUs         | .soma-model (i8)| 168KB-2MB   | Rank 2-4    |

Both backends implement the same trait. The rest of SOMA does not know which is running.

### Program Structure

A program is a sequence of steps. Each step: `(convention_name, arg0_type, arg0_value, ...)`.

```
Step 0: fs.list_dir("/tmp")              <- literal arg
Step 1: redis.set("files:tmp", $0, 300)  <- $0 refs step 0 result
Step 2: EMIT($0)                         <- return to caller
Step 3: STOP
```

For full details on inference pipeline, LoRA adaptation, and model formats,
see [docs/mind-engine.md](mind-engine.md).

---

## Plugin Manager

Everything outside the six core components is a plugin. A SOMA with zero plugins
is a brain in a jar -- it can think but cannot act. Plugins are its body parts.

### Shared State

The Plugin Manager is behind `Arc<RwLock<PluginManager>>`:
- **Read lock** for convention execution (many concurrent requests)
- **Write lock** only for plugin registration (`install_plugin`)

Crashed plugins are tracked via `RwLock<HashSet>` with interior mutability, so
`execute_step(&self)` can mark crashes without requiring a write lock on the
manager itself.

### Convention Routing

Each plugin gets a unique index. Convention IDs are offset by `plugin_idx * 1000`:
- Built-in (PosixPlugin): conventions at 0-999
- Plugin index 2: conventions at 2000-2999
- Plugin index 3: conventions at 3000-3999

This prevents routing conflicts when multiple plugins are loaded. The Mind predicts
convention names (or name hashes), and the Plugin Manager resolves to runtime IDs.

### Plugin Loading

Three modes:
- **Built-in:** Compiled into the binary. Required for embedded targets.
- **Dynamic:** Loaded from `.so` (Linux), `.dylib` (macOS) at runtime via `libloading`. Manifest parsed, Ed25519 signature verified.
- **Catalog (external):** Packaged as `.soma-plugin` archives with manifest, binary, LoRA weights, and training data.

### Dependency Resolution

Plugins declare dependencies in their manifest. The Plugin Manager resolves via
topological sort. Each convention can declare a cleanup action -- if step 3 fails
and step 1 opened a transaction, the cleanup convention (rollback) is called
automatically.

### SomaPlugin Trait

```rust
pub trait SomaPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn conventions(&self) -> Vec<CallingConvention>;
    fn execute(&self, conv_id: u32, args: Vec<Value>) -> Result<Value, PluginError>;
    fn lora_weights(&self) -> Option<&[u8]>;        // optional
    fn training_data(&self) -> Option<Vec<TrainingExample>>; // optional
    fn checkpoint_state(&self) -> Option<Vec<u8>>;   // optional
    fn restore_state(&mut self, state: &[u8]) -> Result<(), PluginError>; // optional
    fn on_load(&mut self, config: &PluginConfig) -> Result<(), PluginError>;
    fn on_unload(&mut self) -> Result<(), PluginError>;
}
```

A plugin provides two things:
1. **Calling conventions** -- operations the Mind can invoke
2. **LoRA knowledge** (optional) -- pre-trained weights that teach the Mind how to
   use the conventions. A plugin with LoRA is like a surgeon with both scalpel and training.

For full details on plugin interface, distribution format, categories, and the
convention catalog, see [docs/plugin-system.md](plugin-system.md).

---

## Memory System

Inspired by complementary learning systems theory (McClelland et al., 1995) and
sleep consolidation research (Diekelmann & Born, 2010).

### Four-Tier Hierarchy

| Tier         | Biological Analogy       | Implementation                    | Lifetime                     |
|--------------|--------------------------|-----------------------------------|------------------------------|
| Permanent    | Neocortical long-term    | Base model weights (ONNX)         | Immutable until re-synthesis |
| Experiential | Hippocampal recent       | LoRA A/B matrices                 | Grows at runtime, checkpointable |
| Working      | Active neural firing     | Decoder hidden states, context    | Per-execution, transient     |
| Diffuse      | Asking a colleague       | Synaptic queries to peer SOMAs    | Network-dependent            |

### Experience Recording

After successful executions, experience is recorded in a ring buffer. Only
successes are stored (per Whitepaper Section 17.1 -- do not reinforce bad programs).
Periodically, LoRA weights are updated via gradient descent on recorded experiences.

### LoRA Implementation

```rust
pub struct LoRALayer {
    base_weight: Tensor,   // frozen, from model file
    lora_a: Tensor,        // trainable, rank x in_features
    lora_b: Tensor,        // trainable, out_features x rank
    scale: f32,            // alpha / rank
}
```

Forward: `y = W_frozen(x) + scale * (x @ A.T) @ B.T`

B is initialized to zero so LoRA has no initial effect. LoRA weights are shared
via `Arc<RwLock>` -- read lock for inference, write lock for adaptation (~10ms).

### Consolidation ("Sleep")

High-magnitude LoRA adaptations are merged into permanent weights:
`W_base += scale * B @ A`, then A and B reset. Proven patterns become permanent.

Triggered by: explicit command, experience count threshold, scheduled timer, or
low-activity period.

### Checkpoint Format (v2)

```
soma_checkpoint.bin:
  magic: "SOMA" (4 bytes)
  version: u32
  base_model_hash: [u8; 32]     // SHA-256 of ONNX models
  plugin_manifest: [PluginInfo]  // which plugins were loaded
  lora_layers: [LoRAState]       // all LoRA A/B matrices
  experience_stats: ExperienceStats
  metadata: {timestamp, soma_id, custom_fields}
```

Restore verifies model hash match before loading LoRA state. Version 2 is
backwards-compatible with v1 via `#[serde(default)]`.

For LoRA adaptation details and consolidation mechanics,
see [docs/mind-engine.md](mind-engine.md).

---

## Synaptic Protocol

Binary wire protocol for SOMA-to-SOMA communication. MCP handles LLM-to-SOMA.
Synaptic handles SOMA-to-SOMA.

### Wire Format

- 22-byte overhead per signal (vs HTTP's 500-2000 bytes)
- CRC32 checksum
- Optional zstd compression
- Optional ChaCha20-Poly1305 encryption
- Big-endian byte order

### 26 Signal Types (6 Categories)

| Category  | Signals                                                  |
|-----------|----------------------------------------------------------|
| Protocol  | HANDSHAKE, HANDSHAKE_ACK, CLOSE, PING, PONG, ERROR, CONTROL |
| Data      | INTENT, RESULT, INVOKE, QUERY, DATA, BINARY              |
| Streaming | STREAM_START, STREAM_DATA, STREAM_END                    |
| Chunked   | CHUNK_START, CHUNK_DATA, CHUNK_END, CHUNK_ACK            |
| Discovery | DISCOVER, DISCOVER_ACK, PEER_QUERY, PEER_LIST            |
| Pub/Sub   | SUBSCRIBE, UNSUBSCRIBE                                   |

SOMA-to-SOMA supports three interaction modes matching MCP: INTENT (Mind inference), INVOKE (direct convention call), and QUERY (state query).

### Transport

Transport-agnostic -- the protocol runs over:
- **TCP** -- primary transport for server-to-server
- **Unix Domain Socket** -- local inter-process (same machine)
- **WebSocket** -- browser Interface SOMAs

### Key Capabilities

- Multiplexed channels on a single connection
- Resumable chunked file transfer (SHA-256 verified)
- Pub/Sub with ephemeral and durable modes, catch-up on reconnect
- Peer discovery via chemical gradient (presence broadcast with decaying TTL)
- Auto-reconnect with exponential backoff and subscription replay
- Rate limiting with graduated response (throttle -> reduce window -> disconnect + blacklist)
- 24h session tokens for connection continuity

For the full wire format, signal definitions, and protocol state machines,
see [docs/synaptic-protocol.md](synaptic-protocol.md).

---

## MCP Server

JSON-RPC 2.0 interface for LLM-to-SOMA communication. The MCP Server is a core
component, not a plugin. It runs alongside the Synaptic Protocol server and exposes
SOMA's complete state and all plugin conventions as MCP tools.

### Tool Categories

**State tools** (query what exists):

| Tool                    | Returns                                      |
|-------------------------|----------------------------------------------|
| `soma.get_state()`      | Full snapshot (bootstrap -- one call, full context) |
| `soma.get_schema()`     | All tables with columns, types, constraints   |
| `soma.get_plugins()`    | Loaded plugins, conventions, health           |
| `soma.get_conventions()`| All callable conventions with arg specs       |
| `soma.get_decisions()`  | Decision log with reasoning                   |
| `soma.get_health()`     | Memory, CPU, connections, error rates         |
| `soma.get_experience()` | LoRA magnitude, adaptation stats              |
| `soma.get_peers()`      | Connected SOMAs via Synaptic Protocol         |

**Action tools** (do things):

| Tool                        | Description                           |
|-----------------------------|---------------------------------------|
| `soma.intent(text)`         | Send intent for Mind execution        |
| `soma.install_plugin(name)` | Install plugin from registry          |
| `soma.checkpoint(label?)`   | Save current state                    |
| `soma.restore(id)`          | Restore from checkpoint               |
| `soma.record_decision(...)` | Record a design decision              |
| `soma.confirm(action_id)`   | Confirm destructive action            |
| `soma.<plugin>.<conv>(...)` | Any loaded plugin convention          |

Plugin conventions are discovered dynamically. When a new plugin is installed, its
conventions immediately appear as MCP tools.

### Authentication

Three roles, set via environment variables:
- **Admin** (`SOMA_MCP_ADMIN_TOKEN`): full access
- **Builder** (`SOMA_MCP_BUILDER_TOKEN`): read + execute
- **Viewer** (`SOMA_MCP_VIEWER_TOKEN`): read-only

Every MCP action is logged as an audit trail.

For full MCP tool definitions and interaction patterns,
see [docs/mcp-interface.md](mcp-interface.md).

---

## Proprioception

The SOMA's self-model: what am I, what can I do, what is my health.

### What SOMA Knows About Itself

- Loaded plugins and their conventions
- Current LoRA magnitude (how much it has adapted)
- Experience count and adaptation cycle count
- RSS memory usage and CPU load
- Connected peers (via Synaptic Protocol)
- Uptime, execution stats, error rates
- Connection count

### Queryable via MCP

All proprioception data is exposed through MCP state tools:

```
soma.get_state()       -> full snapshot including self-model
soma.get_health()      -> memory, CPU, error rate, uptime
soma.get_plugins()     -> loaded plugins, conventions, health
soma.get_conventions() -> all callable conventions
soma.get_experience()  -> LoRA magnitude, adaptation stats
soma.get_peers()       -> connected SOMAs
```

SOMAs do not respond to natural language proprioception queries. "What can you do"
is an MCP state query made by an LLM on behalf of a human, not a SOMA intent.

Proprioception is also used by Interface SOMAs for responsive rendering --
the Interface reads its own screen size, device type, and accessibility settings
to determine how to render semantic signals.

---

## Binary Structure

### Two Binaries

| Binary     | Purpose                                        |
|------------|------------------------------------------------|
| `soma`     | Main runtime (Mind + plugins + protocol + MCP) |
| `soma-dump`| Signal capture tool for Synaptic Protocol debugging |

### CLI Flags

```
soma [OPTIONS]
  --model <dir>        Model directory (ONNX files + tokenizer)
  --mcp                Start MCP server mode
  --bind <addr:port>   Synaptic Protocol listen address
  --config <file>      Configuration file (default: soma.toml)
```

There is no `--repl` flag. SOMA does not have an interactive shell. Humans interact
through an LLM connected via MCP (see [MCP Interface](mcp-interface.md)).

### Configuration

Override order: defaults < `soma.toml` < env vars (`SOMA_*`) < CLI flags.

Key environment variables:
- `SOMA_MCP_ADMIN_TOKEN`, `SOMA_MCP_BUILDER_TOKEN`, `SOMA_MCP_VIEWER_TOKEN`
- `SOMA_LOG_JSON=1` (structured JSON logging)
- `SOMA_MIND_TEMPERATURE` (inference temperature)
- `SOMA_PROTOCOL_BIND` (Synaptic Protocol address)

---

## Error Type System

All subsystem errors fold into a single `SomaError` enum (defined in `errors.rs`,
per Whitepaper Section 11.3). This provides structured context for diagnostics,
MCP error responses, and retry decisions.

| Variant          | Covers                                                     |
|------------------|------------------------------------------------------------|
| `Inference`      | Model load, tokenization, decoding failures                |
| `Plugin`         | Plugin execution failure (inline fields: plugin, message, retryable, step_index, convention) |
| `PluginDetailed` | Plugin failure carrying a `PluginErrorDetail` struct       |
| `Protocol`       | Synaptic Protocol connection, codec, routing errors        |
| `Resource`       | Concurrency, memory, or plugin count limits exceeded       |
| `Convention`     | Referenced convention does not exist in any loaded plugin   |
| `Mcp`            | JSON-RPC server errors (auth, tool dispatch, serialization)|
| `State`          | Decision log or execution history persistence errors       |
| `Auth`           | Authentication or authorization failure                    |
| `Other`          | Catch-all for external crate errors (via `anyhow`)         |

Only `Plugin` and `PluginDetailed` carry a `retryable` flag. All other variants
are non-retryable by default. `SomaError::is_retryable()` checks this.

---

## Runtime Behavior

### Startup Sequence

```
1. Parse CLI + load config (soma.toml)
2. Initialize logging (tracing subscriber)
3. Load Mind Engine (detect backend, load model, load tokenizer, verify)
4. Load Plugins (scan dir, load .so/.dylib, register conventions, attach LoRA)
5. Restore Checkpoint (verify model hash, restore LoRA states)
6. Start Synaptic Protocol (bind listener, connect to peers, broadcast discovery)
7. Start MCP Server (bind, register state + action tools)
8. Ready ("SOMA ready. Plugins: N, Conventions: M, Peers: P, MCP: addr:port")
```

Failure handling: missing model is fatal. Failed plugin is skipped (reduced
capabilities). Corrupt checkpoint starts fresh. Failed Synaptic bind is fatal
on server, retry on embedded. Failed MCP bind is fatal (no LLM can connect).

### Concurrency

- Per-request inference context (encoder stateless, decoder hidden state per-request)
- LoRA weights: `Arc<RwLock>` -- read lock for inference, write lock for adaptation
- Embedded: sequential single-threaded processing

### Error Handling

Graduated response:
1. Retry same step
2. Re-infer (may produce different program)
3. Degrade to partial result
4. Report error

Crashed plugins are caught at `catch_unwind` boundary, unloaded. SOMA continues
with reduced capabilities. Cleanup conventions handle resource leaks.

### Graceful Shutdown

```
1. Stop accepting new requests
2. Notify peers (CLOSE signal)
3. Drain in-flight executions
4. Checkpoint (save LoRA state)
5. Unload plugins (call on_unload)
6. Close listeners
7. Exit
```

On embedded: save LoRA to flash with double-buffer for crash safety.

---

## Metrics and Observability

`SomaMetrics` (in `metrics/mod.rs`, per Whitepaper Sections 11.5 and 18.4) provides
20+ atomic counters exposed as both Prometheus text and JSON. All counters use
`AtomicU64` with relaxed ordering for lock-free concurrent updates.

| Metric                              | Type    | Description                          |
|-------------------------------------|---------|--------------------------------------|
| `soma_inferences_total`             | counter | Total inference requests              |
| `soma_inferences_success`           | counter | Successful inferences                 |
| `soma_inferences_failed`            | counter | Failed inferences                     |
| `soma_inference_duration_sum_ms`    | counter | Cumulative inference duration (ms)    |
| `soma_inference_confidence_avg`     | gauge   | Average inference confidence (0..1)   |
| `soma_programs_executed`            | counter | Total programs executed                |
| `soma_program_steps_executed`       | counter | Total program steps executed           |
| `soma_plugin_calls_total`           | counter | Total plugin convention calls          |
| `soma_plugin_errors_total`          | counter | Failed plugin calls                    |
| `soma_plugin_retries`              | counter | Plugin call retries                    |
| `soma_plugin_duration_sum_ms`       | counter | Cumulative plugin call duration (ms)   |
| `soma_experience_buffer_size`       | gauge   | Current experience buffer entries      |
| `soma_adaptations_total`            | counter | Total LoRA adaptations                 |
| `soma_checkpoints_saved`            | counter | Total checkpoints saved                |
| `soma_memory_rss_bytes`             | gauge   | Current resident set size (bytes)      |
| `soma_lora_magnitude`               | gauge   | Current LoRA adapter magnitude         |
| `soma_protocol_connections_active`  | gauge   | Active Synaptic Protocol connections   |
| `soma_protocol_signals_sent`        | counter | Total signals sent                     |
| `soma_protocol_signals_received`    | counter | Total signals received                 |
| `soma_protocol_bytes_transferred`   | counter | Total bytes transferred                |
| `soma_uptime_seconds`               | gauge   | Seconds since instance started         |

Per-plugin metrics (calls, errors, duration) are tracked via `DashMap<String, PluginMetrics>`
and emitted with a `{plugin="name"}` label. JSON exposition groups metrics by
subsystem (inference, programs, plugins, memory, adaptation, protocol).

---

## Security

### MCP Authentication

Role-based tokens at three levels: admin (full access), builder (read + execute),
viewer (read-only). Destructive actions (DROP TABLE, DELETE without WHERE, plugin
uninstall, checkpoint restore) require two-step confirmation via
`soma.confirm(action_id)`.

### Plugin Trust Levels

| Level      | Description                           | Enforcement              |
|------------|---------------------------------------|--------------------------|
| Built-in   | Compiled into binary                  | Full trust               |
| Vendor     | Signed by known vendor                | Ed25519 signature verify |
| Community  | Code-reviewed                         | Signature verify         |
| Private    | In-house, not distributed             | Trust on load            |
| Untrusted  | Unknown source                        | WASM sandbox (future)    |

Plugin signing uses Ed25519. Plugins declare least-privilege permissions (network,
filesystem, env var scoping). Convention namespacing prevents hijacking.

### Synaptic Protocol Encryption

- **Encryption:** ChaCha20-Poly1305 per-signal (real crypto, not placeholders)
- **Key exchange:** X25519
- **Identity:** Ed25519
- **Rate limiting:** Per-connection with graduated response (throttle -> reduce
  window -> disconnect + blacklist via `PeerBlacklist`)

### Audit Trail

Every MCP action is logged with timestamp, auth role, tool called, and arguments.
Program traces show exactly what was executed. Decision log records what was built
and why.

---

## Design Decisions

These decisions are documented with rationale. They are non-obvious and important
for contributors to understand.

| Decision | Rationale |
|----------|-----------|
| **tract-onnx over ort** | Pure Rust, no C++ dependencies. Sufficient for <50M param models. Trades GPU acceleration for zero-dependency builds. See `mind/onnx_engine.rs`. |
| **`infer(&str)` not `infer(tokens)`** | Tokenizer is internal to the engine. Encapsulation over spec purity. Callers pass intent text, not pre-tokenized input. See `mind/mod.rs`. |
| **Convention IDs = plugin_idx * 1000** | Prevents routing conflicts across plugins. Each plugin gets a unique 1000-ID range. See `plugin/manager.rs register()`. |
| **`Arc<RwLock<PluginManager>>`** | Enables runtime convention registration (MCP Bridge plugin). Write lock only for `install_plugin`. |
| **Interior mutability for crashed plugins** | `RwLock<HashSet>` so `execute_step(&self)` can mark crashes without a write lock on the manager. |
| **Experience records successes only** | Per spec Section 17.1. Do not reinforce bad programs. |
| **Checkpoint v2 with `#[serde(default)]`** | Backwards-compatible with v1. Adds SHA-256 model hash, plugin manifest. |
| **Real crypto (not placeholders)** | ChaCha20-Poly1305, X25519, Ed25519 via dalek crates. Production-grade from day one. |
| **Synchronous postgres via `block_on()`** | The `SomaPlugin` trait is synchronous. Postgres plugin uses `block_on()` for `tokio-postgres` inside the sync interface. Connection pooled and cached. |
| **BPE tokenizer in Rust** | Auto-detected from `tokenizer.json` format. Handles OOV, SQL, URLs. Character-level fallback for unknown subwords. |
| **MCP is core, not a plugin** | MCP Server runs alongside Synaptic Protocol. Without MCP, no LLM can connect. Too critical to be optional. |
| **No REPL** | Humans interact through LLMs via MCP (see [MCP Interface](mcp-interface.md)). SOMA does not converse. |
