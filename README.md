# SOMA

**A neural architecture that IS the program.**

> [!IMPORTANT]
> **Start here →** [**SOMA Whitepaper**](SOMA_Whitepaper.md)
>
> The complete technical specification: architecture, control loop, episodic learning pipeline, policy engine, distributed transport, embedded leaf deployment, and the design rationale behind every major decision.

No application source code. A goal-driven runtime receives intent, selects skills, invokes ports (external system adapters), and orchestrates execution through a typed belief-state control loop. Like a brain is to walking — the neural architecture IS the program.

Two execution paths:

- **LLM-driven** — an LLM calls `invoke_port` via MCP to execute operations directly (database queries, email, S3, auth). SOMA is the body, the LLM is the brain. `--pack auto` discovers all available ports from dylib search paths without any manifest.
- **Autonomous** — SOMA executes goals through its own control loop: skill selection, port invocation, observation, belief update, repeat. Learned routines bypass deliberation (plan-following mode).

The same architecture runs on microcontrollers. `soma-project-esp32` deploys a `no_std` leaf firmware to ESP32-S3 and ESP32 LX6 chips with 12 hardware ports, runtime-configurable pins, mDNS auto-discovery, and an SSD1306 OLED display port. A brain-side loop reading the thermistor and drawing temperature on the OLED was verified on the physical panel — the leaf has no concept of "every 5 seconds" or "read sensor, show on screen". Both are the brain's composition. Server SOMA reaches the leaf via `invoke_remote_skill` over TCP after discovering it via `_soma._tcp.local.`.

## Repository

| Component | What | Status |
|---|---|---|
| [soma-next](soma-next/) | Rust runtime binary. MCP server. Cross-compiles to Android, iOS, and browser. | Production |
| [soma-ports](soma-ports/) | Dynamically loaded port adapters + SDK | Production |
| [soma-project-helperbook](soma-project-helperbook/) | Service marketplace — first real app (postgres + redis + auth) | Production |
| [soma-project-smtp](soma-project-smtp/) | SMTP email delivery via SOMA MCP | Production |
| [soma-project-s3](soma-project-s3/) | AWS S3 object storage via SOMA MCP | Production |
| [soma-project-postgres](soma-project-postgres/) | PostgreSQL queries via SOMA MCP | Production |
| [soma-project-llm](soma-project-llm/) | Ollama + SOMA: natural language → SQL via postgres port | Production |
| [soma-project-mcp](soma-project-mcp/) | Claude Code MCP integration — SOMA as MCP server for Claude | Production |
| [soma-project-s2s](soma-project-s2s/) | SOMA-to-SOMA: transport, delegation, schema/routine transfer | Production |
| [soma-project-multistep](soma-project-multistep/) | End-to-end proof of multi-step autonomous routine learning: episodes → schema → routine → plan-following walks a multi-skill sequence and reaches `Completed`. | Proven |
| [soma-project-autonomy](soma-project-autonomy/) | End-to-end proof of autonomy features: `max_steps` override, `create_goal_async` + `get_goal_status` + `cancel_goal`, webhook-triggered async goals with payload templating (real TCP), cron-scheduled goals, and cross-Runtime checkpoint resume. | Proven |
| [soma-project-esp32](soma-project-esp32/) | Embedded `no_std` leaf firmware. Dual-chip proven on real hardware (ESP32-S3 and ESP32 LX6, with and without wifi). Hardware ports, runtime-configurable pins via flash, mDNS auto-discovery, SSD1306 OLED display port sharing I²C with the i2c port. Brain-side thermistor→display loop verified on physical OLED. | Proven on hardware |
| [soma-project-android](soma-project-android/) | Native Android POC (Kotlin + JNI to `libsoma_android.so`). Rust cross-compilation verified. | POC |
| [soma-project-ios](soma-project-ios/) | Native iOS POC (Swift + C FFI to `libsoma_ios.a`). Rust cross-compilation verified. | POC |
| [soma-project-mcp-bridge](soma-project-mcp-bridge/) | `PortBackend::McpClient` proof. Python, Node.js, and PHP each a pure-stdlib MCP server running as a SOMA port. Writing a port in any language is now "write an MCP server in that language." | Proven |
| [soma-project-web](soma-project-web/) | **soma-next in a browser tab.** Wasm core runtime with in-tab `dom` / `audio` / `voice` ports, autonomous goal execution through the real `SessionController`, plan-following dispatch, and an LLM brain over HTTP. | Proven |
| [soma-project-terminal](soma-project-terminal/) | **Multi-user SOMA-native web platform.** Fallout-inspired terminal UI, conversation-first architecture. Operator logs in via magic link, creates named contexts, talks to a tool-calling chat brain that invokes real SOMA ports via `invoke_port` over MCP. | Production |
| [soma-project-body](soma-project-body/) | Full MCP body with all ports loaded. Claude Code integration, mDNS peer discovery, autonomous goals, routine authoring, world state, scheduling. | Production |

Legacy (not active): `soma-core/`, `soma-plugins/`, `soma-synthesizer/`, `poc/`, `pow/`.

## Quick start

```bash
# Build runtime + ports
cd soma-next && cargo build --release && cargo test
cd ../soma-ports && cargo build --workspace --release

# Auto-discover ports (LLM-driven path, no manifest needed)
cd ../soma-next
SOMA_PORTS_PLUGIN_PATH=../soma-ports/target/release cargo run --release -- --mcp --pack auto

# Run autonomous goal
cargo run --release -- --mcp --pack packs/reference/manifest.json

# Run HelperBook
cd ../soma-project-helperbook
docker compose up -d --wait
scripts/setup-db.sh && scripts/seed-db.sh
cd frontend && npm install && node server.js
```

Call `tools/list` against a running MCP server for the authoritative tool catalog. Call `list_ports` for the authoritative port/capability catalog. Run `cargo test --release --lib` from `soma-next/` for the authoritative test status.

## Architecture

```
Runtime Logic — goal parsing, skill selection, belief state, policy, plan-following
Adapter Layer — bridges traits to implementations
Memory Stores — episodes, schemas, routines
Interfaces    — CLI and MCP server
Distributed   — TCP/TLS, WebSocket, Unix socket, peer delegation
Ports         — built-in (filesystem, http) + dynamic (.dylib/.so via soma-port-sdk)
```

## Memory system

Three tiers matching neuroscience:

```
Episodes (hippocampus)   → raw execution traces, bounded ring buffer, embedding-clustered
Schemas (neocortex)      → patterns extracted via PrefixSpan sequence mining
Routines (basal ganglia) → compiled fast-paths, bypass deliberation, plan-following mode
```

Consolidation cycle: episodes accumulate → HashEmbedder clusters by semantic similarity → PrefixSpan extracts frequent skill subsequences → schemas induced → routines compiled → consolidated episodes evicted.

## Documentation

- [Vision](docs/vision.md) — Why SOMA exists, Web 4, the LLM context problem
- [Architecture](docs/architecture.md) — Runtime layers, type system, skill/port/pack contracts
- [MCP Interface](docs/mcp.md) — `invoke_port`, `dump_state`, scheduler, world state, distributed peer tools, async goals
- [What SOMA Is Not](docs/what-soma-is-not.md) — Not a code generator, LLM wrapper, workflow engine, or chatbot
- [Tradeoffs](docs/tradeoffs.md) — Where SOMA wins, where conventional apps win, architectural costs
- [Neuroscience Architecture](docs/neuroscience-architecture.md) — How SOMA maps to biological neural systems
- [Ports](docs/ports.md) — SDK, dynamic loading, port contract
- [Distributed](docs/distributed.md) — Peer transport, delegation, sync
- [Building Projects](docs/building-projects.md) — How to create `soma-project-*`
- [HelperBook](docs/helperbook.md) — Service marketplace application

## License

[BSL 1.1](LICENSE) — Free for non-commercial use. Converts to Apache-2.0 on 2030-04-08.
