# MCP Interface

## Overview

MCP (Model Context Protocol) is how LLMs interact with SOMA. SOMA exposes its full state and all actions as MCP tools using JSON-RPC 2.0 over stdio. Any MCP-compatible LLM or tool -- Claude, ChatGPT, Cursor, Ollama, or any other -- can drive SOMA through this interface.

SOMA implements the MCP `2024-11-05` protocol version with tool and resource capabilities. On startup in MCP mode, it listens on stdin/stdout for JSON-RPC 2.0 messages (one per line). The server advertises 14 state tools, 12 action tools, and dynamically exposes every loaded plugin convention as an additional tool.

```
soma --model ../models --mcp   # Start SOMA as an MCP server on stdio
```

---

## Core Principle

SOMA does not converse. It does not ask questions. It does not generate natural language. It is a pure executor: receive structured intent, generate program, execute via plugins, return result.

The LLM is the brain -- temporary, replaceable, interchangeable. SOMA is the body and memory -- permanent, queryable, deterministic. MCP is the nerve connecting them.

```
LLM (Claude, ChatGPT, Ollama)   <-- MCP -->   SOMA (pure executor)
SOMA (backend)                   <-- Synaptic Protocol -->   SOMA (interface, worker, ESP32)
```

Synaptic Protocol is for SOMA-to-SOMA communication. MCP is for LLM-to-SOMA communication. They coexist. The human talks to the LLM. The LLM talks to SOMA via MCP. SOMA executes.

---

## Bootstrap: soma.get_state()

The first call any LLM makes when connecting to a SOMA. Returns a full snapshot of everything -- in one call, in roughly one second:

```json
{
  "soma_id": "helperbook-backend",
  "version": "0.1.0",
  "uptime_secs": 170580,

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
      }
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

One call. The LLM knows everything. This is why LLM switching works -- a new LLM calls `soma.get_state()` and has complete context without any prior conversation.

---

## State Tools (Query)

All state tools are read-only. Every auth level (admin, builder, viewer) can call them.

| Tool | Returns | When to Use |
|---|---|---|
| `soma.get_state()` | Full snapshot: mind, plugins, experience, decisions, health, peers | Conversation start (bootstrap) |
| `soma.get_schema(table?)` | Tables, columns, types, constraints, row counts; sample rows for specific table | Data model work |
| `soma.get_plugins()` | Loaded plugins with conventions, versions, trust levels, health | Understanding capabilities |
| `soma.get_conventions()` | All callable conventions with argument specs and descriptions | Understanding what SOMA can do |
| `soma.get_render_state()` | Current UI views, components, connected Interface SOMAs | Understanding what user sees |
| `soma.get_experience()` | Buffer size, success/failure counts, LoRA magnitude, adaptation stats | Understanding learning state |
| `soma.get_decisions(n?, search?)` | Decision log with reasoning; supports count limit and keyword search | Understanding WHY things exist |
| `soma.get_health()` | Uptime, memory, CPU, inference stats, plugin health, error rates | Operational diagnostics |
| `soma.get_recent_activity(n)` | Last N execution records with intents, results, trace IDs | Recent context |
| `soma.get_checkpoints()` | Available checkpoint files with paths and metadata | Recovery options |
| `soma.get_peers()` | Connected SOMAs with names, addresses, plugins, conventions | Network awareness |
| `soma.get_config()` | Current SOMA configuration (secrets redacted) | Understanding setup |
| `soma.get_metrics(format?)` | Prometheus counters and gauges in JSON or text format | Monitoring |
| `soma.get_business_rules()` | Rules derived from decisions containing "rule", "policy", "require", "constraint" | Domain logic |

### soma.get_schema()

Requires a database plugin (postgres or sqlite). Without one, returns an empty result with a note.

```json
// Request: soma.get_schema()
// Response:
{
  "tables": [
    {
      "name": "users",
      "columns": "[id SERIAL PK, phone VARCHAR(20) NOT NULL, name TEXT NOT NULL, role VARCHAR(10)]"
    },
    {
      "name": "appointments",
      "columns": "[id SERIAL PK, client_id INT FK, provider_id INT FK, start_time TIMESTAMPTZ]"
    }
  ],
  "total_tables": 15
}

// Request: soma.get_schema(table: "users")
// Response (includes sample rows):
{
  "table": "users",
  "columns": "[id SERIAL PK, phone VARCHAR(20) NOT NULL, name TEXT NOT NULL, role VARCHAR(10)]",
  "sample_rows": "[{id: 1, phone: +40700000001, name: Ana Ionescu, role: provider}, ...]"
}
```

### soma.get_decisions()

Supports two optional arguments: `n` (number of recent decisions) and `search` (keyword filter).

```json
// Request: soma.get_decisions(n: 5)
// Response:
{
  "count": 5,
  "decisions": [
    {
      "id": 47,
      "what": "Created waitlist table with max_capacity and position columns",
      "why": "Walk-in clients should queue separately [tables: waitlist, appointments]",
      "timestamp": "2026-04-08T10:30:00Z",
      "session_id": "mcp-session"
    }
  ]
}

// Request: soma.get_decisions(search: "review")
// Response:
{
  "search": "review",
  "count": 2,
  "decisions": [...]
}
```

---

## Action Tools

### Core Actions

| Tool | Required Args | Optional Args | Access | Description |
|---|---|---|---|---|
| `soma.intent(text)` | text: String | -- | builder+ | Send structured intent for Mind execution |
| `soma.install_plugin(name)` | name: String | -- | admin | Install plugin from plugins directory |
| `soma.uninstall_plugin(name)` | name: String | -- | admin | Remove loaded plugin |
| `soma.checkpoint(label?)` | -- | label: String | builder+ | Save current state to checkpoint file |
| `soma.restore_checkpoint(path)` | path: String | -- | admin | Restore from checkpoint (requires confirmation) |
| `soma.record_decision(what, why)` | what, why: String | context, related_tables, related_plugins | builder+ | Record a design decision |
| `soma.configure_plugin(name, config)` | name: String, config: Object | -- | admin | Update plugin configuration |
| `soma.confirm(action_id)` | action_id: String | -- | builder+ | Confirm a pending destructive action |
| `soma.reload_design()` | -- | -- | builder+ | Reload UI design from Interface SOMA |
| `soma.render_view(view)` | view: String | data: Object | builder+ | Send semantic render signal to Interface SOMA |
| `soma.update_view(view)` | view: String | patch: Object | builder+ | Incremental view update |
| `soma.shutdown()` | -- | -- | admin | Trigger graceful shutdown |

### soma.intent()

The primary execution tool. The Mind generates a program from the intent text, and the Body executes it via plugins.

```json
// Request:
{"text": "list files in /tmp"}

// Response:
{
  "trace_id": "a1b2c3d4e5f6",
  "success": true,
  "confidence": 0.94,
  "program_steps": 3,
  "execution_time_ms": 12,
  "output": "file1.txt\nfile2.log\ndata/",
  "error": null,
  "trace": ["posix.list_dir(/tmp)", "EMIT", "STOP"]
}
```

On success, the experience is recorded in the buffer (reinforcing the LoRA). On failure, the error is returned but no experience is recorded (Section 17.1: don't reinforce bad programs).

### soma.record_decision()

Records institutional memory -- what was built and why. Survives LLM switches, context window resets, and time.

```json
// Request:
{
  "what": "Created waitlist table with max_capacity and position columns",
  "why": "Walk-in clients should queue separately when all appointment slots are booked",
  "context": "User requested waitlist feature for busy salon periods",
  "related_tables": ["waitlist", "appointments"],
  "related_plugins": ["postgres", "calendar"]
}

// Response:
{
  "success": true,
  "decision": {
    "id": 48,
    "what": "Created waitlist table with max_capacity and position columns [context: User requested waitlist feature for busy salon periods]",
    "why": "Walk-in clients should queue separately when all appointment slots are booked [tables: waitlist, appointments] [plugins: postgres, calendar]",
    "timestamp": "2026-04-08T14:30:00Z",
    "session_id": "mcp-session"
  },
  "context": "User requested waitlist feature for busy salon periods",
  "related_tables": ["waitlist", "appointments"],
  "related_plugins": ["postgres", "calendar"]
}
```

### soma.checkpoint()

Saves the full SOMA state: LoRA weights, experience buffer, decisions, execution history, plugin states, plugin manifest, and base model hash.

```json
// Request:
{"label": "pre-migration"}

// Response:
{
  "success": true,
  "path": "checkpoints/soma-helperbook-pre-migration-1712567400.ckpt",
  "label": "pre-migration",
  "experience_count": 4521,
  "adaptation_count": 89
}
```

---

## Plugin Convention Actions (Dynamic)

Every loaded plugin's conventions are automatically exposed as MCP tools. The naming format is `soma.{plugin}.{convention}`.

When a new plugin is installed at runtime, its conventions immediately appear in `tools/list`. The LLM discovers them via `soma.get_conventions()`.

### Examples

```
soma.posix.open_read(path)         -- Read a file
soma.posix.list_dir(path)          -- List directory contents
soma.postgres.query(sql, params)   -- Execute SQL query
soma.postgres.execute(sql, params) -- Execute SQL statement
soma.redis.get(key)                -- Get Redis value
soma.redis.set(key, value, ttl)    -- Set Redis value
soma.auth.verify_otp(phone, code)  -- Verify OTP code
soma.crypto.hash(data, algorithm)  -- Hash data
soma.http.get(url, headers)        -- HTTP GET request
```

### Argument Handling

Plugin arguments are typed. The MCP server converts JSON values to plugin `Value` types based on the convention's `ArgSpec`:

| ArgType | JSON Input | Plugin Value |
|---|---|---|
| String | `"hello"` | `Value::String("hello")` |
| Int | `42` | `Value::Int(42)` |
| Float | `3.14` | `Value::Float(3.14)` |
| Bool | `true` | `Value::Bool(true)` |
| Bytes | `"base64data"` | `Value::Bytes(...)` |
| Any | `{"key": "val"}` | `Value::Map(...)` preserving structure |

Required arguments that are missing produce an error. Optional arguments that are missing are passed as `Value::Null`.

### Plugin Call Response

```json
{
  "success": true,
  "result": [
    {"id": 1, "name": "Ana Ionescu", "role": "provider"},
    {"id": 2, "name": "Ion Popescu", "role": "client"}
  ]
}
```

---

## Render Actions

Render tools control what the Interface SOMA displays. They send semantic signals -- the LLM controls WHAT to show, the Interface SOMA controls HOW it looks. The LLM never generates HTML/CSS/JS.

```json
// soma.render_view:
{
  "view": "contact_list",
  "data": {
    "sub_tab": "contacts",
    "card_fields": ["name", "photo", "service", "rating", "online_status"],
    "actions": ["chat", "book", "favorite"],
    "filters": ["service", "location", "rating"],
    "sort": "distance_asc"
  }
}

// soma.update_view (incremental):
{
  "view": "contact_list",
  "patch": {
    "sort": "rating_desc"
  }
}
```

These tools return a placeholder response until an Interface SOMA connects via Synaptic Protocol.

---

## Interaction Patterns

### Building an Application

```
Human opens Claude with SOMA MCP connected.

Claude: -> soma.get_state()  [bootstrap -- sees fresh SOMA with filesystem plugin]
Claude: "I see a fresh SOMA. What would you like to build?"
Human:  "A service marketplace app. Here's my spec." [drops spec file]

Claude: -> soma.install_plugin("postgres")
Claude: -> soma.install_plugin("redis")
Claude: -> soma.install_plugin("auth")
Claude: -> soma.postgres.execute("CREATE TABLE users (...)")
Claude: -> soma.postgres.execute("CREATE TABLE connections (...)")
Claude: -> soma.record_decision({
           what: "Created users table with role ENUM(client,provider,both)",
           why: "Every user has one account with dual role capability"
         })
Claude: "Tables created. Now setting up authentication..."
```

The human never writes SQL. Never configures plugins. The LLM translates natural language to SOMA actions.

### End User Experience

End users interact with the rendered UI -- tapping buttons, typing messages, scrolling lists. No LLM needed for normal operation. But an optional LLM layer can enhance the experience:

```
Ana (stylist) taps "smart assistant":
  "Show me my busiest day this month"

LLM: -> soma.get_state()  [knows Ana is a provider]
LLM: -> soma.postgres.query(
         "SELECT date, COUNT(*) FROM appointments
          WHERE provider_id = $1 AND date >= date_trunc('month', NOW())
          GROUP BY date ORDER BY count DESC LIMIT 1",
         [ana_user_id])
LLM: -> soma.render_view({view: "stat_card", data: result})

Ana sees: "Your busiest day this month is April 15 (8 appointments)"
```

### Team Collaboration

Multiple LLMs/users connected to the same SOMA, sharing a single source of truth:

```
Person A (Claude):  "Add a reviews table"
  Claude: -> soma.postgres.execute("CREATE TABLE reviews (...)")
  Claude: -> soma.record_decision({what: "reviews table", why: "..."})

Person B (ChatGPT): "What tables exist?"
  ChatGPT: -> soma.get_schema()
  ChatGPT: "16 tables including reviews, added 5 minutes ago."
```

No communication between Claude and ChatGPT. They both query the same SOMA.

### LLM Switching

```
Week 1: Claude builds HelperBook core (50 changes, decisions recorded)
Week 2: Switch to Ollama (privacy) -> soma.get_state() -> full context
Week 3: Switch to ChatGPT (UI work) -> soma.get_state() -> full context

Zero context loss. Zero rework. SOMA is the continuity.
```

---

## Authentication and Security

### Token-Based Access Control

SOMA uses three auth levels, configured via environment variables:

| Role | Query State | Execute Actions | Install/Uninstall Plugins | Restore Checkpoints | Shutdown |
|---|---|---|---|---|---|
| admin | yes | yes | yes | yes | yes |
| builder | yes | yes | no | no | no |
| viewer | yes | no | no | no | no |

### Configuration

Set tokens via environment variables:

```bash
export SOMA_MCP_ADMIN_TOKEN="your-admin-token-here"
export SOMA_MCP_BUILDER_TOKEN="your-builder-token-here"
export SOMA_MCP_VIEWER_TOKEN="your-viewer-token-here"
```

Enable auth enforcement in `soma.toml`:

```toml
[security]
require_auth = true
require_confirmation = true    # two-step for destructive actions

[mcp]
enabled = true
transport = "stdio"            # or "http"
max_execution_history = 1000
```

When `require_auth = false` (default for local development), all requests pass as anonymous with full access.

### Passing Tokens in Requests

Tokens can be passed in the tool call arguments via `_meta.auth_token` or `_token`:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "soma.install_plugin",
    "arguments": {
      "name": "postgres",
      "_meta": {
        "auth_token": "your-admin-token-here"
      }
    }
  }
}
```

For stdio transport, auth is typically handled at the process level (the process that spawns SOMA controls access).

### Audit Trail

Every MCP tool call is logged with spec-required fields:

```json
{
  "ts": "2026-04-08T15:30:01Z",
  "mcp_client": "claude-desktop",
  "auth_role": "builder",
  "action": "soma.postgres.execute",
  "args": {"sql": "ALTER TABLE users ADD COLUMN bio TEXT"},
  "result": "success",
  "trace_id": "a1b2c3d4e5f6"
}
```

This provides a complete audit trail of who (which LLM/client) did what to the SOMA.

---

## Destructive Action Protection

When `require_confirmation = true`, certain actions require two-step confirmation:

- `soma.restore_checkpoint` -- overwrites current LoRA state
- Plugin convention calls that execute `DROP TABLE`, `DELETE` without `WHERE`, `TRUNCATE`
- `soma.uninstall_plugin` -- removes a loaded plugin

### Confirmation Flow

```
Step 1: LLM calls destructive action
  -> soma.postgres.execute("DROP TABLE users")

Step 2: SOMA returns confirmation_required
  {
    "requires_confirmation": true,
    "action_id": "confirm-7",
    "description": "Restore checkpoint from ./checkpoints/old.ckpt. This will overwrite current LoRA state.",
    "instructions": "Call soma.confirm with the action_id to proceed."
  }

Step 3: LLM shows warning to human, gets approval

Step 4: LLM confirms
  -> soma.confirm(action_id: "confirm-7")

Step 5: SOMA executes the original action
  {
    "success": true,
    "result": "..."
  }
```

Confirmations expire after 60 seconds. If the LLM calls `soma.confirm` with an expired or invalid `action_id`, it receives an error.

The original tool name and arguments are stored with the pending confirmation. When confirmed, SOMA re-dispatches the original call automatically -- the LLM does not need to repeat it.

---

## Context Refresh Pattern

After making changes, the LLM should re-query SOMA to verify the result. This prevents hallucination ("I added the column" when it actually failed).

```
LLM: -> soma.postgres.execute("ALTER TABLE users ADD COLUMN bio TEXT")
LLM: -> soma.get_schema("users")   // verify the change actually applied
LLM: "Done. The users table now has a bio column of type TEXT."
```

The LLM does not trust its own memory of what it did. It trusts SOMA. This is the fundamental pattern: execute, then query, then report.

---

## MCP Resources

In addition to tools, SOMA exposes two MCP resources:

| URI | MIME Type | Content |
|---|---|---|
| `soma://state` | `application/json` | Complete SOMA state + proprioception |
| `soma://metrics` | `text/plain` | Prometheus-compatible runtime metrics |

Resources are read via the standard MCP `resources/read` method:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "resources/read",
  "params": {"uri": "soma://state"}
}
```

---

## JSON-RPC Protocol Details

### Connection Setup

SOMA implements the standard MCP handshake:

```json
// Client sends:
{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}

// Server responds:
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": "2024-11-05",
    "capabilities": {
      "tools": {"listChanged": false},
      "resources": {"subscribe": false, "listChanged": false}
    },
    "serverInfo": {
      "name": "soma-helperbook",
      "version": "0.1.0",
      "description": "SOMA: Neural mind drives hardware directly. Pure executor with permanent state."
    }
  }
}

// Client sends:
{"jsonrpc": "2.0", "method": "initialized"}
```

### Tool Discovery

```json
// Client sends:
{"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}

// Server responds with all tools (state + action + plugin conventions):
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
    "tools": [
      {
        "name": "soma.get_state",
        "description": "Get the complete SOMA state...",
        "inputSchema": {"type": "object", "properties": {}}
      },
      {
        "name": "soma.intent",
        "description": "Execute a natural language intent...",
        "inputSchema": {
          "type": "object",
          "properties": {"text": {"type": "string", "description": "The intent to execute"}},
          "required": ["text"]
        }
      },
      {
        "name": "soma.postgres.query",
        "description": "Execute a SQL query and return rows",
        "inputSchema": {
          "type": "object",
          "properties": {
            "sql": {"type": "string", "description": "SQL query"},
            "params": {"type": "string", "description": "Query parameters"}
          },
          "required": ["sql"]
        }
      }
    ]
  }
}
```

### Tool Execution

```json
// Client sends:
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "tools/call",
  "params": {
    "name": "soma.get_health",
    "arguments": {}
  }
}

// Server responds:
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"status\": \"healthy\", \"proprioception\": {...}, \"metrics\": {...}}"
      }
    ]
  }
}
```

Tool results follow the MCP content format: an array of content blocks with `type` and `text`. Error results include `"isError": true`.

---

## Connecting Your LLM

### Claude Desktop

Add to your MCP configuration (`~/.config/claude/claude_desktop_config.json` or equivalent):

```json
{
  "mcpServers": {
    "my-soma": {
      "command": "/path/to/soma",
      "args": ["--model", "./models", "--mcp"]
    }
  }
}
```

### Cursor / Other MCP Clients

Any MCP-compatible client can connect by spawning the SOMA binary with the `--mcp` flag. The client communicates via stdin/stdout using JSON-RPC 2.0, one message per line.

### Programmatic Access

For scripts or custom tools, pipe JSON-RPC messages directly:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | soma --model ./models --mcp
```

---

## Signal Capture (Debugging)

For debugging Synaptic Protocol traffic between SOMAs (not MCP traffic), use `soma-dump`:

```bash
soma-dump --port 9001                          # Live traffic capture
soma-dump --port 9001 --type INTENT,RESULT     # Filter by signal type
soma-dump --port 9001 --output capture.synaptic # Save for replay
```

For MCP debugging, use state query tools -- ask your LLM or call them directly:

```
"How is the SOMA doing?"       -> soma.get_health()
"What happened last?"          -> soma.get_recent_activity(5)
"Is postgres working?"         -> soma.get_plugins()
"Show me the full state"       -> soma.get_state()
```

---

## Complete Tool Reference

### State Tools (14)

| # | Tool | Input Schema | Description |
|---|---|---|---|
| 1 | `soma.get_state` | `{}` | Complete SOMA state -- the primary context tool |
| 2 | `soma.get_plugins` | `{}` | All loaded plugins with conventions, health |
| 3 | `soma.get_conventions` | `{}` | All callable conventions with arg specs |
| 4 | `soma.get_health` | `{}` | Uptime, memory, metrics, plugin warnings |
| 5 | `soma.get_recent_activity` | `{n?: int}` | Last N execution records (default: 10) |
| 6 | `soma.get_peers` | `{}` | Connected SOMAs with capabilities |
| 7 | `soma.get_experience` | `{}` | Experience buffer, LoRA state, adaptation stats |
| 8 | `soma.get_checkpoints` | `{}` | Available checkpoint files |
| 9 | `soma.get_config` | `{}` | Current config (secrets redacted) |
| 10 | `soma.get_decisions` | `{n?: int, search?: str}` | Decision log with optional filter |
| 11 | `soma.get_metrics` | `{format?: "json"\|"prometheus"}` | Prometheus-compatible metrics |
| 12 | `soma.get_schema` | `{table?: str}` | Database schema (requires db plugin) |
| 13 | `soma.get_business_rules` | `{}` | Rules derived from decision log |
| 14 | `soma.get_render_state` | `{}` | Interface SOMA render state |

### Action Tools (12)

| # | Tool | Input Schema | Access | Description |
|---|---|---|---|---|
| 1 | `soma.intent` | `{text: str}` | builder+ | Execute intent via Mind |
| 2 | `soma.checkpoint` | `{label?: str}` | builder+ | Save state checkpoint |
| 3 | `soma.record_decision` | `{what: str, why: str, context?: str, related_tables?: [str], related_plugins?: [str]}` | builder+ | Record design decision |
| 4 | `soma.confirm` | `{action_id: str}` | builder+ | Confirm destructive action |
| 5 | `soma.install_plugin` | `{name: str}` | admin | Load plugin from disk |
| 6 | `soma.restore_checkpoint` | `{path: str}` | admin | Restore from checkpoint |
| 7 | `soma.shutdown` | `{}` | admin | Graceful shutdown |
| 8 | `soma.uninstall_plugin` | `{name: str}` | admin | Remove loaded plugin |
| 9 | `soma.configure_plugin` | `{name: str, config: obj}` | admin | Update plugin config |
| 10 | `soma.reload_design` | `{}` | builder+ | Reload UI design |
| 11 | `soma.render_view` | `{view: str, data?: obj}` | builder+ | Render named view |
| 12 | `soma.update_view` | `{view: str, patch?: obj}` | builder+ | Update existing view |

### Dynamic Plugin Tools (N per loaded plugin)

Format: `soma.{plugin_name}.{convention_name}(args...)`

Discovered via `soma.get_conventions()` or `tools/list`. Each convention's argument types, descriptions, and required/optional status are included in the tool's `inputSchema`.
