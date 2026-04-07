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
curl -L https://github.com/vmelnic/soma/releases/latest/download/soma-linux-x86_64 -o soma
chmod +x soma

# macOS Apple Silicon
curl -L https://github.com/vmelnic/soma/releases/latest/download/soma-macos-aarch64 -o soma
chmod +x soma

# Verify
./soma --version
# SOMA v0.1.0 (server, onnx)
```

### 2.2 From Source (Rust)

```bash
git clone https://github.com/vmelnic/soma.git
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

soma> add bookmark https://soma.local titled "SOMA Project" tagged "ai,neural"

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

---

## 11. Troubleshooting

### 11.1 Common Problems

**"Mind produces STOP immediately (empty program)"**

The Mind doesn't know how to handle the intent. Causes:
- The intent uses words not in the vocabulary (OOV). Check `vocab.json`.
- No training examples cover this type of intent. Add examples and re-synthesize.
- The model is undertrained. Check synthesis metrics — if E2E accuracy is below 80%, train longer.

Fix: add training examples that cover this intent pattern, then re-synthesize.

```
soma> :trace full
soma> list all contacts near me
  [Trace] Tokens: [12, 45, 89, 2, 7, 1]  ← check for [UNK] tokens (indicates OOV)
  [Trace] Encoder output: [...]
  [Trace] Decoder step 0: STOP (confidence 0.78)  ← Mind chose STOP immediately
  
  Problem: "near" or "contacts" may be OOV, or no geo+contacts training examples
```

**"Plugin not found: convention 'postgres.query' not available"**

The Mind generated a program referencing a convention that isn't loaded. Causes:
- Plugin isn't installed. Run `soma plugin list` to check.
- Plugin failed to load (config error, missing dependency). Check startup logs.
- Model was synthesized with the plugin, but plugin isn't configured in `soma.toml`.
- Convention name mismatch: model was trained with `postgres.query` but plugin registers `pg.query`. Convention names must match exactly.

Fix: install/configure the plugin, or re-synthesize without it.

**"LoRA incompatible: dimension mismatch"**

Plugin LoRA weights don't fit the current Mind. Causes:
- Mind was re-synthesized with different hidden/decoder dimensions.
- Plugin LoRA was trained on a different Mind version.

Fix: re-train the plugin LoRA on the current Mind (`soma-synthesize train-lora --plugin <name>`), or remove the LoRA (plugin works without it, just less accurately).

**"Checkpoint restore failed: model hash mismatch"**

The checkpoint was created with a different model. Causes:
- Model was re-synthesized but old checkpoint is still referenced.
- Checkpoint file was copied from a different SOMA instance.

Fix: start without checkpoint (`soma --no-checkpoint`). Experiential memory is lost but the SOMA operates normally with base knowledge.

**"Synaptic connection refused"**

Can't connect to peer SOMA. Causes:
- Peer isn't running. Check if the target SOMA is up.
- Wrong address/port. Verify `soma.toml` peer config.
- Firewall blocking. Synaptic Protocol uses TCP — ensure the port is open.
- Encryption mismatch. Both SOMAs must agree on encryption (both enabled or both disabled).

Fix: verify peer is running at the expected address, check firewalls, check encryption config.

**"Inference timeout (exceeded 5s)"**

The Mind took too long to generate a program. Causes:
- Decoder stuck in a loop (not predicting STOP). Likely a training issue.
- Model is too large for the hardware (slow inference).
- System under heavy load (CPU contention).

Fix: check `max_program_steps` config (should be 8-16). If the Mind consistently uses all steps without STOP, the model needs more STOP examples in training data.

**"Plugin execution timeout"**

A plugin convention took longer than its `max_latency`. Causes:
- Database query too slow (missing index, large table scan).
- External service down (SMTP server, S3, Twilio).
- Plugin bug (infinite loop, deadlock).

Fix: check `:inspect plugin <name>` for stats. Look at p99 latency. For database: run the generated SQL manually to check performance.

### 11.2 Debug Workflow

```
1. Enable tracing:           soma> :trace full
2. Reproduce the problem:    soma> [the failing intent]
3. Check tokens:             are there [UNK] tokens? → vocabulary issue
4. Check encoder output:     is it all zeros? → model didn't load correctly
5. Check decoder steps:      did it predict wrong opcodes? → training data issue
6. Check plugin execution:   did the right plugin receive the right args?
7. Check plugin response:    did the plugin error or timeout?
```

---

## 12. Training Data Best Practices

### 12.1 How Many Examples Per Convention?

**Minimum: 5 unique intent templates per convention.** Each template should express the same operation in a genuinely different way. "list files" / "show files" / "display files" are too similar — they're synonym variations, not structural variations.

Good set for `postgres.query` (SELECT):
```
1. "list all {table}"                          ← imperative, simple
2. "show me everything in {table}"             ← conversational, simple
3. "find {table} where {column} is {value}"    ← with filter
4. "how many {table} do I have"                ← count query
5. "what {table} were created today"           ← temporal filter
6. "get {table} sorted by {column}"            ← with ordering
7. "search {table} for {value}"                ← search pattern
```

Each template then gets expanded by param pools (table names, column names, values), generating 50-200 training pairs per convention.

**Target: 50-200 expanded training pairs per convention.** Under 30 → undertrained, the Mind guesses. Over 500 → diminishing returns, wastes training time.

### 12.2 Param Pool Diversity

Param pools should cover realistic variation:

```json
"params": {
  "table": {
    "pool": ["contacts", "users", "bookmarks", "appointments", "messages"],
    "type": "string"
  },
  "column": {
    "pool": ["name", "email", "created_at", "status", "rating"],
    "type": "string"
  }
}
```

Bad pool (too small): `"pool": ["users"]` — the Mind only learns "users" and can't generalize to "contacts."

Bad pool (unrealistic): `"pool": ["xyzzy", "foo_bar_baz"]` — the Mind learns patterns that won't match real intents.

### 12.3 Common Mistakes

**Mistake: All examples are the same structure with different words.**
```
"list all contacts" → query("SELECT * FROM contacts")
"list all users"    → query("SELECT * FROM users")
"list all messages" → query("SELECT * FROM messages")
```
The Mind only learns the "list all X" pattern. It can't handle "find contacts where name is Ana" because no structural variation was provided.

**Mistake: Inconsistent convention naming.**
```
Example 1: convention = "postgres.query"
Example 2: convention = "pg.query"
Example 3: convention = "database.query"
```
Pick one name and use it everywhere. Convention names must exactly match the plugin manifest.

**Mistake: Forgetting EMIT and STOP.**
Every training program must end with EMIT (to send result) and STOP. If examples omit these, the Mind learns to generate programs that run but never return results.

**Mistake: Overly complex programs in early examples.**
Start simple (1-2 step programs), then add complexity. If all training examples are 5+ steps, the Mind never learns simple patterns and may overgenerate (adding unnecessary steps to simple intents).

**Mistake: No edge cases.**
Include examples for: empty results ("find contacts named Xyzzy" → returns []), errors ("delete a table that doesn't exist"), boundary values ("find contacts within 0 km"), and ambiguous intents ("do the thing" → should the Mind ask for clarification or fail gracefully?).

### 12.4 Cross-Plugin Examples

When the application requires operations spanning multiple plugins, you MUST provide cross-plugin training examples. The Mind does not automatically combine single-plugin programs.

```json
{
  "intents": ["find contacts nearby and cache the results"],
  "program": [
    {"convention": "postgres.query", "args": [{"type": "literal", "value": "SELECT * FROM contacts WHERE ..."}]},
    {"convention": "redis.set", "args": [{"type": "literal", "value": "contacts:nearby"}, {"type": "ref", "step": 0}]},
    {"convention": "EMIT", "args": [{"type": "ref", "step": 0}]},
    {"convention": "STOP"}
  ]
}
```

Without this, the Mind either queries postgres OR caches in redis — never both in one program.

### 12.5 Validation Checklist

Before synthesis, verify:

- [ ] Every plugin has ≥5 unique intent templates per convention
- [ ] Every convention in the manifest has at least one training example
- [ ] All conventions referenced in examples match plugin manifests exactly
- [ ] Every program ends with EMIT + STOP
- [ ] Param pools have ≥5 diverse values each
- [ ] Cross-plugin operations have dedicated examples
- [ ] Edge cases are covered (empty, error, boundary)
- [ ] Run `soma-synthesize validate` — all checks pass
