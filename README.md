# SOMA

**A neural architecture that IS the program.**

> [!IMPORTANT]
> **Start here →** [**SOMA Whitepaper (v1.0)**](SOMA_Whitepaper.md)
>
> The complete technical specification: architecture, 16-step control loop, episodic learning pipeline, policy engine, distributed transport, embedded leaf deployment, historical validation, and the design rationale behind every major decision. 773 lines, one read.

No application source code. A goal-driven runtime receives intents, selects skills, invokes ports (external system adapters), and orchestrates execution through a typed belief-state control loop. Like a brain is to walking — the neural architecture IS the program.

SOMA operates in two modes:
- **LLM-driven** — An LLM calls `invoke_port` via MCP to directly execute operations (database queries, email, S3, auth). SOMA is the body, the LLM is the brain. `--pack auto` discovers all available ports from dylib search paths without any manifest.
- **Autonomous** — SOMA executes goals through its own control loop: skill selection, port invocation, observation, belief update, repeat. Learned routines bypass deliberation (plan-following mode).

Both modes are production-ready. The LLM-driven path is proven with HelperBook (service marketplace, 3 ports, 32 capabilities). The autonomous path is proven with the reference pack (filesystem skills, episode → schema → routine cycle).

The same architecture runs on microcontrollers. `soma-project-esp32` deploys a `no_std` leaf firmware to ESP32-S3 and ESP32 LX6 chips with 12 hardware ports, runtime-configurable pins, mDNS auto-discovery, and an SSD1306 OLED display port. A brain-side loop reading the thermistor and drawing temperature on the OLED every 5 seconds was verified on the physical panel — the leaf has no concept of "every 5 seconds" or "read sensor, show on screen". Both are the brain's composition. Server SOMA reaches the leaf via `invoke_remote_skill` over TCP after discovering it via `_soma._tcp.local.`.

## Repository

| Component | What | Status |
|---|---|---|
| [soma-next](soma-next/) | Rust runtime binary. 1222 tests, zero warnings. 27 MCP tools. Cross-compiles to Android (aarch64-linux-android), iOS (aarch64-apple-ios), and browser (wasm32-unknown-unknown). | Production |
| [soma-ports](soma-ports/) | 11 dynamically loaded port adapters + SDK | Production |
| [soma-helperbook](soma-helperbook/) | Service marketplace — first real app (postgres + redis + auth) | Production |
| [soma-project-smtp](soma-project-smtp/) | SMTP email delivery via SOMA MCP | Production |
| [soma-project-s3](soma-project-s3/) | AWS S3 object storage via SOMA MCP | Production |
| [soma-project-postgres](soma-project-postgres/) | PostgreSQL queries via SOMA MCP | Production |
| [soma-project-llm](soma-project-llm/) | Ollama LLM + SOMA: natural language → SQL via postgres port | Production |
| [soma-project-mcp](soma-project-mcp/) | Claude Code MCP integration — SOMA as MCP server for Claude | Production |
| [soma-project-s2s](soma-project-s2s/) | SOMA-to-SOMA: transport, delegation, schema/routine transfer (42 tests) | Production |
| [soma-project-multistep](soma-project-multistep/) | End-to-end proof of multi-step autonomous routine learning: episodes → schema → routine → plan-following walks 3 skills against `/tmp` and reaches `Completed`. Five phases, all passing. | Proven |
| [soma-project-esp32](soma-project-esp32/) | Embedded `no_std` leaf firmware. Dual-chip proven on real hardware (ESP32-S3 Sunton 1732S019 and ESP32 LX6 WROOM-32D, both with and without wifi). 12 hardware ports, runtime-configurable pins via flash, mDNS auto-discovery, SSD1306 OLED display port sharing I²C0 with the i2c port via `embedded-hal-bus`. Brain-side thermistor→display loop verified on physical OLED. | Proven on hardware |
| [soma-project-android](soma-project-android/) | Native Android app POC (Kotlin + JNI to `libsoma_android.so`). Rust cross-compilation to `aarch64-linux-android` verified. | POC |
| [soma-project-ios](soma-project-ios/) | Native iOS app POC (Swift + C FFI to `libsoma_ios.a`). Rust cross-compilation to `aarch64-apple-ios` verified. | POC |
| [soma-project-mcp-bridge](soma-project-mcp-bridge/) | `PortBackend::McpClient` proof. Three ports — Python, Node.js, PHP — each a pure-stdlib MCP server running as a SOMA port. Writing a port in any language is now "write an MCP server in that language". | Proven |
| [soma-project-web](soma-project-web/) | **soma-next runs in a browser tab.** ~1.3 MB wasm core runtime with in-tab `dom` / `audio` / `voice` ports, autonomous goal execution through the real `SessionController`, plan-following dispatch, and an LLM brain over HTTP (OpenAI `gpt-5-mini` via `scripts/brain-proxy.mjs`). 18 Playwright tests verifying every phase 1a-1g. | Proven |
| [soma-project-terminal](soma-project-terminal/) | **Multi-user SOMA-native web platform.** Fallout-inspired terminal UI, conversation-first architecture. Operator logs in via magic link, creates named contexts, talks to a tool-calling chat brain (`gpt-4o-mini` or `gpt-5-mini` — wrapper auto-detects family) that invokes real SOMA ports (crypto / postgres / smtp) via `invoke_port` over MCP. Uses `--pack auto` — no manifest, ports auto-discovered from dylib search path. Zero per-context pack generation, zero client-side framework, zero LLM-produced artifacts. Voice input via Whisper. 34 Playwright tests. | Production |

Legacy (not active): soma-core/, soma-plugins/, soma-synthesizer/, poc/, pow/

## Quick Start

```bash
# Build runtime + ports
cd soma-next && cargo build --release && cargo test
cd ../soma-ports && cargo build --workspace --release

# Auto-discover ports (LLM-driven path, no manifest needed)
cd ../soma-next
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_ports","arguments":{}}}' \
| SOMA_PORTS_PLUGIN_PATH=../soma-ports/target/release cargo run --release -- --mcp --pack auto

# Run autonomous goal (autonomous path)
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"create_goal","arguments":{"objective":"list files in /tmp"}}}' \
| cargo run --release -- --mcp --pack packs/reference/manifest.json

# Run HelperBook (LLM-driven path with manifest)
cd ../soma-helperbook
docker compose up -d --wait
scripts/setup-db.sh && scripts/seed-db.sh
cd frontend && npm install && node server.js  # http://localhost:8080
```

## Architecture

```
Runtime Logic — goal parsing, skill selection, belief state, policy, plan-following
Adapter Layer — bridges traits to implementations (embedder, predictor, critic, executor)
Memory Stores — episodes (ring buffer), schemas (PrefixSpan-induced), routines (compiled)
Interfaces — CLI (11 commands), MCP server (24 tools)
Distributed — TCP/TLS, WebSocket, Unix socket, peer delegation
Ports — built-in (filesystem, http) + dynamic (.dylib/.so via soma-port-sdk)
```

## Memory System (Brain-Like)

Three tiers matching neuroscience:

```
Episodes (hippocampus)  → raw execution traces, ring buffer (1024), embedding-clustered
Schemas (neocortex)     → generalized patterns extracted via PrefixSpan sequence mining
Routines (basal ganglia)→ compiled fast-paths, bypass deliberation, plan-following mode
```

Consolidation cycle: episodes accumulate → HashEmbedder clusters by semantic similarity → PrefixSpan extracts frequent skill subsequences → schemas induced → routines compiled → consolidated episodes evicted.

## Documentation

- [Vision](docs/vision.md) — Why SOMA exists, Web 4, the LLM context problem
- [Architecture](docs/architecture.md) — 6-layer runtime, type system, skill/port/pack contracts
- [MCP Interface](docs/mcp.md) — 24 tools, invoke_port, dump_state, scheduler, distributed peer tools, JSON-RPC
- [Neuroscience Architecture](docs/neuroscience-architecture.md) — How SOMA maps to biological neural systems
- [Ports](docs/ports.md) — SDK, dynamic loading, all 12 ports (88 capabilities)
- [Distributed](docs/distributed.md) — Peer transport, delegation, sync
- [Building Projects](docs/building-projects.md) — How to create soma-project-*
- [HelperBook](docs/helperbook.md) — Service marketplace application

## License

[BSL 1.1](LICENSE) — Free for non-commercial use. Converts to Apache-2.0 on 2030-04-08.
