# SOMA Development Plan

What needs to be built, in what order, and why. Based on current implementation state as of April 2026.

---

## Current State

| Component | Lines | Tests | Status |
|---|---|---|---|
| soma-core | ~14,900 Rust | 101 | Functional: Mind, plugins, protocol, MCP, memory, metrics |
| soma-plugins | ~4,500 Rust | - | 6 implemented + 5 implementing = 11 plugins, 83 conventions (crypto, postgres, redis, auth, geo, http-bridge + image, s3, push, timer, smtp) |
| soma-synthesizer | ~4,500 Python | - | Full pipeline: train, export, validate, test, benchmark |
| soma-helperbook | JS + SQL | - | 19-table schema, 5-view frontend, Express→MCP bridge |
| Training data | 124 examples | - | Across 6 plugins + 1 domain. Low coverage (need 50-200 per convention) |

---

## Priority 1: Synaptic Protocol Parity (INVOKE + QUERY)

**Problem:** SOMA-to-SOMA has only INTENT (forces Mind inference). LLM-to-SOMA has three modes (intent, direct convention, state query). A peer SOMA has less control than an LLM.

**Solution:** Add INVOKE (0x12) and QUERY (0x13) signal types.

### What INVOKE does

A sending SOMA calls a convention on the receiving SOMA directly, bypassing Mind inference:

```
Sender → INVOKE {convention: "postgres.query", args: ["SELECT * FROM users", []]}
Receiver → executes plugin_manager.execute_by_name("postgres", "query", args)
Receiver → RESULT {payload: [{id: 1, name: "Ana", ...}]}
```

No tokenization, no encoder, no decoder, no softmax. Direct plugin dispatch.

### What QUERY does

A sending SOMA queries state on the receiving SOMA:

```
Sender → QUERY {query: "get_schema", params: {table: "users"}}
Receiver → reads state, returns structured response
Receiver → RESULT {payload: {columns: [...], row_count: 1234}}
```

Equivalent to MCP's `soma.get_state()`, `soma.get_schema()`, etc.

### Files to change

**soma-core/src/protocol/signal.rs:**
```rust
// Add after Intent = 0x10, Result = 0x11:
Invoke = 0x12,
Query = 0x13,
```
Update `from_u8()` match arms. Signal count: 24 → 26.

**soma-core/src/protocol/server.rs** — `SomaSignalHandler::handle()`:
Add match arms for `SignalType::Invoke` and `SignalType::Query`. INVOKE calls `plugin_manager.execute_by_name()`. QUERY calls state methods (same logic as MCP `handle_tool_call` for state tools). ~40 lines.

**soma-core/src/protocol/connection.rs** — `is_signal_allowed()`:
Add Invoke and Query to always-allowed list. 2 lines.

**soma-core/src/protocol/client.rs:**
Add `send_invoke_and_wait()` and `send_query_and_wait()` following existing `send_intent_and_wait()` pattern. ~50 lines.

**soma-core/src/protocol/websocket.rs:**
Add INVOKE and QUERY match arms in signal handler. ~25 lines.

**soma-core/src/bin/soma_dump.rs** — `signal_type_name()`:
Add `0x12 => "invoke"`, `0x13 => "query"`. 2 lines.

**soma-core/src/protocol/codec.rs:**
Add to `test_all_signal_types_roundtrip()` test array. 2 lines.

**Docs to update:**
- `docs/synaptic-protocol.md` — signal type table, usage examples, count
- `docs/architecture.md` — mention protocol parity
- `SOMA_Whitepaper.md` — §9 signal count (24 → 26), signal list
- `CLAUDE.md` — signal count in structure comment

### What this unlocks

- Backend SOMA → Interface SOMA: direct render calls instead of INTENT interpretation
- Hub SOMA → ESP32: direct convention calls (<1ms vs 50-500ms Mind inference)
- Any SOMA → Any SOMA: full control parity with LLM-to-SOMA
- MCP Bridge plugin (Priority 2): peer SOMAs can INVOKE through the bridge to external MCP servers

---

## Priority 2: MCP Bridge Plugin

**Problem:** SOMA has 62 conventions across 6 plugins. The MCP ecosystem has thousands of tools across hundreds of servers (GitHub, Slack, Stripe, Google Drive, databases, CI/CD). Building per-service plugins is slow. Connecting to existing MCP servers is fast.

**Solution:** One plugin that spawns an external MCP server process, discovers its tools, and registers each tool as a SOMA convention.

### Architecture

```
LLM → MCP → SOMA → mcp_bridge plugin → spawns external MCP server process
                                       → discovers tools via tools/list
                                       → registers each as convention (bridge_idx*1000 + tool_idx)
                                       → executes via tools/call forwarding

Result: LLM calls soma.github.list_repos() → bridge → github MCP server → result
        Peer SOMA INVOKEs mcp_bridge.github.list_repos() → same path
```

### Implementation

**New crate:** `soma-plugins/mcp-bridge/`

```
soma-plugins/mcp-bridge/
  Cargo.toml          # cdylib, depends on soma-plugin-sdk
  src/lib.rs          # SomaPlugin impl
  manifest.json
  training/examples.json
```

**Core logic:**
1. `on_load(config)` — reads MCP server configs from plugin config:
   ```toml
   [plugins.mcp-bridge]
   servers = [
     { name = "github", command = "github-mcp-server", args = ["--token-env", "GITHUB_TOKEN"] },
     { name = "slack", command = "slack-mcp-server", args = [] },
   ]
   ```
2. For each server: spawn process, complete MCP `initialize` handshake, call `tools/list`
3. Register discovered tools as conventions. Convention name = `{server_name}.{tool_name}`
4. `execute(conv_id, args)` — forward to the appropriate MCP server via `tools/call`
5. `on_unload()` — gracefully shutdown all spawned processes

**Uses existing infrastructure:**
- `soma-core/src/plugin/process.rs` — `ProcessManager` already manages child processes with stdio pipes
- `soma-plugins/sdk` — standard `SomaPlugin` trait
- JSON-RPC 2.0 — same protocol SOMA's own MCP server speaks

**Convention discovery is dynamic:** When a new MCP server is configured and SOMA reloads, its tools appear as new conventions. `soma.get_conventions()` returns them. The LLM can use them immediately.

### Training data

The MCP Bridge doesn't need extensive training data for the Mind because:
- LLMs call conventions directly via MCP (no Mind inference)
- Peer SOMAs call via INVOKE (no Mind inference)
- Only `soma.intent("list my github repos")` would need Mind inference

Minimal training data: 5-10 examples per connected server showing natural language → bridge convention mapping.

### What this unlocks

- Connect to any MCP server without writing a plugin
- SOMA goes from 62 conventions to hundreds/thousands overnight
- LLM orchestrates SOMA + external services in one conversation
- Peer SOMAs can INVOKE through the bridge (requires Priority 1: INVOKE signal)

---

## Priority 3: Reflex Layer

**Problem:** Every `soma.intent()` call and every Synaptic INTENT signal runs full Mind inference (encoder + decoder, ~5ms server, 50-500ms ESP32). Most production intents are repetitive — "list files in /tmp" generates the same program every time.

**Solution:** Cache proven (intent → program) pairs. Check cache before Mind inference.

### Architecture

```
intent arrives
  → reflex.try_match(tokens)
    → exact hash match (confidence 1.0)  → execute cached program, skip Mind
    → fuzzy match (confidence > 0.9)     → execute cached program, skip Mind
    → no match                           → full Mind inference
                                           → if successful, record as new reflex
```

### Implementation

**New file:** `soma-core/src/mind/reflex.rs`

```rust
pub struct ReflexLayer {
    /// Token hash → (program, hit_count, last_used)
    exact_matches: HashMap<u64, ReflexEntry>,
    /// For fuzzy matching: token n-gram index
    ngram_index: HashMap<u64, Vec<usize>>,
    /// All stored reflexes for fuzzy search
    entries: Vec<ReflexEntry>,
    /// Config
    max_entries: usize,         // default: 10,000
    fuzzy_threshold: f32,       // default: 0.9
}

struct ReflexEntry {
    tokens: Vec<u32>,
    program: Program,
    hit_count: u64,
    last_used: u64,
    confidence: f32,            // from original Mind inference
}

impl ReflexLayer {
    pub fn try_match(&self, tokens: &[u32]) -> Option<&Program>;
    pub fn record(&mut self, tokens: Vec<u32>, program: Program, confidence: f32);
    pub fn evict_lru(&mut self);     // evict least-recently-used when full
    pub fn serialize(&self) -> Vec<u8>;
    pub fn deserialize(data: &[u8]) -> Self;
}
```

**Integration points:**

`soma-core/src/main.rs` — REPL execution loop:
```rust
// Before: let program = mind.infer(&intent)?;
// After:
let program = if let Some(cached) = reflex.try_match(&tokens) {
    cached.clone()
} else {
    let p = mind.infer(&intent)?;
    if result.success {
        reflex.record(tokens, p.clone(), p.confidence);
    }
    p
};
```

Same change in:
- `soma-core/src/protocol/server.rs` — Synaptic INTENT handler
- `soma-core/src/mcp/server.rs` — `soma.intent()` MCP tool handler

`soma-core/src/memory/checkpoint.rs` — serialize/restore reflex table alongside LoRA state.

`soma-core/src/memory/consolidation.rs` — high-hit-count reflexes get promoted to "permanent" reflex table (never evicted).

### ESP32 impact

| Path | Latency (ESP32) |
|---|---|
| Full Mind inference | 50-500ms |
| Reflex exact match | <1ms |
| INVOKE (Priority 1) | <1ms |
| Reflex + INVOKE combined | <1ms for everything after first execution |

A fresh ESP32 SOMA is slow (everything through Mind). After running for a day, most intents are reflexes. After consolidation, proven reflexes survive restarts. The ESP32 SOMA gets faster over time — like an organism developing muscle memory.

### What this unlocks

- Sub-millisecond intent handling for repeated patterns
- ESP32 real-time operation for proven intents
- Reduced CPU/power for server SOMAs under load
- Natural "learning curve" — fresh SOMA is deliberate, experienced SOMA is fast

---

## Priority 4: HelperBook End-to-End

**Problem:** HelperBook has 19 tables, 6 plugins, and a 5-view frontend — but it's not a working app. No real-time updates, no user sessions, no live chat.

**Current state:**
- Express server proxies HTTP POST → SOMA MCP via stdio
- 5 views: contacts, chat, calendar, profile, provider-card
- All data fetched via MCP JSON-RPC calls to postgres plugin
- No WebSocket, no push, no sessions, no auth flow

### What needs to happen

#### 4.1 User authentication flow

Currently no auth. Need:
- Login screen (phone + OTP via auth plugin)
- Session token stored in browser (httpOnly cookie or sessionStorage)
- Session validation on each API call
- User context in all queries (filter by current user)

**Files:** `frontend/index.html` (add login view), `frontend/server.js` (add session middleware), `frontend/api.js` (attach session token to requests).

Uses existing: `soma-plugins/auth` (generate_otp, verify_otp, create_session, validate_session).

#### 4.2 Real-time updates via WebSocket

Currently: frontend polls or re-fetches on navigation.
Need: server pushes updates when data changes (new message, status change).

**Option A — WebSocket from Express to browser:**
Express server opens WebSocket. When SOMA executes a write operation (INSERT into messages), Express notifies connected browsers. Simpler, works with current architecture.

**Option B — Synaptic Protocol via WebSocket transport:**
Browser connects to SOMA's Synaptic Protocol server via WebSocket. Subscribes to topics (chat messages, status changes). Backend SOMA publishes events. More architecturally correct, uses existing pub/sub.

Option A is faster to implement. Option B is the long-term architecture.

**Files:** `frontend/server.js` (add WebSocket server or Synaptic WebSocket bridge), `frontend/app.js` (WebSocket client, event-driven UI updates).

#### 4.3 Live chat

Currently: chat view loads messages from postgres, no live updates.
Need: sending messages, receiving messages in real-time, message status (sent/delivered/read).

**Steps:**
1. Send: frontend POST → `soma.postgres.execute(INSERT INTO messages ...)` → push to WebSocket
2. Receive: WebSocket push → update chat view DOM
3. Status: UPDATE messages SET status = 'read' when recipient views

**Files:** `frontend/components/chat.js` (send form, WebSocket listener, status updates).

#### 4.4 Calendar and appointments

Currently: calendar view exists but minimal.
Need: create/view/cancel appointments, conflict checking, appointment cards in chat.

Uses: `soma.postgres.query()` for CRUD, existing schema (appointments table with provider_id, client_id, service, start_time, end_time, status).

#### 4.5 Seed data refresh

Current seed: 13 users, 4 chats, 19 messages, 7 appointments, 4 reviews.
Need: realistic test data for demo. More conversations, varied appointment states, review distribution.

**File:** `soma-helperbook/seed.sql`

### What this proves

A real application built entirely through SOMA, with:
- User authentication (auth plugin)
- Real-time messaging (postgres + WebSocket)
- Appointment scheduling (postgres + domain logic)
- Reviews (postgres)
- All data through MCP, no application backend code

---

## Priority 5: Plugin Expansion

**Problem:** 6 plugins (62 conventions) covers basics. Real web apps need 15-20 plugins.

### Plugins to build (ordered by HelperBook need)

#### 5.1 Image Processing (`soma-plugins/image/`)

Needed for: profile photos, service gallery.

| Convention | Args | Returns |
|---|---|---|
| thumbnail | data: Bytes, width: Int, height: Int | Bytes |
| resize | data: Bytes, width: Int, height: Int | Bytes |
| crop | data: Bytes, x: Int, y: Int, w: Int, h: Int | Bytes |
| format_convert | data: Bytes, format: String | Bytes |
| exif_strip | data: Bytes | Bytes |

Rust crate: `image` (pure Rust, no C deps).

#### 5.2 S3 Storage (`soma-plugins/s3/`)

Needed for: file uploads, profile photos, service gallery.

| Convention | Args | Returns |
|---|---|---|
| put_object | bucket: String, key: String, data: Bytes, content_type: String | String (URL) |
| get_object | bucket: String, key: String | Bytes |
| delete_object | bucket: String, key: String | Bool |
| presign_url | bucket: String, key: String, expires_secs: Int | String (URL) |
| list_objects | bucket: String, prefix: String | List |

Rust crate: `aws-sdk-s3` or `rusoto_s3`.

#### 5.3 Push Notifications (`soma-plugins/push/`)

Needed for: new message alerts, appointment reminders.

| Convention | Args | Returns |
|---|---|---|
| send_apns | device_token: String, title: String, body: String, data: Map | Bool |
| send_fcm | device_token: String, title: String, body: String, data: Map | Bool |
| send_webpush | subscription: Map, title: String, body: String | Bool |
| register_device | user_id: String, platform: String, token: String | Bool |

Rust crates: `a2` (APNS), `fcm` or direct HTTP to FCM.

#### 5.4 Text Search (`soma-plugins/search/`)

Needed for: contact search, service search.

| Convention | Args | Returns |
|---|---|---|
| index | collection: String, id: String, document: Map | Bool |
| search | collection: String, query: String, limit: Int | List |
| delete | collection: String, id: String | Bool |
| suggest | collection: String, prefix: String, limit: Int | List |

Options: PostgreSQL full-text search (via postgres plugin, no new plugin needed), or Tantivy (pure Rust search engine) for dedicated search.

#### 5.5 Timer / Scheduler (`soma-plugins/timer/`)

Needed for: appointment reminders, session expiry, periodic tasks.

| Convention | Args | Returns |
|---|---|---|
| set_timeout | callback_intent: String, delay_ms: Int | Handle |
| set_interval | callback_intent: String, interval_ms: Int | Handle |
| cancel | handle: Handle | Bool |
| cron | expression: String, callback_intent: String | Handle |

#### 5.6 SMTP Email (`soma-plugins/smtp/`)

Needed for: email notifications, appointment confirmations.

| Convention | Args | Returns |
|---|---|---|
| send | to: String, subject: String, body: String | Bool |
| send_html | to: String, subject: String, html: String | Bool |
| send_template | to: String, template: String, vars: Map | Bool |

Rust crate: `lettre`.

#### 5.7 MCP Bridge (`soma-plugins/mcp-bridge/`)

See Priority 2. Listed here for completeness — it IS a plugin.

### Per-plugin checklist

For each new plugin:
- [ ] `Cargo.toml` (cdylib, sdk dependency)
- [ ] `src/lib.rs` (implement SomaPlugin trait)
- [ ] `manifest.json` (name, version, conventions)
- [ ] `training/examples.json` (50-200 examples per convention)
- [ ] Add to `soma-plugins/Cargo.toml` workspace members
- [ ] Test: `cargo build --release`, verify .dylib/.so produced
- [ ] Document in `docs/plugin-catalog.md`

---

## Priority 6: Training Data Expansion

**Problem:** 124 total examples across all plugins. The synthesizer needs 50-200 examples per convention for reliable program generation. With 62 conventions, that's 3,100-12,400 examples needed. We have 2% of the minimum.

### Current coverage

| Plugin | Conventions | Examples | Per-Convention Avg | Target Min |
|---|---|---|---|---|
| postgres | 15 | 34 | 2.3 | 750-3,000 |
| redis | 14 | 17 | 1.2 | 700-2,800 |
| crypto | 13 | 15 | 1.2 | 650-2,600 |
| auth | 10 | 12 | 1.2 | 500-2,000 |
| geo | 5 | 12 | 2.4 | 250-1,000 |
| http-bridge | 5 | 9 | 1.8 | 250-1,000 |
| helperbook domain | - | 25 | - | - |
| **Total** | **62** | **124** | **2.0** | **3,100-12,400** |

### Expansion approach

Each training example in the current format has `intents` (4-8 variations) and `params` (parameter pools). The synthesizer expands these via parameter substitution and augmentation. So 124 examples × ~6 intents × ~5 param variations × augmentation = ~10,000+ expanded pairs. This is adequate for initial synthesis but thin for production accuracy.

**What to add:**

1. **More parameter pools per example.** Current postgres examples have 3-5 table names. Expand to 10-15 including HelperBook-specific tables (users, connections, messages, chats, appointments, reviews, services).

2. **More intent variations per example.** Current examples have 4-8 phrasings. Expand to 10-15, covering:
   - Formal: "Query the users table for providers"
   - Casual: "find me all providers"
   - Terse: "get providers"
   - Descriptive: "I need a list of all users who are providers"

3. **Cross-plugin examples.** Multi-step programs spanning plugins:
   - "Query users and cache in redis" → postgres.query + redis.set
   - "Hash the password and store in database" → crypto.bcrypt_hash + postgres.execute
   - "Check user session and get their appointments" → auth.validate_session + postgres.query

4. **Error-provoking examples.** Intents that test edge cases:
   - Empty results: "find users where role = 'admin'" (no admins in seed data)
   - Invalid args: tested by Mind confidence thresholds

5. **HelperBook domain expansion.** Current 25 examples → 100+:
   - Booking flows: search → check availability → create appointment → notify
   - Messaging: send message → mark delivered → mark read
   - Reviews: create review → update provider rating

### Validation

After expansion, run `soma-synthesize validate` to check:
- All conventions have sufficient examples
- No duplicate intents mapping to different programs
- All programs reference valid conventions
- EMIT and STOP present in all programs
- Parameter types match convention signatures

---

## Dependency Graph

```
[Priority 1: INVOKE/QUERY]
        │
        ├──→ [Priority 2: MCP Bridge] ← uses INVOKE for SOMA-to-SOMA bridge calls
        │           │
        │           └──→ [Priority 4: HelperBook] ← can use bridge for external services
        │
        └──→ [Priority 3: Reflex Layer] ← reflexes apply to INVOKE-received intents too
                    │
                    └──→ ESP32 real-time operation

[Priority 5: Plugins] ← independent, parallel with everything
        │
        └──→ [Priority 4: HelperBook] ← needs image, push, timer for full features

[Priority 6: Training Data] ← independent, parallel
        │
        └──→ Better Mind accuracy for all intent-based paths
```

**Can be parallelized:**
- Priority 1 (INVOKE/QUERY) + Priority 5 (new plugins) + Priority 6 (training data) — no dependencies
- Priority 3 (reflex) can start after Priority 1 or independently
- Priority 2 (MCP Bridge) needs Priority 1 for full value but can start without it
- Priority 4 (HelperBook) benefits from all others but auth flow (4.1) can start immediately

---

## Definition of Done

### For the protocol (Priority 1)
- [ ] `cargo test` passes with INVOKE/QUERY round-trip encoding
- [ ] soma-dump displays "invoke" and "query" signal types
- [ ] Two SOMA instances can INVOKE conventions on each other
- [ ] Two SOMA instances can QUERY state from each other
- [ ] Documentation updated (synaptic-protocol.md, architecture.md, whitepaper)

### For MCP Bridge (Priority 2)
- [ ] Plugin spawns external MCP server, completes handshake
- [ ] `tools/list` discovery works, conventions registered
- [ ] LLM can call `soma.{server}.{tool}()` through bridge
- [ ] Peer SOMA can INVOKE through bridge
- [ ] At least one real MCP server tested (e.g., filesystem, or a simple test server)

### For Reflex Layer (Priority 3)
- [ ] Exact match lookup works (<1ms)
- [ ] Fuzzy match works for near-identical intents
- [ ] Reflexes persist across checkpoints
- [ ] Consolidation promotes high-hit reflexes to permanent
- [ ] Metrics track reflex hit rate vs Mind inference rate

### For HelperBook (Priority 4)
- [ ] User can log in with phone + OTP
- [ ] User can send and receive chat messages in real-time
- [ ] User can create and view appointments
- [ ] User can browse contacts and provider profiles
- [ ] Demo works end-to-end with seed data

### For each new plugin (Priority 5)
- [ ] SomaPlugin trait implemented
- [ ] All conventions tested
- [ ] manifest.json complete
- [ ] training/examples.json with 50+ examples
- [ ] `cargo build --release` produces .dylib/.so
- [ ] Documented in plugin-catalog.md

### For training data (Priority 6)
- [ ] `soma-synthesize validate` passes with 0 errors
- [ ] ≥50 examples per convention
- [ ] Cross-plugin examples for common multi-step patterns
- [ ] HelperBook domain examples cover all major flows
