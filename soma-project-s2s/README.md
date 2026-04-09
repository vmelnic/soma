# soma-project-s2s

SOMA-to-SOMA communication proof. Two SOMA instances cooperate over TCP using the distributed wire protocol.

## What this proves

| Level | What | Tests |
|-------|------|-------|
| **1. Transport** | Raw TCP wire protocol: InvokeSkill, Ping/Pong, SubmitGoal, concurrent connections | 16 |
| **2. Delegation** | MCP tools `list_peers`, `invoke_remote_skill`, cross-peer skill invocation, local + remote ports | 18 |
| **3. Transfer** | Schema and routine transfer between peers, stored on receiver, multi-step schemas, MCP `transfer_routine` | 8 |

## Prerequisites

- SOMA binary built: `cd ../soma-next && cargo build --release`
- Postgres port built: `cd ../soma-ports && cargo build --release`
- HelperBook database running (for Level 2 postgres tests): `cd ../soma-helperbook && docker compose up -d postgres`
- Database seeded: `cd ../soma-helperbook && scripts/setup-db.sh && scripts/seed-db.sh`

## Setup

```bash
# Copy binary and port
cp ../soma-next/target/release/soma bin/soma
cp ../soma-ports/target/release/libsoma_port_postgres.dylib packs/postgres/

# macOS: remove quarantine
xattr -d com.apple.quarantine bin/soma 2>/dev/null
codesign -fs - bin/soma
```

## Run tests

```bash
# All levels
scripts/test-all.sh

# Individual levels
node test-level1.js   # Transport only (no DB needed)
node test-level2.js   # Delegation (needs helperbook DB)
node test-level3.js   # Transfer (needs helperbook DB)
```

## Architecture

```
                TCP 9100
  ┌─────────┐ ◄──────────── ┌─────────┐
  │ Peer A  │                │ Peer B  │
  │ (fs)    │                │ (pg)    │
  │ --listen│ ──────────────►│ --peer  │
  │ repl    │   wire proto   │ --mcp   │
  └─────────┘                └─────────┘
       │                          │
   filesystem                 postgres
     port                      port
```

**Peer A**: Filesystem pack, TCP listener. Receives remote skill invocations, goal submissions, and schema/routine transfers.

**Peer B**: Postgres pack, MCP server. Connects to A as `--peer`. LLM (or test script) talks to B via MCP, B delegates to A via `invoke_remote_skill` or `transfer_routine`.

## Wire protocol

4-byte big-endian length prefix + JSON payload. Message types:

- `invoke_skill` → `skill_result`
- `submit_goal` → `goal_result`
- `transfer_routine` → `routine_ok`
- `transfer_schema` → `schema_ok`
- `ping` → `pong`

## New MCP tools (added for s2s)

| Tool | Description |
|------|-------------|
| `list_peers` | List connected remote SOMA peers |
| `invoke_remote_skill` | Invoke a skill on a remote peer by peer_id |
| `transfer_routine` | Push a locally compiled routine to a remote peer |
