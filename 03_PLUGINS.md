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
