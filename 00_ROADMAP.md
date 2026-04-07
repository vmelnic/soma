# SOMA Implementation Roadmap

**Purpose:** Clear, dependency-ordered path from nothing to HelperBook. No timelines. No team estimates. Just: what to build, in what order, and why.

---

## How to Read This

Each step depends on the steps above it within a milestone. Milestones depend on previous milestones unless noted otherwise. Each step produces something testable — you verify it works before moving to the next.

**Recurring pattern:** Every new plugin requires: (1) implement SomaPlugin trait in Rust, (2) write training examples (training/examples.json), (3) re-synthesize Mind with new training data, (4) test via MCP. This pattern repeats and is not restated for every plugin.

---

## Milestone 1: SOMA Core in Rust (Foundation)

Everything depends on this. Nothing works without it. No networking — all local execution.

### Step 1.1: Rust project scaffold

Create the Cargo workspace with the module structure from 01_CORE Section 8.2.

**What you build:**
```
soma/
  Cargo.toml
  src/
    main.rs
    mind/mod.rs          # MindEngine trait, EmbeddedMindEngine
    mind/tensor.rs       # Minimal tensor ops
    mind/tokenizer.rs    # BPE tokenizer, vocab loading
    mind/lora.rs         # LoRA layer (placeholder now)
    plugin/mod.rs        # SomaPlugin trait, PluginManager
    plugin/interface.rs  # Value, CallingConvention, ArgSpec types
    memory/mod.rs        # placeholder
    protocol/mod.rs      # placeholder
    mcp/mod.rs           # placeholder
    config.rs            # TOML config loading
```

**Verify:** `cargo build` succeeds.

### Step 1.2: Core types and plugin manager

**What you build:**
- `Value` enum: Null, Bool, Int, Float, String, Bytes, List, Map, Handle
- `CallingConvention` struct: name, args spec, return type, estimated latency
- `SomaPlugin` trait: name, version, conventions, execute, on_load, on_unload
- `PluginManager`: register plugins, build convention name→ID catalog, route by name
- Built-in "echo" test plugin

**Verify:** `plugin_manager.execute("echo.echo", vec![Value::String("hello")])` → `Value::String("hello")`

**Spec reference:** 03_PLUGINS.md Sections 2-3, 01_CORE.md Section 5

### Step 1.3: Synthesizer — .soma-model export

Update the Python POW Synthesizer to export `.soma-model` binary format.

**What you build (Python):**
- `export_soma_model()`: serialize vocab, architecture config, quantized weights to binary
- Int8 quantization with calibration
- Tokenizer export (vocab.json)

**Verify:** Train model on POW1 data. Export as `.soma-model`. File <1MB. Header shows `SOMA` magic.

**Spec reference:** 01_CORE.md Section 4.4, 07_SYNTHESIZER.md Section 7.2

### Step 1.4: Mind Engine — embedded backend

Custom inference in pure Rust. Zero dependencies.

**What you build:**
- `MindEngine` trait: `load()`, `infer()`, `info()`
- `EmbeddedMindEngine`: load `.soma-model`, load vocab.json
- Tensor ops: matmul, sigmoid, tanh, softmax, argmax, embedding lookup, mean pooling, attention
- BiLSTM encoder + GRU decoder forward passes
- Autoregressive decoder loop until STOP or max_steps

**Verify:** Load `.soma-model` from Step 1.3. Inference on "list files in /tmp". Output matches Python POW.

**Spec reference:** 01_CORE.md Sections 4.2, 4.4, 4.8

### Step 1.5: Program execution loop

Connect Mind → Plugin Manager. Complete pipeline.

**What you build:**
- Tokenize → infer → resolve arguments (literal, span, ref) → execute steps → EMIT → return
- Error handling: failed step returns error with context

**Verify:** "echo hello" → Mind → program → echo plugin → "hello". Span test: "echo goodbye world" extracts "goodbye world".

**Spec reference:** 01_CORE.md Sections 4.8, 5.5, 13

### Step 1.6: Filesystem plugin

First real plugin with actual system calls.

**What you build:**
- `FilesystemPlugin`: read_file, write_file, list_dir, exists, delete, file_info, create_dir
- Async via tokio::fs

**Verify:** "list files in /tmp" → real listing. "create test.txt with hello" → file on disk. "read test.txt" → "hello".

**Spec reference:** 05_PLUGIN_CATALOG.md Section 4

### Step 1.7: Configuration system

**What you build:**
- Parse `soma.toml`: soma ID, log level, mind config, plugin configs
- Env var resolution for secrets
- Config precedence: defaults → file → env → CLI

**Verify:** Plugin receives its config in on_load().

**Spec reference:** 01_CORE.md Section 15

---

**Milestone 1 checkpoint:** Rust binary loads Mind, accepts text intents, generates programs, executes against filesystem plugin. Rust port of POW1. No networking.

---

## Milestone 2: MCP Server (LLM Drives SOMA)

Most important milestone after Core. MCP is JSON-RPC over HTTP — independent of Synaptic Protocol. You can talk to Claude before SOMAs talk to each other.

### Step 2.1: MCP server

**What you build:**
- MCP server (JSON-RPC over HTTP+SSE) in SOMA binary
- Tool registration: plugin conventions → MCP tools
- `soma.intent(text)` tool
- Bind to `--mcp` address (default :3000)

**Verify:** Start SOMA. Connect Claude Desktop to localhost:3000. Claude sees tools. "List files in /tmp" → Claude calls `soma.intent()` → file listing returned.

**Spec reference:** 09_CONVERSATIONAL_INTERACTION.md Sections 3, 6

### Step 2.2: State exposure

**What you build:**
- `soma.get_state()` — full snapshot
- `soma.get_plugins()`, `soma.get_conventions()`, `soma.get_health()`, `soma.get_recent_activity(n)`

**Verify:** New Claude conversation. `soma.get_state()` → Claude knows everything without prior context.

**Spec reference:** 09_CONVERSATIONAL_INTERACTION.md Section 5

### Step 2.3: Plugin management via MCP

**What you build:**
- `soma.install_plugin(name)` — runtime load
- `soma.uninstall_plugin(name)` — runtime unload
- MCP tool list updates dynamically

**Verify:** Start with filesystem only. Claude installs echo plugin via MCP. New tools appear.

### Step 2.4: Decision recording

**What you build:**
- `soma.record_decision(what, why, context?)` — store with timestamp
- `soma.get_decisions(n?)` — retrieve log
- Storage: local JSON file

**Verify:** Record decision → new conversation → `soma.get_decisions()` → decision visible.

**Spec reference:** 09_CONVERSATIONAL_INTERACTION.md Section 6.4

---

**Milestone 2 checkpoint:** Talk to Claude. Claude drives SOMA. Knows state, capabilities, history. Install plugins, execute intents, record decisions — all through conversation.

---

## Milestone 3: Data Layer (PostgreSQL + Redis)

### Step 3.1: PostgreSQL plugin

**What you build:**
- Conventions: query, execute, query_one, begin/commit/rollback, create_table, alter_table, table_exists, list_tables, table_schema
- Connection pool (`deadpool-postgres`)
- Training data: 5+ intent templates per convention

**Verify:** Through Claude: create table → insert → query → correct results.

**Spec reference:** 05_PLUGIN_CATALOG.md Section 2

### Step 3.2: `soma.get_schema()` MCP tool

**What you build:**
- Query `information_schema` for tables, columns, types, constraints, row counts

**Verify:** Through Claude: "what tables exist?" → full schema returned. New conversation → same schema, zero context needed.

### Step 3.3: Redis plugin

**What you build:**
- Conventions: get, set, delete, exists, incr, expire, hget, hset, hgetall, publish, subscribe
- Training data. Connection via `redis-rs`

**Verify:** Through Claude: set with TTL → get → value returned.

**Spec reference:** 05_PLUGIN_CATALOG.md Section 3

### Step 3.4: Re-synthesize Mind

**What you build (Python):**
- Merge training data: filesystem + postgres + redis
- Re-train. Export `.soma-model`

**Verify:** "create users table, insert user, cache count in redis" → multi-plugin program works.

**Spec reference:** 07_SYNTHESIZER.md Sections 2-5

---

**Milestone 3 checkpoint:** Real data layer. Schema queryable via MCP. Any LLM understands full data model in one call.

---

## Milestone 4: Memory + Adaptation (SOMA Learns)

### Step 4.1: LoRA in Rust

**What you build:**
- `LoRALayer`: forward, merge, magnitude
- Attach/detach to Mind layers
- Load `.lora` files

**Verify:** With/without postgres LoRA → measurable confidence difference.

**Spec reference:** 01_CORE.md Section 4.7

### Step 4.2: Experience recording and adaptation

**What you build:**
- Experience buffer, adaptation trigger, LoRA gradient update
- `soma.get_experience()` MCP tool

**Verify:** 20 intents → adapt → re-run → confidence increase.

**Spec reference:** 01_CORE.md Section 17

### Step 4.3: Checkpoint and restore

**What you build:**
- Checkpoint: model hash + LoRA + experience + plugins + decisions
- `soma.checkpoint()`, `soma.restore()`, `soma.get_checkpoints()` MCP tools
- Auto-checkpoint on shutdown

**Verify:** Adapt → checkpoint → kill → restart → same state.

**Spec reference:** 01_CORE.md Sections 6, 16

### Step 4.4: Consolidation

**What you build:**
- Merge LoRA into base weights, reset LoRA
- Configurable trigger

**Verify:** Adapt → consolidate → knowledge persists → new LoRA adaptations accumulate independently.

---

**Milestone 4 checkpoint:** SOMA remembers. Adapts. Survives restarts. Consolidates. Decision log provides institutional memory.

---

## Milestone 5: Synaptic Protocol (SOMA ↔ SOMA)

### Step 5.1: Signal types and binary codec

**What you build:**
- `Signal` struct, all signal types, binary encode/decode, CRC32

**Verify:** Encode → decode → match. Fuzz → no panics.

**Spec reference:** 02_SYNAPTIC_PROTOCOL.md Sections 4, 17

### Step 5.2: TCP transport

**What you build:**
- TCP listener (tokio), handshake, signal framing, PING/PONG

**Verify:** Two SOMAs handshake and exchange PING/PONG.

**Spec reference:** 02_SYNAPTIC_PROTOCOL.md Sections 3, 12, 18

### Step 5.3: Signal routing and intent forwarding

**What you build:**
- SignalRouter dispatch by type
- INTENT → Mind → RESULT flow between SOMAs
- Sequence number correlation

**Verify:** SOMA-A sends intent to SOMA-B, gets result back.

**Spec reference:** 01_CORE.md Section 14

### Step 5.4: Discovery

**What you build:**
- DISCOVER broadcasting, peer registry, PEER_QUERY/PEER_LIST

**Verify:** 3 SOMAs discover each other.

**Spec reference:** 02_SYNAPTIC_PROTOCOL.md Section 7

---

**Milestone 5 checkpoint:** SOMAs communicate. Intent forwarding and discovery work.

---

## Milestone 6: HelperBook Core Plugins

### Step 6.1: Crypto plugin

Conventions: hash_sha256, hash_argon2, verify_argon2, random_bytes, random_uuid, jwt_sign, jwt_verify, hmac_sha256. Training data.

**Verify:** "generate UUID" → UUID. "hash password" → argon2 hash.

**Spec reference:** 05_PLUGIN_CATALOG.md Section 11

### Step 6.2: Auth plugin

Conventions: generate_otp, verify_otp, create_session, validate_session, revoke_session, list_sessions. Depends on: crypto, postgres. Training data.

**Verify:** Full OTP flow. Full session flow.

**Spec reference:** 05_PLUGIN_CATALOG.md Section 10

### Step 6.3: Messaging plugin

Conventions: send, get_messages, mark_read, create_chat, typing, unread_count. Depends on: postgres, redis. Training data.

**Verify:** Create chat → send message → get messages → mark read.

**Spec reference:** 05_PLUGIN_CATALOG.md Section 20

### Step 6.4: HelperBook data model

Through Claude + MCP: create all 15+ tables from 04_HELPERBOOK Section 4.2. Record decisions.

**Verify:** Full flow: create user → connection request → accept → chat → message → appointment → complete → review.

### Step 6.5: Re-synthesize with all plugins

Merge all training data. Re-train. Export. Train per-plugin LoRAs.

**Verify:** `soma.intent("create user and send welcome message")` → multi-plugin program works.

---

**Milestone 6 checkpoint:** HelperBook core works. Users, auth, connections, messaging — end to end.

---

## Milestone 7: Web Frontend

Pragmatic: simple JS/TS renderer, not neural Interface SOMA. Neural rendering is future research (06_INTERFACE_SOMA.md).

### Step 7.1: WebSocket bridge

Synaptic Protocol over WebSocket. Browser connects, sends/receives signals as binary frames.

**Verify:** Browser opens WS, sends signal, receives response.

### Step 7.2: Semantic renderer

Lightweight JS/TS app. Receives semantic JSON. Renders views: contacts, chat, calendar, profile. Sends events back. Uses pencil.dev design tokens as CSS.

**Verify:** Browser shows contacts from postgres. Click contact → chat renders.

### Step 7.3: Real-time updates

Subscribe to channels. STREAM_DATA on new messages. UI updates live.

**Verify:** Two tabs. Message in A → appears in B instantly.

---

**Milestone 7 checkpoint:** HelperBook has a web interface. Real users can use it.

---

## Milestone 8: MCP Bridge Plugin

Can be built any time after Milestone 2. Strategically important but not on critical path.

Connects to external MCP servers → their tools become SOMA conventions. Reduces need for some Milestone 9 plugins.

**Verify:** Connect GitHub MCP server. Through Claude: create issue → works.

**Spec reference:** 05_PLUGIN_CATALOG.md Section 1

---

## Milestone 9: Remaining HelperBook Plugins

Build as needed. Each follows the recurring pattern.

| Plugin | Consider MCP Bridge Instead? |
|---|---|
| Calendar | No — core feature |
| Twilio | Yes — has MCP server |
| SMTP | Yes — Gmail MCP server |
| Push (APNs/FCM) | No — platform-specific |
| Image processing | No — local processing |
| S3 | Yes — AWS MCP server |
| Search | No — custom logic |
| AI inference | Partial — LLM API |
| Geolocation | No — math + PostGIS |
| Reviews | No — core feature |
| Analytics | No — custom aggregation |
| i18n | No — string management |
| Offline cache | No — client-side |
| Webhooks | No — signature verification |
| ID verification | Partial — external API |
| Jobs | No — infrastructure |

---

## Milestone 10: ONNX Mind Engine

`OnnxMindEngine` via `ort` crate. Same trait, faster execution. Build when EmbeddedMindEngine becomes bottleneck (~50+ conventions).

---

## Milestone 11: Production Hardening

Before real users:
- Startup failure handling (01_CORE Section 11.2)
- Graceful shutdown (01_CORE Section 16)
- Error retry loop (01_CORE Section 13.3)
- Resource limits and backpressure (01_CORE Section 20)
- MCP auth tokens (09_INTERACTION Section 11)
- Destructive action confirmation (09_INTERACTION Section 11.3)
- Structured logging with trace IDs (01_CORE Section 18)
- Metrics export (01_CORE Section 18.4)
- Connection recovery (02_PROTOCOL Section 14)
- Rate limiting (02_PROTOCOL Section 20)
- Plugin error cleanup (03_PLUGINS Section 15)

---

## Summary: Critical Path

```
Milestone 1: SOMA Core          ← foundation
     ↓
Milestone 2: MCP Server         ← talk to Claude
     ↓
Milestone 3: PostgreSQL + Redis  ← data
     ↓
Milestone 4: Memory + LoRA       ← learning
     ↓
Milestone 5: Synaptic Protocol   ← SOMA ↔ SOMA
     ↓
Milestone 6: HelperBook plugins   ← business logic
     ↓
Milestone 7: Web frontend         ← users

Parallel (after Milestone 2):
  Milestone 8: MCP Bridge         ← ecosystem

Later:
  Milestone 9-11                   ← features, performance, hardening
```

At Milestone 2, you talk to Claude. From there, building is a conversation.
