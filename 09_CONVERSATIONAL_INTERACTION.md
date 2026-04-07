# SOMA Conversational Interaction — Specification

**Status:** Design  
**Depends on:** SOMA Core, MCP Plugin, Synaptic Protocol  
**Impacts:** 01_CORE (refactor), 06_INTERFACE_SOMA (refactor)

---

## 1. Core Principle

SOMA does not converse. SOMA does not ask questions. SOMA does not understand product specs. SOMA does not generate natural language responses.

SOMA is a pure executor: receive structured intent → generate program → execute via plugins → return result.

Conversation — understanding, reasoning, asking questions, explaining — is the job of an external LLM (Claude, ChatGPT, Ollama, or any other). The LLM connects to SOMA via MCP. The LLM brings intelligence. SOMA brings state, memory, and execution.

**This is a fundamental architectural decision: SOMA is the body, the LLM is the companion brain.**

---

## 2. Why SOMA Should NOT Converse

### 2.1 Size Constraint

Conversational understanding requires 1B+ parameters. SOMA must run on ESP32 (50K params) to cloud (50M params). Embedding conversation into SOMA would make it too large for embedded targets — violating the universal principle.

### 2.2 LLMs Already Exist

Claude, ChatGPT, Llama, Phi, Qwen — the world has extraordinary conversational AI. Rebuilding this inside SOMA is wasteful. SOMA should do what LLMs cannot: directly control hardware, run plugins, manage state, execute programs deterministically.

### 2.3 Freedom of Choice

Different humans prefer different LLMs. Some want Claude. Some want ChatGPT. Some want a local model for privacy. Some want to switch between them. If conversation lives inside SOMA, the user is locked in. If it's external, they choose.

### 2.4 Clean Separation of Concerns

LLMs are non-deterministic (same prompt, different response). SOMA must be deterministic (same intent, same program). Mixing them in one model creates unpredictable behavior. Keeping them separate means: the LLM can be creative and exploratory, SOMA is reliable and predictable.

---

## 3. Architecture

```
┌─────────────────────────────────────────────────────┐
│  Human                                               │
│  (builder, developer, end user — anyone)             │
└───────────────────────┬─────────────────────────────┘
                        │ natural language
                        ▼
┌─────────────────────────────────────────────────────┐
│  LLM Layer                                           │
│  (Claude, ChatGPT, Ollama, local Llama, etc.)        │
│                                                      │
│  Responsibilities:                                   │
│    - Understand natural language                     │
│    - Ask clarifying questions                        │
│    - Read documents, specs, designs                  │
│    - Decompose complex ideas into structured calls   │
│    - Explain results in human terms                  │
│    - Maintain conversation context (short-term)      │
│                                                      │
│  NOT a SOMA. Just an LLM. Replaceable, swappable.   │
└───────────────────────┬─────────────────────────────┘
                        │ MCP (tool calls + state queries)
                        ▼
┌─────────────────────────────────────────────────────┐
│  SOMA                                                │
│  (pure executor + permanent state)                   │
│                                                      │
│  Responsibilities:                                   │
│    - Receive structured intents                      │
│    - Generate programs via Mind                      │
│    - Execute programs via plugins                    │
│    - Maintain persistent state (database, files)     │
│    - Maintain experiential memory (LoRA, checkpoint) │
│    - Expose FULL state via MCP                       │
│    - Render UI via Interface SOMA (renderer)         │
│                                                      │
│  Runs on: ESP32 to cloud. Deterministic.             │
│  No conversation. No natural language generation.    │
└─────────────────────────────────────────────────────┘
```

---

## 4. SOMA as Permanent Memory — Solving LLM Context Loss

### 4.1 The Problem

LLMs lose context. After 100 messages, the LLM forgets what was said at message 1. After the conversation closes, everything is gone. If you switch LLMs, context doesn't transfer.

This is the fundamental problem with using LLMs for any persistent work.

### 4.2 The Solution: SOMA IS the Memory

The LLM doesn't need to remember — it queries SOMA. SOMA holds the truth permanently.

```
Session 1 (Monday, using Claude):
  Human: "Create a users table with phone, name, role"
  Claude → SOMA: soma.execute("CREATE TABLE users ...")
  SOMA: table created, state updated

Session 2 (Wednesday, using ChatGPT):
  Human: "Add a bio field to the users table"
  ChatGPT → SOMA: soma.get_schema()
  SOMA returns: {users: {columns: [id, phone, name, role]}}
  ChatGPT: knows the full schema without ANY prior conversation
  ChatGPT → SOMA: soma.execute("ALTER TABLE users ADD COLUMN bio TEXT")

Session 3 (Friday, using local Ollama):
  Human: "What does the users table look like?"
  Ollama → SOMA: soma.get_schema()
  SOMA returns: {users: {columns: [id, phone, name, role, bio]}}
  Ollama: "The users table has: id, phone, name, role, and bio."
```

Three different LLMs. Three different sessions. Zero context loss. SOMA holds the truth.

### 4.3 What Lives Where

| SOMA (permanent, queryable) | LLM (ephemeral, per-session) |
|---|---|
| Database schema and data | Current conversation thread |
| Plugin configuration | Human's conversational tone |
| Business rules (experiential memory) | Reasoning behind current question |
| Decision log (why things were built) | Creative suggestions in progress |
| Execution history | Draft ideas not yet committed |
| LoRA adaptations | — |
| Render state (UI structure) | — |
| Checkpoints (full snapshots) | — |
| Connected peers and topology | — |
| Resource usage and health | — |

**Rule: if it matters beyond the current conversation, it goes in SOMA. If it's only relevant right now, it stays in the LLM.**

### 4.4 The Paradigm Inversion

```
Today's paradigm:
  LLM is brain AND memory → memory degrades → context lost → work repeated

SOMA paradigm:
  LLM is brain (temporary, replaceable)
  SOMA is body + memory (permanent, queryable)
  → memory never degrades
  → context always available
  → LLMs are interchangeable
```

---

## 5. MCP State Exposure — The Full State API

### 5.1 Bootstrap: Get Everything

When an LLM starts a conversation with a SOMA:

```
LLM: → soma.get_state()

Returns:
{
  "soma_id": "helperbook-backend",
  "version": "0.1.0",
  "uptime": "47h 23m",

  "plugins": {
    "loaded": [
      {"name": "postgres", "version": "0.1.0", "conventions": 12, "healthy": true},
      {"name": "redis", "version": "0.1.0", "conventions": 14, "healthy": true},
      {"name": "auth", "version": "0.1.0", "conventions": 12, "healthy": true},
      {"name": "messaging", "version": "0.2.0", "conventions": 15, "healthy": true}
    ],
    "total_conventions": 53
  },

  "database": {
    "tables": {
      "users": {
        "columns": [
          {"name": "id", "type": "SERIAL", "primary_key": true},
          {"name": "phone", "type": "VARCHAR(20)", "not_null": true},
          {"name": "name", "type": "TEXT", "not_null": true},
          {"name": "role", "type": "VARCHAR(10)", "default": "client"}
        ],
        "row_count": 1234,
        "indexes": ["idx_users_phone"]
      },
      "connections": { "...": "..." },
      "messages": { "...": "..." },
      "appointments": { "...": "..." }
    },
    "total_tables": 15,
    "total_rows": 45678
  },

  "render_state": {
    "interface_somas_connected": 2,
    "active_views": ["contact_list", "chat", "calendar", "profile"],
    "design_loaded": "helperbook.pen",
    "components": ["contact_card", "appointment_card", "chat_bubble", "nav_bar"]
  },

  "experience": {
    "total_executions": 4521,
    "success_rate": 0.97,
    "adaptations": 89,
    "consolidations": 3,
    "lora_magnitude": 0.045
  },

  "decisions": {
    "recent": [
      {"when": "2h ago", "what": "added waitlist table", "why": "walk-in clients need separate queue"},
      {"when": "5h ago", "what": "added phone field to appointments", "why": "providers need to call clients"},
      {"when": "1d ago", "what": "created review system", "why": "both parties review after service completion"}
    ],
    "total": 47
  },

  "health": {
    "memory_mb": 234,
    "cpu_pct": 12,
    "connections": 3,
    "error_rate_1h": 0.002,
    "last_checkpoint": "2h ago"
  }
}
```

**One call. The LLM knows everything. Takes ~1 second.**

### 5.2 Targeted Queries

For focused work, the LLM queries specific state:

| Tool | Returns | When to Use |
|---|---|---|
| `soma.get_state()` | Full snapshot | Conversation start (bootstrap) |
| `soma.get_schema()` | All tables with columns, types, constraints, row counts | Data model work |
| `soma.get_schema(table)` | Specific table with sample rows | Focused table work |
| `soma.get_plugins()` | Loaded plugins with conventions and configs | Understanding capabilities |
| `soma.get_conventions()` | All callable conventions with full arg specs | Understanding what SOMA can do |
| `soma.get_render_state()` | Current UI views, components, design | Understanding what user sees |
| `soma.get_experience()` | LoRA state, adaptation stats | Understanding learning state |
| `soma.get_decisions(n?)` | Decision log with reasoning | Understanding WHY things exist |
| `soma.get_health()` | Memory, CPU, connections, error rates | Operational diagnostics |
| `soma.get_recent_activity(n)` | Last N executions with intents and results | Recent context |
| `soma.get_checkpoints()` | Available checkpoints with timestamps and sizes | Recovery options |
| `soma.get_peers()` | Connected SOMAs, their capabilities, health | Network awareness |
| `soma.get_config()` | Current configuration (secrets redacted) | Understanding setup |
| `soma.get_business_rules()` | Extracted rules from experiential memory | Domain logic |

### 5.3 Context Refresh Pattern

After making changes, the LLM verifies by re-querying:

```
LLM: → soma.execute("ALTER TABLE users ADD COLUMN bio TEXT")
LLM: → soma.get_schema("users")  // verify the change
LLM: "Done. The users table now has a bio column."
```

The LLM doesn't trust its own memory — it trusts SOMA. This prevents hallucination ("I added the column" when it actually failed).

---

## 6. MCP Action Tools

### 6.1 Core Actions

| Tool | Args | Description |
|---|---|---|
| `soma.intent(text)` | intent: String | Send structured intent for Mind execution |
| `soma.install_plugin(name)` | name: String | Install plugin from registry |
| `soma.uninstall_plugin(name)` | name: String | Remove plugin |
| `soma.checkpoint(label?)` | label: String (optional) | Save current state |
| `soma.restore(checkpoint_id)` | id: String | Restore from checkpoint |
| `soma.record_decision(what, why, context?)` | strings | Record a design decision |
| `soma.reload_design(path)` | path: String | Reload design from .pen file |
| `soma.configure_plugin(name, config)` | name + map | Update plugin config |

### 6.2 Plugin Convention Actions

Every loaded plugin's conventions are exposed as MCP tools:

```
soma.postgres.query(sql, params?)
soma.postgres.execute(sql, params?)
soma.postgres.create_table(name, columns)
soma.redis.get(key)
soma.redis.set(key, value, ttl?)
soma.auth.generate_otp(phone)
soma.auth.verify_otp(phone, code)
soma.messaging.send(chat_id, sender, type, content)
soma.calendar.create_event(title, start, end, data)
soma.calendar.check_conflict(participant, start, end)
soma.image.thumbnail(data, width, height)
...
```

These are discovered dynamically. When a new plugin is installed, its conventions immediately appear as MCP tools. The LLM can call `soma.get_conventions()` to see what's available.

### 6.3 Render Actions

```
soma.render_view(spec)    — send semantic signal to Interface SOMA
soma.update_view(diff)    — send incremental update
soma.get_render_state()   — see what's currently rendered
```

The LLM never generates HTML/CSS/JS. It composes semantic view specifications:

```json
{
  "view": "contact_list",
  "sub_tab": "contacts",
  "card_fields": ["name", "photo", "service", "rating", "online_status"],
  "actions": ["chat", "book", "favorite"],
  "filters": ["service", "location", "rating"],
  "sort": "distance_asc"
}
```

The Interface SOMA renders this using its design knowledge. The LLM controls WHAT to show. The Interface SOMA controls HOW it looks.

### 6.4 Decision Recording

The LLM should record decisions after making structural changes:

```
soma.record_decision({
  what: "Created waitlist table with max_capacity and position columns",
  why: "Walk-in clients should queue separately when all appointment slots are booked",
  context: "User requested waitlist feature for busy salon periods",
  related_tables: ["waitlist", "appointments"],
  related_plugins: ["postgres", "calendar"]
})
```

Future LLMs (or the same LLM in a new session) can query decisions to understand not just WHAT exists but WHY it exists. This is institutional memory that survives LLM switches, context window resets, and time.

---

## 7. Interaction Patterns

### 7.1 Building an Application (Builder/Creator)

```
Human opens Claude with SOMA MCP connected.

Claude: → soma.get_state()  [bootstrap]
Claude: "I see a fresh SOMA with filesystem and crypto plugins. 
         What would you like to build?"
Human: "I want to build an app where people find and manage 
        service providers. Here's my spec."
       [drops HelperBook.md]

Claude: [reads spec via file upload, understands 32 sections]
Claude: → soma.install_plugin("postgres")
Claude: → soma.install_plugin("redis")  
Claude: → soma.install_plugin("auth")
Claude: → soma.install_plugin("messaging")
Claude: → soma.install_plugin("calendar")
Claude: → soma.install_plugin("search")
Claude: → soma.install_plugin("push")
Claude: → soma.install_plugin("smtp")
Claude: → soma.install_plugin("twilio")
Claude: → soma.install_plugin("image-proc")
Claude: → soma.install_plugin("s3")

Claude: "I've installed the plugins we'll need. Let me start with 
         the data model. Based on your spec, I'm creating the user 
         model with dual roles..."

Claude: → soma.postgres.execute("CREATE TABLE users (...)")
Claude: → soma.postgres.execute("CREATE TABLE connections (...)")
Claude: → soma.record_decision({
           what: "Created users table with role ENUM(client,provider,both)",
           why: "Every user has one account with dual role capability"
         })

Claude: "Tables created. Now let me set up the authentication flow..."
```

The human never writes SQL. The human never configures plugins. The human talks. The LLM translates to SOMA actions.

### 7.2 Using an Application (End User)

End users interact with the rendered Interface SOMA — tapping buttons, typing messages, scrolling lists. Standard app interaction. No LLM needed for normal use.

But an optional LLM layer can enhance the end user experience:

```
Ana (stylist) opens HelperBook.
Normal interaction: tap Contacts, scroll, tap Chat, type message.

Ana: [taps a "smart assistant" button]
     "Show me my busiest day this month"

LLM: → soma.get_state()  [knows Ana is a provider]
LLM: → soma.postgres.query(
         "SELECT date, COUNT(*) FROM appointments 
          WHERE provider_id = $1 AND date >= date_trunc('month', NOW())
          GROUP BY date ORDER BY count DESC LIMIT 1",
         [ana_user_id])
LLM: → soma.render_view({view: "stat_card", data: result})

Ana sees: "Your busiest day this month is April 15 (8 appointments)"
```

The LLM is optional for end users. The app works without it. But with it, the app becomes conversational.

### 7.3 ESP32 / Embedded (No LLM, No Conversation)

```
ESP32 SOMA with GPIO and I2C plugins.
No LLM connected. No MCP.
Receives structured intents via Synaptic Protocol from a hub SOMA.

Hub SOMA → ESP32: INTENT {payload: "read temperature sensor"}
ESP32 Mind: generates program → i2c.read(0x48, 2) → EMIT result
ESP32 → Hub: RESULT {payload: {temperature: 23.5}}
```

The ESP32 SOMA never converses. It receives structured intents (from a hub, from a timer, from a sensor trigger) and executes. Pure body.

If a human wants to talk to the ESP32, they talk to an LLM that sends signals to the hub that sends intents to the ESP32. The ESP32 doesn't know a human is involved.

### 7.4 Team Collaboration

```
Person A (using Claude) and Person B (using ChatGPT)
Both connected to the same SOMA via MCP.

Person A: "Add a reviews table"
Claude: → soma.postgres.execute("CREATE TABLE reviews (...)")
Claude: → soma.record_decision({what: "reviews table", why: "..."})

Person B: "What tables exist?"
ChatGPT: → soma.get_schema()
ChatGPT: "I see 16 tables including a reviews table that was 
          added 5 minutes ago. It has rating, feedback, tags..."

No communication between Claude and ChatGPT.
They both query the same SOMA. SOMA is the shared truth.
```

### 7.5 LLM Switching Mid-Project

```
Week 1: Using Claude to build HelperBook core
  Claude makes 50 changes, records decisions

Week 2: Switch to local Ollama (privacy concerns)
  Ollama: → soma.get_state()
  Ollama knows EVERYTHING Claude built — every table, every decision, every rule
  Ollama continues building from exactly where Claude left off

Week 3: Switch to ChatGPT (better at UI composition)
  ChatGPT: → soma.get_state()
  ChatGPT continues from where Ollama left off

Zero context loss. Zero rework. SOMA is the continuity.
```

---

## 8. Interface SOMA Revised Role

### 8.1 What Changes

With the LLM handling conversation, the Interface SOMA simplifies dramatically:

**Old role:** Understand conversation + render UI + absorb designs + handle events + adapt from experience

**New role:** Receive semantic signals → render UI using design knowledge → send events back as signals

The Interface SOMA is a pure renderer. It doesn't understand "make that bigger." It receives a new view specification with bigger elements and renders it.

### 8.2 How UI Changes Happen

```
Human: "The online status dots are too small"

LLM: [understands the request]
LLM: → soma.get_render_state()  [sees current UI spec]
LLM: → soma.render_view({
         view: "contact_list",
         card_fields: [...],
         style_overrides: {
           "online_status": { "size": "14px" }
         }
       })

Interface SOMA: [receives new semantic signal]
                [renders with larger dots]
                [doesn't know WHY they're larger — it just renders]
```

### 8.3 Design Absorption

Design files (pencil.dev .pen files) are still absorbed as LoRA knowledge by the Interface SOMA — this doesn't change. The Interface SOMA needs design knowledge to render beautifully. But it doesn't need conversational intelligence.

```
LLM: → soma.reload_design("helperbook.pen")
Interface SOMA: [loads design LoRA, re-renders everything in new design language]
```

### 8.4 Interface SOMA Size

Without conversational understanding, the Interface SOMA is small:
- Mind: 50K-500K params (just needs to map semantic signals to render programs)
- Design LoRA: ~100KB
- DOM renderer plugin: ~200KB
- Total WASM: ~1-3MB (down from 5-10MB with conversation)

Faster load. Less memory. Runs on cheaper devices.

---

## 9. MCP as the Universal Protocol

### 9.1 MCP Replaces What We Called "Conversational Input"

Every spec that mentioned "the human types an intent into the Interface SOMA" or "REPL" — that interaction is now: the human talks to an LLM, the LLM calls SOMA via MCP.

The human never directly sends raw intents to SOMA. The LLM translates human language to structured MCP calls.

### 9.2 MCP is Already Standard

MCP is supported by Claude, Cursor, Windsurf, and a growing ecosystem. By exposing SOMA as an MCP server, SOMA becomes instantly usable from any MCP-compatible AI tool. No custom client needed. No REPL to build. No CLI to maintain.

### 9.3 Multiple LLMs, One SOMA

```
┌──────┐  ┌──────┐  ┌──────┐  ┌──────┐
│Claude│  │ChatGPT│ │Ollama│  │Cursor│
└──┬───┘  └──┬───┘  └──┬───┘  └──┬───┘
   │         │         │         │
   └────┬────┴────┬────┴────┬────┘
        │         │         │
        ▼         ▼         ▼
       MCP       MCP       MCP
        │         │         │
        └────┬────┴────┬────┘
             │         │
             ▼         ▼
        ┌─────────────────┐
        │      SOMA       │
        │  (single source │
        │   of truth)     │
        └─────────────────┘
```

### 9.4 SOMA-to-SOMA Still Uses Synaptic Protocol

MCP is for LLM↔SOMA communication. Synaptic Protocol is for SOMA↔SOMA communication. They coexist:

```
LLM ←→ SOMA (backend)     via MCP
SOMA (backend) ←→ SOMA (interface)  via Synaptic Protocol
SOMA (backend) ←→ SOMA (worker)    via Synaptic Protocol
SOMA (backend) ←→ SOMA (ESP32)     via Synaptic Protocol
```

---

## 10. Impact on Other Specifications

### 10.1 Changes to 01_CORE_REFACTORING

- **Remove REPL.** SOMA has no interactive shell. Debugging happens via MCP state queries from any LLM or admin tool.
- **Remove proprioception natural language queries.** "What can you do" is not a SOMA intent. It's an MCP tool call: `soma.get_conventions()`.
- **Add MCP server as core component.** MCP server runs alongside Synaptic Protocol server. Both are always available.
- **Remove "hot-reload via intent."** Plugin loading/unloading is triggered via MCP: `soma.install_plugin(name)`, not via natural language intent.
- **Simplify Signal Router.** The router no longer needs to handle natural language intents from humans. It handles structured signals from other SOMAs only.
- **Admin interface.** A simple HTTP dashboard (via http-bridge plugin) can provide health monitoring, logs, and state inspection for humans who don't want to use an LLM. This replaces the REPL.

### 10.2 Changes to 06_INTERFACE_SOMA

- **Remove conversational input.** The Interface SOMA has no chat input for humans. Humans talk to the LLM, not to the Interface SOMA.
- **Remove "dual input" architecture.** Only one input: semantic signals from Backend SOMA (via Synaptic Protocol).
- **Simplify Mind.** The Interface SOMA's Mind only maps semantic signals to render programs. Smaller model, faster inference.
- **Design absorption stays.** The Interface SOMA still absorbs design files as LoRA knowledge.
- **Event handling stays.** DOM events still flow back as Synaptic signals to the Backend SOMA.

### 10.3 Changes to 04_HELPERBOOK

- **Building HelperBook.** The "conversational building process" (Section 10) happens via LLM + MCP, not via direct SOMA conversation.
- **End user interaction.** Users interact with the rendered UI. Optional LLM assistant for power features.

### 10.4 Changes to 08_DEVELOPER_GUIDE

- **Remove REPL-based workflow.** Replace with: connect your LLM to SOMA via MCP.
- **Getting started becomes:** Install SOMA binary, start it, connect Claude/ChatGPT/Ollama via MCP, start talking.
- **Debug workflow becomes:** Ask the LLM "what's the SOMA status" → LLM calls `soma.get_health()`.

---

## 11. Security Considerations

### 11.1 MCP Authentication

Not every LLM connection should have full access to SOMA. MCP connections authenticate via tokens:

```toml
[mcp]
enabled = true
bind = "0.0.0.0:3000"

[mcp.auth]
# Tokens with different permission levels
tokens = [
  { token_env = "SOMA_MCP_ADMIN_TOKEN", role = "admin", permissions = "all" },
  { token_env = "SOMA_MCP_BUILDER_TOKEN", role = "builder", permissions = "read_write" },
  { token_env = "SOMA_MCP_VIEWER_TOKEN", role = "viewer", permissions = "read_only" },
]
```

| Role | Can Query State | Can Execute Actions | Can Install Plugins | Can Restore Checkpoints |
|---|---|---|---|---|
| admin | Yes | Yes | Yes | Yes |
| builder | Yes | Yes | No | No |
| viewer | Yes | No | No | No |

### 11.2 Audit Trail

Every MCP action is logged:

```json
{
  "ts": "2026-04-07T15:30:01Z",
  "mcp_client": "claude-desktop",
  "auth_role": "builder",
  "action": "soma.postgres.execute",
  "args": {"sql": "ALTER TABLE users ADD COLUMN bio TEXT"},
  "result": "success",
  "trace_id": "abc123"
}
```

This provides a complete audit trail of who (which LLM/client) did what to the SOMA.

### 11.3 Destructive Action Protection

Certain actions require confirmation:

```
LLM: → soma.postgres.execute("DROP TABLE users")

SOMA: {
  "status": "confirmation_required",
  "message": "This will permanently delete the users table (1,234 rows). 
              Call soma.confirm(action_id) to proceed.",
  "action_id": "xyz789",
  "expires_in": "60s"
}

LLM: [shows warning to human, asks for confirmation]
Human: "Yes, do it"
LLM: → soma.confirm("xyz789")

SOMA: {
  "status": "executed",
  "result": "users table dropped"
}
```

Destructive actions (DROP TABLE, DELETE without WHERE, plugin uninstall, checkpoint restore) always require a two-step confirmation. The SOMA never executes them on the first call.
