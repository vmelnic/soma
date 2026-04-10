# MCP Interface

## Overview

SOMA exposes its full runtime as 19 JSON-RPC 2.0 tools over stdio. Any MCP-aware client (Claude Desktop, ChatGPT, custom tooling) can submit goals, control sessions, inspect state, invoke ports, invoke skills on remote peers (including embedded ESP32 leaves discovered via mDNS), transfer routines between instances, and query metrics through this interface.

## Protocol

- **Transport**: stdin/stdout, line-delimited JSON (one request per line, one response per line)
- **Handshake**: client sends `initialize`, server responds with capabilities and version
- **Tool invocation**: two styles supported:
  - MCP standard: `tools/call` with `{"name": "<tool>", "arguments": {...}}`
  - Direct method: use the tool name as the JSON-RPC method
- **Response**: `{"jsonrpc": "2.0", "result": ..., "id": ...}` on success, `{"jsonrpc": "2.0", "error": {"code": ..., "message": ...}, "id": ...}` on failure

### Handshake

```json
{"jsonrpc":"2.0","method":"initialize","params":{},"id":1}
```

Response:

```json
{"jsonrpc":"2.0","result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"soma-next","version":"0.1.0"}},"id":1}
```

### Tool Discovery

```json
{"jsonrpc":"2.0","method":"tools/list","params":{},"id":2}
```

Returns `{"tools": [...]}` with all 16 tool definitions including `name`, `description`, and `input_schema`.

### Error Codes

| Code | Meaning |
|------|---------|
| -32700 | Parse error (malformed JSON) |
| -32600 | Invalid request (wrong jsonrpc version) |
| -32601 | Method not found |
| -32602 | Invalid params |
| -32603 | Internal error |

## Tools Reference

### 1. create_goal

Submit a goal to the SOMA runtime. Creates a session, runs the control loop to completion (or first non-continue state), and returns the result.

**Input**:

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| `objective` | string | yes | The goal objective description |
| `constraints` | array of objects | no | Constraints on goal execution |
| `risk_budget` | number | no | Maximum risk budget (0.0 - 1.0) |
| `latency_budget_ms` | integer | no | Maximum latency in milliseconds |
| `resource_budget` | number | no | Maximum resource budget |
| `priority` | string | no | `"low"`, `"normal"`, `"high"`, or `"critical"` |
| `permissions_scope` | array of strings | no | Required permission scopes |

**Response**:

```json
{"session_id":"<uuid>","goal_id":"<uuid>","status":"completed","objective":"list files in /tmp","result":{"steps":3,"last_skill":"fs.list_directory"}}
```

Status is one of: `created`, `completed`, `failed`, `aborted`, `waiting_for_input`, `waiting_for_remote`, `error`.

**Example**:

```json
{"jsonrpc":"2.0","method":"create_goal","params":{"objective":"list files in /tmp"},"id":3}
```

### 2. inspect_session

Get session status, working memory, and budget for a session.

**Input**:

| Param | Type | Required |
|-------|------|----------|
| `session_id` | string (UUID) | yes |

**Response**:

```json
{"session_id":"<uuid>","status":"Running","objective":"list files in /tmp","working_memory":{"active_bindings":2,"unresolved_slots":[],"current_subgoal":null,"candidate_shortlist":[]},"budget_remaining":{"risk_remaining":0.5,"latency_remaining_ms":30000,"resource_remaining":100.0,"steps_remaining":100},"step_count":3,"created_at":"2026-04-09T12:00:00Z","updated_at":"2026-04-09T12:00:01Z"}
```

### 3. inspect_belief

Get the current belief state for a session, including resources, facts, uncertainties, and world hash.

**Input**:

| Param | Type | Required |
|-------|------|----------|
| `session_id` | string (UUID) | yes |

**Response**:

```json
{"session_id":"<uuid>","belief":{"belief_id":"<uuid>","resources":2,"facts":[{"fact_id":"f1","subject":"directory","predicate":"exists","confidence":1.0}],"uncertainties":[],"active_bindings":3,"world_hash":"a1b2c3"}}
```

### 4. inspect_resources

List resources known to the runtime. Resources are derived from registered port specs.

**Input**:

| Param | Type | Required |
|-------|------|----------|
| `resource_type` | string | no |
| `resource_id` | string | no |

**Response**:

```json
{"resources":[{"port_id":"postgres","name":"PostgreSQL","kind":"Database","capabilities":3},{"port_id":"smtp","name":"SMTP","kind":"Network","capabilities":2}]}
```

### 5. inspect_packs

List loaded packs and their contents.

**Input**:

| Param | Type | Required |
|-------|------|----------|
| `pack_id` | string | no |

**Response**:

```json
{"packs":[{"pack_id":"helperbook","namespace":"helperbook","version":"0.1.0","skills":12,"ports":3}]}
```

### 6. inspect_skills

List available skills across all loaded packs.

**Input**:

| Param | Type | Required |
|-------|------|----------|
| `pack` | string | no | Filter by pack name |
| `kind` | string | no | `"primitive"`, `"composite"`, `"routine"`, or `"delegated"` |

**Response**:

```json
{"skills":[{"skill_id":"fs.list_directory","name":"list_directory","namespace":"fs","pack":"reference","kind":"Primitive","description":"List files in a directory","risk_class":"Low","determinism":"Deterministic","inputs":{"type":"object","properties":{"path":{"type":"string"}}},"outputs":{"type":"object","properties":{"files":{"type":"array"}}}}]}
```

### 7. inspect_trace

Get the session trace (step-by-step execution log) with pagination.

**Input**:

| Param | Type | Required |
|-------|------|----------|
| `session_id` | string (UUID) | yes |
| `from_step` | integer | no | Starting step index (default 0) |
| `limit` | integer | no | Max steps to return |

**Response**:

```json
{"session_id":"<uuid>","trace":{"total_steps":3,"from_step":0,"returned":3,"steps":[{"step_index":0,"selected_skill":"fs.list_directory","observation_id":"<uuid>","candidate_skills":["fs.list_directory","fs.stat"],"predicted_scores":[{"skill_id":"fs.list_directory","score":0.95}],"critic_decision":"accept","progress_delta":0.33,"belief_patch":{},"policy_decisions":[{"action":"execute","decision":"allow","reason":"within budget"}],"termination_reason":null,"rollback_invoked":false,"timestamp":"2026-04-09T12:00:00Z"}]}}
```

### 8. pause_session

Pause a running session.

**Input**:

| Param | Type | Required |
|-------|------|----------|
| `session_id` | string (UUID) | yes |

**Response**:

```json
{"session_id":"<uuid>","status":"Paused"}
```

### 9. resume_session

Resume a paused session.

**Input**:

| Param | Type | Required |
|-------|------|----------|
| `session_id` | string (UUID) | yes |

**Response**:

```json
{"session_id":"<uuid>","status":"Running"}
```

### 10. abort_session

Abort a session. Cannot be resumed after abort.

**Input**:

| Param | Type | Required |
|-------|------|----------|
| `session_id` | string (UUID) | yes |

**Response**:

```json
{"session_id":"<uuid>","status":"Aborted"}
```

### 11. list_sessions

List all sessions with their current status. No parameters.

**Input**: none

**Response**:

```json
{"sessions":[{"session_id":"<uuid>","status":"completed"},{"session_id":"<uuid>","status":"running"}]}
```

### 12. query_metrics

Get runtime metrics (sessions, skills, ports, uptime).

**Input**:

| Param | Type | Required |
|-------|------|----------|
| `metric_names` | array of strings | no | Specific metrics to return |

**Response**:

```json
{"metrics":{"active_sessions":2,"total_goals":15,"total_steps":47,"skills_executed":42,"ports_invoked":31,"uptime_seconds":3600}}
```

### 13. query_policy

Query policy decisions for a given action.

**Input**:

| Param | Type | Required |
|-------|------|----------|
| `action` | string | yes | The action to check |
| `target` | string | no | Target resource or skill |
| `session_id` | string | no | Session context |

**Response**:

```json
{"action":"execute_skill","decision":{"allowed":true,"effect":"allow","matched_rules":[],"reason":"no policy rules loaded","constraints":null}}
```

### 14. dump_state

Dump full runtime state as structured JSON. Returns a complete snapshot of every subsystem.

**Input**:

| Param | Type | Required |
|-------|------|----------|
| `sections` | array of strings | no | Which sections to include. Omit or pass `["full"]` for everything. Values: `full`, `belief`, `episodes`, `schemas`, `routines`, `sessions`, `skills`, `ports`, `packs`, `metrics` |

**Response** (with `sections: ["full"]`):

```json
{"belief":[{"session_id":"<uuid>","belief_id":"<uuid>","resources":[...],"facts":[{"fact_id":"f1","subject":"dir","predicate":"exists","value":"true","confidence":1.0}],"uncertainties":[]}],"episodes":[...],"schemas":[...],"routines":[...],"sessions":[{"session_id":"<uuid>","status":"completed","objective":"list files in /tmp","budget_remaining":{"risk_remaining":0.45,"latency_remaining_ms":28000,"resource_remaining":97.0,"steps_remaining":97},"trace_steps":3,"working_memory":{"active_bindings":2,"unresolved_slots":[],"current_subgoal":null,"candidate_shortlist":[]},"trace":[{"step_index":0,"selected_skill":"fs.list_directory","observation_id":"<uuid>","critic_decision":"accept","progress_delta":0.33,"timestamp":"2026-04-09T12:00:00Z"}],"created_at":"2026-04-09T12:00:00Z","updated_at":"2026-04-09T12:00:01Z"}],"skills":[{"skill_id":"fs.list_directory","name":"list_directory","namespace":"fs","pack":"reference","kind":"Primitive","description":"List files in a directory","inputs":{"type":"object"},"outputs":{"type":"object"},"risk_class":"Low","determinism":"Deterministic","capability_requirements":[]}],"ports":[{"port_id":"postgres","name":"PostgreSQL","namespace":"db","kind":"Database","capabilities":[{"capability_id":"query","name":"query","purpose":"Execute SQL query"}]}],"packs":[{"pack_id":"helperbook","name":"helperbook","namespace":"helperbook","version":"0.1.0","description":"HelperBook service marketplace","skills_count":12,"ports_count":3,"schemas_count":5,"routines_count":2,"policies_count":1}],"metrics":{"active_sessions":0,"total_goals":1,"self_model":{"uptime_seconds":120,"rss_bytes":15400000,"loaded_packs":1,"registered_skills":12,"registered_ports":3}}}
```

### 15. invoke_port

Invoke a capability on a loaded port. Returns a `PortCallRecord` with the result, latency, success status, and tracing metadata.

**Input**:

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| `port_id` | string | yes | Port identifier (e.g. `"smtp"`, `"postgres"`, `"s3"`) |
| `capability_id` | string | yes | Capability to invoke (e.g. `"send_plain"`, `"query"`, `"put_object"`) |
| `input` | object | no | Input payload for the capability (defaults to `{}`) |

**Response**:

```json
{"port_id":"postgres","capability_id":"query","success":true,"raw_result":{"rows":[{"id":1,"name":"Alice"}]},"structured_result":null,"failure_class":null,"latency_ms":12}
```

**Examples**:

Query postgres:
```json
{"jsonrpc":"2.0","method":"invoke_port","params":{"port_id":"postgres","capability_id":"query","input":{"sql":"SELECT id, name FROM users LIMIT 5"}},"id":10}
```

Set a redis key:
```json
{"jsonrpc":"2.0","method":"invoke_port","params":{"port_id":"redis","capability_id":"set","input":{"key":"session:abc","value":"active","ttl":3600}},"id":11}
```

Send email:
```json
{"jsonrpc":"2.0","method":"invoke_port","params":{"port_id":"smtp","capability_id":"send_plain","input":{"to":"user@example.com","subject":"Welcome","body":"Hello from SOMA"}},"id":12}
```

### 16. list_ports

List all loaded ports and their capabilities. Use this to discover available ports before invoking them.

**Input**:

| Param | Type | Required |
|-------|------|----------|
| `namespace` | string | no | Filter by namespace |

**Response**:

```json
{"ports":[{"port_id":"postgres","name":"PostgreSQL","namespace":"db","kind":"Database","capabilities":[{"capability_id":"query","name":"query","purpose":"Execute a SQL query","effect_class":"Read","risk_class":"Low","input_schema":{"type":"object","properties":{"sql":{"type":"string"}}},"output_schema":{"type":"object","properties":{"rows":{"type":"array"}}}}]},{"port_id":"smtp","name":"SMTP","namespace":"email","kind":"Network","capabilities":[{"capability_id":"send_plain","name":"send_plain","purpose":"Send a plain-text email","effect_class":"Write","risk_class":"Medium","input_schema":{"type":"object","properties":{"to":{"type":"string"},"subject":{"type":"string"},"body":{"type":"string"}}},"output_schema":{"type":"object","properties":{"message_id":{"type":"string"}}}}]}]}
```

### 17. list_peers

List all known distributed peers — both static peers registered via `--peer` / `--unix-peer` at boot and peers discovered at runtime via `--discover-lan` (mDNS). Each entry reports whether the peer has a reachable executor and whether the MCP layer recognizes it.

**Input**: none.

**Response**:

```json
{
  "count": 1,
  "peers": [
    {
      "peer_id": "lan-soma-esp32-ccdba79df9e8",
      "has_executor": true,
      "registered": true
    }
  ]
}
```

Use this to confirm an embedded leaf is reachable before calling `invoke_remote_skill`. A peer with `has_executor: true` means the outbound transport is ready; `registered: true` means the MCP layer has a handle to it.

### 18. invoke_remote_skill

Invoke a skill on a remote peer. Same shape as `invoke_port` but dispatched through the distributed transport layer to another SOMA instance or an embedded leaf. The runtime routes the call through `RemoteExecutor::invoke_skill` and returns the peer's `RemoteSkillResponse` serialized as JSON.

**Input**:

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| `peer_id` | string | yes | Peer ID as reported by `list_peers`. For mDNS-discovered embedded leaves: `lan-soma-<chip>-<mac>`. |
| `skill_id` | string | yes | Fully qualified skill ID on the remote peer (e.g., `display.draw_text`, `thermistor.read_temp`). |
| `input` | object | yes | Arbitrary JSON value matching the remote skill's input schema. |

**Response**:

```json
{
  "skill_id": "display.draw_text",
  "peer_id": "lan-soma-esp32-ccdba79df9e8",
  "success": true,
  "observation": {"rendered": true},
  "latency_ms": 0,
  "timestamp": "2026-04-10T20:12:01.346662+00:00",
  "trace_id": "00000000-0000-0000-0000-000000000000"
}
```

The embedded leaf use case is the cleanest demonstration: an LLM reads a sensor by calling `invoke_remote_skill thermistor.read_temp` and writes to an OLED by calling `invoke_remote_skill display.draw_text`. The leaf has no concept of the composition; the LLM (brain) composes two primitive invocations into behavior. See `soma-project-esp32/scripts/thermistor-to-display.py` for a working periodic loop.

### 19. transfer_routine

Push a compiled routine to a remote peer for local storage and future invocation. The peer stores the routine via its `RoutineStore` (or its leaf-side equivalent); subsequent `InvokeSkill { skill_id: routine.routine_id }` calls on that peer walk the compiled skill path locally. Used to promote schemas learned on one SOMA to other peers, or to hand an embedded leaf a fixed multi-step sequence it can execute without server round-trips per step.

**Input**:

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| `peer_id` | string | yes | Target peer ID from `list_peers`. |
| `routine` | object | yes | Routine JSON (see `types::Routine` for the full shape). Required fields: `routine_id`, `description`, `steps` (array of `{skill_id, input}`). |

**Response**:

```json
{"status": "stored", "routine_id": "demo_pulse", "steps": 4}
```

Transfer semantics depend on the peer: a server peer uses `LocalDispatchHandler::with_stores(..., routine_store)` to persist the routine to its `RoutineStore`; an embedded leaf stores the routine in RAM (vanishes on reboot — the leaf intentionally has no disk persistence for routines).

## Key Tools for LLMs

### dump_state

The most important tool for LLM integration. A single call returns the complete runtime state -- belief, episodes, schemas, routines, sessions (with full traces), skills, ports, packs, and metrics including the self-model (uptime, RSS, counts). An LLM calls this once and has full context to reason about the system without reading source code.

Use `sections` to request only what you need:

```json
{"jsonrpc":"2.0","method":"dump_state","params":{"sections":["sessions","skills","ports"]},"id":5}
```

Or omit `sections` (or pass `["full"]`) to get everything.

The `metrics` section includes `self_model` with proprioception data:

```json
{"metrics":{"active_sessions":1,"total_goals":5,"total_steps":23,"skills_executed":20,"ports_invoked":15,"self_model":{"uptime_seconds":3600,"rss_bytes":15400000,"loaded_packs":2,"registered_skills":24,"registered_ports":6,"peer_connections":0}}}
```

### invoke_port

Direct port capability execution. Bypasses the goal/session/skill pipeline -- the MCP client calls the port directly. Useful when an LLM knows exactly which port and capability it needs.

Discovery workflow:
1. Call `list_ports` to see available ports and capabilities
2. Call `invoke_port` with the desired `port_id`, `capability_id`, and `input`

The response is a `PortCallRecord` with `success`, `raw_result`, `structured_result`, `failure_class`, and `latency_ms`.

### list_ports

Port discovery. Returns every loaded port with full capability metadata including input/output schemas, effect class, and risk class. Call this first when you need to know what external systems are available.

## Starting the MCP Server

```bash
soma --mcp --pack packs/helperbook/manifest.json
```

Multiple packs:
```bash
soma --mcp --pack packs/reference/manifest.json --pack packs/helperbook/manifest.json
```

No packs (minimal runtime, no skills):
```bash
soma --mcp
```

If no `--pack` is specified and `packs/reference/manifest.json` exists, it loads automatically.

The server reads `soma.toml` from the current directory for configuration.

**Distributed flags** (additive to `--mcp`):

```bash
# Static peer registration
soma --mcp --peer 10.0.0.42:9999

# Listen for incoming peer connections
soma --mcp --listen 0.0.0.0:9999

# mDNS LAN auto-discovery — picks up any SOMA peer (server or embedded leaf)
# announcing _soma._tcp.local. on the local network
soma --mcp --discover-lan

# Combined: discover embedded leaves AND accept inbound connections
soma --mcp --discover-lan --listen 0.0.0.0:9999
```

`--discover-lan` is how an LLM driving a server SOMA reaches physical ESP32 leaves without static configuration. The discovered peers appear in `list_peers` as soon as the mDNS browser resolves them; `invoke_remote_skill` then dispatches against them exactly like any other peer.

### Environment Variables

| Variable | Description |
|----------|-------------|
| `SOMA_PORTS_PLUGIN_PATH` | Colon-separated search paths for port shared libraries |
| `SOMA_PORTS_REQUIRE_SIGNATURES` | Set to `true` to require Ed25519 signatures on port libraries |

### Claude Desktop Configuration

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "soma": {
      "command": "/path/to/soma",
      "args": ["--mcp", "--pack", "/path/to/packs/helperbook/manifest.json"]
    }
  }
}
```

## Invocation Styles

Both styles are equivalent and produce identical responses:

**MCP standard** (tools/call):
```json
{"jsonrpc":"2.0","method":"tools/call","params":{"name":"create_goal","arguments":{"objective":"list files in /tmp"}},"id":1}
```

**Direct method**:
```json
{"jsonrpc":"2.0","method":"create_goal","params":{"objective":"list files in /tmp"},"id":1}
```
