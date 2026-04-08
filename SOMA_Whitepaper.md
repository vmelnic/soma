# SOMA: A Universal Neural Architecture for Direct Intent-to-Execution Computing

**Version 0.4 — April 2026**

---

## Abstract

SOMA (from Greek σῶμα, "body") is a computational paradigm in which a trained neural architecture maps structured intents directly to executable programs — sequences of plugin convention calls with resolved arguments — without generating, compiling, or interpreting source code at any layer. The runtime is a single Rust binary (~14MB) with six components: a Mind Engine (BiLSTM encoder + GRU autoregressive decoder), Plugin Manager, Memory System with neuroscience-inspired LoRA adaptation, Synaptic Protocol for inter-instance communication, MCP Server for LLM integration, and Proprioception. SOMA scales from ESP32 microcontrollers (50K parameters, 168KB RAM) to cloud servers (50M+ parameters). Conversational intelligence is delegated to external LLMs that connect via the Model Context Protocol (MCP); SOMA provides deterministic execution, permanent state, and experiential memory.

We present the architecture, a working implementation (101 tests, 6 plugins, 62 conventions), and three proofs of work validating core claims: (1) a neural mind generates multi-step programs of libc function calls executed by a generic bridge with zero domain logic, (2) LoRA experiential memory measurably improves performance on novel phrasings with checkpoint/restore and consolidation, and (3) two SOMA instances discover each other and exchange data via a binary wire protocol, with the neural mind deciding routing.

---

## 1. Introduction

### 1.1 The Problem with Software

Every program ever written follows the same pattern: a human understands what needs to happen, then encodes that understanding into a formal language the machine can execute. The encoding is lossy — the developer's mental model of behavior is compressed into syntax, type systems, and control flow. The resulting artifact (the codebase) grows in complexity, accumulates technical debt, and eventually resists modification. The gap between intent and execution is bridged by millions of developers writing billions of lines of code.

SOMA eliminates this gap — not by generating code more efficiently, but by removing application code from the equation.

### 1.2 The SOMA Paradigm

A SOMA is a computational organism with five components:

- **Mind**: A trained neural model that maps structured intents to execution programs — sequences of plugin convention calls with resolved arguments. The model IS the program.
- **Body**: Plugins that interface with the physical and digital world. Everything outside the Mind — databases, filesystems, sensors, network protocols, user interfaces — is a plugin.
- **Memory**: Experiential LoRA layers that accumulate over time, enabling improvement from experience without full retraining. Grounded in complementary learning systems theory (McClelland et al., 1995).
- **Protocol**: The Synaptic Protocol, a binary wire protocol for SOMA-to-SOMA communication.
- **State**: The complete, persistent, queryable truth — database schema, plugin configurations, decisions, execution history, experiential memory. Transferable across LLM sessions.

### 1.3 The Interaction Model

The human does not interact with SOMA directly. They speak to any LLM (Claude, ChatGPT, Ollama), which translates intent into structured MCP tool calls. SOMA executes deterministically. The LLM explains results. SOMA holds all state permanently — when a new LLM session begins, `soma.get_state()` returns complete context in one call.

This separation is fundamental: the LLM is the brain (temporary, replaceable), SOMA is the body and memory (permanent, deterministic). Any LLM can drive any SOMA. Switching LLMs loses zero context.

### 1.4 Contributions

This paper makes the following contributions:

1. A formal architecture for intent-to-execution computing without application code as an intermediate artifact.
2. A four-tier memory system (permanent, experiential, working, diffuse) with neuroscience-grounded LoRA adaptation and consolidation.
3. A binary wire protocol (Synaptic Protocol) for inter-instance communication with 22-byte overhead, encryption, and discovery.
4. A working Rust implementation with 101 tests, 6 production plugins (62 conventions), and a Python synthesis pipeline.
5. Experimental validation through three proofs of work demonstrating program generation, experiential learning, and multi-instance communication.

---

## 2. Related Work

### 2.1 Neural Program Synthesis

Neural program synthesis maps specifications to programs. DeepCoder (Balog et al., 2017) uses neural networks to guide search over a DSL. RobustFill (Devlin et al., 2017) generates string transformation programs from input-output examples. AlphaCode (Li et al., 2022) generates competition-level code from natural language. These systems generate source code as text — a human-readable artifact that must be parsed, compiled, or interpreted. SOMA generates programs as structured data (sequences of convention IDs, argument types, and references) that are executed directly by the plugin manager. No intermediate textual representation exists.

### 2.2 Tool-Augmented Language Models

Toolformer (Schick et al., 2023) teaches LLMs to call external APIs by inserting tool calls into text. ToolLLM (Qin et al., 2023) extends this to 16K+ APIs. Gorilla (Patil et al., 2023) fine-tunes LLMs for API call generation. These approaches embed tool use within the LLM itself — the LLM decides which tool to call and generates the call as text. SOMA inverts this: the LLM provides natural language understanding, but program generation is delegated to a specialized small model (the Mind, 800K-50M parameters) that operates deterministically. The Mind does not understand language — it maps tokenized intents to programs. This separation enables: (a) SOMA to run on embedded hardware where LLMs cannot, (b) deterministic execution independent of LLM non-determinism, and (c) experiential memory that persists across LLM sessions.

### 2.3 Low-Rank Adaptation and Continual Learning

LoRA (Hu et al., 2021) enables parameter-efficient fine-tuning by injecting low-rank matrices into frozen model weights. Recent work extends LoRA to mixture-of-experts: X-LoRA (Buehler & Buehler, 2024) dynamically weights multiple LoRA adapters per token, MoLoRA (2025) routes per-token to specialized adapters, and InfLoRA (Liang & Li, 2024) addresses interference in continual learning. SOMA uses LoRA for runtime experiential memory — successful executions update adapter weights, and periodic consolidation merges high-magnitude adaptations into permanent weights. This is distinct from fine-tuning: SOMA's LoRA operates at inference time on a deployed system, not during offline training.

### 2.4 Positioning

SOMA differs from prior work along three axes:

| | Neural Program Synthesis | Tool-Augmented LLMs | SOMA |
|---|---|---|---|
| **Output** | Source code (text) | API calls (text) | Programs (structured data) |
| **Execution** | Parsed/compiled/interpreted | LLM + tool dispatcher | Direct plugin dispatch |
| **Memory** | None | LLM context window | Permanent LoRA + state |
| **Scale** | Server only | Server only | ESP32 to cloud |
| **Determinism** | Varies | Non-deterministic | Deterministic at temp=0 |

---

## 3. Foundational Principles

### 3.1 No Application Code

SOMA does not generate source code. The Mind generates **programs**: sequences of plugin convention calls with typed arguments. Programs are ephemeral internal data structures, never serialized to a human-readable language.

Programs may contain domain-specific strings as convention arguments — SQL queries, file paths, email templates. These are data values within the program, not source code. The Mind learns to compose them from training data.

### 3.2 Everything Is a Plugin

The SOMA Core contains exactly six components: Mind Engine, Plugin Manager, Memory System, Synaptic Protocol, MCP Server, and Proprioception. Every capability — filesystem, database, email, rendering, authentication, GPIO — is a plugin. An ESP32 SOMA loads GPIO and I2C plugins. A web backend loads PostgreSQL and Redis. Same core, different body.

### 3.3 Deterministic Execution

Given the same intent, model weights, and softmax temperature (typically 0.0 for production), the Mind produces the same program. Plugins execute deterministically where the underlying operation is deterministic. At temperature > 0, the Mind samples from the output distribution, introducing controlled variation. This property separates SOMA from LLMs, which are inherently non-deterministic even at temperature 0 due to floating-point non-associativity.

### 3.4 Separation of Concerns

SOMA does not converse, ask questions, or generate natural language. It receives structured intents, generates programs, executes them, and returns structured results. Conversational intelligence belongs to the external LLM layer. This is a deliberate architectural decision: conversational understanding requires 1B+ parameters; SOMA runs on ESP32 with 50K parameters. Keeping conversation external keeps SOMA small, universal, and focused.

Any LLM can drive any SOMA via MCP. The human chooses their preferred LLM. SOMA is LLM-agnostic.

---

## 4. Architecture

### 4.1 System Overview

```
┌──────────────────────────────────────────────────┐
│  Human                                            │
└─────────────────────┬────────────────────────────┘
                      │ natural language
                      ▼
┌──────────────────────────────────────────────────┐
│  LLM (Claude, ChatGPT, Ollama, etc.)              │
└─────────────────────┬────────────────────────────┘
                      │ MCP (JSON-RPC 2.0)
                      ▼
┌──────────────────────────────────────────────────┐
│  SOMA Core (single Rust binary, ~14MB)            │
│  ┌──────────┐ ┌──────────┐ ┌──────────────────┐ │
│  │   Mind   │ │ Synaptic │ │    MCP Server    │ │
│  │  Engine  │ │ Protocol │ │  (LLM ↔ SOMA)   │ │
│  └────┬─────┘ └────┬─────┘ └───────┬──────────┘ │
│  ┌────┴─────────────┴───────────────┴──────────┐ │
│  │            Plugin Manager                    │ │
│  └────┬────────────────────────────────────────┘ │
│  ┌────┴──────┐  ┌──────────────┐                │
│  │  Memory   │  │Proprioception│                │
│  │ (LoRA +   │  │ (self-model) │                │
│  │checkpoint)│  └──────────────┘                │
│  └───────────┘                                   │
└──────────────────────────────────────────────────┘
         │                        │
    ┌────┴────┐              ┌────┴────┐
    │ Plugins │              │  Peer   │
    │ (body)  │              │  SOMAs  │
    └─────────┘              └─────────┘
```

MCP exposes two tool categories: **state queries** (`soma.get_state()`, `soma.get_schema()`, `soma.get_plugins()`, `soma.get_decisions()`, `soma.get_health()`, etc.) and **actions** (`soma.intent()`, plugin conventions as `soma.{plugin}.{convention}()`, `soma.checkpoint()`, `soma.record_decision()`). All plugin conventions are discoverable dynamically via `soma.get_conventions()`.

MCP connections authenticate with tokens at three levels: admin (full access), builder (read + execute), viewer (read-only). Destructive actions require two-step confirmation. Every action is logged as an audit trail.

### 4.2 Why Rust

The runtime is a single Rust binary with zero runtime dependencies.

- **Single binary deployment**: `./soma` — one file, copy and run.
- **No garbage collector**: critical for real-time signal processing.
- **True concurrency**: tokio async runtime, no GIL.
- **Memory safety**: no segfaults or buffer overflows — critical for hardware control.
- **Cross-compilation**: x86-64, ARM64, RISC-V, and ESP32 (via `no_std`).
- **Small binaries**: server ~14MB, embedded ~200KB-2MB.

Python is used only for the Synthesizer — a build tool that trains the Mind. Analogous to a compiler written in C for programs that do not require C.

### 4.3 Dual Mind Engine

The Mind Engine defines an abstract `MindEngine` trait with a single method: `infer(&str) -> Program`. Two backends implement it:

**ONNX Engine (server, desktop, Raspberry Pi):**
Uses `tract-onnx` (pure Rust, no C++ dependencies). Model files: `encoder.onnx` + `decoder.onnx` (float32). LoRA: rank 4-64 on all target layers. Dynamic plugin loading (.so/.dylib).

**Embedded Engine (ESP32, microcontrollers):**
Custom inference in pure `no_std` Rust. `.soma-model` format (int8 quantized). Implements only required operations: matmul, sigmoid, tanh, softmax, argmax, embedding lookup (~3-5K lines). LoRA: rank 2-4, output heads only.

| Target | Parameters | Model Size | RAM | Conventions |
|---|---|---|---|---|
| ESP32 (no PSRAM) | ~50K | ~100KB | ~168KB | 8-16 |
| ESP32 (PSRAM) | ~200K | ~400KB | ~2MB | 16-32 |
| Raspberry Pi | ~800K | ~1.6MB | ~20MB | 32-64 |
| Desktop/Server | ~800K-50M | 3-200MB | 50MB-2GB | 64-500+ |

The SOMA Core does not know which backend is running. Both implement the same trait and produce the same output format.

---

## 5. The Mind

### 5.1 Program Definition

A program **P** is a finite sequence of steps **s₁, s₂, ..., sₙ** where n ≤ `max_program_steps` (default: 16). Each step **sᵢ** is a tuple:

```
sᵢ = (opcode, arg₁, arg₂, ...)
```

where:
- **opcode** ∈ {convention₁, ..., conventionₖ, EMIT, STOP} — an index into the convention catalog plus two control opcodes.
- Each **argⱼ** has a type ∈ {literal, span, ref}:
  - **literal**: a constant value embedded in the program.
  - **span(start, end)**: a substring extracted from the input intent by token position.
  - **ref(step_index)**: a pointer to the output of a previous step sᵢ where i < current step.

The decoder generates steps autoregressively until it predicts STOP or reaches the step limit. EMIT signals which step's output to return to the caller.

**Example** — "list files in /tmp and cache the result":

```
s₁ = (fs.list_dir,     span(4,4))          → arg extracted as "/tmp"
s₂ = (redis.set,       literal("files:tmp"), ref(s₁), literal(300))
s₃ = (EMIT,            ref(s₁))
s₄ = (STOP)
```

### 5.2 Neural Architecture

**Encoder (BiLSTM):** 2-layer bidirectional LSTM. Input: tokenized intent (BPE vocabulary, 2K-6K tokens). Output: contextualized encoding **H** ∈ ℝ^(T×2d) where T is sequence length and d is hidden dimension (default: 128). A pooled representation **h₀** = mean(H) serves as the decoder's initial hidden state.

**Decoder (GRU):** Autoregressive GRU cell. At each step t, produces:
- **Opcode logits** ∈ ℝ^(K+2): softmax over K conventions + EMIT + STOP.
- **Argument type logits** ∈ ℝ^(A×3): per argument slot, probability of literal/span/ref.
- **Span position logits** ∈ ℝ^(A×2×T): start and end positions over input tokens.
- **Ref logits** ∈ ℝ^(A×t): pointer attention over previous steps.

where K = number of conventions, A = max argument slots per step (default: 4).

### 5.3 Intent Complexity Classes

**Class 1 — Direct mapping.** One or two program steps. "Read file hello.txt" → `fs.read_file("hello.txt"), EMIT, STOP`. The majority of operations.

**Class 2 — Multi-step.** Multiple plugins chained via references. "Upload photo, generate thumbnail, store both" → 4-5 steps across image-proc, s3, and postgres plugins.

**Class 3 — Decomposed.** The LLM decomposes complex requests into multiple `soma.intent()` calls via MCP, each Class 1 or 2. The Mind stays simple; the LLM handles planning.

### 5.4 Inference Complexity

Encoder: O(T × d²) for 2-layer BiLSTM where T is intent token count and d is hidden dimension.
Decoder: O(n × (K + A×T)) per step for n steps, dominated by the softmax over K conventions.
Total: O(T × d² + n × K) for typical intents where T < 50 and n < 16.

On server (tract-onnx): 1-10ms per inference.
On ESP32: 50-500ms depending on model size and convention count.

---

## 6. The Synthesizer

The Synthesizer is a Python + PyTorch build tool. It is the only component that requires Python. Everything it produces is consumed by the Rust SOMA Core.

### 6.1 Pipeline

```
Inputs:                              Outputs:
  Plugin training data    ──────→    encoder.onnx + decoder.onnx (server)
  Architecture config     ──────→    model.soma-model (embedded, int8)
  Target specification    ──────→    vocab.json, catalog.json
                                     *.lora (per-plugin LoRA weights)
```

1. **Collect** training data from plugins. Each provides `training/examples.json` with (intent, program) pairs.
2. **Build tokenizer** — BPE with character-level fallback. Handles multilingual input, SQL, file paths, URLs.
3. **Expand** pairs via parameter pools and augmentation (synonym replacement, word dropout, typo injection).
4. **Train** the Mind. Combined cross-entropy loss over: opcode prediction, argument type, span position, ref pointers. Target: >95% opcode accuracy, >85% end-to-end program match.
5. **Train plugin LoRA** (optional): freeze base weights, attach LoRA adapters (rank=8, alpha=16), train on single-plugin data.
6. **Export**: ONNX float32 for server; `.soma-model` int8 quantized (post-training quantization with calibration) for embedded. Expected accuracy impact: 1-3% for int8 vs float32.

---

## 7. Plugins — The Body

### 7.1 Definition

A plugin provides:

1. **Calling conventions**: operations the Mind can invoke. Each has a name, argument specification (types, constraints), return type, estimated latency, and optional cleanup action for error recovery.
2. **LoRA knowledge** (optional): pre-trained weight adaptations that teach the Mind how to use the conventions effectively. Installing a plugin with LoRA is gaining the tool AND the expertise.

Every plugin implements the `SomaPlugin` trait: `name()`, `version()`, `conventions()`, `execute()`, `on_load()`, `on_unload()`. Optional: `lora_weights()`, `training_data()`, `execute_stream()`, `checkpoint_state()`, `restore_state()`.

### 7.2 Distribution and Categories

Plugins are packaged as `.soma-plugin` archives (binary + manifest + LoRA + training data). 40 plugins are specified across five tiers:

- **Core (T0)**: MCP bridge, PostgreSQL, Redis, filesystem, crypto.
- **Foundation (T1)**: HTTP bridge, S3, SMTP, Twilio, push, auth, image processing, geolocation, text search, DOM renderer, design knowledge, timer.
- **Features (T2)**: audio/video, WebRTC, calendar, messaging, reviews, analytics, AI inference, job queue, webhooks.
- **Specialized (T3)**: SPI, UART, BLE, PDF, data export.
- **Embedded**: GPIO, I2C, WiFi (built-in, not dynamically loaded).

Six plugins are implemented: crypto (13 conventions), PostgreSQL (15), Redis (14), auth (10), geo (5), HTTP bridge (5) — totaling 62 conventions.

### 7.3 LoRA Plugin Knowledge — Mixture of Experts

Multiple plugin LoRAs are active simultaneously. A gating mechanism dynamically weights which plugin's LoRA to activate per operation. This is consistent with MoE research: X-LoRA (Buehler & Buehler, 2024), L-MoE (2025), LoRA-Mixer (2025), MoLoRA (2025) — all demonstrating that independently trained LoRAs compose at inference time without retraining.

### 7.4 MCP Bridge Plugin

The most strategically important plugin. Connects SOMA to the MCP ecosystem — hundreds of existing MCP servers (GitHub, Slack, Stripe, etc.) become SOMA capabilities without per-service plugins. Each MCP tool is dynamically registered as a SOMA convention.

---

## 8. Memory Architecture

### 8.1 Four-Tier Memory

Grounded in complementary learning systems theory (McClelland et al., 1995) and sleep consolidation research (Diekelmann & Born, 2010; Klinzing et al., 2019):

| Tier | Biological Analogy | Implementation | Lifetime |
|---|---|---|---|
| **Permanent** | Neocortical long-term | Base model weights | Immutable until re-synthesis |
| **Experiential** | Hippocampal recent memory | LoRA A/B matrices | Runtime, checkpointable |
| **Working** | Active neural firing | Decoder hidden states | Per-execution, transient |
| **Diffuse** | Asking a colleague | Synaptic queries to peers | Network-dependent |

### 8.2 Experiential Memory (LoRA)

After successful executions, SOMA records experience. Periodically, LoRA weights update via gradient descent on frozen decoder hidden states.

Implementation: for `nn.Linear` layers, `y = W_frozen(x) + scale · (x · Aᵀ) · Bᵀ` where only A and B are trainable. B is zero-initialized so LoRA has no initial effect. For `nn.GRUCell`, LoRA is applied to both W_ih and W_hh gate weights with a reimplemented forward pass computing `W' = W_base + scale · B · A` before gate computation, preserving correct gradient flow.

### 8.3 Consolidation

High-magnitude LoRA adaptations are periodically merged into permanent weights: `W_base += scale · B · A`, then A and B reset. Proven patterns become permanent memory. On embedded, consolidation writes to flash infrequently (daily/weekly) to respect flash endurance limits (~100K cycles).

### 8.4 Checkpoint and Restore

A checkpoint serializes: base model hash (SHA-256), all LoRA A/B matrices, experience statistics, plugin manifest, and critical plugin state. Restore verifies model hash match. The checkpoint is the mind at that moment.

### 8.5 Institutional Memory

Beyond neural memory, SOMA stores permanent queryable state: database schema, decision log (what was built, why, when, by which LLM session), plugin configurations, and execution history. Exposed via MCP — when any LLM calls `soma.get_state()`, it receives all of this. No context is ever lost. This solves the LLM context problem: the LLM is ephemeral, SOMA is permanent.

---

## 9. The Synaptic Protocol

The Synaptic Protocol is the SOMA-to-SOMA communication layer. MCP is for LLM↔SOMA; Synaptic is for SOMA↔SOMA.

Binary wire protocol, 22-byte overhead per signal (vs HTTP's 500-2000 bytes). Transport-agnostic: TCP, Unix Domain Socket, WebSocket. Big-endian. CRC32 checksum. Optional zstd compression.

**26 signal types across six categories:**
- **Protocol**: HANDSHAKE, HANDSHAKE_ACK, CLOSE, PING, PONG, ERROR, CONTROL
- **Data**: INTENT, RESULT, INVOKE, QUERY, DATA, BINARY
- **Streaming**: STREAM_START, STREAM_DATA, STREAM_END
- **Chunked**: CHUNK_START, CHUNK_DATA, CHUNK_END, CHUNK_ACK (resumable)
- **Discovery**: DISCOVER, DISCOVER_ACK, PEER_QUERY, PEER_LIST
- **Pub/Sub**: SUBSCRIBE, UNSUBSCRIBE

INVOKE and QUERY signals provide direct convention calling and state querying between SOMAs, giving SOMA-to-SOMA communication full parity with LLM-to-SOMA via MCP.

Key capabilities: multiplexed channels, resumable uploads (SHA-256 verified), pub/sub with durable subscriptions and catch-up, peer discovery via presence broadcasting with decaying TTL, auto-reconnect with exponential backoff, and protocol versioning negotiated at handshake.

**Security**: ChaCha20-Poly1305 per-signal encryption, X25519 key exchange, Ed25519 identity — all via dalek crates (production cryptography, not placeholders). Rate limiting per connection with graduated response: throttle → reduce window → disconnect + blacklist.

---

## 10. The Interface SOMA

The Interface SOMA runs on the user's device. It is a SOMA instance with a renderer plugin as its body (DOM for browsers, UIKit for iOS, Compose for Android). It receives semantic signals from Backend SOMAs via Synaptic Protocol and renders them using design knowledge absorbed from pencil.dev .pen files as LoRA.

It does not converse. The human tells the LLM; the LLM updates the view specification via MCP; the Backend sends updated semantic signals; the Interface renders.

**Semantic signals, not markup.** Backend SOMAs send meaning:

```json
{
  "view": "contact_list",
  "data": [{"name": "Ana M.", "service": "Hair Stylist", "rating": 4.8,
            "online": true, "distance_km": 2.3}],
  "actions": ["chat", "book", "favorite"],
  "filters": ["service", "location", "rating"]
}
```

The Interface Mind decides HOW to render based on proprioception (screen size, device type, accessibility settings) and design LoRA. Same signal renders as a grid on desktop, a list on phone, voice output on a speaker.

**Browser deployment**: compiles to WebAssembly (~1-3MB). Synaptic Protocol over WebSocket transport. First meaningful paint: ~550ms after WASM cached. Note: for the current milestone (HelperBook), a pragmatic JS renderer is used. Neural rendering is a research direction documented for future implementation.

---

## 11. Runtime Behavior

**Startup**: Load config → Load Mind model → Load plugins → Restore checkpoint → Start Synaptic Protocol → Start MCP Server → Ready. Failed plugin: skip and continue with reduced capabilities. Corrupt checkpoint: start fresh.

**Concurrency**: Per-request inference context. Encoder is stateless (sharable). Decoder hidden state is per-request. LoRA weights shared via `Arc<RwLock>` — read lock for inference (many concurrent), write lock for adaptation (~10ms pause). Embedded: sequential.

**Error handling**: Retry same step → re-infer (may produce different program at temp > 0) → degrade to partial result → report error. Crashed plugins caught at `catch_unwind` boundary, unloaded; SOMA continues with reduced capabilities. Cleanup conventions handle resource leaks.

**Shutdown**: Stop accepting → notify peers → drain in-flight → checkpoint → unload plugins → close listeners → exit. Embedded: save LoRA to flash with double-buffer for crash safety.

**Observability**: Structured logging via `tracing` (JSON lines). Trace ID propagation across SOMA networks. 21 Prometheus-compatible metrics covering inference, plugins, memory, protocol, and adaptation. Signal capture via `soma-dump` CLI tool.

---

## 12. Security

**MCP**: Role-based auth tokens (admin/builder/viewer). Destructive action two-step confirmation with 60s expiry. Full audit trail.

**Plugins**: Five trust levels — built-in (full trust), community (code review), vendor (signed), private (in-house), untrusted (WASM sandbox via wasmtime, future). Ed25519 signing. Least privilege declarations.

**Synaptic Protocol**: ChaCha20-Poly1305 encryption, X25519 key exchange, Ed25519 identity. Per-connection rate limiting with graduated response.

---

## 13. Implementation Status

The SOMA paradigm is not theoretical. The following components are implemented and tested:

### 13.1 SOMA Core (`soma-core/`)

Single Rust binary (~14MB). 101 tests passing. Approximately 14,900 lines of Rust across: Mind Engine (tract-onnx inference with LoRA and temperature), Plugin Manager (`Arc<RwLock>`, topological sort, convention routing via `plugin_idx*1000` offset, crashed plugin tracking), Memory System (experience ring buffer, LoRA adaptation via gradient descent, checkpoint v2 with SHA-256 model hash, consolidation), Synaptic Protocol (17 source modules: signal codec, TCP/WebSocket/Unix transport, pub/sub, chunked transfer, streaming, discovery, relay, rate limiting, encryption, offline queue), MCP Server (JSON-RPC 2.0, 22+ tools, three auth roles, audit trail), Proprioception (RSS tracking, health reporting), Metrics (21 Prometheus counters), and State (decision log, execution history).

### 13.2 Plugins (`soma-plugins/`)

Six plugins implemented as Rust cdylib crates plus a shared SDK. Approximately 4,500 lines:

| Plugin | Conventions | Description |
|---|---|---|
| crypto | 13 | Hash, sign, encrypt, JWT, random generation |
| postgres | 15 | Query, execute, ORM-style operations |
| redis | 14 | Strings, hashes, lists, pub/sub, keys |
| auth | 10 | OTP, sessions, TOTP, password hashing |
| geo | 5 | Distance, radius filter, geocoding |
| http-bridge | 5 | HTTP client (GET/POST/PUT/DELETE) |

### 13.3 Synthesizer (`soma-synthesizer/`)

Full Python + PyTorch training pipeline (~4,500 lines). CLI commands: train, train-lora, export, validate, test, benchmark. Produces ONNX models and int8 quantized `.soma-model` format.

### 13.4 HelperBook (`soma-helperbook/`)

First real-world SOMA application — a service marketplace connecting clients with service providers. 19-table PostgreSQL schema (users, connections, messages, chats, appointments, reviews, services, etc.), seed data (13 users, 4 chats, 19 messages, 7 appointments, 4 reviews), Express bridge frontend, Docker Compose for PostgreSQL + Redis.

---

## 14. Experimental Validation

Three proofs of work validate the core claims. Complete source code and trained model artifacts are available in the repository (`poc/`, `pow/pow1`, `pow/pow2`, `pow/pow3`).

### 14.1 POW 1 — The Model IS the Program

**Claim (§3, §5):** A neural architecture can map human intent directly to hardware operations without code as an intermediate step.

**Method:** A body discovery module scans the target system (macOS ARM64) and catalogs 16 libc calling conventions — `open`, `read`, `write`, `opendir`, `readdir`, `stat`, `getcwd`, `uname`, `gettimeofday`, and others — as structured data entries with argument schemas, ctypes type signatures, and calling patterns.

A seq2seq neural network (BiLSTM encoder + GRU autoregressive decoder, ~800K parameters, vocabulary size 223) is synthesized to map natural language intent to sequences of convention IDs with data dependencies. A generic execution bridge dispatches on 7 calling patterns (direct, buffered_read, write_bytes, struct_query, iterate, buffered_str, synapse_send) — analogous to CPU addressing modes. No function name appears in the execution path.

**Results:**

```
intent> list files in /tmp

  [Mind] Program (5 steps):
    $0 = libc.opendir("/tmp")
    $1 = libc.readdir($0)
    $2 = libc.closedir($0)
    $3 = EMIT($1)
    STOP

  [Body] (12 items): file1.txt, file2.txt, ...
```

The model generates multi-step programs that correctly sequence libc calls with data dependencies ($0, $1 references). Adding a new libc function requires only a catalog data entry declaring its pattern — no code changes.

**Prior validation (POC):** An earlier proof of concept with 644K parameters and 15 operations achieved 22/22 (100%) accuracy on a test set of 9 known-pattern + 13 novel phrasings, with 2,300 synthesized training examples and ~60s training time on Apple M4.

| Conventional (Alexa, Siri) | SOMA |
|---|---|
| NLU classifies intent | Mind generates multi-step program |
| Hand-coded skill handler executes | Generic bridge dispatches via patterns |
| Adding skill = writing code | Adding capability = catalog data entry |
| Model selects which program to run | Model IS the program |

### 14.2 POW 2 — The Model GROWS as the Program

**Claim (§8):** A SOMA accumulates experiential memory through LoRA adaptation, can checkpoint/restore its mind state, and consolidates experience into permanent memory.

**Method:** The base model from POW 1 is deliberately synthesized on only 50% of intent templates, leaving 50% as novel phrasings. LoRA adapters (rank 8, alpha 2.0) are applied to the decoder GRU and all output heads, adding 44,480 trainable parameters on top of 1,071,864 frozen base parameters.

Controlled experiment protocol:

1. **Baseline**: Measure model confidence on 12 novel phrasings never seen during synthesis.
2. **Experience**: Execute the novel phrasings and record (input, program) pairs in an experience buffer. 12 novel + 6 known-good intents, replayed 4x = 72 total experiences.
3. **Adaptation**: Run 40 LoRA training cycles on sampled experience batches (lr=2e-3). Only LoRA parameters update; base weights frozen.
4. **Post-adaptation**: Re-measure confidence on the same 12 novel phrasings.
5. **Rollback**: Reset LoRA to zero. Verify confidence returns to baseline.

**Results** (measured on Apple M4, April 2026):

```
Intent                                    Before   After    Delta
show directory listing for /tmp           98.9%    98.9%    +0.00%
enumerate all files in /var/log           62.6%    77.9%   +15.30% ↑
what exists in ~/Documents                99.4%    98.6%    -0.81%
get directory contents of /etc            72.7%    75.7%    +3.00% ↑
output the contents of hello.txt          64.7%    81.8%   +17.05% ↑
show hello.txt contents                   99.9%    99.9%    -0.01%
what does test.txt contain                83.9%    95.7%   +11.72% ↑
describe this computer                    98.5%    97.9%    -0.58%
show computer details                     99.1%    98.2%    -0.96%
what's the date today                     99.8%    99.6%    -0.21%
scan /tmp for files                       54.3%    63.6%    +9.40% ↑
peek at hello.txt                         78.5%    92.1%   +13.52% ↑

Baseline avg:  84.36%
Adapted avg:   89.98%
Delta:         +5.62%
Improved:      6/12 intents
```

Adaptation loss converged from 0.132 (cycle 10) to 0.065 (cycle 40). The largest improvements occurred on intents with lowest baseline confidence: "scan /tmp for files" (+9.4%), "output the contents of hello.txt" (+17.1%), "enumerate all files in /var/log" (+15.3%). Intents already at >98% confidence showed negligible change, consistent with LoRA's inability to significantly improve already-correct predictions.

Rollback validation: resetting LoRA to zero returned all confidences to exact baseline values, confirming the improvement exists solely in the LoRA weights.

The improvement exists in the LoRA weights — rollback eliminates it. The four-tier memory hierarchy is operational: permanent memory (frozen base), experiential memory (LoRA), working memory (hidden states). Checkpoint/restore serializes and restores the complete experiential state. Consolidation merges experience into permanent memory via `W_base += scale · B · A`.

### 14.3 POW 3 — SOMAs Communicate via Synaptic Protocol

**Claim (§9):** Multiple SOMA instances can discover each other and exchange data, with the neural mind deciding when and what to communicate.

**Method:** Two SOMA instances (SOMA-A on port 9001, SOMA-B on port 9002) on the same host, each with its own mind, body, and synapse server. The model (17 conventions, vocabulary size 239) treats `send_signal` as a body capability alongside libc functions. The Mind learns during synthesis that intents containing "send to soma-b" produce programs ending with `send_signal` instead of EMIT. The routing decision is neural, not coded.

**Demonstration protocol:**

1. **Discovery**: SOMA-A broadcasts presence → SOMA-B discovers SOMA-A.
2. **Data delegation**: "list files in /tmp and send to soma-b" → SOMA-A lists files via libc, sends result to SOMA-B via TCP.
3. **Content sharing**: "read /tmp/test.txt and send to soma-b" → SOMA-A reads file, sends content.
4. **Time sharing**: "get the time and send to soma-b" → SOMA-A gets time, sends to SOMA-B.
5. **Local verification**: "what time is it" → SOMA-A gets time, EMITs locally (does NOT send). Proves the model distinguishes local display from network transmission.

**Results:** All five tests pass. The demo captures signal count, data types (file listing, file content, timestamps), and protocol metadata. The bridge is fully pattern-based — all execution (libc calls and network sends alike) flows through 7 generic patterns. No function name appears in the execution path.

### 14.4 Summary

| POW | Sections Validated | Core Claim | Result |
|---|---|---|---|
| 1 | §3, §5, §6 | Neural mind generates multi-step programs of discovered functions; generic bridge executes via patterns | Validated |
| 2 | §8 | LoRA experiential memory improves performance; checkpoint/restore serializes mind state; consolidation merges to permanent memory | Validated |
| 3 | §9 | SOMAs discover peers, exchange data via Synaptic Protocol; neural mind decides routing (EMIT vs SEND) | Validated |

Combined, these experiments demonstrate: a neural architecture is synthesized onto a target system, discovers its body, generates programs of body operations from natural language intent, accumulates experiential memory through LoRA adaptation, serializes/restores its complete state, and communicates with peer SOMAs through a binary protocol — all without generating, compiling, or interpreting code at any layer.

---

## 15. Discussion

### 15.1 The Bootstrap Problem

SOMA replaces application code, but the SOMA Core itself is coded in Rust and the Synthesizer in Python. This is the same bootstrapping problem faced by every compiler: the first instance must be hand-written. The SOMA Core and Synthesizer are the last programs that need to be hand-coded. Once operational, SOMAs assist in their own evolution via MCP + LLM, and eventually a SOMA with PyTorch as a plugin could synthesize other SOMAs.

### 15.2 Coexistence and Migration

SOMA does not require replacing existing systems. Three coexistence models: (A) SOMA behind a traditional API via HTTP bridge plugin — transparent to clients, (B) SOMA orchestrating legacy services via MCP bridge, (C) gradual migration — one microservice at a time becomes a SOMA.

SOMA excels at: data-driven applications, CRUD, API orchestration, IoT, automation, multi-service coordination. Traditional code remains preferable for: performance-critical inner loops (game engines, codecs), UI frameworks themselves (though SOMA can use them as plugins), and systems requiring formal verification (until SOMA's verification story matures).

### 15.3 Real-Time Guarantees

Embedded and industrial applications demand hard real-time guarantees. Neural inference has variable latency. For hard real-time operations, SOMA uses **deterministic pathways**: pre-compiled programs with known timing bounds, bypassing Mind inference at runtime. For soft real-time (web applications, IoT), inference latency (1-10ms on server, 50-500ms on ESP32) is acceptable.

### 15.4 Comparison with Existing Paradigms

| Aspect | Traditional Software | AI Code Generation | SOMA |
|---|---|---|---|
| Artifact | Source code | Source code (AI-generated) | Neural weights (no code) |
| Maintenance | Manual | Regenerate (may break) | Adapts from experience |
| Context | Developer's head | LLM context window (lost) | SOMA state (permanent) |
| Platform | Framework-specific | Same | Plugin-based (swap body) |
| Embedded | Separate toolchain | Poor support | Same architecture, smaller model |
| Collaboration | Git, PRs, meetings | Prompt sharing | Query same SOMA state |

### 15.5 Ethical Considerations

SOMA reduces the need for traditional software development. This is the explicit goal, not an unintended side effect. The ethical response: honest acknowledgment and focus on new roles — plugin developers, synthesizer specialists, domain knowledge trainers, SOMA operators.

SOMA executes what it is told via MCP. Destructive actions require confirmation. Every execution is logged. Every decision is recorded. Program traces show exactly what happened. There is no hidden logic. The human remains in control: the LLM may suggest, but execution requires explicit MCP tool calls, which require human approval for critical actions.

---

## 16. Limitations and Open Questions

### 16.1 Model Capacity

How much complexity can a 50M-parameter Mind encode? A 200K-line codebase represents ~5,000 unique decisions. Empirical testing on HelperBook (62 conventions, 19 tables) will establish scaling curves.

### 16.2 LoRA Composition

How many plugin LoRAs compose before MoE gating degrades? Research shows 8-16 experts work well (Buehler & Buehler, 2024). Real applications may need 20+. This requires experimentation.

### 16.3 Formal Verification

Can SOMA programs be formally verified? The deterministic execution model and finite program structure (bounded steps, typed arguments, known conventions) make this more tractable than verifying LLM outputs, but the tools do not yet exist.

### 16.4 Training Data Quality

The Mind is only as good as its training data. Poor examples produce poor programs. The Synthesizer includes validation (convention coverage, duplicate detection, conflict detection), but systematic quality assurance across hundreds of plugins is an open problem.

### 16.5 Adversarial Intents

Can an adversary craft intents that cause SOMA to generate harmful programs? Input validation at the LLM layer provides one defense (the LLM can refuse harmful requests), but SOMA itself has no ethical reasoning. Convention-level permission boundaries (plugins declare capabilities) provide a second defense. A formal analysis of attack surfaces is needed.

### 16.6 Experiential Drift

After thousands of LoRA adaptations and consolidations, the Mind's behavior may drift from its original synthesis. Detection via periodic validation against held-out test sets and correction via targeted LoRA reset are plausible approaches but untested at scale.

---

## 17. Research Roadmap

Dependency-ordered. Milestones 1-6 are complete; 7 is in progress.

| Milestone | Status | Description |
|---|---|---|
| **1. SOMA Core** | ✓ Complete | Rust binary, mind engine, plugin manager, config |
| **2. MCP Server** | ✓ Complete | State exposure, action tools, auth. LLM can drive SOMA. |
| **3. Data Layer** | ✓ Complete | PostgreSQL + Redis plugins, schema exposure |
| **4. Memory** | ✓ Complete | LoRA in Rust, experience, adaptation, checkpoint, consolidation |
| **5. Synaptic Protocol** | ✓ Complete | TCP/WebSocket/Unix transport, codec, routing, discovery, encryption |
| **6. HelperBook Core** | ✓ Complete | 6 plugins, 19-table schema, Express frontend |
| **7. Web Frontend** | In progress | Real-time updates, WebSocket push |
| **8. MCP Bridge** | Planned | Connect to external MCP servers |
| **9. Production** | Partial | Metrics and auth done; retry, resource limits remaining |
| **10. ONNX Runtime** | Planned | GPU/NPU acceleration when tract-onnx is bottleneck |
| **11. Self-Hosting** | Research | SOMA that synthesizes other SOMAs |
| **12. Neuromorphic** | Research | Synthesis onto Intel Loihi or equivalent |

---

## 18. Conclusion

SOMA is a neural architecture that IS the program. It receives structured intents via MCP from any LLM, generates execution programs through a trained Mind, and orchestrates plugins that interface with the world. Its state is permanent, queryable, and transferable across LLM sessions. Its memory grows from experience. Its body adapts through plugins.

The architecture is implemented: a 14MB Rust binary with 101 tests, 6 production plugins providing 62 conventions, a full synthesis pipeline, and a 19-table application (HelperBook) with Express frontend. Three proofs of work validate the core claims: program generation without code, experiential learning via LoRA, and multi-instance communication via Synaptic Protocol.

SOMA does not generate code. It eliminates the need for it.

---

## References

- Balog, M. et al. (2017). "DeepCoder: Learning to Write Programs." ICLR 2017.
- Devlin, J. et al. (2017). "RobustFill: Neural Program Learning under Noisy I/O." ICML 2017.
- Li, Y. et al. (2022). "Competition-Level Code Generation with AlphaCode." Science, 378(6624).
- Schick, T. et al. (2023). "Toolformer: Language Models Can Teach Themselves to Use Tools." NeurIPS 2023.
- Qin, Y. et al. (2023). "ToolLLM: Facilitating Large Language Models to Master 16000+ Real-world APIs." arXiv:2307.16789.
- Patil, S. et al. (2023). "Gorilla: Large Language Model Connected with Massive APIs." arXiv:2305.15334.
- Gulwani, S. et al. (2017). "Program Synthesis." Foundations and Trends in Programming Languages, 4(1-2).
- Hu, E.J. et al. (2021). "LoRA: Low-Rank Adaptation of Large Language Models." ICLR 2022.
- Buehler, E.L. & Buehler, M.J. (2024). "X-LoRA: Mixture of Low-Rank Adapter Experts." APL Machine Learning, 2(2), 026119.
- Wu, X. et al. (2024). "Mixture of LoRA Experts." arXiv:2404.13628.
- L-MoE (2025). "End-to-End Training of a Lightweight Mixture of Low-Rank Adaptation Experts." arXiv:2510.17898.
- LoRA-Mixer (2025). "Coordinate Modular LoRA Experts Through Serial Attention Routing." OpenReview.
- MoLoRA (2025). "Composable Specialization via Per-Token Adapter Routing." arXiv:2603.15965.
- Liang, Y.S. & Li, W.J. (2024). "InfLoRA: Interference-Free Low-Rank Adaptation for Continual Learning." CVPR 2024.
- Wu, Y. et al. (2025). "SD-LoRA: Scalable Decoupled Low-Rank Adaptation for Class Incremental Learning." ICLR 2025.
- McClelland, J.L., McNaughton, B.L. & O'Reilly, R.C. (1995). "Why There Are Complementary Learning Systems in the Hippocampus and Neocortex." Psychological Review, 102(3), 419–457.
- Diekelmann, S. & Born, J. (2010). "The Memory Function of Sleep." Nature Reviews Neuroscience, 11, 114–126.
- Klinzing, J.G., Niethard, N. & Born, J. (2019). "Mechanisms of Systems Memory Consolidation During Sleep." Nature Neuroscience, 22, 1598–1610.
- Yang, W. et al. (2024). "Sharp Wave Ripples Tag Memories for Consolidation." Science.
- Daume, J. et al. (2024). "Control of Working Memory by Phase–Amplitude Coupling of Human Hippocampal Neurons." Nature.
- Baddeley, A.D. & Hitch, G. (1974). "Working Memory." Psychology of Learning and Motivation, 8, 47–89.
- Squire, L.R. (2004). "Memory Systems of the Brain." Neurobiology of Learning and Memory, 82(3), 171–177.
- McCloskey, M. & Cohen, N.J. (1989). "Catastrophic Interference in Connectionist Networks." Psychology of Learning and Motivation, 24, 109–165.
- Karpathy, A. (2017). "Software 2.0." Medium.
- Thompson, K. (1984). "Reflections on Trusting Trust." Communications of the ACM.
- Mead, C. (1990). "Neuromorphic Electronic Systems." Proceedings of the IEEE.
- Davies, M. et al. (2018). "Loihi: A Neuromorphic Manycore Processor with On-Chip Learning." IEEE Micro.
- Hennessy, J. & Patterson, D. (2019). "A New Golden Age for Computer Architecture." Communications of the ACM.
- Lee, E.A. (2008). "Cyber Physical Systems: Design Challenges." ISORC.
- Amodei, D. et al. (2016). "Concrete Problems in AI Safety." arXiv.
- Emelyanov, P. (2011). "CRIU: Checkpoint/Restore In Userspace." Linux Plumbers Conference.
