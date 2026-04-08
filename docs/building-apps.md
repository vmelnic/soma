# Building Applications with SOMA

## Overview

SOMA applications are built by conversation. You talk to an LLM (Claude, ChatGPT, Ollama, etc.), the LLM drives SOMA through MCP tool calls, and SOMA executes against real databases and services. There is no framework, no boilerplate, and no build tools for the application itself.

What you need:

- **SOMA binary** -- built from `soma-core/` (Rust)
- **Plugins** -- built from `soma-plugins/` (Rust)
- **An LLM** -- any MCP-compatible LLM (Claude Desktop, Cursor, etc.)
- **Docker** -- for PostgreSQL and Redis

The LLM brings intelligence (understanding your requests, decomposing them into structured calls). SOMA brings execution (database operations, auth, caching) and permanent memory (schema, decisions, experience). You can switch LLMs between sessions -- SOMA holds the state.

---

## Step 1: Project Setup

Create a project directory with the infrastructure files.

```bash
mkdir my-app && cd my-app
```

Create `docker-compose.yml` for PostgreSQL and Redis. Copy from `soma-helperbook/docker-compose.yml` and change the database name:

```yaml
name: my-app

services:
  postgres:
    image: postgres:17
    ports: ["5432:5432"]
    environment:
      POSTGRES_USER: soma
      POSTGRES_PASSWORD: soma
      POSTGRES_DB: myapp
    volumes: [pgdata:/var/lib/postgresql/data]
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U soma -d myapp"]
      interval: 5s
      timeout: 5s
      retries: 5

  redis:
    image: redis:7-alpine
    ports: ["6379:6379"]
    healthcheck:
      test: ["CMD", "redis-cli", "ping"]
      interval: 5s
      timeout: 5s
      retries: 5

volumes:
  pgdata:
```

Start the services:

```bash
docker compose up -d --wait
```

Both services will report healthy before the command returns.

---

## Step 2: Build and Configure SOMA

### Build the binaries

From the SOMA repository root:

```bash
# Build the runtime
cd soma-core
cargo build --release
# Binary at: soma-core/target/release/soma

# Build the plugins
cd ../soma-plugins
cargo build --release
# Plugin libraries at: soma-plugins/target/release/*.dylib (macOS) or *.so (Linux)
```

### Create soma.toml

Create the SOMA configuration file in your project directory. Start from `soma-helperbook/soma.toml` and adapt the database name and paths:

```toml
[soma]
id = "my-app-backend"
log_level = "info"
plugins_directory = "../soma-plugins/target/release"

[mind]
model_dir = "../models"
max_program_steps = 32
temperature = 0.8

[mind.lora]
adaptation_enabled = true
adapt_every_n_successes = 3

[memory]
checkpoint_dir = "./checkpoints"
auto_checkpoint = true

[protocol]
bind = "127.0.0.1:9999"

[mcp]
transport = "stdio"
enabled = true

[security]
require_auth = false

[plugins.postgres]
host = "localhost"
port = 5432
database = "myapp"
username = "soma"
password_env = "SOMA_PG_PASSWORD"

[plugins.redis]
url = "redis://localhost:6379/0"

[plugins.auth]
session_ttl_hours = 720
otp_length = 6
db_host = "localhost"
db_port = 5432
db_name = "myapp"
db_user = "soma"
db_password_env = "SOMA_PG_PASSWORD"

[plugins.crypto]
```

All fields have defaults. Override only what you need. Override order: defaults < TOML < env vars (`SOMA_*`) < CLI flags. See `soma-core/soma.toml.example` for every field with comments.

---

## Step 3: Start SOMA

Set environment variables and start the binary:

```bash
export SOMA_PG_PASSWORD=soma

cd /path/to/soma/soma-core
cargo run --release --bin soma -- \
    --config /path/to/my-app/soma.toml \
    --model /path/to/soma/models \
    --mcp
```

SOMA loads the Mind, discovers plugins, connects to PostgreSQL and Redis, and starts the MCP server on stdio.

For convenience, create `scripts/start-mcp.sh` -- see `soma-helperbook/scripts/start-mcp.sh` for the pattern: set `SOMA_PG_PASSWORD`, then `cargo run --release --bin soma -- --config ... --model ... --mcp`.

---

## Step 4: Connect Your LLM

SOMA uses MCP (Model Context Protocol) over stdio. The LLM spawns SOMA as a subprocess and communicates via JSON-RPC 2.0 on stdin/stdout.

### Claude Desktop / Cursor

Add SOMA as an MCP server. In Claude Desktop's MCP config (or Cursor's equivalent):

```json
{
  "mcpServers": {
    "my-app": {
      "command": "/path/to/soma/soma-core/target/release/soma",
      "args": ["--config", "/path/to/my-app/soma.toml",
               "--model", "/path/to/soma/models", "--mcp"],
      "env": { "SOMA_PG_PASSWORD": "soma" }
    }
  }
}
```

### First interaction

The first thing the LLM does when connecting is call `soma.get_state()`. This returns a full snapshot: loaded plugins, database schema (if any tables exist), decisions, recent activity, and health. One call gives the LLM complete context.

```
You: "What do you see?"

LLM calls: soma.get_state()
LLM: "I see a SOMA with postgres, redis, auth, and crypto plugins loaded.
      The database is empty -- no tables yet. What would you like to build?"
```

---

## Step 5: Design Your Data Model

Describe what you are building. The LLM translates your description into SQL and executes it through SOMA.

```
You: "I'm building a task management app. I need users, projects,
     and tasks. Users belong to projects. Tasks have assignees and statuses."

LLM calls: soma.postgres.execute("CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email VARCHAR(255) UNIQUE NOT NULL,
    name TEXT NOT NULL, created_at TIMESTAMP DEFAULT NOW())")

LLM calls: soma.postgres.execute("CREATE TABLE projects (...)")
LLM calls: soma.postgres.execute("CREATE TABLE project_members (...)")
LLM calls: soma.postgres.execute("CREATE TABLE tasks (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID REFERENCES projects(id),
    title TEXT NOT NULL,
    assignee_id UUID REFERENCES users(id),
    status VARCHAR(20) DEFAULT 'todo'
        CHECK (status IN ('todo','in_progress','done')),
    due_date TIMESTAMP, created_at TIMESTAMP DEFAULT NOW())")

LLM calls: soma.record_decision({
    what: "Created 4 tables: users, projects, project_members, tasks",
    why: "Core data model for task management with project-based access",
    related_tables: ["users", "projects", "project_members", "tasks"]
})
```

After creating tables, verify:

```
LLM calls: soma.get_schema()
```

This returns all tables with columns, types, constraints, and row counts.

Every structural change should be followed by `soma.record_decision()`. This creates permanent institutional memory -- future LLM sessions (even with a different LLM) can call `soma.get_decisions()` to understand not just what exists but why.

---

## Step 6: Write the Schema File

Once the LLM has created your tables, export the schema to `schema.sql` so you can recreate the database from scratch. Wrap it in `BEGIN` / `COMMIT`. See `soma-helperbook/schema.sql` for the pattern (19 tables, indexes, seed categories, migration tracking).

Create `scripts/setup-db.sh` to apply it -- start Docker, then `PGPASSWORD=soma psql -h localhost -U soma -d myapp -f schema.sql`. See `soma-helperbook/scripts/setup-db.sh` for the complete script.

---

## Step 7: Build Business Logic

The LLM drives all business operations through SOMA's plugin conventions. Every loaded plugin's conventions are automatically exposed as MCP tools with the naming pattern `soma.{plugin}.{convention}`.

### Common patterns

**CRUD operations** via the postgres plugin:

```
soma.postgres.execute("INSERT INTO tasks (...) VALUES (...)")
soma.postgres.query("SELECT * FROM tasks WHERE project_id = $1", [project_id])
soma.postgres.execute("UPDATE tasks SET status = $1 WHERE id = $2", ["done", task_id])
soma.postgres.execute("DELETE FROM tasks WHERE id = $1", [task_id])
```

**Caching** via postgres + redis:

```
soma.postgres.query("SELECT * FROM projects WHERE id = $1", [id])
soma.redis.set("project:{id}", result, 3600)
```

**Auth flows** via the auth plugin:

```
soma.auth.generate_otp(phone)
soma.auth.verify_otp(phone, code)
soma.auth.create_session(user_id)
```

**Natural language intents** via the Mind:

```
soma.intent("list all tasks assigned to user X that are overdue")
```

The Mind generates a program (sequence of plugin convention calls) and executes it. This requires the Mind to be trained on your plugin conventions -- see Step 11.

Direct plugin calls (`soma.postgres.query(...)`) bypass the Mind -- the LLM constructs the exact SQL and parameters. This is the most reliable path for building. `soma.intent("...")` sends natural language to the Mind, which generates a program. It requires training data (see Step 11) and is mainly useful for end-user-facing features.

---

## Step 8: Build the Frontend

SOMA applications use a simple architecture for the web frontend: an Express server bridges HTTP requests to SOMA's MCP interface.

### Directory structure

```
frontend/
  server.js         # Express server -- spawns SOMA, bridges HTTP to MCP
  index.html        # Single-page app shell
  css/app.css       # Styles
  js/
    api.js          # MCP call helper
    app.js          # App initialization
    components/     # UI components
```

### Express bridge (server.js)

The Express server spawns SOMA as a child process and forwards JSON-RPC messages between the browser and SOMA's stdin/stdout. The key parts:

1. **Spawn SOMA** with `--config`, `--model`, `--mcp` flags and `stdio: ['pipe', 'pipe', 'pipe']`
2. **Buffer stdout** -- SOMA writes one JSON-RPC response per line; parse each complete line and match by request `id`
3. **MCP initialization** -- on first request, send `initialize` + `notifications/initialized` before forwarding tool calls
4. **Bridge endpoint** -- `POST /api/mcp` forwards the JSON-RPC body to SOMA's stdin, waits for the matching response, returns it

See `soma-helperbook/frontend/server.js` for the complete working implementation (170 lines). Copy it and change the paths to your `soma.toml` and binary.

### Frontend (index.html)

No build step. Plain HTML + Tailwind CSS from CDN + vanilla JavaScript. See `soma-helperbook/frontend/index.html` for the structure -- it loads Tailwind, Google Fonts, and Lucide Icons from CDN, then includes component scripts.

### API helper (js/api.js)

```javascript
let requestId = 1;

async function mcpCall(toolName, args = {}) {
  const response = await fetch('/api/mcp', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: requestId++,
      method: 'tools/call',
      params: { name: toolName, arguments: args }
    })
  });
  const data = await response.json();
  if (data.error) throw new Error(data.error.message);
  // MCP tool results are in data.result.content[0].text
  const text = data.result?.content?.[0]?.text;
  try { return JSON.parse(text); } catch { return text; }
}
```

Then from any component:

```javascript
// Fetch tasks
const tasks = await mcpCall('soma.postgres.query', {
  sql: 'SELECT * FROM tasks WHERE project_id = $1 ORDER BY created_at DESC',
  params: [projectId]
});

// Create a task
await mcpCall('soma.postgres.execute', {
  sql: 'INSERT INTO tasks (project_id, title, assignee_id) VALUES ($1, $2, $3)',
  params: [projectId, 'Fix login bug', userId]
});
```

### Run the frontend

```bash
cd frontend
npm init -y
npm install express
node server.js
# Open http://localhost:8080
```

---

## Step 9: Seed Test Data

Create `seed.sql` with INSERT statements for test users, projects, tasks, etc. See `soma-helperbook/seed.sql` for the pattern. Create `scripts/seed-db.sh` to apply it (same structure as `setup-db.sh` but running `seed.sql` instead of `schema.sql`).

Or have the LLM seed data directly through MCP:

```
You: "Add some test users and a few sample tasks"

LLM calls: soma.postgres.execute("INSERT INTO users ...")
LLM calls: soma.postgres.execute("INSERT INTO tasks ...")
```

Verify with `soma.get_schema()` -- it returns row counts for each table.

---

## Step 10: Iterate

Add features by talking to the LLM. The pattern is always: describe the feature, LLM creates/modifies tables, LLM records the decision, update the frontend.

**Switching LLMs**: Start a new session with a different LLM. The new LLM calls `soma.get_state()` and has full context -- schema, decisions, recent activity. No onboarding required.

**Checkpointing**: Before risky changes, `soma.checkpoint("before-refactor")`. If something breaks: `soma.get_checkpoints()` then `soma.restore_checkpoint(path)`.

---

## Step 11: Train the Mind (Optional)

If you want `soma.intent()` to handle natural language requests, train the Mind on your plugin conventions plus domain-specific examples. Create `domain/training.json` (see `soma-helperbook/domain/helperbook-training.json` for the format), then:

```bash
pip install -e /path/to/soma/soma-synthesizer
soma-synthesize validate --plugins /path/to/soma/soma-plugins
soma-synthesize train \
    --plugins /path/to/soma/soma-plugins \
    --domain /path/to/my-app/domain/training.json \
    --output /path/to/soma/models
```

Requires Python 3.10+ and PyTorch 2.0+. For LLM-driven development (where the LLM calls plugin conventions directly), Mind training is not required.

---

## Project Structure Reference

```
my-app/
  soma.toml              # SOMA configuration
  docker-compose.yml     # PostgreSQL + Redis
  schema.sql             # Database schema (all CREATE TABLE statements)
  seed.sql               # Test data
  checkpoints/           # SOMA state snapshots (auto-managed)
  scripts/
    setup-db.sh          # Apply schema to PostgreSQL
    seed-db.sh           # Seed test data
    clean-db.sh          # Truncate all tables
    start-mcp.sh         # Start SOMA in MCP mode
    synthesize.sh        # Train Mind with plugin + domain data
  domain/
    training.json        # Domain-specific training data for Mind synthesis
  frontend/
    server.js            # Express bridge: HTTP to SOMA MCP
    index.html           # Single-page app shell
    package.json         # Express dependency
    css/
      app.css            # Styles
    js/
      api.js             # MCP call helper
      app.js             # App initialization
      components/        # UI components
```

---

## Common Patterns

### Add a new feature

1. Describe the feature to the LLM
2. LLM creates/modifies tables via `soma.postgres.execute()`
3. LLM verifies with `soma.get_schema()`
4. LLM records the decision via `soma.record_decision()`
5. Update frontend JavaScript to display the new data

### Add a plugin

1. Add plugin configuration to `soma.toml` under `[plugins.name]`
2. Restart SOMA to load the new plugin
3. Verify: LLM calls `soma.get_conventions()` to see the new conventions
4. Use new conventions via MCP (e.g., `soma.geo.distance()`)

Plugin installation via `soma.install_plugin(name)` is specified but currently requires the plugin binary to be present in the configured `plugins_directory`.

### Checkpoint and restore

1. `soma.checkpoint("before-migration")` -- save current state
2. Make changes
3. If something breaks: `soma.restore_checkpoint(path)` -- restore

Checkpoints save LoRA weights, experience buffer, and adaptation state. They do not back up the database -- use `pg_dump` for that.

### Debug

| What to check | MCP tool |
|--------------|----------|
| Overall system state | `soma.get_state()` |
| System health (memory, CPU, errors) | `soma.get_health()` |
| Database tables and row counts | `soma.get_schema()` |
| Recent executions and errors | `soma.get_recent_activity(10)` |
| Why something was built | `soma.get_decisions()` |
| Available plugins and conventions | `soma.get_plugins()`, `soma.get_conventions()` |
| Connected SOMA peers | `soma.get_peers()` |
| Current configuration | `soma.get_config()` |
| Prometheus metrics | `soma.get_metrics()` |

For live signal capture between SOMAs:

```bash
cargo run --bin soma-dump -- 127.0.0.1:9999
```

### Record decisions

After any structural change, call `soma.record_decision({what, why, related_tables})`. This is how SOMA sessions become portable across LLMs and time -- a new LLM calls `soma.get_decisions()` and understands the full history.

---

## MCP Tools Quick Reference

All tools are listed in `soma-core/src/mcp/tools.rs`. See `docs/mcp-interface.md` for full details.

**State tools** (read-only): `soma.get_state`, `soma.get_schema`, `soma.get_plugins`, `soma.get_conventions`, `soma.get_health`, `soma.get_recent_activity`, `soma.get_decisions`, `soma.get_checkpoints`, `soma.get_peers`, `soma.get_config`, `soma.get_experience`, `soma.get_metrics`, `soma.get_business_rules`, `soma.get_render_state`

**Action tools**: `soma.intent`, `soma.checkpoint`, `soma.restore_checkpoint`, `soma.record_decision`, `soma.confirm`, `soma.install_plugin`, `soma.uninstall_plugin`, `soma.configure_plugin`, `soma.shutdown`, `soma.render_view`, `soma.update_view`, `soma.reload_design`

**Plugin conventions**: Every loaded plugin convention is exposed as `soma.{plugin}.{convention}` (e.g., `soma.postgres.query`, `soma.redis.set`, `soma.auth.verify_otp`). Call `soma.get_conventions()` for the full list.

---

## Security

For production, enable auth tokens in `soma.toml`:

```toml
[security]
require_auth = true
admin_token_env = "SOMA_MCP_ADMIN_TOKEN"
builder_token_env = "SOMA_MCP_BUILDER_TOKEN"
viewer_token_env = "SOMA_MCP_VIEWER_TOKEN"
```

| Role | Query state | Execute actions | Install/restore |
|------|------------|----------------|----------------|
| admin | Yes | Yes | Yes |
| builder | Yes | Yes | No |
| viewer | Yes | No | No |

Destructive actions (DROP TABLE, DELETE without WHERE, plugin uninstall) require two-step confirmation via `soma.confirm(action_id)`.

---

## HelperBook as Reference

The `soma-helperbook/` directory is a complete working example. It implements a service marketplace with:

- 19 database tables (users, connections, chats, messages, appointments, reviews, etc.)
- PostgreSQL + Redis via Docker
- 4 plugins (postgres, redis, auth, crypto)
- Express bridge serving a vanilla JS frontend
- Setup, seed, clean, start, and synthesize scripts

To run it:

```bash
cd soma-helperbook
docker compose up -d --wait
scripts/setup-db.sh
scripts/seed-db.sh
cd ../soma-plugins && cargo build --release
cd ../soma-helperbook/frontend && npm install && node server.js
# Open http://localhost:8080
```

Study its structure when building your own application. The patterns are directly transferable.
