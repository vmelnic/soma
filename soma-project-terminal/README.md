# soma-project-terminal

**The real next-generation web project.** A multi-user SOMA-native
platform where users create and interact with their own *contexts*
(pack manifests) inside a Fallout-terminal-inspired UI. Each
context is a full SOMA runtime — its own skills, its own learned
routines, its own memory — and the LLM composes behavior from
natural-language intent.

This is the inversion of `soma-helperbook`. HelperBook is an Express
frontend that calls SOMA for its backend. `soma-project-terminal`
**is SOMA** — the Node layer is a thin HTTP gateway + MCP stdio
client, and every side effect (Postgres queries, SMTP delivery,
cryptographic primitives) goes through `soma-next`'s existing port
catalog via `invoke_port`.

## Current status — commit 1 shipped

| Commit | What it adds |
|---|---|
| **1** ✅ | docker-compose (postgres + redis + mailcatcher), schema (users + sessions + magic_tokens), Fallout terminal UI shell, magic-link auth flow via `crypto.random_string` + `crypto.sha256` + `postgres.query` + `smtp.send_plain` — all routed through a spawned `soma --mcp` process |
| 2 | Context registry pack (`context.create` / `context.list_mine` / `context.load`), context table, post-login context list UI |
| 3 | Conversational chat shell + wasm runtime + per-context brain prompts (`gpt-4o-mini`) |
| 4 | Dynamic pack loading — `soma_load_pack` wasm entry + runtime hot-swap between contexts |
| 5 | Per-context memory isolation (namespaced episode/schema/routine stores) |
| 6 | LLM-to-PackSpec (`gpt-5-mini`) — user describes a context in natural language, brain emits a valid PackSpec |
| 7 | Voice input (mic button + Whisper + SpeechRecognition fallback) |

## Architecture

```
┌────────────────────────────────────────────────────────────────┐
│ browser (Fallout terminal UI)                                  │
│   - /login        — magic-link email entry                     │
│   - /contexts     — list + new (commit 2+)                     │
│   - /c/<id>       — conversational chat with a loaded context  │
└───────────┬────────────────────────────────────────────────────┘
            │ HTTP cookies + /api/*
            ▼
┌────────────────────────────────────────────────────────────────┐
│ Node HTTP gateway (backend/server.mjs)                         │
│   - static files from frontend/                                │
│   - /api/auth/{request-link,verify,logout} + /api/me           │
│   - /api/invoke_port   (commit 2+)                             │
│   - /api/brain         (commit 3+, gpt-4o-mini)                │
│   - /api/generate_pack (commit 6+, gpt-5-mini)                 │
│                                                                │
│   SomaMcpClient — spawns `soma --mcp --pack platform.json`,    │
│   speaks JSON-RPC 2.0 over stdio, exposes invokePort()         │
└───────────┬────────────────────────────────────────────────────┘
            │ MCP stdio (tools/call invoke_port)
            ▼
┌────────────────────────────────────────────────────────────────┐
│ soma-next (spawned subprocess)                                 │
│   Pack: packs/platform/manifest.json                           │
│   Ports loaded via dylibs in bin/:                             │
│     - crypto   (libsoma_port_crypto.dylib)                     │
│     - postgres (libsoma_port_postgres.dylib)                   │
│     - smtp     (libsoma_port_smtp.dylib)                       │
└───────────┬────────────────────────────────────────────────────┘
            │
            ▼
┌────────────────────────────────────────────────────────────────┐
│ docker-compose: postgres (5433) + redis (6380) + mailcatcher   │
│                                                                │
│ tables (commit 1):                                             │
│   users          (id, email, created_at, last_login)           │
│   sessions       (token_hash, user_id, expires_at, ...)        │
│   magic_tokens   (token_hash, email, expires_at, used_at)      │
└────────────────────────────────────────────────────────────────┘
```

**Only the Node gateway + frontend live in this project.** The
runtime is soma-next, the ports are soma-ports, the database /
mail sink are off-the-shelf containers. Nothing is reimplemented
here that already exists elsewhere in the repo.

## Prerequisites

- Rust stable, with `soma-next` + `soma-ports` already built:
  ```bash
  (cd ../soma-next  && cargo build --release)
  (cd ../soma-ports && cargo build --workspace --release)
  ```
- Node 20+ (we use the built-in `--env-file` flag; no dotenv package).
- Docker Desktop for the postgres / redis / mailcatcher stack.

## First run

Three terminals.

### Terminal 0 — one-time setup

```bash
cd soma-project-terminal
cp .env.example .env          # no secrets needed for commit 1 dev stack
./scripts/copy-binaries.sh    # copies soma + crypto/postgres/smtp dylibs into bin/
./scripts/start.sh            # brings up docker: postgres + redis + mailcatcher
./scripts/setup-db.sh         # applies schema.sql
```

### Terminal 1 — backend (Node + spawned soma-next)

```bash
./scripts/start-backend.sh
# [run-brain] (not used — different script)
# [soma-mcp] soma-next MCP server ready
# [http] soma-project-terminal listening on http://127.0.0.1:8765
# [http] SOMA_POSTGRES_URL=postgres://soma:soma@localhost:5433/soma_terminal
# [http] SOMA_SMTP=localhost:1025
```

The backend spawns `bin/soma --mcp --pack packs/platform/manifest.json`
as a child process and holds the stdio handle for the lifetime of the
server. Every HTTP request's side effects route through MCP
`invoke_port` calls to that child.

### Terminal 2 — browser

Open http://localhost:8765/ — the Fallout terminal boots, asks for
an operator email, and dispatches a magic-link message. Check
Mailcatcher for the link:

```
http://localhost:1080
```

Click the link → the terminal authenticates and shows the
post-login view. Commit 2+ wires the context list here.

## What commit 1 proved

1. **SOMA-native plumbing**. Every backend side effect goes through
   `SomaMcpClient.invokePort(portId, capabilityId, input)`:
   - `crypto.random_string` generates raw magic/session tokens
   - `crypto.sha256` hashes them before storing (zero plaintext at rest)
   - `postgres.query` reads rows
   - `postgres.execute` writes rows
   - `smtp.send_plain` delivers magic-link emails
   `pg` and `nodemailer` are **not** in the dependency graph.

2. **Session persistence survives soma-next restarts.** Sessions
   live in Postgres, not in the auth port's in-memory HashMap. The
   auth port is available in this pack but we deliberately don't
   use it for session lifecycle — persistence is a database
   concern, not a port concern.

3. **Magic-link flow is end-to-end verified:**
   - Request `/api/auth/request-link` → token inserted + email dispatched
   - Verify `/api/auth/verify?token=X` → session created, httpOnly cookie set
   - `/api/me` → current user resolved from session cookie or Bearer header
   - `/api/auth/logout` → session revoked (not deleted — audit trail preserved)
   - Replay attack (verify same token twice) → rejected
   - Post-logout `/api/me` → 401 unauthenticated

4. **Typed-parameter quirks around the postgres port are documented
   in code**, not papered over. The port serializes every param as
   TEXT, which means:
   - TIMESTAMPTZ columns: use `NOW() + INTERVAL '...'` in SQL.
   - UUID columns: use `$N::text::uuid` (the double cast forces
     Postgres to infer the param as TEXT, so tokio-postgres accepts
     the `&str` bind).

## File layout

```
soma-project-terminal/
├── docker-compose.yml        # postgres + redis + mailcatcher (all with non-default ports)
├── schema.sql                # users, sessions, magic_tokens
├── .env.example              # POSTGRES_URL, SMTP_*, OPENAI_*, TTLs, public base URL
├── .gitignore                # bin/, node_modules/, test-results/
├── README.md
├── scripts/
│   ├── start.sh              # docker compose up -d --wait
│   ├── setup-db.sh           # psql < schema.sql
│   ├── clean-db.sh           # TRUNCATE all tables (dev only)
│   ├── copy-binaries.sh      # cp soma-next binary + 3 dylibs → bin/
│   └── start-backend.sh      # node --env-file=.env backend/server.mjs
├── backend/
│   ├── package.json          # zero runtime dependencies
│   ├── server.mjs            # HTTP gateway (node:http), auth routes, static files
│   ├── soma-mcp.mjs          # MCP stdio client class (SomaMcpClient)
│   └── auth.mjs              # magic-link flow as a factory taking the MCP client
├── packs/
│   └── platform/
│       └── manifest.json     # combined crypto + postgres + smtp pack spec
├── bin/                      # gitignored, populated by copy-binaries.sh
│   ├── soma                  # copy of soma-next/target/release/soma
│   ├── libsoma_port_crypto.dylib
│   ├── libsoma_port_postgres.dylib
│   └── libsoma_port_smtp.dylib
└── frontend/
    ├── index.html            # terminal shell with login/request/sent/authenticated views
    ├── app.mjs               # view router + fetch calls to /api/auth/*
    └── terminal.css          # VT323 + CRT scanlines + phosphor glow
```

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `preflight: missing required binaries` | `bin/` is empty | `./scripts/copy-binaries.sh` |
| `[soma-mcp] soma-next exited code=1` | Pack manifest failed to load | Run `bin/soma --mcp --pack packs/platform/manifest.json` standalone and read the stderr |
| `postgres.execute failed: error serializing parameter N` | A parameter's column type isn't what the postgres port sends (TEXT) | For UUIDs use `$N::text::uuid`; for TIMESTAMPTZ use `NOW() + INTERVAL '...'` in SQL instead of binding |
| `smtp.send_plain failed: STARTTLS is not supported` | Mailcatcher doesn't speak TLS | Set `SOMA_SMTP_STARTTLS=false` in `.env` |
| Browser shows "awaiting verification" forever | Magic-link email didn't arrive | Check http://localhost:1080 (mailcatcher web UI) |
| `/api/me` returns 401 after successful verify | Cookie not set on the response | Make sure you're opening the verify URL in a browser (it redirects with a Set-Cookie), not in Playwright where you need to read the JSON response body instead |

## What's next

Commit 2 will add the context registry table + the platform pack's
`context.create` / `context.list_mine` / `context.load` skills + a
post-login UI that lists your existing contexts and lets you start
a new one. Once that's in place, commit 3 brings the conversational
chat shell and the LLM brain into the loop.
