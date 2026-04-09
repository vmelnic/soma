# SOMA

**A neural architecture that IS the program.**

No application source code. A goal-driven runtime receives intents, selects skills, invokes ports (external system adapters), and orchestrates execution through a typed belief-state control loop. Like a brain is to walking — the neural architecture IS the program.

SOMA operates in two modes:
- **LLM-driven** — An LLM calls `invoke_port` via MCP to directly execute operations (database queries, email, S3, auth). SOMA is the body, the LLM is the brain.
- **Autonomous** — SOMA executes goals through its own control loop: skill selection, port invocation, observation, belief update, repeat. Learned routines bypass deliberation (plan-following mode).

Both modes are production-ready. The LLM-driven path is proven with HelperBook (service marketplace, 3 ports, 32 capabilities). The autonomous path is proven with the reference pack (filesystem skills, episode → schema → routine cycle).

## Repository

| Component | What | Status |
|---|---|---|
| [soma-next](soma-next/) | Rust runtime binary. 1177 tests, zero warnings. | Production |
| [soma-ports](soma-ports/) | 11 dynamically loaded port adapters + SDK | Production |
| [soma-helperbook](soma-helperbook/) | Service marketplace — first real app (postgres + redis + auth) | Production |
| [soma-project-smtp](soma-project-smtp/) | SMTP email delivery via SOMA MCP | Production |
| [soma-project-s3](soma-project-s3/) | AWS S3 object storage via SOMA MCP | Production |
| [soma-project-postgres](soma-project-postgres/) | PostgreSQL queries via SOMA MCP | Production |

Legacy (not active): soma-core/, soma-plugins/, soma-synthesizer/, poc/, pow/

## Quick Start

```bash
# Build runtime + ports
cd soma-next && cargo build --release && cargo test
cd ../soma-ports && cargo build --workspace --release

# Run HelperBook (LLM-driven path)
cd ../soma-helperbook
docker compose up -d --wait
scripts/setup-db.sh && scripts/seed-db.sh
cd frontend && npm install && node server.js  # http://localhost:8080

# Run autonomous goal (autonomous path)
cd ../soma-next
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"create_goal","arguments":{"objective":"list files in /tmp"}}}' \
| cargo run --release -- --mcp --pack packs/reference/manifest.json
```

## Architecture

```
Runtime Logic — goal parsing, skill selection, belief state, policy, plan-following
Adapter Layer — bridges traits to implementations (embedder, predictor, critic, executor)
Memory Stores — episodes (ring buffer), schemas (PrefixSpan-induced), routines (compiled)
Interfaces — CLI (11 commands), MCP server (16 tools)
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
- [MCP Interface](docs/mcp.md) — 16 tools, invoke_port, dump_state, JSON-RPC
- [Ports](docs/ports.md) — SDK, dynamic loading, all 12 ports (88 capabilities)
- [Distributed](docs/distributed.md) — Peer transport, delegation, sync
- [Building Projects](docs/building-projects.md) — How to create soma-project-*
- [HelperBook](docs/helperbook.md) — Service marketplace application

## License

[BSL 1.1](LICENSE) — Free for non-commercial use. Converts to Apache-2.0 on 2030-04-08.
