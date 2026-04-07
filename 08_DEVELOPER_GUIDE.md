# SOMA Developer Guide

**Status:** Reference  
**Audience:** Anyone building with SOMA for the first time

---

## 1. What You're Getting Into

SOMA is not a framework. You don't write code and plug it in. You synthesize a Mind, attach plugins (body parts), and describe what you want. The SOMA IS the application.

This guide walks you from zero to a working SOMA that responds to intents, uses plugins, communicates with other SOMAs, and adapts from experience.

---

## 2. Install

### 2.1 From Binary (Recommended)

```bash
# Linux x86_64
curl -L https://github.com/soma-project/soma/releases/latest/download/soma-linux-x86_64 -o soma
chmod +x soma

# macOS Apple Silicon
curl -L https://github.com/soma-project/soma/releases/latest/download/soma-macos-aarch64 -o soma
chmod +x soma

# Verify
./soma --version
# SOMA v0.1.0 (server, onnx)
```

### 2.2 From Source (Rust)

```bash
git clone https://github.com/soma-project/soma.git
cd soma
cargo build --release
# Binary at target/release/soma
```

### 2.3 Synthesizer (Python)

The Synthesizer is separate — it's the build tool:

```bash
pip install soma-synthesize
```

Requires Python 3.10+ and PyTorch 2.0+.

---

## 3. Your First SOMA

### 3.1 Create a Project

```bash
mkdir my-soma && cd my-soma

# Initialize with a starter model (pre-trained on filesystem plugin)
soma init --template filesystem
```

This creates:

```
my-soma/
  soma.toml            # Configuration
  models/
    server/
      encoder.onnx     # Pre-trained Mind
      decoder.onnx
    vocab.json
    catalog.json
  plugins/             # Plugin directory (empty — filesystem is built-in)
  checkpoints/         # Experiential memory storage
```

### 3.2 Run It

```bash
soma --repl
```

```
SOMA v0.1.0 | Mind: 823K params | Plugins: filesystem (17 conv)
Synaptic Protocol listening on :9001
Ready.

soma> list files in the current directory
  [Mind] Program (4 steps, 96.8%):
    $0 = fs.list_dir(".")
    $1 = EMIT($0)
    STOP
  [Body] soma.toml, models, plugins, checkpoints

soma> create a file called hello.txt with content "Hello from SOMA"
  [Mind] Program (2 steps, 94.1%):
    $0 = fs.write_file("hello.txt", "Hello from SOMA")
    $1 = EMIT("done")
    STOP
  [Body] done

soma> read hello.txt
  [Mind] Program (2 steps, 97.5%):
    $0 = fs.read_file("hello.txt")
    $1 = EMIT($0)
    STOP
  [Body] Hello from SOMA
```

Your first SOMA is running. It understands filesystem operations and executes them from natural language.

---

## 4. Install a Plugin

### 4.1 From the Registry

```bash
soma plugin install postgres
# Downloaded postgres v0.1.0 (1.2MB)
# Conventions: 12
# LoRA knowledge: included
```

### 4.2 Configure It

Add to `soma.toml`:

```toml
[plugins.postgres]
host = "localhost"
port = 5432
database = "myapp"
username = "soma"
password_env = "SOMA_PG_PASSWORD"
```

### 4.3 Use It

```bash
export SOMA_PG_PASSWORD=secret
soma --repl
```

```
SOMA v0.1.0 | Plugins: filesystem (17), postgres (12) [+LoRA]
Ready.

soma> create a table called users with id, name, and email columns
  [Mind] Program (2 steps, 93.4%):
    $0 = postgres.create_table("users", {id: "SERIAL PRIMARY KEY", name: "TEXT NOT NULL", email: "TEXT"})
    $1 = EMIT("done")
    STOP
  [Body] done

soma> insert a user named Alice with email alice@example.com
  [Mind] Program (2 steps, 95.1%):
    $0 = postgres.execute("INSERT INTO users (name, email) VALUES ($1, $2)", ["Alice", "alice@example.com"])
    $1 = EMIT($0)
    STOP
  [Body] 1 row affected

soma> list all users
  [Mind] Program (2 steps, 97.8%):
    $0 = postgres.query("SELECT * FROM users")
    $1 = EMIT($0)
    STOP
  [Body] [{id: 1, name: "Alice", email: "alice@example.com"}]
```

---

## 5. Connect Two SOMAs

### 5.1 Start a Second SOMA

Terminal 1:
```bash
soma --repl --bind 0.0.0.0:9001
```

Terminal 2:
```bash
soma --repl --bind 0.0.0.0:9002 --peer localhost:9001
```

### 5.2 Send a Signal

On SOMA-B (port 9002):
```
soma> send "hello from SOMA-B" to soma on port 9001
  [Mind] Program:
    $0 = synapse.send("soma-9001", "hello from SOMA-B")
    STOP
  [Body] sent
```

On SOMA-A (port 9001), the message arrives as a Synaptic signal.

### 5.3 Discover Peers

```
soma> who is connected
  [Proprioception]
    Peers: 1
    - soma-9002 at localhost:9002 (plugins: filesystem)
      Latency: 1ms
      Connected: 2m ago
```

---

## 6. Create Your First Plugin

### 6.1 Scaffold

```bash
soma plugin new my-plugin
cd plugins/my-plugin
```

Creates:

```
my-plugin/
  Cargo.toml
  manifest.toml
  src/lib.rs
  training/examples.json
```

### 6.2 Implement

Edit `src/lib.rs`:

```rust
use soma_plugin::prelude::*;

pub struct MyPlugin;

impl SomaPlugin for MyPlugin {
    fn name(&self) -> &str { "my-plugin" }
    fn version(&self) -> &str { "0.1.0" }
    fn description(&self) -> &str { "Does something useful" }

    fn conventions(&self) -> Vec<CallingConvention> {
        vec![
            CallingConvention {
                id: 0,
                name: "greet".into(),
                description: "Greet someone by name".into(),
                args: vec![
                    ArgSpec::required("name", ArgType::String, "Person to greet"),
                ],
                returns: ReturnSpec::Value(ValueType::String),
                is_deterministic: true,
                estimated_latency: Duration::from_millis(1),
                side_effects: vec![],
                cleanup: None,
            },
        ]
    }

    fn execute(&self, conv_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        match conv_id {
            0 => {
                let name = args[0].as_string()?;
                Ok(Value::String(format!("Hello, {}!", name)))
            },
            _ => Err(PluginError::UnknownConvention(conv_id)),
        }
    }

    fn on_load(&mut self, _config: &PluginConfig) -> Result<(), PluginError> { Ok(()) }
    fn on_unload(&mut self) -> Result<(), PluginError> { Ok(()) }
}

#[no_mangle]
pub extern "C" fn soma_plugin_create() -> Box<dyn SomaPlugin> {
    Box::new(MyPlugin)
}
```

### 6.3 Add Training Data

Edit `training/examples.json`:

```json
{
  "schema_version": "1.0",
  "plugin": "my-plugin",
  "plugin_version": "0.1.0",
  "examples": [
    {
      "id": "greet_001",
      "intents": [
        "say hello to {name}",
        "greet {name}",
        "hi {name}"
      ],
      "program": [
        {"convention": "my-plugin.greet", "args": [{"name": "name", "type": "span", "extract": "name"}]},
        {"convention": "EMIT", "args": [{"name": "data", "type": "ref", "step": 0}]},
        {"convention": "STOP"}
      ],
      "params": {
        "name": {
          "pool": ["Alice", "Bob", "Charlie", "Ana", "Ion"],
          "type": "string"
        }
      }
    }
  ]
}
```

### 6.4 Build and Load

```bash
cd plugins/my-plugin
cargo build --release
# Produces: target/release/libmy_plugin.so (or .dylib)
cp target/release/libmy_plugin.so ../../plugins/

# Re-synthesize to include new plugin's training data
cd ../..
soma-synthesize train --plugins ./plugins --output ./models
```

### 6.5 Use It

```bash
soma --repl
```

```
SOMA v0.1.0 | Plugins: filesystem (17), my-plugin (1)
Ready.

soma> say hello to Alice
  [Mind] Program (2 steps, 92.4%):
    $0 = my-plugin.greet("Alice")
    $1 = EMIT($0)
    STOP
  [Body] Hello, Alice!
```

---

## 7. Build an Application

### 7.1 The HelperBook Pattern

A real application is:

1. Install relevant plugins (postgres, redis, auth, messaging, etc.)
2. Configure them in `soma.toml`
3. Synthesize a Mind that knows all their conventions
4. Start the SOMA
5. Describe your application via intents (schema, business rules, views)
6. Connect an Interface SOMA for UI
7. The application exists

### 7.2 Quick Example: Bookmark App

```bash
# Setup
soma init --template empty
soma plugin install postgres sqlite filesystem
soma-synthesize train --plugins ./plugins --output ./models

# Configure
cat >> soma.toml << EOF
[plugins.sqlite]
path = "./data/bookmarks.db"
EOF

# Run
soma --repl
```

```
soma> create a table called bookmarks with id, url, title, tags, and created_at

soma> add bookmark https://soma-project.dev titled "SOMA Project" tagged "ai,neural"

soma> find all bookmarks tagged ai

soma> export all bookmarks as a csv file to bookmarks.csv
```

Four intents. A working bookmark app with a database and file export. No code.

---

## 8. Debug REPL Reference

### 8.1 Commands

| Command | Description |
|---|---|
| `:status` | SOMA health: mind, plugins, peers, memory |
| `:inspect mind` | Model details, conventions, LoRA state |
| `:inspect plugin <name>` | Plugin details, config, stats |
| `:trace on/off/full` | Toggle program trace verbosity |
| `:checkpoint` | Save current state |
| `:restore <file>` | Restore from checkpoint |
| `:plugins` | List loaded plugins |
| `:conventions` | List all available conventions |
| `:peers` | List connected SOMAs |
| `:config` | Show current configuration |
| `:adapt` | Trigger manual LoRA adaptation |
| `:consolidate` | Trigger manual consolidation (sleep cycle) |
| `:history` | Show recent intents and results |
| `:quit` | Graceful shutdown |

---

## 9. Project Structure Reference

```
my-soma/
  soma.toml                 # Configuration
  models/
    server/
      encoder.onnx          # Mind (server target)
      decoder.onnx
    embedded/
      model.soma-model      # Mind (ESP32 target)
    vocab.json              # Tokenizer vocabulary
    catalog.json            # Convention catalog
    meta.json               # Synthesis metadata
  plugins/
    postgres.so             # Downloaded/built plugins
    redis.so
    my-plugin.so
    my-plugin/              # Plugin source (if developing)
      Cargo.toml
      manifest.toml
      src/lib.rs
      training/examples.json
      lora/my-plugin.lora
  lora/
    postgres.lora           # Plugin LoRA knowledge
    redis.lora
  checkpoints/
    soma-*.ckpt             # Experiential memory snapshots
  data/
    app.db                  # SQLite database (if used)
    search/                 # Search index (if used)
  training/
    domain.json             # App-specific training data
```

---

## 10. Common Patterns

### 10.1 Adding a Feature

```
# You want to add email notifications
soma plugin install smtp
# Edit soma.toml with SMTP config
# Add training data for email-related intents
soma-synthesize train --plugins ./plugins --domain ./training/domain.json --output ./models
# Restart SOMA
soma --repl
# "send an email to alice@example.com with subject Hello"
```

### 10.2 Connecting Frontend and Backend

```
# Terminal 1: Backend SOMA
soma --bind 0.0.0.0:9001

# Terminal 2: Interface SOMA (browser)
soma-interface --backend localhost:9001 --port 8080
# Open http://localhost:8080
# Interface SOMA renders UI from Backend SOMA's semantic signals
```

### 10.3 Deploying

```bash
# Production
soma \
  --config production.toml \
  --checkpoint ./checkpoints/latest.ckpt \
  --bind 0.0.0.0:9001 \
  --log-level info
```

Single binary. One config file. One checkpoint. No Docker required (though works in Docker too).

### 10.4 Monitoring

```bash
# Live signal capture
soma-dump --port 9001

# Metrics (if http-bridge loaded with metrics endpoint)
curl http://localhost:8080/metrics
```
