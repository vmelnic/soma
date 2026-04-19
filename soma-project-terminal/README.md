# soma-project-terminal

A multi-user SOMA-native web platform with a Fallout-inspired
terminal UI. Operators log in with a magic link, create
**contexts** (scoped conversations), and talk to a tool-calling
LLM brain that invokes real SOMA ports via `invoke_port` over MCP.
No application source code per context. No client-side framework.
No wasm in the browser.

This is the inversion of `soma-project-helperbook`. HelperBook is an
Express app that calls SOMA for its backend. `soma-project-terminal`
**is SOMA**: the Node layer is a thin HTTP gateway + MCP stdio
client, and every backend side effect -- database reads, magic-link
email, token hashing -- goes through soma-ports via
`invoke_port`. The browser is a dumb chat client.

## What you get

| Capability | Where it lives |
|---|---|
| Magic-link auth (email -> token -> session cookie) | `backend/auth.mjs` via `crypto.random_string` + `crypto.sha256` + `postgres` + `smtp` ports |
| Context registry -- operator's conversations | `backend/contexts.mjs`, `contexts` table |
| Per-context chat with tool-calling brain | `backend/messages.mjs` + `backend/brain.mjs`, `messages` table |
| Voice input (Whisper) | `backend/brain.mjs` `transcribeAudio`, mic button in chat |

## Architecture

```
+-----------------------------------------------------------------+
| Browser tab -- Fallout terminal UI                              |
|                                                                 |
|   Full-width chat panel. VT323 font, phosphor green,           |
|   CRT scanlines, vignette. One <div> transcript,               |
|   one <input>, one mic button. Zero wasm, zero framework.      |
+-----------------------+-----------------------------------------+
                        | HTTP + cookies
                        v
+-----------------------------------------------------------------+
| Node HTTP gateway (backend/server.mjs)                          |
|                                                                 |
|   Auth        /api/auth/{request-link,verify,logout}  /api/me   |
|   Contexts    /api/contexts[/:id]                               |
|   Chat        /api/contexts/:id/messages       GET / POST       |
|   Voice       /api/transcribe                  POST (whisper-1) |
|   Health      /api/health                                       |
|                                                                 |
|   Zero runtime dependencies. Backend package.json is empty.     |
|   SomaMcpClient spawns bin/soma --mcp --pack auto once,         |
|   speaks JSON-RPC 2.0 over stdio, exposes invokePort().         |
+-----------------------+-----------------------------------------+
                        | MCP stdio (tools/call invoke_port)
                        v
+-----------------------------------------------------------------+
| soma-next (spawned subprocess, --pack auto)                     |
|   Auto-discovers all port dylibs from SOMA_PORTS_PLUGIN_PATH.  |
|   No manifest needed. Port catalog built from loaded libraries. |
|   Port dylibs in bin/:                                          |
|     - libsoma_port_crypto.dylib                                 |
|     - libsoma_port_postgres.dylib                               |
|     - libsoma_port_smtp.dylib                                   |
+-----------------------+-----------------------------------------+
                        |
                        v
+-----------------------------------------------------------------+
| docker-compose: postgres(5433) + redis(6380) + mailcatcher(1025)|
|                                                                 |
| Tables:                                                         |
|   users         (id, email, created_at, last_login)             |
|   magic_tokens  (token_hash, email, expires_at, used_at)        |
|   sessions      (token_hash, user_id, expires_at, revoked_at)   |
|   contexts      (id, user_id, name, description, kind)          |
|   messages      (id, context_id, role, content, created_at)     |
+-----------------------------------------------------------------+
```

Only the Node gateway + frontend live in this project. The runtime
is `soma-next`, the ports are `soma-ports`, the database + mail
sink are off-the-shelf containers. Nothing is reimplemented here
that already exists elsewhere in the repo.

## Conversation-first architecture

One master soma-next process with auto-discovered ports. Every
operator context is a scoped conversation against that single
runtime. Zero per-context pack generation, zero per-context
artifacts.

- **Chat brain** (`backend/brain.mjs`) -- model family auto-detected:
  chat models (`gpt-4o-mini`) get `temperature` + `max_tokens`;
  reasoning models (`gpt-5-mini`, `o1`, `o3`) get `reasoning_effort`
  with no max_completion_tokens. Operator flips `OPENAI_CHAT_MODEL`
  in `.env` and the wrapper does the right thing.
- **Single tool: `invoke_port`** -- the brain discovers capabilities
  through the system prompt (port catalog baked in at startup via
  one `list_ports` call), not through introspection tool calls.
- **Port alias resolution** -- `SomaMcpClient` builds a short-name
  map at startup (`postgres` -> `soma.ports.postgres`) so backend
  code uses clean names regardless of the port's internal ID.
- **Per-context data isolation** via prompt-taught SQL namespacing --
  the system prompt instructs the brain to prefix stored artifacts
  with a per-context namespace string.

## `--pack auto` mode

The terminal project uses soma-next's `--pack auto` flag, which
auto-discovers all port libraries in the search path without any
manifest file:

```
SOMA_PORTS_PLUGIN_PATH=./bin soma --mcp --pack auto
```

This scans `bin/` for all `libsoma_port_*.dylib` files, loads each
one, calls `spec()` to get the port's self-reported capabilities,
and registers them. Built-in ports (filesystem, http) are always
included. The LLM calls `list_ports` to discover what's available
and `invoke_port` to use it.

## Prerequisites

- Rust stable with `soma-next` and `soma-ports` already built in
  release mode:
  ```bash
  (cd ../soma-next  && cargo build --release)
  (cd ../soma-ports && cargo build --workspace --release)
  ```
- Node 20+ (we use the built-in `--env-file` flag; no dotenv package).
- Docker Desktop for the postgres / redis / mailcatcher stack.

## First run

```bash
cd soma-project-terminal
cp .env.example .env                     # edit OPENAI_API_KEY
./scripts/copy-binaries.sh               # cp soma + 3 dylibs -> bin/
./scripts/start.sh                       # docker compose up -d --wait
./scripts/setup-db.sh                    # psql < schema.sql
./scripts/start-backend.sh               # node --env-file=.env backend/server.mjs
```

Open [http://localhost:8765/](http://localhost:8765/). Enter an
email, pick up the magic link from
[http://localhost:1080](http://localhost:1080) (Mailcatcher), click
it. Create a context, start talking.

## SOMA-native design

The hard rule: **every backend side effect goes through
`SomaMcpClient.invokePort(portId, capabilityId, input)`**. No
direct `pg`, no `nodemailer`, no `crypto.randomBytes`.

Consequences:

- `backend/package.json` has an empty `dependencies` object. The
  only `devDependency` is `@playwright/test`.
- Every SQL query is a string passed to
  `invokePort("postgres", "query"|"execute", {sql, params})`.
- Magic-link tokens: `invokePort("crypto", "random_string")` ->
  `invokePort("crypto", "sha256")` -> stored as hex digest.
- Email: `invokePort("smtp", "send_plain", {to, subject, body})`.

The OpenAI chat brain and Whisper are the only network calls that
don't go through SOMA (no LLM port yet).

## File layout

```
soma-project-terminal/
+-- docker-compose.yml          # postgres(5433) + redis(6380) + mailcatcher(1025)
+-- schema.sql                  # users, sessions, magic_tokens, contexts, messages
+-- .env.example                # POSTGRES_URL, SMTP_*, OPENAI_*, BRAIN_FAKE, TTLs
+-- .gitignore                  # bin/, node_modules/, test-results/
+-- package.json                # Playwright devDep (no runtime deps)
+-- playwright.config.js        # webServer auto-starts ./scripts/start-backend.sh
+-- scripts/
|   +-- start.sh                # docker compose up -d --wait
|   +-- setup-db.sh             # psql < schema.sql
|   +-- clean-db.sh             # TRUNCATE all tables
|   +-- copy-binaries.sh        # cp soma + 3 dylibs -> bin/
|   +-- start-backend.sh        # node --env-file=.env backend/server.mjs
+-- backend/                    # zero runtime dependencies
|   +-- server.mjs              # HTTP gateway, preflight, all routes
|   +-- soma-mcp.mjs            # SomaMcpClient (--pack auto, alias resolution)
|   +-- auth.mjs                # magic-link flow (crypto + postgres + smtp)
|   +-- contexts.mjs            # context CRUD
|   +-- messages.mjs            # chat transcript CRUD
|   +-- brain.mjs               # chatCompletion, runChatTurn, transcribeAudio
+-- bin/                        # gitignored, populated by copy-binaries.sh
|   +-- soma
|   +-- libsoma_port_crypto.dylib
|   +-- libsoma_port_postgres.dylib
|   +-- libsoma_port_smtp.dylib
+-- frontend/
|   +-- index.html              # Fallout terminal shell
|   +-- app.mjs                 # view router, chat, mic
|   +-- terminal.css            # VT323 + phosphor glow + CRT scanlines
+-- tests/                      # Playwright suite (34 tests, ~9s headless)
    +-- global-setup.mjs        # targeted DELETE, not TRUNCATE
    +-- helpers.mjs             # loginAs() -- full magic-link round trip
    +-- mailcatcher.mjs         # HTTP API client for pulling tokens
    +-- auth.spec.js            # magic-link auth, styling, replay attack
    +-- contexts.spec.js        # CRUD + cross-tenant isolation + UI
    +-- chat.spec.js            # transcript + tool-calling + ::tool escape
    +-- transcribe.spec.js      # Whisper route + mic UI
```

## Testing

Tests run headless Chromium against the real stack. Playwright's
`webServer` auto-starts the backend (which spawns `soma --mcp
--pack auto`) and `globalSetup` cleans test data before the suite.

```bash
./scripts/start.sh               # once -- docker stack
./scripts/setup-db.sh            # once -- schema
./scripts/copy-binaries.sh       # once -- native binaries
npm install                      # once -- Playwright
npx playwright test
```

The suite forces `BRAIN_FAKE=1` via `playwright.config.js` so
tests never burn OpenAI quota. Fake mode has a `::tool` escape
trigger that drives real MCP tool calls deterministically.

**34 tests, ~9 seconds, headless Chromium.**

| Suite | What it asserts |
|---|---|
| `auth.spec.js` | Fallout styling (VT323, phosphor green, CRT scanlines), magic-link round trip, replay-attack rejection |
| `contexts.spec.js` | CRUD + recent-first ordering + cross-user 404 + UI |
| `chat.spec.js` | Transcript + tool-calling via `::tool` escape + cross-user isolation + UI round trip |
| `transcribe.spec.js` | Whisper route + stubbed MediaRecorder UI |

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `error: missing required binaries` | `bin/` is empty | `./scripts/copy-binaries.sh` |
| `port 'X' is not registered` | Port dylib not in `bin/` or `SOMA_PORTS_PLUGIN_PATH` not set | Check `ls bin/libsoma_port_*.dylib` |
| `postgres.execute failed: error serializing parameter N` | Postgres port serializes everything as TEXT | Use `$N::text::uuid` for UUIDs, `NOW() + INTERVAL` for timestamps |
| `smtp.send_plain failed: STARTTLS is not supported` | Mailcatcher doesn't speak TLS | Set `SOMA_SMTP_STARTTLS=false` in `.env` |
| Browser shows "awaiting verification" forever | Magic-link email didn't arrive | Check http://localhost:1080 (Mailcatcher) |

## OpenAI parameter families

| Parameter | Chat (`gpt-4o-mini`) | Reasoning (`gpt-5-mini`, `o1`, `o3`) |
|---|---|---|
| `temperature` | accepted | **rejected, API 400s** |
| `max_tokens` | accepted | rejected |
| `reasoning_effort` | rejected | accepted (`"low"` / `"medium"` / `"high"`) |

`backend/brain.mjs` auto-detects the model family from
`OPENAI_CHAT_MODEL` and sends the right parameters.
