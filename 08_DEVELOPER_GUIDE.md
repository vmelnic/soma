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
soma --config soma.toml
```

```
SOMA v0.1.0 | Mind: 823K params | Plugins: filesystem (17 conv)
Synaptic Protocol listening on :9001
MCP Server listening on :3000
Ready.
```

### 3.3 Connect an LLM

Connect Claude, ChatGPT, or any MCP-compatible LLM to `localhost:3000`. In Claude Desktop, add to MCP config:

```json
{
  "mcpServers": {
    "my-soma": {
      "url": "http://localhost:3000/mcp"
    }
  }
}
```

Now talk to Claude:

```
You: "List files in the current directory"

Claude: → soma.intent("list files in the current directory")
Claude: "Here are the files: soma.toml, models, plugins, checkpoints"

You: "Create a file called hello.txt with content Hello from SOMA"

Claude: → soma.intent("create a file called hello.txt with content Hello from SOMA")
Claude: "Done. hello.txt created."

You: "Read hello.txt"

Claude: → soma.intent("read hello.txt")
Claude: "The file contains: Hello from SOMA"
```

Your first SOMA is running. You talk to your LLM. Your LLM drives SOMA. SOMA executes.

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
soma --config soma.toml
```

```
SOMA v0.1.0 | Plugins: filesystem (17), postgres (12) [+LoRA]
MCP Server listening on :3000
Ready.
```

Connect your LLM and talk:

```
You: "Create a table called users with id, name, and email columns"

LLM: → soma.intent("create a table called users with id, name, and email columns")
LLM: "Done. Users table created with id (SERIAL), name (TEXT), email (TEXT)."

You: "Insert a user named Alice with email alice@example.com"

LLM: → soma.intent("insert a user named Alice with email alice@example.com")
LLM: "Inserted. 1 row affected."

You: "List all users"

LLM: → soma.intent("list all users")
LLM: "One user found: {id: 1, name: Alice, email: alice@example.com}"
```

---

## 5. Connect Two SOMAs

### 5.1 Start a Second SOMA

Terminal 1:
```bash
soma --bind 0.0.0.0:9001 --mcp 0.0.0.0:3000
```

Terminal 2:
```bash
soma --bind 0.0.0.0:9002 --mcp 0.0.0.0:3001 --peer localhost:9001
```

### 5.2 Send a Signal

Connect your LLM to SOMA-B (port 3001):

```
You: "Send hello from SOMA-B to the peer SOMA"

LLM: → soma.intent("send hello from SOMA-B to soma on port 9001")
LLM: "Sent."
```

On SOMA-A (port 9001), the message arrives as a Synaptic signal.

### 5.3 Discover Peers

```
You: "Who is connected?"

LLM: → soma.get_peers()
LLM: "One peer: soma-9002 at localhost:9002, 
      plugins: filesystem. Latency: 1ms."
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
                max_latency: Duration::from_secs(5),
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

### 6.4 Write Manifest

Create `manifest.toml`:

```toml
[plugin]
name = "my-plugin"
version = "0.1.0"
description = "Greets people by name"
author = "Your Name"

[plugin.platform]
targets = ["x86_64-linux", "x86_64-macos", "aarch64-linux"]

[plugin.dependencies]
# required = ["crypto"]    # if your plugin needs another plugin
# optional = ["redis"]     # nice to have, works without

# [plugin.config_schema]   # if your plugin needs config in soma.toml
# No config needed for this simple plugin
```

See 03_PLUGINS.md Section 4 for full manifest format and 05_PLUGIN_CATALOG.md for examples.

### 6.5 Build and Re-Synthesize

```bash
cd plugins/my-plugin
cargo build --release
# Produces: target/release/libmy_plugin.so (or .dylib on macOS)
cp target/release/libmy_plugin.so ../../plugins/

# Re-synthesize Mind to include new plugin's training data
cd ../..
soma-synthesize train --plugins ./plugins --output ./models
```

**Important:** Without re-synthesis, the Mind doesn't know your plugin's conventions. It can't generate programs that call `my-plugin.greet`. Re-synthesis is required every time you add, remove, or change a plugin's conventions.

### 6.6 Train Plugin LoRA (optional)

If you want the Mind to use your plugin more effectively without full re-synthesis:

```bash
soma-synthesize train-lora \
  --plugin my-plugin \
  --base-model ./models/server \
  --output ./lora
```

This produces `lora/my-plugin.lora`. Copy it into your plugin's `lora/` directory. The SOMA loads it alongside the plugin — the Mind immediately benefits from pre-trained knowledge of your conventions.

See 07_SYNTHESIZER.md Section 6 for LoRA training details.

### 6.7 Package as .soma-plugin (optional, for distribution)

```bash
soma plugin package ./plugins/my-plugin
# Produces: my-plugin-0.1.0.soma-plugin
```

This bundles: compiled binary, manifest, training data, and LoRA weights into a single archive. Other SOMA users can install it with `soma plugin install my-plugin`.

See 03_PLUGINS.md Section 4 for the .soma-plugin format.

### 6.8 Use It

```bash
soma --config soma.toml
```

Connect your LLM:

```
You: "Say hello to Alice"

LLM: → soma.intent("say hello to Alice")
LLM: "Hello, Alice!"
```

You can also call the convention directly (bypassing Mind inference):

```
LLM: → soma.my-plugin.greet("Alice")
LLM: "Hello, Alice!"
```

---

## 7. Build an Application

### 7.1 The HelperBook Pattern

A real application is:

1. Install relevant plugins (postgres, redis, auth, messaging, etc.)
2. Configure them in `soma.toml`
3. Synthesize a Mind that knows all their conventions
4. Start the SOMA
5. Build your application through an LLM via MCP (create schema, configure auth, set up messaging)
6. Connect a web frontend (simple JS renderer via WebSocket)
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
soma --config soma.toml
# Connect your LLM via MCP
```

```
You: "Create a table called bookmarks with id, url, title, tags, and created_at"

You: "Add bookmark https://soma-project.dev titled SOMA Project tagged ai,neural"

You: "Find all bookmarks tagged ai"

You: "Export all bookmarks as a csv file to bookmarks.csv"
```

Four requests to your LLM. A working bookmark app with database and file export. No code.

---

## 8. MCP Debugging Reference

### 8.1 State Query Tools

All debugging happens through MCP. Ask your LLM, or call tools directly:

| MCP Tool | What It Shows |
|---|---|
| `soma.get_state()` | Full snapshot: plugins, schema, experience, health |
| `soma.get_health()` | Memory, CPU, error rate, uptime, connections |
| `soma.get_schema()` | All database tables with columns and row counts |
| `soma.get_plugins()` | Loaded plugins, conventions, health status |
| `soma.get_conventions()` | All callable conventions with arg specs |
| `soma.get_experience()` | LoRA magnitude, adaptation count, recent changes |
| `soma.get_decisions()` | Decision log: what was built and why |
| `soma.get_recent_activity(n)` | Last N executions with results |
| `soma.get_peers()` | Connected SOMAs and their capabilities |
| `soma.get_config()` | Current configuration (secrets redacted) |

### 8.2 Action Tools

| MCP Tool | What It Does |
|---|---|
| `soma.intent(text)` | Send structured intent to Mind |
| `soma.install_plugin(name)` | Install plugin from registry |
| `soma.uninstall_plugin(name)` | Remove plugin |
| `soma.checkpoint(label?)` | Save current state |
| `soma.restore(checkpoint_id)` | Restore from checkpoint |
| `soma.record_decision(what, why)` | Record a design decision |
| `soma.confirm(action_id)` | Confirm destructive action |

### 8.3 Signal Capture (CLI)

```bash
soma-dump --port 9001                          # Live traffic capture
soma-dump --port 9001 --type INTENT,RESULT     # Filter by signal type
soma-dump --port 9001 --output capture.synaptic # Save for replay
soma-replay --input capture.synaptic --target localhost:9001 # Replay
```

---|---|
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
soma --config soma.toml
# Connect your LLM, then: "Send an email to alice@example.com with subject Hello"
```

### 10.2 Connecting Frontend and Backend

```bash
# Terminal 1: Backend SOMA
soma --bind 0.0.0.0:9001 --mcp 0.0.0.0:3000

# Terminal 2: Interface SOMA (browser renderer)
soma-interface --backend localhost:9001 --port 8080
# Open http://localhost:8080 — Interface SOMA renders UI

# Your LLM connects to the Backend SOMA MCP at localhost:3000
# LLM sends view specs → Backend sends semantic signals → Interface renders
```

### 10.3 Deploying

```bash
soma \
  --config production.toml \
  --checkpoint ./checkpoints/latest.ckpt \
  --bind 0.0.0.0:9001 \
  --mcp 0.0.0.0:3000 \
  --log-level info
```

Single binary. One config file. One checkpoint. MCP for LLM access. No Docker required (though works in Docker too).

### 10.4 Monitoring

```bash
# Live signal capture
soma-dump --port 9001

# Ask your LLM
"How is the SOMA doing?"
→ LLM calls soma.get_health() via MCP
→ "Healthy. 234MB RAM, 12% CPU, 3 peers connected, 0.2% error rate."

# Admin dashboard (if http-bridge plugin loaded with --admin)
open http://localhost:8080/admin
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

Check SOMA logs (with `log_level = "trace"` in soma.toml) for details:

```
[TRACE] Intent: "list all contacts near me"
[TRACE] Tokens: [12, 45, 89, 2, 7, 1]  ← check for [UNK] tokens (indicates OOV)
[TRACE] Encoder output: [...]
[TRACE] Decoder step 0: STOP (confidence 0.78)  ← Mind chose STOP immediately

Problem: "near" or "contacts" may be OOV, or no geo+contacts training examples
```

Or ask your LLM: "Show me recent activity" → LLM calls `soma.get_recent_activity(5)` to see failed intents.

**"Plugin not found: convention 'postgres.query' not available"**

The Mind generated a program referencing a convention that isn't loaded. Causes:
- Plugin isn't installed. Ask your LLM: "What plugins are loaded?" → `soma.get_plugins()`.
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
1. Set log_level = "trace" in soma.toml (or --log-level trace)
2. Reproduce: send the failing intent via LLM → soma.intent("...")
3. Check logs: are there [UNK] tokens? → vocabulary issue
4. Check logs: encoder output all zeros? → model didn't load correctly
5. Check logs: wrong opcodes predicted? → training data issue
6. Ask LLM: "What happened with the last intent?"
   → LLM calls soma.get_recent_activity(1) → sees error details
7. Ask LLM: "Is the postgres plugin healthy?"
   → LLM calls soma.get_plugins() → sees plugin status and error counts
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