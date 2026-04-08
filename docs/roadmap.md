# Roadmap

Cross-referenced against the [Whitepaper](../SOMA_Whitepaper.md) Section 17 and the actual codebase as of April 2026.

---

## Current Status

| Deliverable | State | Summary |
|---|---|---|
| **soma-core** | ~14,900 lines Rust, 101 tests passing | Mind engine (tract-onnx), plugin manager (builtin + dynamic), full Synaptic Protocol (16 modules, ~5,600 lines), MCP server (22+ tools), memory system, proprioception, metrics, state management |
| **soma-plugins** | ~4,500 lines Rust across 7 crates | SDK + 6 plugins: crypto (13 conventions), postgres (15), redis (14), auth (10), geo (5), http-bridge (5) |
| **soma-synthesizer** | ~4,500 lines Python across 12 modules | Full pipeline: train, train-lora, export, validate, test, benchmark, export-experience. BiLSTM+GRU model with 11 output heads. BPE + word-level tokenizers |
| **soma-helperbook** | PostgreSQL schema + Express frontend | 19 tables, seed data, Docker Compose (PostgreSQL + Redis), Express server bridging HTTP to SOMA MCP, plain JS frontend with 6 view components |

---

## Milestones

### Milestone 1: SOMA Core -- Foundation

Build the Rust binary that loads a Mind, accepts intents, generates programs, and executes them against plugins. Everything depends on this.

| Step | Description | Status |
|---|---|---|
| 1.1 | Rust project scaffold, module structure | Done |
| 1.2 | Core types: Value (10 variants), Convention, SomaPlugin trait, PluginManager | Done |
| 1.3 | Synthesizer: `.soma-model` export with int8 quantization | Done |
| 1.4 | Mind engine: tract-onnx inference, BiLSTM+GRU, BPE tokenizer, LoRA layers | Done |
| 1.5 | Program execution loop: tokenize, infer, resolve args, execute steps, EMIT | Done |
| 1.6 | Filesystem plugin: PosixPlugin with 22 conventions (libc + high-level fs) | Done |
| 1.7 | Configuration: TOML (8 sections), env var overrides (SOMA_*), CLI flags | Done |

**Status: Complete** &#9745;

---

### Milestone 2: MCP Server -- LLM Drives SOMA

JSON-RPC over HTTP+SSE. Claude (or any LLM) talks to SOMA through MCP tools.

| Step | Description | Status |
|---|---|---|
| 2.1 | MCP server: JSON-RPC 2.0, tool registration, `soma.intent()` | Done |
| 2.2 | State exposure: `get_state`, `get_plugins`, `get_conventions`, `get_health`, `get_recent_activity` | Done |
| 2.3 | Plugin management: `install_plugin`, `uninstall_plugin`, dynamic tool list updates | Done |
| 2.4 | Decision recording: `record_decision`, `get_decisions`, persistent storage | Done |
| -- | Auth: admin/builder/viewer roles from env vars, audit trail | Done |

**Status: Complete** &#9745;

---

### Milestone 3: Data Layer -- PostgreSQL + Redis

Real data storage and schema introspection via MCP.

| Step | Description | Status |
|---|---|---|
| 3.1 | PostgreSQL plugin: 15 conventions (query, execute, ORM-style find/count/aggregate) | Done |
| 3.2 | `soma.get_schema()` MCP tool: tables, columns, types, constraints | Done |
| 3.3 | Redis plugin: 14 conventions (strings, hashes, lists, pub/sub, keys) | Done |
| 3.4 | Re-synthesize Mind with merged training data | Done |

**Status: Complete** &#9745;

---

### Milestone 4: Memory + Adaptation -- SOMA Learns

LoRA in pure Rust. Experience recording. Adaptation without retraining.

| Step | Description | Status |
|---|---|---|
| 4.1 | LoRA layers: forward, merge, magnitude, reset. Attach/detach to Mind layers | Done |
| 4.2 | Experience buffer (ring, successes only), adaptation trigger, gradient update | Done |
| 4.3 | Checkpoint v2: model hash + LoRA + experience + plugins. Auto-checkpoint on shutdown | Done |
| 4.4 | Consolidation: merge LoRA into base weights, configurable trigger | Done |
| -- | Cached LoRA adaptation: gradient descent on frozen decoder hidden states | Done |

**Status: Complete** &#9745;

---

### Milestone 5: Synaptic Protocol -- SOMA-to-SOMA Communication

Binary wire protocol over TCP. SOMAs discover each other and forward intents.

| Step | Description | Status |
|---|---|---|
| 5.1 | Signal types (24) and binary codec: CRC32, zstd compression, ChaCha20-Poly1305 | Done |
| 5.2 | TCP transport: listener, handshake, PING/PONG, heartbeat, RTT, session tokens | Done |
| 5.3 | Signal routing: SignalRouter, DashMap pending requests, 30s timeout | Done |
| 5.4 | Discovery: PeerRegistry, TTL forwarding, PEER_QUERY/PEER_LIST | Done |
| -- | Encryption: ChaCha20-Poly1305, X25519 key exchange, Ed25519 signatures | Done |
| -- | Pub/sub: wildcards, durable subscriptions, catch-up, fan-out | Done |
| -- | Streaming: stream lifecycle, frame counting | Done |
| -- | Chunked transfer: resumable, SHA-256 verification | Done |
| -- | Rate limiting: graduated response, CONTROL signal, PeerBlacklist | Done |
| -- | Offline queue: priority queue with expiry | Done |
| -- | Relay: multi-hop, loop prevention, max_hops | Done |
| -- | WebSocket transport adapter | Done |
| -- | Unix Domain Socket transport | Done |
| -- | Client: auto-reconnect, subscription replay | Done |

**Status: Complete** &#9745;

---

### Milestone 6: HelperBook Core Plugins

Business logic for the first real SOMA application.

| Step | Description | Status |
|---|---|---|
| 6.1 | Crypto plugin: 13 conventions (hash, sign, encrypt, JWT, random) | Done |
| 6.2 | Auth plugin: 10 conventions (OTP, sessions, TOTP) | Done |
| 6.3 | Messaging plugin: chat persistence via PostgreSQL | Done (via postgres plugin + schema) |
| 6.4 | HelperBook data model: 19 tables (users, connections, messages, appointments, reviews, etc.) | Done |
| 6.5 | Additional plugins: geo (5 conventions), http-bridge (5 conventions) | Done |
| -- | Domain training data for Mind synthesis | Done (`helperbook-training.json`) |

**Status: Complete** &#9745;

---

### Milestone 7: Web Frontend

Pragmatic web interface. Not neural rendering (that is future research — see [Web 4](web4.md)).

| Step | Description | Status |
|---|---|---|
| 7.1 | Express server bridging HTTP to SOMA MCP | Done |
| 7.2 | Frontend views: contacts, chat, calendar, profile, provider cards, navigation | Done (6 JS components) |
| 7.3 | WebSocket bridge: Synaptic Protocol over WebSocket for browser connections | Partial (transport exists in soma-core, not yet wired to frontend) |
| 7.4 | Real-time updates: subscribe to channels, live message push | Not started |

**Status: In Progress** &#9744;

The Express server and basic frontend views exist (plain JS + Tailwind CSS). WebSocket transport is implemented in soma-core (`protocol/websocket.rs`, 211 lines) but the frontend currently uses HTTP polling through the Express bridge rather than direct WebSocket connections. Real-time push (two-tab instant messaging) is not yet functional.

---

### Milestone 8: MCP Bridge Plugin

Connect SOMA to external MCP servers. Their tools become SOMA conventions.

| Step | Description | Status |
|---|---|---|
| 8.1 | MCP client: connect to external MCP servers, discover their tools | Not started |
| 8.2 | Convention mapping: external MCP tools become SOMA conventions | Not started |
| 8.3 | Dynamic registration: tools appear/disappear as servers connect/disconnect | Not started |

**Status: Planned** &#9744;

Can be built any time (only depends on Milestone 2). Strategically important: reduces the need for many Milestone 9 plugins by giving SOMA access to the MCP ecosystem (GitHub, Slack, AWS, etc.).

---

### Milestone 9: Remaining HelperBook Plugins

Additional plugins for full HelperBook functionality. Each follows the standard pattern: implement SomaPlugin trait, write training examples, re-synthesize Mind, test via MCP.

| Plugin | MCP Bridge Alternative? | Status |
|---|---|---|
| Calendar | No -- core feature | Not started |
| Twilio (SMS) | Yes -- has MCP server | Not started |
| SMTP (email) | Yes -- Gmail MCP server | Not started |
| Push (APNs/FCM) | No -- platform-specific | Not started |
| Image processing | No -- local processing | Not started |
| S3 (storage) | Yes -- AWS MCP server | Not started |
| Search | No -- custom logic | Not started |
| AI inference | Partial -- LLM API | Not started |
| Reviews | No -- core feature | Not started |
| Analytics | No -- custom aggregation | Not started |
| i18n | No -- string management | Not started |
| Offline cache | No -- client-side | Not started |
| Webhooks | No -- signature verification | Not started |
| ID verification | Partial -- external API | Not started |
| Jobs (background) | No -- infrastructure | Not started |

**Status: Planned** &#9744;

Plugins marked "Yes" under MCP Bridge can potentially be handled by Milestone 8 instead. Priority: Calendar, Reviews, and Search are core features that need native plugins.

---

### Milestone 10: ONNX Engine Optimization

Full ONNX Runtime integration via the `ort` crate for production-grade performance.

| Step | Description | Status |
|---|---|---|
| 10.1 | `OnnxMindEngine` via `ort` crate, same `MindEngine` trait | Not started (tract-onnx sufficient for current scale) |
| 10.2 | GPU acceleration, batch inference | Not started |
| 10.3 | Performance benchmarking vs. EmbeddedMindEngine | Not started |

**Status: Planned** &#9744;

The current tract-onnx backend (pure Rust, zero C++ dependencies) works well for models under 50M parameters. This milestone becomes relevant when convention counts exceed ~50 and inference latency becomes a bottleneck.

---

### Milestone 11: Production Hardening

Everything needed before real users touch the system.

| Item | Spec Reference | Status |
|---|---|---|
| Startup failure handling | [Architecture](architecture.md) | Partial (basic startup exists) |
| Graceful shutdown | [Architecture](architecture.md) | Partial (checkpoint on shutdown) |
| Error retry loop | [Architecture](architecture.md) | Not started |
| Resource limits and backpressure | [Architecture](architecture.md) | Not started |
| MCP auth tokens | [MCP Interface](mcp-interface.md) | Done (admin/builder/viewer roles) |
| Destructive action confirmation | [MCP Interface](mcp-interface.md) | Not started |
| Structured logging with trace IDs | [Architecture](architecture.md) | Partial (tracing crate in use, no trace IDs) |
| Metrics export | [Architecture](architecture.md) | Done (20 Prometheus counters, JSON + text) |
| Connection recovery | [Synaptic Protocol](synaptic-protocol.md) | Done (auto-reconnect in client) |
| Rate limiting | [Synaptic Protocol](synaptic-protocol.md) | Done (graduated response, blacklist) |
| Plugin error cleanup | [Plugin System](plugin-system.md) | Partial (crashed plugin tracking exists) |

**Status: Partially Complete** &#9744;

Several production features (metrics, auth, rate limiting, connection recovery) are already implemented. Remaining work focuses on retry logic, resource limits, destructive action guards, and trace ID propagation.

---

## Deferred -- Research, Not Current Scope

These items appear in the whitepaper (milestones 12-13) and CLAUDE.md but are explicitly outside the current implementation roadmap.

| Item | Description | Prerequisite |
|---|---|---|
| EmbeddedMindEngine | ESP32 target, `no_std`, 50K params, 168KB RAM | Core |
| TransformerMind | Alternative architecture (stub exists in `model.py`, not implemented) | Core |
| WASM sandbox | `wasmtime` for untrusted plugins | Plugins |
| LoRA MoE gating | Mixture-of-Experts across plugin LoRAs, 8-16 experts | Memory |
| soma-replay | Signal replay tool for debugging | Protocol |
| soma-mock | Mock SOMA for integration testing | Protocol |
| Plugin registry | Download/cache plugins from a central registry | Plugins |
| Diffuse memory tier | Peer queries for distributed knowledge | Protocol, Memory |
| Self-hosting SOMA | SOMA that synthesizes other SOMAs (whitepaper milestone 12) | All |
| Neuromorphic hardware | Intel Loihi or equivalent (whitepaper milestone 13) | All |
| Neural rendering | Interface SOMA (see [Web 4](web4.md)) | All |
| Formal verification | Verify SOMA programs for correctness/safety | Core |

---

## Critical Path

```
M1: SOMA Core .............. [DONE]
 |
M2: MCP Server ............. [DONE]    M8: MCP Bridge .... [PLANNED]
 |                                      |  (parallel, needs M2)
M3: Data Layer ............. [DONE]     |
 |                                      |
M4: Memory + Adaptation .... [DONE]     |
 |                                      |
M5: Synaptic Protocol ...... [DONE]     |
 |                                      |
M6: HelperBook Plugins ..... [DONE]     |
 |                                      |
M7: Web Frontend ........... [IN PROGRESS]
 |                                      |
M9: Remaining Plugins ...... [PLANNED] <--- some replaceable by M8
 |
M10: ONNX Optimization .... [PLANNED]
 |
M11: Production Hardening .. [PARTIAL]
 |
M12: Self-Hosting ......... [DEFERRED]
 |
M13: Neuromorphic ......... [DEFERRED]
```

**Current position:** Milestones 1-6 are complete. Milestone 7 (web frontend) is the active workfront. Milestones 8-11 are planned. Milestones 12-13 are long-term research.

---

## Next Actions

1. **Finish Milestone 7** -- Wire WebSocket transport to the frontend for real-time message delivery. Replace HTTP polling with persistent connections.
2. **Evaluate Milestone 8** -- MCP Bridge could eliminate the need for several Milestone 9 plugins (Twilio, SMTP, S3) and accelerate HelperBook completion.
3. **Begin Milestone 11 incrementally** -- Production hardening items like retry loops and resource limits can be added alongside feature work.
