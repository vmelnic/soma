# SOMA

**A neural architecture that IS the program.**

No application source code. A trained neural Mind receives structured intents, generates execution programs, and orchestrates plugins that interface with hardware, databases, networks, and other systems.

```
Human <-> LLM (Claude, ChatGPT, Ollama) <-> MCP <-> SOMA Core <-> Plugins (body)
                                                        |
                                                  Peer SOMAs (Synaptic Protocol)
```

SOMA is a pure executor — small, deterministic, and capable of running on hardware from ESP32 microcontrollers to cloud servers. Conversational intelligence is provided by external LLMs that connect via MCP. The LLM brings understanding. SOMA brings state, memory, and execution.

Read the [Whitepaper](SOMA_Whitepaper.md) for the full vision. Read [Web 4](docs/web4.md) for what this means for the future of software.

## Repository

| Component | What | Language |
|---|---|---|
| [soma-core](soma-core/) | Runtime binary (14MB). The production deliverable. | Rust |
| [soma-plugins](soma-plugins/) | Plugin workspace — SDK + 6 plugins (crypto, postgres, redis, auth, geo, http-bridge) | Rust |
| [soma-synthesizer](soma-synthesizer/) | Build tool. Trains the Mind, exports ONNX models. | Python |
| [soma-helperbook](soma-helperbook/) | First real-world application. Service marketplace. | JS + SQL |

## Quick Start

```bash
# Build the runtime
cd soma-core && cargo build --release && cargo test

# Build plugins
cd ../soma-plugins && cargo build --release

# Run with MCP (connect your LLM)
cd ../soma-core
cargo run --bin soma -- --model ../models --mcp

# Or run HelperBook (the example app)
cd ../soma-helperbook
docker compose up -d --wait
scripts/setup-db.sh && scripts/seed-db.sh
cd frontend && npm install && node server.js
# Open http://localhost:8080
```

See [Getting Started](docs/getting-started.md) for full setup instructions.

## Architecture

Six core components in a single binary:

```
Mind Engine (ONNX inference)  +  Plugin Manager  +  Memory System
+ Synaptic Protocol (SOMA-to-SOMA)  +  MCP Server (LLM-to-SOMA)  +  Proprioception
```

- **Mind Engine** — Maps intents to execution programs (BiLSTM encoder + GRU decoder)
- **Plugin Manager** — Loads and routes conventions to plugins (the body)
- **Memory System** — Four-tier: permanent weights, experiential LoRA, working memory, diffuse (peer) memory
- **Synaptic Protocol** — Binary wire protocol for SOMA-to-SOMA communication
- **MCP Server** — JSON-RPC interface for LLM-to-SOMA interaction
- **Proprioception** — Self-model: what am I, what can I do, what's my health

See [Architecture](docs/architecture.md) for details.

## Documentation

### Vision and Concepts
- [SOMA Whitepaper](SOMA_Whitepaper.md) — The full specification and research paper
- [Web 4: Neural Execution](docs/web4.md) — How SOMA represents the next evolution of the web

### Getting Started
- [Getting Started](docs/getting-started.md) — Build, install, and run SOMA
- [Building Applications](docs/building-apps.md) — Step-by-step guide to build a new app with SOMA

### Core Systems
- [Architecture](docs/architecture.md) — How the 6 components fit together
- [Mind Engine](docs/mind-engine.md) — Neural inference, LoRA adaptation, tokenizer, memory system
- [Synaptic Protocol](docs/synaptic-protocol.md) — Binary wire protocol specification
- [MCP Interface](docs/mcp-interface.md) — LLM integration, all MCP tools, authentication

### Plugins
- [Plugin System](docs/plugin-system.md) — Architecture: trait, conventions, Value type, loading
- [Plugin Catalog](docs/plugin-catalog.md) — All plugins and conventions (reference)
- [Plugin Development](docs/plugin-development.md) — Tutorial: build a plugin from scratch

### Build Tools
- [Synthesizer](docs/synthesizer.md) — Training pipeline (PyTorch to ONNX)

### Applications
- [HelperBook](docs/helperbook.md) — First real-world SOMA application

### Project
- [Roadmap](docs/roadmap.md) — What's done, what's next, what's deferred

## Key Design Decisions

- **No application code** — The Mind generates programs, not source code. No codebases to maintain.
- **LLM is brain, SOMA is body** — SOMA doesn't converse. External LLMs handle conversation via MCP.
- **Permanent state** — `soma.get_state()` gives any LLM full context in one call. Switch LLMs anytime.
- **Single Rust binary** — No Python, Node, or JVM at runtime. Copy and run.
- **Real crypto** — ChaCha20-Poly1305, X25519, Ed25519 via dalek crates.
- **Experience learning** — LoRA adaptation from successful executions. The system improves over time.

## Status

| Milestone | Status |
|---|---|
| SOMA Core (runtime binary) | Done |
| MCP Server (LLM integration) | Done |
| Data Layer (PostgreSQL + Redis) | Done |
| Memory & Adaptation (LoRA) | Done |
| Synaptic Protocol (SOMA-to-SOMA) | Done |
| HelperBook Core (6 plugins + schema) | Done |
| Web Frontend | In Progress |
| MCP Bridge (ecosystem access) | Planned |
| Production Hardening | Partially Done |

See [Roadmap](docs/roadmap.md) for details.

## License

This project is proprietary. All rights reserved.
