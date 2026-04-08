# Getting Started

## What is SOMA

SOMA (from Greek "body") is a computational paradigm where a neural architecture IS the program. A trained neural Mind receives structured intents, generates execution programs, and orchestrates plugins that interface with hardware, databases, networks, and other systems. No application source code is written, compiled, or interpreted.

The human talks to any LLM (Claude, ChatGPT, Ollama). The LLM drives SOMA via MCP. SOMA executes deterministically.

See [SOMA_Whitepaper.md](../SOMA_Whitepaper.md) for the full vision and [docs/web4.md](web4.md) for the Web 4 framing.

## Prerequisites

| Requirement | Version | Needed for |
|-------------|---------|------------|
| **Rust** | Stable toolchain | soma-core, soma-plugins |
| **Python** | 3.10+ | soma-synthesizer only (build tool, not runtime) |
| **PyTorch** | 2.0+ | soma-synthesizer only |
| **Docker** | Any recent | Database plugins (PostgreSQL, Redis) |
| **Node.js** | Any recent | HelperBook frontend only |
| **MCP-compatible LLM** | - | Claude, ChatGPT, Ollama, Cursor, etc. |

Only Rust is required for the core runtime. Everything else is optional depending on what you need.

## Build SOMA Core

```bash
cd soma-core
cargo build --release
```

Produces two binaries in `target/release/`:
- `soma` (~14MB) -- the SOMA runtime
- `soma-dump` (~896KB) -- Synaptic Protocol signal capture tool

Run the test suite:

```bash
cargo test
```

This runs 101+ tests across all modules.

## Build Plugins

```bash
cd soma-plugins
cargo build --release
```

Produces `.dylib` (macOS) or `.so` (Linux) files in `target/release/`.

Available plugins:

| Plugin | Conventions | Description |
|--------|-------------|-------------|
| crypto | 13 | Hash, sign, encrypt, JWT, random generation |
| postgres | 15 | Query, execute, ORM-style find/count/aggregate |
| redis | 14 | Strings, hashes, lists, pub/sub, keys |
| auth | 10 | OTP verification, session management, TOTP |
| geo | 5 | Distance, radius filter, geocoding |
| http-bridge | 5 | HTTP client (GET/POST/PUT/DELETE) |

Each plugin also includes `training/examples.json` for Mind synthesis.

## Install Synthesizer

The Synthesizer is a Python build tool -- the SOMA equivalent of a compiler. It trains the Mind and exports ONNX models. It is NOT needed at runtime.

```bash
cd soma-synthesizer
pip install -e .
```

Verify the installation:

```bash
soma-synthesize --help
```

## Run SOMA

### Interactive REPL (development)

```bash
cd soma-core
cargo run --bin soma -- --model ../models
```

This drops you into an interactive REPL:

```
intent> list files in /tmp

  [Mind] Program (4 steps, 100%):
    $0 = libc.opendir("/tmp")
    $1 = libc.readdir($0)
    $2 = libc.closedir($0)
    $3 = EMIT($1)
    STOP
```

REPL commands: `:status` `:inspect` `:checkpoint` `:consolidate` `:decisions` `:metrics`

### Single intent (scripting)

```bash
cd soma-core
cargo run --bin soma -- --model ../models --intent "list files in /tmp"
```

Runs one intent and exits. Useful for scripts and testing.

### MCP Server mode (production)

```bash
cd soma-core
cargo run --bin soma -- --model ../models --mcp
```

Runs JSON-RPC 2.0 on stdio. Connect any MCP-compatible LLM and it can drive SOMA.

Available MCP tools include:
- **State:** `soma.get_state`, `soma.get_plugins`, `soma.get_conventions`, `soma.get_health`, `soma.get_metrics`
- **Actions:** `soma.intent`, `soma.checkpoint`, `soma.record_decision`, `soma.install_plugin`, `soma.shutdown`
- **Plugins:** Every loaded convention exposed as `soma.{plugin}.{convention}`

### Signal capture tool

```bash
cd soma-core
cargo run --bin soma-dump -- 127.0.0.1:9999
```

Captures and displays Synaptic Protocol signals on the wire. Supports filtering:

```bash
cargo run --bin soma-dump -- 127.0.0.1:9999 --signal-type intent --channel 5
cargo run --bin soma-dump -- 127.0.0.1:9999 --count 100 --raw
```

### Connect an LLM

In Claude Desktop, add to your MCP configuration:

```json
{
  "mcpServers": {
    "soma": {
      "url": "http://localhost:3000/mcp"
    }
  }
}
```

Then talk to Claude naturally. Claude translates your intent into SOMA tool calls. SOMA executes. Claude explains the result.

### CLI reference

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

## Train a Mind (Synthesizer)

Validate training data before training:

```bash
soma-synthesize validate --plugins ../soma-plugins
```

Train a Mind from plugin training data:

```bash
soma-synthesize train --plugins ../soma-plugins --output ../models
```

Train with domain-specific data:

```bash
soma-synthesize train \
  --plugins ../soma-plugins \
  --domain ./domain/training.json \
  --config synthesis_config.toml \
  --output ../models
```

Train LoRA for a specific plugin:

```bash
soma-synthesize train-lora \
  --plugin postgres \
  --base-model ../models \
  --output ../models/lora
```

The synthesizer produces:
- `encoder.onnx` + `decoder.onnx` -- the Mind model
- `tokenizer.json` -- vocabulary
- `catalog.json` -- convention catalog
- `meta.json` -- metadata with SHA-256 model hash

## Run HelperBook (Example App)

HelperBook is the first real-world SOMA application -- a service marketplace where clients find and book service providers.

```bash
cd soma-helperbook

# Start PostgreSQL + Redis
docker compose up -d --wait

# Apply schema (19 tables)
scripts/setup-db.sh

# Seed test data
scripts/seed-db.sh

# Build plugins (if not already done)
cd ../soma-plugins && cargo build --release && cd ../soma-helperbook

# Start frontend
cd frontend && npm install && node server.js
```

Open [http://localhost:8080](http://localhost:8080).

## Configuration

All configuration lives in `soma.toml`. See [soma.toml.example](../soma-core/soma.toml.example) for all available fields with documentation.

Override order (later wins):

```
defaults < soma.toml < environment variables (SOMA_*) < CLI flags
```

Key environment variables:

| Variable | Purpose |
|----------|---------|
| `SOMA_MCP_ADMIN_TOKEN` | MCP admin auth token |
| `SOMA_MCP_BUILDER_TOKEN` | MCP builder auth token |
| `SOMA_MCP_VIEWER_TOKEN` | MCP viewer auth token |
| `SOMA_LOG_JSON=1` | JSON-structured log output |
| `SOMA_MIND_TEMPERATURE` | Softmax temperature override |
| `SOMA_PROTOCOL_BIND` | Synaptic Protocol bind address override |

## What's Next

- [docs/architecture.md](architecture.md) -- understand how SOMA works internally
- [docs/mcp-interface.md](mcp-interface.md) -- connect your LLM and use all MCP tools
- [docs/mind-engine.md](mind-engine.md) -- how the neural Mind generates programs
- [docs/plugin-development.md](plugin-development.md) -- build a plugin
- [docs/web4.md](web4.md) -- SOMA as Web 4 infrastructure
- [08_DEVELOPER_GUIDE.md](../08_DEVELOPER_GUIDE.md) -- full developer guide for building with SOMA
