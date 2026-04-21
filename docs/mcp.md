# MCP Interface

## Overview

SOMA exposes its runtime as JSON-RPC 2.0 tools over stdio. Any MCP-aware client (Claude Desktop, ChatGPT, custom tooling) can submit goals, control sessions, inspect state, invoke ports, invoke skills on remote peers (including embedded ESP32 leaves discovered via mDNS), transfer and replicate routines between instances, manage schedules, manipulate world state, and query metrics through this interface.

Run `tools/list` at runtime for the authoritative tool catalog with full input schemas.

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

Returns `{"tools": [...]}` with all tool definitions including `name`, `description`, and `input_schema`.

### Error Codes

| Code | Meaning |
|------|---------|
| -32700 | Parse error (malformed JSON) |
| -32600 | Invalid request (wrong jsonrpc version) |
| -32601 | Method not found |
| -32602 | Invalid params |
| -32603 | Internal error |

## Tool Categories

Tools are grouped by concern. Call `tools/list` for the full catalog with schemas, or read `build_tools()` in `src/interfaces/mcp.rs`.

**Core** -- goal creation, session lifecycle, inspection, metrics, policy, port invocation, pack management: `create_goal`, `inspect_session`, `inspect_belief`, `inspect_resources`, `inspect_packs`, `inspect_skills`, `inspect_trace`, `pause_session`, `resume_session`, `abort_session`, `list_sessions`, `query_metrics`, `query_policy`, `dump_state`, `invoke_port`, `list_ports`, `list_capabilities`, `reload_pack`, `unload_pack`.

**Async goals** -- fire-and-forget goals with background execution, status polling, cancellation, observation streaming: `create_goal_async`, `get_goal_status`, `cancel_goal`, `stream_goal_observations`.

**Brain integration** -- belief projection, session input provision, plan injection, skill redirect, routine matching: `inspect_belief_projection`, `provide_session_input`, `inject_plan`, `find_routines`, `claim_session`.

**Session transfer** -- cross-instance session migration and handoff: `handoff_session`, `migrate_session`.

**Scheduler** -- one-shot and recurring schedules with port-call or message payloads: `schedule`, `list_schedules`, `cancel_schedule`.

**Distributed** -- peer discovery, remote skill invocation, routine transfer/replication, belief sync: `list_peers`, `invoke_remote_skill`, `transfer_routine`, `replicate_routine`, `sync_beliefs`.

**World state** -- fact patching, snapshots, TTL expiration, autonomous routine triggers: `patch_world_state`, `dump_world_state`, `set_routine_autonomous`, `expire_world_facts`.

**Learning** -- routine execution, episode consolidation, routine authoring/versioning/rollback/review, routine search: `trigger_consolidation`, `execute_routine`, `author_routine`, `list_routine_versions`, `rollback_routine`, `review_routine`.

## Representative Usage

### invoke_port

Direct port capability execution. Bypasses the goal/session/skill pipeline.

```json
{"jsonrpc":"2.0","method":"invoke_port","params":{"port_id":"postgres","capability_id":"query","input":{"sql":"SELECT id, name FROM users LIMIT 5"}},"id":10}
```

Response is a `PortCallRecord`:

```json
{"port_id":"postgres","capability_id":"query","success":true,"raw_result":{"rows":[{"id":1,"name":"Alice"}]},"structured_result":null,"failure_class":null,"latency_ms":12}
```

Discovery workflow: call `list_ports` to see available ports and capabilities, then `invoke_port` with the desired `port_id`, `capability_id`, and `input`.

### create_goal

Submit a goal to the autonomous control loop. Runs to completion (or first non-continue state).

```json
{"jsonrpc":"2.0","method":"create_goal","params":{"objective":"list files in /tmp"},"id":3}
```

Optional params: `constraints`, `risk_budget`, `latency_budget_ms`, `resource_budget`, `priority`, `permissions_scope`. Status is one of: `created`, `completed`, `failed`, `aborted`, `waiting_for_input`, `waiting_for_remote`, `error`.

### create_goal_async

Fire-and-forget goal. Returns immediately with a `goal_id`, runs in background. Poll with `get_goal_status`, cancel with `cancel_goal`, stream trace events with `stream_goal_observations`.

```json
{"jsonrpc":"2.0","method":"create_goal_async","params":{"objective":"create a users table and insert sample data","max_steps":50},"id":4}
```

### dump_state

Full runtime state snapshot. A single call returns belief, episodes, schemas, routines, sessions (with traces), skills, ports, packs, and metrics including the self-model.

```json
{"jsonrpc":"2.0","method":"dump_state","params":{"sections":["sessions","skills","ports"]},"id":5}
```

Omit `sections` or pass `["full"]` for everything.

### invoke_remote_skill

Invoke a skill on a remote peer through the distributed transport layer.

```json
{"jsonrpc":"2.0","method":"invoke_remote_skill","params":{"peer_id":"lan-soma-esp32-ccdba79df9e8","skill_id":"display.draw_text","input":{"line":0,"text":"Hello from SOMA"}},"id":20}
```

## Starting the MCP Server

```bash
soma --mcp --pack packs/helperbook/manifest.json
```

Multiple packs:
```bash
soma --mcp --pack packs/reference/manifest.json --pack packs/helperbook/manifest.json
```

Auto-discovery (no manifests):
```bash
SOMA_PORTS_PLUGIN_PATH=../soma-ports/target/release soma --mcp --pack auto
```

No packs (minimal runtime):
```bash
soma --mcp
```

If no `--pack` is specified and `packs/reference/manifest.json` exists, it loads automatically.

**Distributed flags** (additive to `--mcp`):

```bash
# Static peer registration
soma --mcp --peer 10.0.0.42:9999

# Listen for incoming peer connections
soma --mcp --listen 0.0.0.0:9999

# mDNS LAN auto-discovery
soma --mcp --discover-lan

# MCP over WebSocket
soma --mcp --mcp-ws-listen 127.0.0.1:9200

# HTTP webhook listener
soma --mcp --webhook-listen 127.0.0.1:9200

# Combined: discover embedded leaves AND accept inbound connections
soma --mcp --discover-lan --listen 0.0.0.0:9999
```

`--discover-lan` is how an LLM driving a server SOMA reaches physical ESP32 leaves without static configuration.

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
