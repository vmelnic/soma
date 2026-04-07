# SOMA Plugin System — Specification

**Status:** Design  
**Depends on:** SOMA Core, Synaptic Protocol  
**Blocks:** All domain-specific SOMA capabilities

---

## 1. Principle

Everything that is not Mind, Memory, Protocol, or Plugin Loading is a plugin. A SOMA with zero plugins is a brain in a jar — it can think but cannot act. Plugins are its body parts: eyes, hands, legs, voice.

---

## 2. What a Plugin Is

A plugin provides two things:

1. **Calling Conventions** — operations the Mind can invoke (like libc functions in POW 1). These are the "muscles" the Mind orchestrates.
2. **LoRA Knowledge** (optional) — pre-trained weight adaptations that teach the Mind HOW to use the calling conventions effectively. This is the "training" for using a tool — not just having a hammer, but knowing when and how to swing it.

A plugin WITHOUT LoRA knowledge: the Mind has the capability but needs to learn through experience (runtime adaptation) how to use it effectively.

A plugin WITH LoRA knowledge: the Mind immediately knows how to use it. Like a surgeon who has both the scalpel (convention) and the training (LoRA).

---

## 3. Plugin Interface

### 3.1 Rust Trait

```rust
pub trait SomaPlugin: Send + Sync {
    // === Identity ===
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn description(&self) -> &str;

    // === Capabilities ===
    
    /// Calling conventions this plugin provides
    fn conventions(&self) -> Vec<CallingConvention>;
    
    /// Execute a convention by ID with resolved arguments
    fn execute(&self, conv_id: u32, args: Vec<Value>) -> Result<Value, PluginError>;
    
    /// Execute asynchronously (for I/O-bound operations)
    fn execute_async(&self, conv_id: u32, args: Vec<Value>) 
        -> Pin<Box<dyn Future<Output = Result<Value, PluginError>> + Send>>;
    
    // === Knowledge ===
    
    /// LoRA weights that teach the Mind to use this plugin
    fn lora_weights(&self) -> Option<&[u8]>;  // serialized LoRA state
    
    /// Training examples: (intent, program) pairs for synthesis
    fn training_data(&self) -> Option<Vec<TrainingExample>>;
    
    // === Streaming (optional) ===
    
    /// Can this plugin produce streaming output?
    fn supports_streaming(&self) -> bool { false }
    
    /// Start a streaming operation, returns a channel receiver
    fn execute_stream(&self, conv_id: u32, args: Vec<Value>) 
        -> Result<StreamReceiver, PluginError> {
        Err(PluginError::NotSupported)
    }
    
    // === Lifecycle ===
    fn on_load(&mut self, config: &PluginConfig) -> Result<(), PluginError>;
    fn on_unload(&mut self) -> Result<(), PluginError>;
}
```

### 3.2 CallingConvention

```rust
pub struct CallingConvention {
    pub id: u32,                          // unique within this plugin
    pub name: String,                      // "query", "cache_set", "render_element"
    pub description: String,               // human-readable for proprioception
    pub args: Vec<ArgSpec>,                // argument schema
    pub returns: ReturnSpec,               // return type
    pub is_deterministic: bool,            // same input = same output?
    pub estimated_latency: Duration,       // helps Mind plan
    pub side_effects: Vec<SideEffect>,     // "writes_disk", "sends_network", etc.
}

pub struct ArgSpec {
    pub name: String,                      // "sql", "path", "data"
    pub arg_type: ArgType,                 // String, Int, Float, Bytes, Ref, Any
    pub required: bool,
    pub description: String,
}
```

### 3.3 Value Type

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
    Handle(u64),          // opaque handle (file descriptor, connection, etc.)
    Signal(Box<Signal>),  // a Synaptic Protocol signal (for forwarding)
}
```

---

## 4. Plugin Distribution Format

### 4.1 Package Structure

```
plugin-postgres-0.1.0.soma-plugin
├── manifest.toml           # Plugin metadata
├── plugin.so               # Compiled plugin (or .dylib / .dll)
├── lora/
│   └── postgres.lora       # Pre-trained LoRA weights
├── training/
│   └── examples.json       # Training examples for synthesis
└── README.md               # Documentation
```

### 4.2 Manifest

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
count = 12   # number of calling conventions

[lora]
included = true
rank = 8
target_layers = ["op_head", "gru", "a0t_head", "a1t_head"]

[dependencies]
# native libraries needed
libpq = ">=14.0"

[config]
# default configuration keys
host = "localhost"
port = 5432
database = ""
username = ""
password = ""
```

### 4.3 Registry

Plugins are distributed via a registry (like crates.io for Rust or npm for Node):

```
soma plugin search postgres
soma plugin install postgres@0.1.0
soma plugin install stripe@0.3.2
soma plugin list
soma plugin remove redis
```

Installed plugins are stored in the SOMA's plugin directory and loaded at startup (or dynamically at runtime via intent: "load the stripe plugin").

---

## 5. Plugin Categories

### 5.1 Renderer Plugins

These give the Interface SOMA its "body" — the ability to produce visual/auditory output.

| Plugin | Body | Output |
|---|---|---|
| `dom-renderer` | Browser DOM | HTML elements, CSS styles, DOM events |
| `uikit-renderer` | iOS UIKit/SwiftUI | Native iOS views |
| `compose-renderer` | Android Jetpack Compose | Native Android composables |
| `terminal-renderer` | Terminal/CLI | ANSI text output |
| `canvas-renderer` | HTML Canvas / WebGL | 2D/3D graphics |
| `audio-renderer` | Audio output device | Sound playback |

Each renderer plugin provides conventions like:
```
create_element(tag, attributes) → handle
set_style(handle, property, value)
set_content(handle, text)
append_child(parent_handle, child_handle)
add_event_listener(handle, event_type, channel_id)
remove_element(handle)
```

The Mind generates programs of these operations to compose interfaces.

### 5.2 Storage Plugins

| Plugin | Provides |
|---|---|
| `postgres` | SQL queries, schema operations, transactions |
| `redis` | Key-value, pub/sub, caching, sessions |
| `sqlite` | Embedded SQL (for mobile/desktop) |
| `s3` | Object storage, presigned URLs, multipart |
| `filesystem` | Local file I/O (read, write, list, delete) |
| `memory-store` | In-process key-value (for transient state) |

### 5.3 Communication Plugins

| Plugin | Provides |
|---|---|
| `smtp` | Email sending (MIME, attachments, templates) |
| `twilio` | SMS sending, phone verification |
| `apns` | Apple Push Notifications |
| `fcm` | Firebase Cloud Messaging (Android push) |
| `webrtc` | Peer-to-peer media (signaling via Synaptic Protocol) |
| `http-bridge` | HTTP server for legacy browser compatibility |

### 5.4 Processing Plugins

| Plugin | Provides |
|---|---|
| `image-processing` | Resize, thumbnail, crop, EXIF strip, format conversion |
| `audio-processing` | Transcode, normalize, voice message handling |
| `video-processing` | Transcode, thumbnail extraction, streaming prep |
| `crypto` | Hashing, encryption, signing, token generation |
| `ai-inference` | Run additional ML models (NLP, classification, embedding) |
| `text-search` | Full-text search indexing and querying |

### 5.5 Auth Plugins

| Plugin | Provides |
|---|---|
| `otp-auth` | Phone number verification, OTP generation/validation |
| `social-auth` | Google/Apple Sign-In OAuth flows |
| `session-manager` | Session token creation, validation, revocation |
| `totp-2fa` | Time-based one-time password for 2FA |
| `id-verification` | Face matching, document verification |

### 5.6 Domain Plugins

| Plugin | Provides |
|---|---|
| `calendar` | Scheduling, conflict detection, reminders, recurrence |
| `messaging` | Message storage, delivery tracking, read receipts, threading |
| `reviews` | Rating storage, aggregation, moderation |
| `geolocation` | Distance calculation, radius search, geocoding |
| `localization` | String translation, locale handling, currency formatting |
| `analytics` | Event tracking, aggregation, dashboards |

### 5.7 Design Knowledge Plugins

| Plugin | Provides |
|---|---|
| `pencil-design` | Absorbs .pen files → LoRA knowledge of design language |
| `figma-design` | Absorbs Figma exports → LoRA knowledge |
| `material-design` | Google Material Design system knowledge |
| `human-interface` | Apple HIG design system knowledge |
| `custom-design` | Custom design tokens and patterns |

---

## 6. Plugin LoRA Knowledge

### 6.1 How Plugin Knowledge Works

When a plugin includes LoRA weights, loading the plugin means:

1. Load the plugin's calling conventions into the catalog
2. Load the plugin's LoRA weights and ATTACH them to the Mind
3. The Mind immediately knows how to use the plugin's conventions

This is like installing a skill. You don't just get the tool — you get the expertise.

### 6.2 LoRA Composition (Mixture of Experts)

Multiple plugin LoRAs are active simultaneously. A gating network (part of the Mind) learns which plugin's LoRA to weight for each operation:

```
Intent: "save the booking to the database and send a confirmation email"

Gating network activates:
  PostgreSQL LoRA: 0.8 (high — database operation)
  SMTP LoRA: 0.6 (moderate — email involved)
  Redis LoRA: 0.0 (not relevant)
  
Mind generates program using combined knowledge:
  $0 = postgres.query("INSERT INTO bookings ...")
  $1 = smtp.send_email(to=$client_email, subject="Booking Confirmed", ...)
  $2 = EMIT($0)
  STOP
```

### 6.3 LoRA Hot-Loading

Plugins (and their LoRA knowledge) can be loaded at runtime:

```
intent> "load the stripe plugin"

[Plugin Manager] Loading stripe@0.1.0...
[Plugin Manager] 8 conventions registered
[LoRA Manager] Attaching Stripe LoRA (rank 8, 15K params)
[Mind] Stripe knowledge integrated

intent> "charge $50 to customer cus_abc123"

[Mind] Program:
  $0 = stripe.create_payment_intent(amount=5000, currency="usd", customer="cus_abc123")
  $1 = EMIT($0)
  STOP
```

The Mind can use Stripe immediately because the LoRA knowledge was pre-trained.

### 6.4 Training Data for Synthesis

Plugins include training examples so the Synthesizer can train new models that know the plugin:

```json
[
  {
    "intent": "find all contacts in a 10km radius",
    "program": [
      {"convention": "postgres.query", "args": {"sql": "SELECT * FROM contacts WHERE ST_DWithin(location, ST_MakePoint($1,$2), 10000)", "params": ["span:lat", "span:lon"]}},
      {"convention": "EMIT", "args": {"ref": 0}}
    ]
  },
  {
    "intent": "cache the search results for 5 minutes",
    "program": [
      {"convention": "redis.set", "args": {"key": "span:cache_key", "value": "ref:0", "ttl": 300}},
      {"convention": "EMIT", "args": {"ref": 0}}
    ]
  }
]
```

The Synthesizer merges training data from all target plugins and trains a model that knows all of them.

---

## 7. Plugin Development Guide

### 7.1 Creating a New Plugin (Rust)

```rust
use soma_plugin::{SomaPlugin, CallingConvention, ArgSpec, Value, PluginError};

pub struct MyPlugin {
    // plugin state
}

impl SomaPlugin for MyPlugin {
    fn name(&self) -> &str { "my-plugin" }
    fn version(&self) -> &str { "0.1.0" }
    fn description(&self) -> &str { "Does something useful" }
    
    fn conventions(&self) -> Vec<CallingConvention> {
        vec![
            CallingConvention {
                id: 0,
                name: "do_thing".into(),
                description: "Does the thing".into(),
                args: vec![
                    ArgSpec { name: "input".into(), arg_type: ArgType::String, required: true, description: "The input".into() },
                ],
                returns: ReturnSpec::String,
                is_deterministic: true,
                estimated_latency: Duration::from_millis(10),
                side_effects: vec![],
            },
        ]
    }
    
    fn execute(&self, conv_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        match conv_id {
            0 => {
                let input = args[0].as_string()?;
                Ok(Value::String(format!("processed: {}", input)))
            },
            _ => Err(PluginError::UnknownConvention(conv_id)),
        }
    }
    
    fn on_load(&mut self, config: &PluginConfig) -> Result<(), PluginError> { Ok(()) }
    fn on_unload(&mut self) -> Result<(), PluginError> { Ok(()) }
}

// Export function for dynamic loading
#[no_mangle]
pub extern "C" fn soma_plugin_create() -> Box<dyn SomaPlugin> {
    Box::new(MyPlugin {})
}
```

### 7.2 Adding LoRA Knowledge

After creating the plugin, train LoRA knowledge:

1. Write training examples (intent → program pairs using your conventions)
2. Run the Synthesizer with your plugin's conventions included
3. Extract the LoRA weights that are specific to your plugin's operations
4. Package them in the `lora/` directory of your plugin

### 7.3 Testing

```rust
#[test]
fn test_do_thing() {
    let plugin = MyPlugin {};
    let result = plugin.execute(0, vec![Value::String("hello".into())]);
    assert_eq!(result.unwrap(), Value::String("processed: hello".into()));
}
```

---

## 8. Built-in vs External Plugins

### 8.1 Built-in Plugins

For embedded targets (ESP32, microcontrollers) where dynamic loading isn't available, plugins are compiled directly into the SOMA binary:

```rust
// In soma main.rs for embedded target
let mut plugins: Vec<Box<dyn SomaPlugin>> = vec![
    Box::new(GpioPlugin::new()),
    Box::new(I2cPlugin::new()),
    Box::new(TimerPlugin::new()),
];
```

### 8.2 External Plugins

For server/desktop targets, plugins are .so/.dylib files loaded dynamically:

```rust
let plugin = unsafe { load_plugin("./plugins/postgres.so")? };
plugin_manager.register(plugin);
```

### 8.3 Hybrid

A SOMA binary can have some built-in plugins and load additional ones dynamically. The Mind doesn't distinguish — all conventions appear in the same catalog.

---

## 9. Plugin Isolation and Safety

### 9.1 Capability Boundaries

Each plugin declares its side effects. The SOMA can restrict which plugins are allowed based on the deployment context:

```toml
# soma.toml
[security]
allow_network = ["smtp", "twilio", "http-bridge"]
allow_disk_write = ["postgres", "filesystem", "s3"]
deny = ["webrtc"]  # this SOMA doesn't need video calls
```

### 9.2 Resource Limits

Plugins can be resource-limited:

```toml
[plugins.postgres]
max_connections = 10
query_timeout = "30s"
max_result_size = "10MB"

[plugins.image-processing]
max_memory = "256MB"
max_processing_time = "10s"
```

### 9.3 Plugin Sandboxing (Future)

For untrusted plugins, run them in a WebAssembly sandbox (via `wasmtime`). The plugin compiles to WASM instead of native code. WASM provides memory isolation and capability restriction at the VM level.

---

## 10. Plugin Dependencies

### 10.1 The Problem

Plugins don't exist in isolation. The messaging plugin needs storage (postgres or sqlite). The auth plugin needs crypto. The image-processing plugin needs storage (s3 or filesystem). If a required dependency isn't loaded, the plugin can't function.

### 10.2 Dependency Declaration

```toml
# In plugin manifest.toml

[dependencies]
# Required: plugin MUST be loaded, or this plugin refuses to load
required = [
  { name = "postgres", min_version = "0.1.0" },
]

# Optional: plugin works without it, but enables extra features if present
optional = [
  { name = "redis", min_version = "0.1.0", enables = ["caching"] },
  { name = "s3", min_version = "0.1.0", enables = ["media_storage"] },
]

# Conflicts: cannot coexist with these plugins
conflicts = []
```

### 10.3 Resolution Order

The Plugin Manager resolves dependencies during loading:

```
1. Scan all plugins to load (from config or directory)
2. Build dependency graph
3. Topological sort: load dependencies before dependents
4. Detect cycles: A depends on B, B depends on A → error, refuse both
5. Load in order: independent plugins first, then dependents

Example:
  crypto (no deps)           → load first
  postgres (no deps)         → load second
  auth (requires crypto)     → load third
  messaging (requires postgres, optional redis) → load fourth
  redis (no deps)            → can load anytime
```

### 10.4 Missing Dependency Handling

| Scenario | Behavior |
|---|---|
| Required dep missing | Plugin refuses to load. Log error. SOMA continues without this plugin. |
| Required dep wrong version | Same as missing. Version mismatch = incompatible. |
| Optional dep missing | Plugin loads with reduced functionality. Log info. |
| Dep loads after dependent | Plugin Manager supports lazy resolution: if messaging loaded before postgres, messaging's `on_load` is deferred until postgres is available (with timeout). |
| Dep unloaded at runtime | Dependent plugin is notified via `on_dependency_removed(name)`. It can degrade gracefully or request unload itself. |

### 10.5 Inter-Plugin Communication

Plugins don't call each other directly. All communication goes through the Mind's program generation. If the messaging plugin needs to store a message, the Mind generates:

```
$0 = messaging.prepare_message(...)
$1 = postgres.query("INSERT INTO messages ...", $0)
$2 = EMIT($1)
```

The Mind orchestrates. Plugins are isolated.

**Exception: Plugin Services.** For low-level shared functionality (connection pooling, crypto primitives), plugins can expose services that other plugins consume directly:

```rust
pub trait PluginService: Send + Sync {
    fn service_name(&self) -> &str;
}

// Crypto plugin exposes a service
pub struct CryptoService { ... }
impl CryptoService {
    pub fn hash_password(&self, password: &str) -> String;
    pub fn verify_password(&self, password: &str, hash: &str) -> bool;
    pub fn generate_token(&self) -> String;
}

// Auth plugin consumes it
impl AuthPlugin {
    fn on_load(&mut self, config: &PluginConfig) -> Result<()> {
        self.crypto = config.get_service::<CryptoService>("crypto")?;
        Ok(())
    }
}
```

Services are registered during plugin loading and accessed by name. This is for infrastructure-level sharing only — business logic always flows through the Mind.

---

## 11. Plugin Versioning and Updates

### 11.1 Plugin Version Format

Semantic versioning: `major.minor.patch`

- **Patch** (0.1.0 → 0.1.1): Bug fixes. No convention changes. LoRA weights compatible. Drop-in replacement.
- **Minor** (0.1.0 → 0.2.0): New conventions added. Existing conventions unchanged. LoRA weights may need retraining for new conventions but work for old ones.
- **Major** (0.1.0 → 1.0.0): Breaking changes. Convention signatures changed or removed. LoRA weights incompatible. Re-synthesis required.

### 11.2 Update Process

```
Current state: postgres v0.1.0 loaded, Mind synthesized with postgres v0.1.0

Update to postgres v0.2.0:
  1. Stop accepting new intents (brief pause)
  2. Drain in-flight programs using postgres conventions
  3. Unload postgres v0.1.0 (on_unload → close connections)
  4. Load postgres v0.2.0 (on_load → new connections)
  5. Convention catalog rebuilt:
     - Existing conventions (query, execute, ...) → same names, compatible
     - New conventions (query_prepared, listen_notify) → available but Mind doesn't know them yet
  6. Resume accepting intents
  7. Mind uses old conventions immediately (same names)
  8. To use new conventions: re-synthesize with v0.2.0 training data (optional, non-blocking)
```

### 11.3 Side-by-Side Versions

During migration, two versions can coexist briefly:

```toml
# soma.toml
[plugins]
postgres_old = { path = "./plugins/postgres-0.1.0.so", alias = "postgres" }
postgres_new = { path = "./plugins/postgres-0.2.0.so", alias = "postgres_v2" }
```

Both are loaded with different aliases. The Mind continues using `postgres.*` conventions (old). New programs can be manually tested with `postgres_v2.*`. Once validated, switch the alias.

### 11.4 LoRA Compatibility on Update

| Plugin Version Change | LoRA Weights |
|---|---|
| Patch update | Compatible. Keep existing LoRA. |
| Minor update (conventions added) | Partially compatible. LoRA works for old conventions. New conventions have no LoRA — learn through experience or retrain. |
| Major update (conventions changed) | Incompatible. Discard plugin LoRA. Re-synthesize or learn from scratch. |
| Base Mind updated (new architecture) | ALL plugin LoRAs incompatible if hidden dimensions changed. Re-synthesize everything. |

---

## 12. Plugin Configuration Validation

### 12.1 Schema Declaration

Each plugin declares its config schema in the manifest:

```toml
# manifest.toml
[config.schema]

[config.schema.host]
type = "string"
required = true
default = "localhost"
description = "Database host"

[config.schema.port]
type = "integer"
required = true
default = 5432
min = 1
max = 65535
description = "Database port"

[config.schema.password]
type = "string"
required = true
secret = true                # read from env var, never log
env_key = "SOMA_PG_PASSWORD"
description = "Database password"

[config.schema.max_connections]
type = "integer"
required = false
default = 10
min = 1
max = 100
description = "Maximum connection pool size"

[config.schema.ssl_mode]
type = "enum"
required = false
default = "prefer"
values = ["disable", "prefer", "require", "verify-full"]
description = "SSL connection mode"
```

### 12.2 Validation Timing

```
Plugin load sequence:
  1. Read manifest.toml → extract config schema
  2. Merge config sources: manifest defaults → soma.toml [plugins.X] → env vars
  3. Validate against schema:
     - All required fields present?
     - Types correct?
     - Values within range/enum?
     - Secret fields resolved from env vars?
  4. If validation fails:
     - Missing required non-secret field → error, refuse to load
     - Missing secret field → error with hint: "Set SOMA_PG_PASSWORD environment variable"
     - Invalid type/range → error with expected vs actual
  5. If validation passes → call plugin.on_load(validated_config)
```

### 12.3 Runtime Revalidation

If config is changed at runtime (via intent "change postgres port to 5433"):

```
1. Validate new config against schema
2. If valid → call plugin.on_config_changed(new_config)
3. Plugin decides: reconnect, restart, or reject change
4. If plugin rejects → revert to old config, notify requester
```

### 12.4 Config Diagnostics

```
intent> "check plugin configs"

[Proprioception]
  postgres: ✓ all config valid
  redis: ✓ all config valid
  smtp: ⚠ password not set (SOMA_SMTP_PASS). Email sending will fail.
  s3: ✗ bucket "helperbook-media" not accessible. Check credentials.
  twilio: ✗ not configured. SMS notifications disabled.
```

---

## 13. Streaming Conventions

### 13.1 Convention Return Types

Each convention declares its return type. The Mind needs to know this to plan programs correctly:

```rust
pub enum ReturnSpec {
    Value(ValueType),       // single value: string, int, map, etc.
    Stream(StreamSpec),     // produces a stream of values
    Handle(HandleType),     // opaque handle (file descriptor, connection)
    Void,                   // no return (side effect only)
}

pub struct StreamSpec {
    pub item_type: ValueType,   // type of each stream item
    pub estimated_items: Option<u32>,  // hint for planning
    pub backpressure: bool,     // can receiver slow down sender?
}
```

### 13.2 How the Mind Handles Streams

The Mind knows (from synthesis) which conventions produce streams vs single values. It generates different program patterns:

**Single value (normal):**
```
$0 = postgres.query("SELECT count(*) FROM contacts")
$1 = EMIT($0)
STOP
```

**Stream producer:**
```
$0 = postgres.query_stream("SELECT * FROM contacts ORDER BY name")
$1 = STREAM_EMIT($0, channel=100)    // forward stream to Synaptic channel
STOP
```

`STREAM_EMIT` is a special control opcode that connects a plugin's stream output to a Synaptic Protocol channel. Each item the plugin produces becomes a STREAM_DATA signal.

### 13.3 Stream-to-Value Conversion

If the Mind needs to use a stream result in a subsequent step, it must collect it:

```
$0 = postgres.query_stream("SELECT name FROM contacts")
$1 = COLLECT($0)            // collect stream into a list Value
$2 = redis.cache_set("contacts:names", $1, ttl=300)
STOP
```

`COLLECT` is a built-in control opcode. It buffers the stream into a single Value (list). This is bounded by `max_collect_size` to prevent memory exhaustion.

### 13.4 Stream References

A stream handle ($0 in the examples above) CANNOT be used as a `ref` in the same way as a regular Value. The Mind learns this during synthesis — training data never shows `ref:stream_step` used in non-stream contexts.

---

## 14. Plugin State Persistence

### 14.1 The Problem

Plugins have runtime state:
- PostgreSQL: connection pool, prepared statement cache
- Redis: connection, subscription list
- Messaging: delivery queue, pending read receipts
- Image processing: thumbnail cache

When the SOMA checkpoints, should this state be saved? When restoring, can it be rebuilt?

### 14.2 State Categories

| Category | Persisted? | Rebuild Strategy |
|---|---|---|
| **Reconstructible** — connection pools, caches, prepared statements | No | Plugin rebuilds on `on_load()` from config. Fast, no data loss. |
| **Ephemeral** — in-flight requests, temp buffers | No | Lost on checkpoint/restore. Acceptable — retry handles it. |
| **Critical** — pending delivery queue, unprocessed events | Yes | Plugin must serialize this in checkpoint. |

### 14.3 Plugin Checkpoint Interface

```rust
pub trait SomaPlugin {
    // ... existing methods ...
    
    /// Serialize plugin state for checkpoint (optional)
    fn checkpoint_state(&self) -> Option<Vec<u8>> {
        None  // default: no state to persist
    }
    
    /// Restore plugin state from checkpoint (optional)
    fn restore_state(&mut self, state: &[u8]) -> Result<(), PluginError> {
        Ok(())  // default: nothing to restore
    }
}
```

The SOMA Core checkpoint includes a section for each plugin's state:

```
soma_checkpoint.bin:
  ...
  plugin_states: [
    { name: "messaging", version: "0.1.0", state: [bytes] },
    { name: "calendar", version: "0.2.0", state: [bytes] },
  ]
  ...
```

### 14.4 State Size Limits

Plugin state in checkpoints is bounded:

```toml
[memory.checkpoint]
max_plugin_state_size = "1MB"    # per plugin
max_total_plugin_state = "10MB"  # all plugins combined
```

Plugins exceeding the limit have their state truncated (with warning) or excluded from the checkpoint.

### 14.5 Reconnection on Restore

After restoring from checkpoint, plugins reconnect to external services:

```
Restore from checkpoint:
  1. Load plugin binaries
  2. Call plugin.on_load(config) → reconnect to postgres, redis, etc.
  3. Call plugin.restore_state(saved_state) → restore critical state
  4. Plugin is ready
  
Time to restore: dominated by external reconnection (typically <5s)
```

---

## 15. Error Cleanup and Transactions

### 15.1 The Problem

A Mind-generated program may fail mid-execution:

```
$0 = postgres.begin_transaction()    ← succeeds
$1 = postgres.query("INSERT ...")    ← succeeds
$2 = smtp.send_email(...)            ← FAILS (SMTP server down)
$3 = postgres.commit($0)             ← never reached
STOP
```

The database transaction is now open and uncommitted. Who rolls it back?

### 15.2 Cleanup Hooks

Each convention can declare a cleanup action:

```rust
pub struct CallingConvention {
    // ... existing fields ...
    
    /// If this convention produces a resource that needs cleanup on error,
    /// specify which convention to call and how.
    pub cleanup: Option<CleanupSpec>,
}

pub struct CleanupSpec {
    pub convention_name: String,  // "postgres.rollback" or "close_fd"
    pub pass_result_as: u8,       // which arg position receives the step's result
}
```

Example:

```rust
CallingConvention {
    name: "begin_transaction",
    cleanup: Some(CleanupSpec {
        convention_name: "rollback_transaction",
        pass_result_as: 0,  // rollback(transaction_handle)
    }),
    ...
}
```

### 15.3 Core Cleanup Execution

When a program fails at step N:

```
1. Record error at step N
2. Walk backwards from step N-1 to step 0
3. For each step that has a cleanup spec AND produced a result:
   a. Call the cleanup convention with the step's result
   b. Log cleanup action
4. Report error to requester (includes which cleanups were executed)
```

```
Program failure at step 2:
  Step 0: begin_transaction → handle=TX1 → has cleanup (rollback)
  Step 1: query(INSERT) → ok → no cleanup needed
  Step 2: send_email → FAILED
  
  Cleanup: rollback_transaction(TX1) → ok
  
  Report: "Email send failed. Database transaction rolled back."
```

### 15.4 File Handle Cleanup

Same pattern for file handles:

```
$0 = filesystem.open("/tmp/data.csv", "w")  → cleanup: filesystem.close($0)
$1 = filesystem.write($0, data)             → fails
// Cleanup: filesystem.close($0)
```

### 15.5 No Cleanup Available

If a step has no cleanup spec and a later step fails, the resource leaks. This is a plugin design issue, not a Core issue. Well-designed plugins always declare cleanup for resource-acquiring conventions.

The Core logs a warning: "Step 0 (postgres.begin_transaction) has no cleanup spec. Possible resource leak."

---

## 16. Training Data Format

### 16.1 Training Example Schema

Each plugin provides training examples as a JSON array:

```json
{
  "schema_version": "1.0",
  "plugin": "postgres",
  "plugin_version": "0.1.0",
  "examples": [
    {
      "id": "pg_001",
      "intents": [
        "find all contacts near downtown",
        "search for contacts in the downtown area",
        "show contacts close to downtown",
        "who is near downtown"
      ],
      "program": [
        {
          "convention": "postgres.query",
          "args": [
            { "name": "sql", "type": "literal", "value": "SELECT * FROM contacts WHERE ST_DWithin(location, ST_MakePoint($1,$2), 5000) ORDER BY ST_Distance(location, ST_MakePoint($1,$2))" },
            { "name": "params", "type": "span", "extract": "location" }
          ]
        },
        {
          "convention": "EMIT",
          "args": [
            { "name": "data", "type": "ref", "step": 0 }
          ]
        },
        { "convention": "STOP" }
      ],
      "params": {
        "location": {
          "pool": ["downtown", "city center", "main street", "north side", "the park", "central station"],
          "type": "string"
        }
      },
      "tags": ["geospatial", "contacts", "search"]
    }
  ]
}
```

### 16.2 Field Descriptions

| Field | Purpose |
|---|---|
| `intents` | Multiple natural language phrasings for the same operation. Synthesizer generates training pairs from all combinations of intent × param values. |
| `program` | Sequence of convention calls. Each step has a convention name and argument spec. |
| `args[].type` | `"literal"` = fixed value, `"span"` = extract from intent text, `"ref"` = reference previous step result. |
| `params` | Named parameters with value pools. The synthesizer substitutes these into intents and program args to generate diverse training data. |
| `tags` | Categories for balanced training (ensure the synthesizer doesn't oversample one tag). |

### 16.3 Multi-Plugin Training Examples

Some operations span multiple plugins. These are provided as separate training files:

```json
{
  "schema_version": "1.0",
  "plugin": "_cross_plugin",
  "requires_plugins": ["postgres", "redis"],
  "examples": [
    {
      "id": "cross_001",
      "intents": [
        "get contacts and cache the results",
        "fetch contacts with caching",
        "load contacts from cache or database"
      ],
      "program": [
        {
          "convention": "redis.cache_get",
          "args": [{ "name": "key", "type": "literal", "value": "contacts:all" }]
        },
        {
          "convention": "postgres.query",
          "args": [{ "name": "sql", "type": "literal", "value": "SELECT * FROM contacts" }],
          "condition": "step_0_is_null"
        },
        {
          "convention": "redis.cache_set",
          "args": [
            { "name": "key", "type": "literal", "value": "contacts:all" },
            { "name": "value", "type": "ref", "step": 1 },
            { "name": "ttl", "type": "literal", "value": 300 }
          ],
          "condition": "step_0_is_null"
        },
        {
          "convention": "EMIT",
          "args": [{ "name": "data", "type": "ref", "step": 0, "fallback_step": 1 }]
        },
        { "convention": "STOP" }
      ]
    }
  ]
}
```

Note: conditional steps (`condition`) are an advanced feature. The initial Mind architecture doesn't support conditional execution — it generates linear programs. Conditional examples are included for future Mind architectures that support branching.

### 16.4 Synthesizer Consumption

The Synthesizer:

1. Collects training data from all loaded plugins
2. Cross-references: every convention referenced in examples must exist in some plugin's catalog
3. Generates training pairs: expand intents × param pools
4. Balances: ensure roughly equal representation per plugin and per tag
5. Trains the Mind model
6. Exports to ONNX / .soma-model

---

## 17. LoRA Knowledge Compatibility

### 17.1 What Must Match

Plugin LoRA weights are low-rank matrices (A, B) that modify specific layers of the Mind. For LoRA weights to be compatible:

| Dimension | Must Match |
|---|---|
| Base Mind hidden_dim | A.shape[1] must equal the layer's input features |
| Base Mind decoder_dim | B.shape[0] must equal the layer's output features |
| LoRA target layers | Names must match (e.g., "op_head", "gru") |
| LoRA rank | Can differ — but the Mixture of Experts gating must handle mixed ranks |

### 17.2 Compatibility Matrix

```
Plugin LoRA trained on Mind v1 (hidden=128, decoder=256):
  ├── Mind v1 (hidden=128, decoder=256): ✓ exact match
  ├── Mind v1.1 (hidden=128, decoder=256, new layer added): ✓ old layers match
  ├── Mind v2 (hidden=256, decoder=512): ✗ dimension mismatch
  └── Mind v1 different architecture (Transformer): ✗ layer names don't match
```

### 17.3 LoRA Version Metadata

Plugin LoRA files include metadata for compatibility checking:

```json
{
  "lora_version": "1.0",
  "plugin": "postgres",
  "plugin_version": "0.1.0",
  "trained_on": {
    "mind_architecture": "bilstm_gru",
    "mind_version": "0.1.0",
    "hidden_dim": 128,
    "decoder_dim": 256,
    "target_layers": ["op_head", "gru", "a0t_head", "a1t_head", "s0s_q", "s0e_q"],
    "rank": 8,
    "alpha": 2.0
  },
  "training_stats": {
    "examples": 5000,
    "epochs": 40,
    "final_loss": 0.0023
  }
}
```

### 17.4 Incompatible LoRA Handling

When loading a plugin whose LoRA is incompatible with the current Mind:

```
1. Load plugin conventions (always works — conventions are data)
2. Attempt to load LoRA:
   a. Check dimensions → mismatch detected
   b. Log warning: "postgres LoRA incompatible (trained on mind v1, running mind v2)"
   c. Skip LoRA loading
3. Plugin operates without pre-trained knowledge
4. Mind learns to use plugin conventions through experience (runtime adaptation)
5. Proprioception reports: "postgres plugin loaded WITHOUT LoRA knowledge"
```

### 17.5 LoRA Migration Tool

For major Mind updates, a migration tool re-trains plugin LoRAs:

```bash
soma-migrate-lora \
  --old-mind models/v1/ \
  --new-mind models/v2/ \
  --plugin-training-data plugins/postgres/training/ \
  --output plugins/postgres/lora/postgres_v2.lora
```

This uses the plugin's training data to produce new LoRA weights for the new Mind architecture. Plugin authors run this and publish updated LoRA files.

---

## 18. Platform-Specific Plugins

### 18.1 Platform Declaration

```toml
# manifest.toml
[compatibility]
platforms = ["x86_64-linux", "aarch64-linux", "aarch64-macos"]
# or
platforms = ["esp32"]
# or
platforms = ["wasm32"]   # browser-only (WebAssembly)
# or
platforms = ["*"]         # universal (pure Rust, no native deps)
```

### 18.2 Platform Detection at Load Time

```
Plugin Manager loading dom-renderer:
  1. Read manifest → platforms = ["wasm32"]
  2. Current platform = "aarch64-macos"
  3. Mismatch → skip plugin
  4. Log: "dom-renderer skipped: requires wasm32, running aarch64-macos"
```

### 18.3 Platform-Specific Plugin Variants

A single plugin package can contain multiple platform binaries:

```
plugin-postgres-0.1.0.soma-plugin/
  manifest.toml
  lora/postgres.lora              # platform-independent
  training/examples.json          # platform-independent
  bin/
    x86_64-linux/plugin.so
    aarch64-linux/plugin.so
    aarch64-macos/plugin.dylib
    # no ESP32 — postgres doesn't run on ESP32
```

The Plugin Manager selects the correct binary for the current platform.

### 18.4 Embedded-Only Plugins

Some plugins only make sense on embedded targets:

```toml
# gpio plugin manifest
[compatibility]
platforms = ["esp32", "esp32s3", "esp32c3", "rpi"]

[hardware]
requires = ["gpio"]   # hardware capability requirement
```

These plugins are always built-in (compiled into the SOMA binary) for their target platforms. They're never dynamically loaded because embedded targets don't support dynamic loading.

### 18.5 Browser-Only Plugins

Plugins that target the browser (Interface SOMA running as WebAssembly):

```toml
# dom-renderer manifest
[compatibility]
platforms = ["wasm32"]

[browser]
requires = ["dom"]
```

These compile to WASM and are loaded by the Interface SOMA running in a browser. They access the DOM through `web-sys` / `wasm-bindgen` Rust crates.

---

## 19. Plugin Performance Contracts

### 19.1 Latency Declaration

Each convention declares its expected latency:

```rust
CallingConvention {
    name: "query",
    estimated_latency: Duration::from_millis(10),  // typical
    max_latency: Duration::from_secs(30),           // absolute max before timeout
    ...
}
```

The Mind uses `estimated_latency` for planning — it may prefer a cached result (1ms) over a database query (10ms) when both can answer the same intent.

### 19.2 Timeout Enforcement

The Core enforces `max_latency` at the plugin boundary:

```rust
let result = tokio::time::timeout(
    convention.max_latency,
    plugin.execute_async(conv_id, args)
).await;

match result {
    Ok(Ok(value)) => { /* success */ },
    Ok(Err(plugin_error)) => { /* plugin returned error */ },
    Err(_) => {
        // TIMEOUT — plugin took too long
        tracing::warn!(
            plugin = %plugin_name,
            convention = %conv_name,
            timeout = ?convention.max_latency,
            "Plugin execution timed out"
        );
        // Record as slow execution (proprioception)
        // Retry or degrade
    },
}
```

### 19.3 Performance Monitoring

The Core tracks actual latency per convention:

```rust
pub struct ConventionStats {
    pub call_count: u64,
    pub total_time: Duration,
    pub avg_time: Duration,
    pub p50_time: Duration,
    pub p99_time: Duration,
    pub timeout_count: u64,
    pub error_count: u64,
}
```

Queryable via proprioception:

```
intent> "how is postgres performing"

[Proprioception] postgres plugin performance:
  query:          avg=8ms, p99=45ms, timeouts=0, errors=2 (of 1,234 calls)
  execute:        avg=3ms, p99=12ms, timeouts=0, errors=0 (of 456 calls)
  begin_txn:      avg=1ms, p99=3ms,  timeouts=0, errors=0 (of 89 calls)
```

### 19.4 Dead Plugin Detection

If a plugin's conventions consistently timeout or error:

```
Convention timeout rate > 50% over 60s window:
  1. Log error: "postgres.query is failing 60% of the time"
  2. Proprioception alert: "postgres plugin unhealthy"
  3. If timeout rate > 90% for 5 minutes:
     a. Mark plugin as "degraded"
     b. Mind's feedback layer learns to avoid this plugin
     c. If alternative exists (sqlite instead of postgres): Mind routes there
     d. If no alternative: report to human via Synaptic Protocol
```

### 19.5 Embedded Performance

Embedded plugins have tighter constraints:

```toml
# soma-embedded.toml
[resources]
max_plugin_execution_time = "500ms"   # absolute max for any convention
```

Any convention exceeding 500ms on ESP32 is killed. This prevents a single bad operation from blocking the entire SOMA (which is single-threaded on embedded).

---

## 20. Plugin Security — Threat Model

### 20.1 Trust Levels

| Level | Source | Trust | Isolation |
|---|---|---|---|
| **Built-in** | Compiled into SOMA binary | Full trust | None (same process, same memory) |
| **Community** | Published to registry, open source | Moderate trust | Code review + optional WASM sandbox |
| **Vendor** | Published by service provider (Stripe, AWS) | Moderate trust | Signed packages + optional WASM sandbox |
| **Private** | Developed in-house | High trust | None (same process) |
| **Untrusted** | Unknown source, unreviewed | No trust | MANDATORY WASM sandbox |

### 20.2 Attack Vectors

| Attack | Mechanism | Mitigation |
|---|---|---|
| **Memory read** | Plugin reads Mind weights, LoRA, or other plugin memory | WASM sandbox: linear memory isolation. Plugin can only access its own memory. |
| **Memory write** | Plugin corrupts Mind state or other plugin state | WASM sandbox: writes restricted to own memory. Native: panic at catch_unwind boundary. |
| **Data exfiltration** | Plugin sends user data to attacker-controlled server | Capability restrictions: `allow_network` config controls which plugins can make outbound connections. WASM sandbox: no network access unless explicitly granted. |
| **Denial of service** | Plugin infinite loops, allocates unbounded memory | Timeout enforcement (Section 19.2). Memory limits per plugin (Section 9.2). |
| **Convention hijacking** | Plugin registers a convention with same name as a trusted plugin to intercept calls | Convention namespacing: conventions are prefixed with plugin name. `postgres.query` can only be registered by the `postgres` plugin. |
| **LoRA poisoning** | Malicious LoRA weights that cause the Mind to generate harmful programs | LoRA weights loaded from untrusted sources are sandboxed: applied with a magnitude cap, tested against a validation suite before permanent attachment. |
| **Checkpoint tampering** | Plugin inserts malicious state into checkpoint | Plugin state in checkpoints is tagged with plugin name and version. Checkpoints are integrity-checked (CRC32 + optional signature). |

### 20.3 WASM Sandbox Details

For untrusted plugins, the WASM sandbox provides:

```
┌─────────────────────────────────────────┐
│  SOMA Core Process                       │
│                                          │
│  ┌────────────────────────────────┐      │
│  │  wasmtime VM                    │     │
│  │  ┌──────────────────────────┐  │     │
│  │  │  Untrusted Plugin        │  │     │
│  │  │  (compiled to WASM)      │  │     │
│  │  │                          │  │     │
│  │  │  Own linear memory only  │  │     │
│  │  │  No direct OS access     │  │     │
│  │  │  No network access       │  │     │
│  │  │  No filesystem access    │  │     │
│  │  │  CPU time limited        │  │     │
│  │  │  Memory limited          │  │     │
│  │  └──────────────────────────┘  │     │
│  │                                │     │
│  │  Host functions (allowlist):   │     │
│  │   - return Value to Core       │     │
│  │   - log message                │     │
│  │   - request permission         │     │
│  └────────────────────────────────┘     │
│                                          │
│  Trusted plugins: native, same process   │
└─────────────────────────────────────────┘
```

WASM plugins communicate with the Core exclusively through host functions. They cannot access anything outside their sandbox. The performance overhead of WASM is ~10-30% vs native — acceptable for untrusted code.

### 20.4 Plugin Signing

Published plugins can be cryptographically signed:

```toml
# manifest.toml
[signing]
signature = "ed25519:abc123..."
signer = "soma-community"
signer_key = "ed25519_public:xyz789..."
```

The SOMA verifies signatures before loading:

```
1. Plugin registry publishes signer public keys
2. SOMA downloads plugin package
3. Verify: Ed25519 signature over package hash matches signer's public key
4. If valid → load (at appropriate trust level)
5. If invalid → refuse to load, warn user
```

### 20.5 Least Privilege

Plugins should request only the capabilities they need:

```toml
# manifest.toml
[permissions]
network_outbound = ["postgres://localhost:5432"]   # only this address
filesystem_read = ["/etc/ssl/certs"]                # only SSL certs
filesystem_write = []                               # no disk writes
environment_vars = ["SOMA_PG_PASSWORD"]             # only this var
```

The Core enforces these declarations. A plugin requesting broader access than declared is suspicious — log warning, ask user for confirmation.