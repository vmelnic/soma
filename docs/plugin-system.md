# Plugin System

## Overview

Everything that is not Mind, Memory, Synaptic Protocol, MCP Server, or Plugin Loading is a plugin. A SOMA with zero plugins is a brain in a jar -- it can think but cannot act. Plugins are its body parts: eyes, hands, legs, voice.

A plugin provides two things:

1. **Calling conventions** -- operations the Mind can invoke. These are the "muscles" the Mind orchestrates: `query(sql, params)`, `send_email(to, subject, body)`, `gpio.write(pin, value)`.
2. **LoRA knowledge** (optional) -- pre-trained weight adaptations that teach the Mind HOW to use the calling conventions effectively. Installing a plugin with LoRA is like gaining a skill: you get the tool AND the expertise.

Same core, different body. The Mind is universal; plugins determine what a particular SOMA can do. A SOMA with PostgreSQL and Redis plugins is a database server. Swap those for GPIO and I2C plugins and the same Mind runs on a microcontroller. The Mind does not care -- it generates programs against whatever conventions are available.

All plugin conventions are automatically exposed as MCP tools. This is how LLMs discover and use SOMA capabilities.

For how to build a plugin, see [docs/plugin-development.md](plugin-development.md). For the full convention reference across all 40+ plugins, see [docs/plugin-catalog.md](plugin-catalog.md).

---

## The SomaPlugin Trait

Every plugin implements the `SomaPlugin` trait, defined in the `soma-plugin-sdk` crate (`soma-plugins/sdk/src/lib.rs`) and re-exported by `soma-core/src/plugin/interface.rs`. External plugins depend only on the lightweight SDK crate, not all of soma-core.

```rust
pub trait SomaPlugin: Send + Sync {
    // === Identity ===
    fn name(&self) -> &str;
    fn version(&self) -> &str { "0.1.0" }
    fn description(&self) -> &str { "" }
    fn trust_level(&self) -> TrustLevel { TrustLevel::BuiltIn }

    // === Capabilities ===
    fn supports_streaming(&self) -> bool { false }
    fn conventions(&self) -> Vec<Convention>;
    fn execute(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError>;
    fn execute_async(&self, convention_id: u32, args: Vec<Value>)
        -> Pin<Box<dyn Future<Output = Result<Value, PluginError>> + Send + '_>> {
        // Default: delegates to sync execute()
    }

    // === Knowledge ===
    fn lora_weights(&self) -> Option<Vec<u8>> { None }
    fn training_data(&self) -> Option<serde_json::Value> { None }

    // === Streaming ===
    fn execute_stream(&self, convention_id: u32, args: Vec<Value>)
        -> Result<Vec<Value>, PluginError> { /* default: error */ }

    // === State Persistence ===
    fn checkpoint_state(&self) -> Option<serde_json::Value> { None }
    fn restore_state(&mut self, state: &serde_json::Value) -> Result<(), PluginError> { Ok(()) }

    // === Lifecycle ===
    fn on_load(&mut self, config: &PluginConfig) -> Result<(), PluginError> { Ok(()) }
    fn on_unload(&mut self) -> Result<(), PluginError> { Ok(()) }

    // === Meta ===
    fn dependencies(&self) -> Vec<PluginDependency> { Vec::new() }
    fn permissions(&self) -> PluginPermissions { PluginPermissions::default() }
    fn config_schema(&self) -> Option<serde_json::Value> { None }
}
```

**Required methods:** `name()`, `conventions()`, `execute()`. Everything else has a default implementation and is optional.

**Key design choice:** `execute(&self)` takes `&self`, not `&mut self`. Plugins must handle their own interior mutability if they maintain state. This enables concurrent read access through `Arc<RwLock<PluginManager>>`.

Dynamic plugins export a C ABI init function for loading:

```rust
#[no_mangle]
pub extern "C" fn soma_plugin_init() -> *mut dyn SomaPlugin {
    Box::into_raw(Box::new(MyPlugin::new()))
}
```

---

## Value Type

`Value` is the universal data type flowing between plugins and the Mind. Every convention argument and return value is a `Value`.

```rust
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Map(HashMap<String, Value>),
    Handle(u64),      // opaque handle (fd, connection, transaction, etc.)
    Signal(Vec<u8>),  // serialized Synaptic Protocol signal
}
```

`Handle` represents opaque resources -- file descriptors, database connections, transaction objects. The Mind can pass handles between program steps without understanding what they contain. `Signal` carries serialized Synaptic Protocol signals for inter-SOMA forwarding.

Helper methods (`as_str()`, `as_int()`, `as_float()`, `as_bool()`, `as_handle()`, `as_bytes()`) return `Result<T, PluginError>` for safe extraction with type-checked errors.

---

## Calling Conventions

A calling convention is a single operation a plugin provides. It is the fundamental unit of capability.

```rust
pub struct Convention {
    pub id: u32,                       // unique within this plugin (0, 1, 2, ...)
    pub name: String,                  // "query", "cache_set", "render_element"
    pub description: String,           // shown in MCP tool descriptions and proprioception
    pub call_pattern: String,          // invocation pattern
    pub args: Vec<ArgSpec>,            // argument schema
    pub returns: ReturnSpec,           // return type
    pub is_deterministic: bool,        // same input = same output?
    pub estimated_latency_ms: u32,     // helps Mind plan (prefer faster alternatives)
    pub max_latency_ms: u32,           // timeout -- Core kills execution if exceeded
    pub side_effects: Vec<SideEffect>, // "writes_disk", "sends_network", etc.
    pub cleanup: Option<CleanupSpec>,  // resource release on error
}
```

### Arguments

```rust
pub struct ArgSpec {
    pub name: String,          // "sql", "path", "data"
    pub arg_type: ArgType,     // String, Int, Float, Bool, Bytes, Handle, Any
    pub required: bool,
    pub description: String,
}
```

`ArgType` maps to JSON Schema types for MCP tool generation: `String`/`Bytes`/`Any` become `"string"`, `Int`/`Handle` become `"integer"`, `Float` becomes `"number"`, `Bool` becomes `"boolean"`.

Arguments in a Mind-generated program are resolved from three sources:

- **Span**: extracted directly from the intent text (e.g., `"span:path"` extracts a file path)
- **Ref**: pointer to a previous step's result (e.g., `$0` refers to step 0's output)
- **Literal**: a constant value embedded in the program

### Return and Cleanup

```rust
pub enum ReturnSpec {
    Value(String),   // returns a Value of the described type
    Stream(String),  // returns streaming output
    Handle,          // returns an opaque handle
    Void,            // no return value
}

pub struct CleanupSpec {
    pub convention_id: u32,   // which convention to call for cleanup
    pub pass_result_as: u8,   // which arg position gets the result to clean up
}
```

`CleanupSpec` enables automatic resource cleanup on error. If step 3 of a program fails and step 1 opened a file descriptor, the Plugin Manager invokes the cleanup convention (e.g., `close_fd`) in reverse order, passing the handle as an argument.

### Convention IDs

Convention IDs are globally namespaced: `global_id = plugin_idx * 1000 + local_id`. The first plugin's conventions are 0-999, the second's are 1000-1999, and so on. This prevents routing conflicts when multiple plugins use the same local IDs (e.g., both use 0 for their first convention).

---

## Plugin Manager

The Plugin Manager (`soma-core/src/plugin/manager.rs`) loads plugins, maintains the convention catalog, and routes program steps to the correct plugin.

### Structure

```rust
pub struct PluginManager {
    plugins: Vec<Box<dyn SomaPlugin>>,
    routing: HashMap<u32, (usize, u32)>,          // global_id -> (plugin_index, local_id)
    name_routing: HashMap<String, u32>,            // "plugin.convention" -> global_id
    catalog_routing: HashMap<u32, u32>,            // model_catalog_id -> global_id
    crashed_plugins: RwLock<HashSet<usize>>,       // interior mutability
    denied_plugins: RwLock<HashSet<String>>,        // permission enforcement
    convention_stats: RwLock<HashMap<u32, ConventionStats>>,
    metrics: Option<Arc<SomaMetrics>>,
}
```

The Plugin Manager is held behind `Arc<RwLock<PluginManager>>` in the runtime. Read lock for execution (concurrent), write lock only for `register()` and `unregister()`.

### Registration

Registration flow when `register(plugin)` is called:

1. **Check dependencies** -- iterate the plugin's `dependencies()`. If any required dependency is not already loaded, refuse registration and log an error.
2. **Validate config** -- call `config_schema()` and validate against the provided config.
3. **Surface LoRA weights** -- if `lora_weights()` returns data, log that LoRA is available for Mind attachment.
4. **Assign index** -- `plugin_idx = plugins.len()`, compute `id_offset = plugin_idx * 1000`.
5. **Register conventions** -- for each convention, compute `global_id = id_offset + conv.id`. Insert into `routing` (global_id -> plugin_index, local_id) and `name_routing` ("plugin_name.convention_name" -> global_id). Skip on conflict.
6. **Push plugin** -- add to `plugins` vec.

For batch registration, `register_all()` performs a topological sort first (see Dependencies section below).

### Execution

When the Mind generates a program, each step contains a convention ID. The execution flow:

1. **Resolve catalog ID** -- the Mind outputs model catalog IDs. `resolve_catalog_id()` maps these to global routing IDs via `catalog_routing`.
2. **Route** -- look up `routing[global_id]` to find (plugin_index, local_conv_id).
3. **Check crashed** -- if the plugin index is in `crashed_plugins`, return error immediately.
4. **Check denied** -- if the plugin name is in `denied_plugins`, return `PermissionDenied`.
5. **Execute with timeout** -- if the convention declares `max_latency_ms` and a tokio runtime is available, wrap in `tokio::time::timeout`. Otherwise, execute synchronously.
6. **Catch panics** -- all execution is wrapped in `std::panic::catch_unwind`. A plugin panic marks the plugin as crashed (via interior mutability on `crashed_plugins`) rather than crashing the SOMA process.
7. **Retry** -- if the error is retryable (`Failed` or `ConnectionRefused`), wait 100ms and try once more.
8. **Record metrics** -- update `ConventionStats` (call count, total time, error count, timeout count) and Prometheus counters.

### Name Resolution

Conventions can be addressed by name rather than numeric ID. The format is `"plugin_name.convention_name"` -- for example, `"postgres.query"` or `"posix.read_file"`. The `resolve_by_name()` method looks up the name in `name_routing` and returns the global routing ID.

This decouples model training from plugin loading order. The model predicts convention names (or name hashes), and the Plugin Manager resolves names to runtime IDs.

### Crashed Plugin Tracking

`crashed_plugins` uses `RwLock<HashSet<usize>>` for interior mutability. This is necessary because `execute_step(&self)` takes `&self` (not `&mut self`) to allow concurrent execution, but must still be able to mark a plugin as crashed when it panics. Once marked, a crashed plugin is permanently disabled for the lifetime of the SOMA process.

### ConventionStats

Per-convention execution statistics for metrics and proprioception:

```rust
pub struct ConventionStats {
    pub call_count: u64,
    pub total_time_ms: u64,
    pub error_count: u64,
    pub timeout_count: u64,
    recent_durations: Vec<u64>,  // ring buffer (last 100) for percentile tracking
}
```

Exposes `avg_duration_ms()`, `p50_duration_ms()`, and `p99_duration_ms()` for performance monitoring.

### Program Execution

`execute_program()` runs a full Mind-generated program:

1. Iterate steps sequentially.
2. Resolve arguments: `Span` values are extracted from intent text (with `~` expansion), `Ref` values reference previous step results, `Literal` values are constants.
3. Special IDs: `EMIT_ID` captures output, `STOP_ID` terminates the program.
4. On step failure: invoke cleanup conventions in reverse order (LIFO) for all previously successful steps that declared a `CleanupSpec`. Then close any remaining open handles.
5. Returns `ProgramResult` with success flag, output value, execution trace, and optional error.

---

## Built-in Plugins

Built-in plugins are compiled directly into the SOMA binary. They require no dynamic loading.

### PosixPlugin

The PosixPlugin (`soma-core/src/plugin/builtin.rs`) provides 25 filesystem and system conventions via libc:

| ID | Convention | Description |
|----|-----------|-------------|
| 0-4 | `open_read`, `create_file`, `read_content`, `write_content`, `close_fd` | Low-level file I/O |
| 5-7 | `open_dir`, `read_dir_entries`, `close_dir` | Directory traversal |
| 8-10 | `delete_file`, `create_dir`, `rename_path` | File management |
| 11-15 | `check_access`, `file_stat`, `get_cwd`, `get_time`, `get_uname` | System queries |
| 16-19 | `read_file`, `write_file`, `list_dir_simple`, `copy_file` | High-level fs ops |
| 20-24 | `read_chunk`, `append_file`, `read_bytes`, `write_bytes`, `write_chunk` | Byte-level I/O |

Conventions that open handles (like `open_read`) declare cleanup conventions (like `close_fd`) so the Plugin Manager can release resources on error.

---

## Dynamic Plugins

External plugins are compiled as `cdylib` crates producing `.so` (Linux), `.dylib` (macOS), or `.dll` (Windows) shared libraries. Loading is handled by `soma-core/src/plugin/dynamic.rs`.

### Loading

`load_plugin_from_path()` performs:

1. **Signature verification** -- if `<path>.sig` and `<path>.pub` files exist alongside the library, Ed25519 signature verification is performed. Failed verification is an error. Missing signature files allow loading with a debug log.
2. **Library loading** -- via `libloading::Library::new()`.
3. **Symbol lookup** -- finds the `soma_plugin_init` symbol (C ABI function returning `*mut dyn SomaPlugin`).
4. **Initialization** -- calls the init function, converts the raw pointer to `Box<dyn SomaPlugin>`.
5. **Library leak** -- the library handle is intentionally leaked (`std::mem::forget`) because the plugin uses symbols from it and the library must stay loaded.

### Directory Scanning

`scan_plugin_directory()` finds plugins in two locations within a directory:

1. **Top-level library files** -- e.g., `plugins/libfoo.dylib`
2. **Subdirectories with manifests** -- directories containing `manifest.json` or `manifest.toml` with a matching library file named `lib<dirname>.<ext>` (the `.soma-plugin` package layout)

### Manifest

Each dynamic plugin has a manifest parsed from `manifest.toml`:

```rust
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub platforms: Vec<String>,
    pub conventions_count: usize,
    pub lora_included: bool,
    pub dependencies: Vec<String>,
}
```

---

## Plugin Distribution

### Package Format

Plugins are distributed as `.soma-plugin` archives:

```
plugin-postgres-0.1.0.soma-plugin
  manifest.toml           # Plugin metadata
  plugin.so               # Compiled binary (.so / .dylib / .dll)
  lora/
    postgres.lora          # Pre-trained LoRA weights
  training/
    examples.json          # Training examples for synthesis
  README.md               # Documentation
```

### Manifest

```toml
[plugin]
name = "postgres"
version = "0.1.0"
description = "PostgreSQL database operations"
author = "SOMA Community"
license = "MIT"

[compatibility]
soma_core = ">=0.1.0"
platforms = ["x86_64-linux", "aarch64-linux", "aarch64-macos"]

[conventions]
count = 12

[lora]
included = true
rank = 8
target_layers = ["op_head", "gru", "a0t_head", "a1t_head"]

[dependencies]
libpq = ">=14.0"

[config]
host = "localhost"
port = 5432
```

### Registry

Plugins are distributed via a registry:

```
soma plugin search postgres
soma plugin install postgres@0.1.0
soma plugin list
soma plugin remove redis
```

Installed plugins are stored in the SOMA's plugin directory and loaded at startup or dynamically at runtime via MCP (`soma.install_plugin("stripe")`).

---

## Plugin Categories

### Bridge

| Plugin | Purpose |
|--------|---------|
| `mcp-bridge` | Connects to external MCP servers (GitHub, Slack, Stripe, etc.) -- their tools become SOMA conventions. The most strategically important plugin. |
| `http-bridge` | HTTP client for REST APIs; HTTP server for browser compatibility. |

### Storage

| Plugin | Purpose |
|--------|---------|
| `postgres` | SQL queries, schema operations, transactions |
| `redis` | Key-value, pub/sub, caching, sessions |
| `sqlite` | Embedded SQL for mobile/desktop |
| `s3` | Object storage, presigned URLs, multipart |
| `filesystem` | Local file I/O |
| `memory-store` | In-process key-value for transient state |

### Communication

| Plugin | Purpose |
|--------|---------|
| `smtp` | Email sending (MIME, attachments, templates) |
| `twilio` | SMS sending, phone verification |
| `apns` / `fcm` | Push notifications (Apple / Android) |
| `webrtc` | Peer-to-peer media (signaling via Synaptic Protocol) |

### Processing

| Plugin | Purpose |
|--------|---------|
| `image-processing` | Resize, crop, format conversion, EXIF strip |
| `audio-processing` | Transcode, normalize, voice handling |
| `video-processing` | Transcode, thumbnail extraction, streaming prep |
| `crypto` | Hashing, encryption, signing, JWT, token generation |

### Auth

| Plugin | Purpose |
|--------|---------|
| `otp-auth` | Phone OTP generation/validation |
| `social-auth` | Google/Apple Sign-In OAuth |
| `session-manager` | Session creation, validation, revocation |
| `totp-2fa` | Time-based one-time passwords |

### Domain

| Plugin | Purpose |
|--------|---------|
| `calendar` | Scheduling, conflict detection, recurrence |
| `messaging` | Message storage, delivery tracking, threading |
| `reviews` | Ratings, aggregation, moderation |
| `geolocation` | Distance, radius search, geocoding |
| `analytics` | Event tracking, aggregation |

### Renderer

| Plugin | Output |
|--------|--------|
| `dom-renderer` | Browser DOM (HTML, CSS, events) |
| `uikit-renderer` | iOS native views |
| `compose-renderer` | Android composables |
| `terminal-renderer` | ANSI terminal |
| `canvas-renderer` | HTML Canvas / WebGL |

### Design Knowledge

| Plugin | Source |
|--------|--------|
| `pencil-design` | pencil.dev .pen files |
| `figma-design` | Figma exports |
| `material-design` | Google Material Design |
| `human-interface` | Apple HIG |

### Embedded (built-in only)

GPIO, I2C, SPI, UART, WiFi, BLE, Timer. These are compiled into the binary for targets where dynamic loading is unavailable (ESP32, microcontrollers).

---

## LoRA Plugin Knowledge

### How It Works

When a plugin includes LoRA weights, loading the plugin means:

1. Load the plugin's calling conventions into the catalog
2. Load the plugin's LoRA weights and attach them to the Mind
3. The Mind immediately knows how to use the plugin's conventions

A plugin WITHOUT LoRA: the Mind has the capability but must learn through runtime experience (adaptation). A plugin WITH LoRA: the Mind immediately knows how to use it. Like a surgeon who has both the scalpel (convention) and the training (LoRA).

### LoRA Composition (Mixture of Experts)

Multiple plugin LoRAs are active simultaneously. A gating network (part of the Mind) dynamically weights which plugin's LoRA to activate for each operation:

```
Intent: "save the booking to the database and send a confirmation email"

Gating network activates:
  PostgreSQL LoRA: 0.8 (high -- database operation)
  SMTP LoRA: 0.6 (moderate -- email involved)
  Redis LoRA: 0.0 (not relevant)

Mind generates program using combined knowledge:
  $0 = postgres.query("INSERT INTO bookings ...")
  $1 = smtp.send_email(to=$client_email, subject="Booking Confirmed", ...)
  $2 = EMIT($0)
  STOP
```

This is consistent with MoE research: X-LoRA (Buehler, 2024), L-MoE (2025), LoRA-Mixer (2025), MoLoRA (2025) -- focused LoRAs can be trained independently and composed at inference time without retraining.

### Hot-Loading

Plugins and their LoRA knowledge can be loaded at runtime without restart:

```
soma.install_plugin("stripe")
  -> Plugin Manager: 8 conventions registered
  -> LoRA Manager: Attaching Stripe LoRA (rank 8, 15K params)
  -> Mind: Stripe knowledge integrated
```

The Plugin Manager tracks which plugins provide LoRA via `plugins_with_lora_weights()` and retrieves weights via `get_plugin_lora_weights(name)`.

### Training Data for Synthesis

Plugins include training examples (intent, program) pairs so the Synthesizer can train models that know the plugin:

```json
[
  {
    "intent": "find all contacts in a 10km radius",
    "program": [
      {"convention": "postgres.query", "args": {"sql": "SELECT ... ST_DWithin(...)", "params": ["span:lat", "span:lon"]}},
      {"convention": "EMIT", "args": {"ref": 0}}
    ]
  }
]
```

The Synthesizer merges training data from all target plugins and trains a model that knows all of them.

---

## Isolation and Safety

### Trust Levels

```rust
pub enum TrustLevel {
    BuiltIn,      // compiled into binary, fully trusted
    Community,    // from the plugin registry, signed
    Vendor,       // from a verified vendor
    Private,      // organization-internal
    Untrusted,    // unknown origin
}
```

### Plugin Signing

Plugins are signed with Ed25519. The `verify_plugin_signature()` function reads the plugin binary, a `.sig` file (64-byte signature), and a `.pub` file (32-byte public key), then verifies using the same Ed25519 implementation used by the Synaptic Protocol. Verification is mandatory when signature files are present.

### Capability Boundaries

Plugins declare what they need via `PluginPermissions`:

```rust
pub struct PluginPermissions {
    pub filesystem: Vec<String>,    // paths this plugin accesses
    pub network: Vec<String>,       // hosts/ports this plugin connects to
    pub env_vars: Vec<String>,      // environment variables this plugin reads
    pub process_spawn: bool,        // whether the plugin spawns child processes
}
```

The SOMA can restrict plugins in `soma.toml`:

```toml
[security]
allow_network = ["smtp", "twilio", "http-bridge"]
allow_disk_write = ["postgres", "filesystem", "s3"]
deny = ["webrtc"]
```

The `denied_plugins` set in the Plugin Manager enforces runtime denial. Denied plugins can be managed dynamically via `deny_plugin()` and `allow_plugin()`.

### Resource Limits

Plugins can be resource-limited via configuration:

```toml
[plugins.postgres]
max_connections = 10
query_timeout = "30s"
max_result_size = "10MB"

[plugins.image-processing]
max_memory = "256MB"
max_processing_time = "10s"
```

Convention-level timeouts are enforced by the Plugin Manager using `max_latency_ms` and `tokio::time::timeout`.

### WASM Sandbox (future)

For untrusted plugins, a planned WebAssembly sandbox (via `wasmtime`) will provide memory isolation and capability restriction at the VM level. Plugins compile to WASM instead of native code.

---

## Dependencies

### Declaration

Plugins declare dependencies in their manifest and via the trait:

```rust
pub struct PluginDependency {
    pub name: String,
    pub required: bool,
}
```

In `manifest.toml`:

```toml
[dependencies]
required = [
  { name = "postgres", min_version = "0.1.0" },
]
optional = [
  { name = "redis", min_version = "0.1.0", enables = ["caching"] },
]
conflicts = []
```

### Resolution

The Plugin Manager resolves dependencies during loading via topological sort. `register_all()` implements DFS with three-color marking (white/gray/black) for cycle detection:

1. Scan all plugins to load
2. Build dependency graph from each plugin's `dependencies()`
3. Topological sort: load dependencies before dependents
4. Detect cycles: if A depends on B and B depends on A, both are marked cyclic and skipped
5. Register in sorted order, skipping cyclic plugins

### Missing Dependencies

| Scenario | Behavior |
|----------|----------|
| Required dep missing | Plugin refuses to load. Error logged. SOMA continues without it. |
| Required dep wrong version | Same as missing -- version mismatch is incompatible. |
| Optional dep missing | Plugin loads with reduced functionality. Info logged. |
| Dep unloaded at runtime | Dependent plugin is notified to degrade gracefully. |

### Inter-Plugin Communication

Plugins do not call each other directly. All communication goes through the Mind's program generation:

```
$0 = messaging.prepare_message(...)
$1 = postgres.query("INSERT INTO messages ...", $0)
$2 = EMIT($1)
```

The Mind orchestrates. Plugins are isolated. For low-level shared functionality (connection pooling, crypto primitives), the spec defines a `PluginService` trait for infrastructure-level sharing -- but business logic always flows through the Mind.
