# soma-project-terminal

A multi-user SOMA-native web platform with a Fallout-inspired
terminal UI. Operators log in with a magic link, create
**contexts** (their projects), describe what they want to build in
natural language (text or voice), and the LLM compiles a SOMA
PackSpec on the fly. Each context has its own wasm runtime inside
the browser tab and its own isolated memory (episodes, schemas,
routines) in Postgres.

This is the inversion of `soma-helperbook`. HelperBook is an
Express app that calls SOMA for its backend. `soma-project-terminal`
**is SOMA**: the Node layer is a thin HTTP gateway + MCP stdio
client, and every backend side effect — database reads, magic-link
email, token hashing — goes through soma-ports via
`invoke_port`. The browser runs a real `soma-next` wasm runtime
that hot-swaps packs per context.

## What you get

| Capability | Where it lives |
|---|---|
| Magic-link auth (email → token → session cookie) | `backend/auth.mjs` via `crypto.random_string` + `crypto.sha256` + `postgres` + `smtp` ports |
| Context registry — operator's "projects" | `backend/contexts.mjs`, `contexts` table |
| Per-context chat transcript (gpt-4o-mini) | `backend/messages.mjs` + `backend/brain.mjs`, `messages` table |
| Per-context PackSpec storage | `contexts.pack_spec` TEXT column |
| Dynamic pack loading in the browser | `frontend/runtime.mjs`, wasm bundle from `soma-project-web` |
| Per-context memory — episodes, schemas, routines | `backend/memory.mjs`, three dedicated tables |
| LLM-to-PackSpec generation (gpt-5-mini) | `backend/brain.mjs` `generatePackSpec` |
| Voice input (Whisper) | `backend/brain.mjs` `transcribeAudio`, mic button in chat form |

## Architecture

```
┌────────────────────────────────────────────────────────────────────┐
│ Browser tab — Fallout terminal UI                                  │
│                                                                    │
│   BRAIN CHANNEL (left)        │  SOMA RUNTIME — BODY (right)       │
│   - /api/contexts/.../messages│  - soma-next wasm runtime          │
│     (gpt-4o-mini chat)        │  - hot-swaps pack per context      │
│   - [ MIC ] → MediaRecorder   │  - dom / audio / voice in-tab      │
│     → /api/transcribe         │    ports                           │
│     (whisper-1)               │  - soma_inject_routine rehydrate   │
│                               │                                    │
│   MEMORY — PER CONTEXT        │  [ GENERATE PACK ] button          │
│   - EPISODES / SCHEMAS /      │    → /api/contexts/.../pack/       │
│     ROUTINES counts from      │      generate (gpt-5-mini)         │
│     /api/contexts/.../memory  │    → rebuilds the runtime below    │
└───────────┬────────────────────────────────────────────────────────┘
            │ HTTP cookies + /api/*
            ▼
┌────────────────────────────────────────────────────────────────────┐
│ Node HTTP gateway (backend/server.mjs)                             │
│                                                                    │
│   Auth        /api/auth/{request-link,verify,logout}  /api/me      │
│   Contexts    /api/contexts[/:id]                                  │
│                /api/contexts/:id/pack          PUT                 │
│                /api/contexts/:id/pack/generate POST (gpt-5-mini)   │
│   Chat        /api/contexts/:id/messages       GET / POST          │
│   Memory      /api/contexts/:id/memory         GET / DELETE        │
│                /api/contexts/:id/memory/{episodes,schemas,routines}│
│   Voice       /api/transcribe                  POST (whisper-1)    │
│   Health      /api/health                                          │
│                                                                    │
│   Zero runtime dependencies. Backend/package.json is empty.        │
│   SomaMcpClient spawns bin/soma --mcp --pack platform.json once,   │
│   speaks JSON-RPC 2.0 over stdio, exposes invokePort().            │
└───────────┬────────────────────────────────────────────────────────┘
            │ MCP stdio (tools/call invoke_port)
            ▼
┌────────────────────────────────────────────────────────────────────┐
│ soma-next (spawned subprocess)                                     │
│   Pack: packs/platform/manifest.json (merged crypto+postgres+smtp) │
│   Port dylibs in bin/:                                             │
│     - libsoma_port_crypto.dylib                                    │
│     - libsoma_port_postgres.dylib                                  │
│     - libsoma_port_smtp.dylib                                      │
└───────────┬────────────────────────────────────────────────────────┘
            │
            ▼
┌────────────────────────────────────────────────────────────────────┐
│ docker-compose: postgres(5433) + redis(6380) + mailcatcher(1025)   │
│                                                                    │
│ Tables:                                                            │
│   users         (id, email, created_at, last_login)                │
│   magic_tokens  (token_hash, email, expires_at, used_at)           │
│   sessions      (token_hash, user_id, expires_at, revoked_at)      │
│   contexts      (id, user_id, name, description, kind, pack_spec)  │
│   messages      (id, context_id, role, content, created_at)        │
│   episodes      (id, context_id, payload, created_at)              │
│   schemas       (id, context_id, name, payload, created_at)        │
│   routines      (id, context_id, name, payload, created_at)        │
└────────────────────────────────────────────────────────────────────┘
```

Only the Node gateway + frontend live in this project. The runtime
is `soma-next`, the ports are `soma-ports`, the database + mail
sink are off-the-shelf containers, and the browser wasm bundle
comes from `soma-project-web`'s build. Nothing is reimplemented
here that already exists elsewhere in the repo.

## The two-brain split

The terminal runs **two independent brains** against **one body
per context**, which is the realest embodiment of SOMA's
architecture in the repo today:

- **Chat brain** — `gpt-4o-mini`. Accepts natural-language
  conversation, keeps the operator grounded, suggests which SOMA
  ports could satisfy what they're describing. Drives
  `/api/contexts/:id/messages`. Transcript persists per context.
- **Pack brain** — `gpt-5-mini` reasoning model. Reads the context
  name + description + chat history and emits a minimal
  `{pack_id, pack_name, skills[]}` shape. The backend expands that
  into a full PackSpec matching `frontend/packs/hello/manifest.json`
  and stores it on `contexts.pack_spec`. Drives
  `/api/contexts/:id/pack/generate`.
- **Body** — `soma-next` wasm runtime inside the browser tab. One
  instance at a time; `soma_boot_runtime` is called afresh each
  time the operator switches contexts, hot-swapping ports and
  skills in-place. Stored routines for the context are then
  re-injected via `soma_inject_routine`.

The chat and pack wrappers live in the **same** `backend/brain.mjs`
file but are distinct functions — they must be, because reasoning
models reject `temperature` and `max_tokens` and chat models
reject `reasoning_effort`. Mixing them always regresses.

Both wrappers honor `BRAIN_FAKE=1`, which returns deterministic
canned responses (echo for chat, slugified pack for generate,
byte-count echo for transcribe). The Playwright suite always runs
with `BRAIN_FAKE=1` so tests never burn OpenAI quota.

## Prerequisites

- Rust stable with `soma-next` and `soma-ports` already built in
  release mode:
  ```bash
  (cd ../soma-next  && cargo build --release)
  (cd ../soma-ports && cargo build --workspace --release)
  ```
- `soma-project-web` also built so the wasm bundle exists:
  ```bash
  (cd ../soma-project-web && ./scripts/build.sh)
  ```
- Node 20+ (we use the built-in `--env-file` flag; no dotenv package).
- Docker Desktop for the postgres / redis / mailcatcher stack.

## First run

Three terminals.

### Terminal 0 — one-time setup

```bash
cd soma-project-terminal
cp .env.example .env                     # edit OPENAI_API_KEY if you want real brains
./scripts/copy-binaries.sh               # cp soma + 3 dylibs → bin/
./scripts/copy-frontend-assets.sh        # cp wasm pkg + hello manifest → frontend/
./scripts/start.sh                       # docker compose up -d --wait
./scripts/setup-db.sh                    # psql < schema.sql
```

### Terminal 1 — backend

```bash
./scripts/start-backend.sh
# [soma-mcp] soma-next MCP server ready
# [http] soma-project-terminal listening on http://127.0.0.1:8765
# [http] SOMA_POSTGRES_URL=postgres://soma:soma@localhost:5433/soma_terminal
# [http] SOMA_SMTP=localhost:1025
```

The backend runs a preflight check at startup. Missing native
binaries (soma-next + port dylibs) fail the boot; missing wasm
assets only warn — the auth endpoints still work for debugging,
but the browser runtime panel will not boot.

### Terminal 2 — browser

Open [http://localhost:8765/](http://localhost:8765/). The
terminal boots, asks for an operator email, dispatches a
magic-link message. Pick it up from Mailcatcher at
[http://localhost:1080](http://localhost:1080). Click the link
to authenticate.

From there:

1. **Create a context.** Give it a name and a one-sentence
   description. It lands in `contexts` with `kind='draft'` and
   `pack_spec=null`. The runtime panel boots the fallback `hello`
   pack.
2. **Talk to it.** Type into the brain channel. `gpt-4o-mini`
   (or the fake brain) replies and the full transcript persists
   to the `messages` table.
3. **Dictate.** Click `[ MIC ]`, speak, click again. The browser
   records via `MediaRecorder`, uploads the blob to
   `/api/transcribe`, Whisper returns text, and the text is
   appended to the chat input — ready to review and submit.
4. **Generate a pack.** Click `[ GENERATE PACK ]`. The reasoning
   brain reads the context + chat and emits a minimal pack shape.
   The backend expands it into a full PackSpec, stores it on
   `contexts.pack_spec`, and flips `kind` to `'active'`. The
   runtime panel hot-swaps to the new pack, showing the new pack
   id and `SOURCE: context`.
5. **Switch contexts.** Each context has its own isolated memory
   (episodes, schemas, routines). Opening another context boots
   the wasm body with that context's pack and rehydrates its
   routines via `soma_inject_routine`.

## SOMA-native design

The hard rule is that **every backend side effect goes through
`SomaMcpClient.invokePort(portId, capabilityId, input)`**. No
direct `pg`, no `nodemailer`, no `crypto.randomBytes`. The gateway
is a thin JSON-RPC shim over a long-running `soma --mcp` child
process.

Consequences:

- `backend/package.json` has an empty `dependencies` object. The
  only `devDependency` is `@playwright/test` for the browser suite.
- Every SQL query in every module (`auth.mjs`, `contexts.mjs`,
  `messages.mjs`, `memory.mjs`) is literally a string passed to
  `invokePort("postgres", "query"|"execute", {sql, params})`.
- Magic-link tokens are generated by `invokePort("crypto",
  "random_string", {length})`, hashed by `invokePort("crypto",
  "sha256", {data})`, and stored as the hex digest. No plaintext
  at rest, no JS-side crypto.
- Email delivery is `invokePort("smtp", "send_plain", {to,
  subject, body})`.

The two OpenAI brains and Whisper are the **only** network calls
that don't go through SOMA, because there's no LLM port yet and
the Whisper endpoint isn't exposed through `soma-ports`. They use
Node 20's global `fetch` and `FormData`/`Blob`; the backend stays
zero-dep.

## Tenant isolation

Every per-context resource — pack specs, messages, episodes,
schemas, routines — is scoped by `context_id`, which in turn has
a `user_id` FK. Every read and write runs an ownership check
before touching the target table. In SQL this shows up as:

```sql
-- contexts.loadContext
WHERE id = $1::text::uuid AND user_id = $2::text::uuid

-- messages / memory
SELECT 1 FROM contexts
WHERE id = $1::text::uuid AND user_id = $2::text::uuid
```

A cross-tenant id probe — operator A trying to read operator B's
context or memory by id — resolves to "not found", the same
shape as a genuinely unknown id. No leak of existence across
tenants. The Playwright suite proves this explicitly for every
per-context route: pack `PUT`, `GET`/`POST`/`DELETE` on messages
and memory, and `POST /pack/generate`.

An unrelated but essential choice: `backend/server.mjs`'s
`getSessionToken` prefers `Authorization: Bearer` over the session
cookie. Cookie-first would let a stale browser session silently
override an intentional API call, which breaks multi-tenant scope
on any endpoint that takes a Bearer token.

## File layout

```
soma-project-terminal/
├── docker-compose.yml          # postgres(5433) + redis(6380) + mailcatcher(1025)
├── schema.sql                  # users, sessions, magic_tokens, contexts,
│                               # messages, episodes, schemas, routines
├── .env.example                # POSTGRES_URL, SMTP_*, OPENAI_*, BRAIN_FAKE, TTLs
├── .gitignore                  # bin/, frontend/pkg/, node_modules/, test-results/
├── package.json                # Playwright devDep (no runtime deps)
├── playwright.config.js        # webServer auto-starts ./scripts/start-backend.sh
├── README.md
├── scripts/
│   ├── start.sh                # docker compose up -d --wait
│   ├── setup-db.sh             # psql < schema.sql
│   ├── clean-db.sh             # TRUNCATE all tables
│   ├── copy-binaries.sh        # cp soma + 3 dylibs → bin/
│   ├── copy-frontend-assets.sh # cp wasm pkg + hello pack → frontend/
│   └── start-backend.sh        # node --env-file=.env backend/server.mjs
├── backend/                    # zero runtime dependencies
│   ├── package.json            # empty "dependencies": {}
│   ├── server.mjs              # HTTP gateway, preflight, all routes
│   ├── soma-mcp.mjs            # SomaMcpClient (JSON-RPC over stdio)
│   ├── auth.mjs                # magic-link flow (crypto + postgres + smtp ports)
│   ├── contexts.mjs            # context CRUD + setPackSpec
│   ├── messages.mjs            # chat transcript CRUD, ownership via context join
│   ├── memory.mjs              # episodes / schemas / routines CRUD
│   └── brain.mjs               # chatCompletion, reasoningCompletion,
│                               # generatePackSpec, transcribeAudio,
│                               # buildSystemPrompt — four distinct OpenAI
│                               # wrappers in one zero-dep module
├── packs/
│   └── platform/
│       └── manifest.json       # merged crypto + postgres + smtp pack for soma-next
├── bin/                        # gitignored, populated by copy-binaries.sh
│   ├── soma
│   ├── libsoma_port_crypto.dylib
│   ├── libsoma_port_postgres.dylib
│   └── libsoma_port_smtp.dylib
├── frontend/
│   ├── index.html              # Fallout shell, views: loading / request-link /
│   │                           # link-sent / authenticated / context-detail / error
│   ├── app.mjs                 # view router, fetch calls, chat + mic + pack button
│   ├── runtime.mjs             # wasm boot, hot-swap bootPack, injectRoutine
│   ├── terminal.css            # VT323 + phosphor glow + CRT scanlines
│   ├── pkg/                    # gitignored, populated by copy-frontend-assets.sh
│   │   ├── soma_next_bg.wasm
│   │   └── soma_next.js
│   └── packs/
│       └── hello/
│           └── manifest.json   # fallback pack for contexts with pack_spec=null
└── tests/                      # Playwright suite (61 tests, ~15s headless)
    ├── global-setup.mjs        # TRUNCATE all tables + clear mailcatcher
    ├── helpers.mjs             # loginAs() — full magic-link round trip
    ├── mailcatcher.mjs         # HTTP API client for pulling tokens from emails
    ├── auth.spec.js            # magic-link auth, styling, replay attack
    ├── contexts.spec.js        # CRUD + cross-tenant isolation + UI
    ├── chat.spec.js            # transcript + brain + cross-tenant isolation
    ├── pack.spec.js            # manifest storage + UI hot-swap between contexts
    ├── memory.spec.js          # same-operator + cross-operator memory isolation
    ├── generate.spec.js        # LLM-to-PackSpec round trip + UI
    └── transcribe.spec.js      # Whisper route + stubbed MediaRecorder UI tests
```

## Testing

Tests run headless Chromium against the real stack. Playwright's
`webServer` auto-starts `./scripts/start-backend.sh` (which spawns
the real `soma --mcp` child process) and `globalSetup` truncates
every table before the suite runs.

```bash
./scripts/start.sh                   # once — docker stack
./scripts/setup-db.sh                # once — schema
./scripts/copy-binaries.sh           # once — native binaries
./scripts/copy-frontend-assets.sh    # once — wasm bundle
npm install                          # once — Playwright
npx playwright test
```

The suite forces `BRAIN_FAKE=1` via `playwright.config.js` so
tests never burn OpenAI quota. Fake mode returns deterministic
canned responses for chat, pack generation, and transcription —
the full request/response wire runs end-to-end, only the model
call is stubbed.

Current run: **61 tests, ~15s wall time, headless Chromium.**

What each suite covers:

| Suite | What it asserts |
|---|---|
| `auth.spec.js` | Fallout styling (VT323, phosphor green, CRT scanlines), magic-link round trip, replay-attack rejection, invalid-email 400, health endpoint |
| `contexts.spec.js` | CRUD + recent-first ordering + cross-user 404 + UI list/create/detail |
| `chat.spec.js` | Transcript round trip with fake brain + ordering + cross-user isolation on read and write + UI round trip including wasm body boot |
| `pack.spec.js` | `PUT /pack` storage, bare-body acceptance, invalid shape 400, cross-user 404, UI hot-swap between two contexts with distinct packs |
| `memory.spec.js` | Fresh-context empty + same-operator isolation (A → B stays empty) + cross-operator isolation + invalid payload 400 + `DELETE /memory` scoped to one context + UI counts updating across context switch |
| `generate.spec.js` | Fake `gpt-5-mini` path: unauth 401, valid PackSpec round-trips, slugified pack id from context name, chat-history grounding, cross-user 404, idempotent back-to-back, UI hot-swap to generated pack |
| `transcribe.spec.js` | `/api/transcribe` unauth 401 / empty 400 / JSON CT guard 400 / fake Whisper 200 + UI `MediaRecorder` stubbed round trip + "append to draft instead of clobber" |

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `error: missing required binaries` | `bin/` is empty | `./scripts/copy-binaries.sh` |
| `warning: missing frontend wasm assets` | `frontend/pkg/` is empty — auth works but the runtime panel won't boot in the browser | `./scripts/copy-frontend-assets.sh` (after building `soma-project-web`) |
| `[soma-mcp] soma-next exited code=1` | Platform pack manifest failed to load | Run `bin/soma --mcp --pack packs/platform/manifest.json` standalone and read the stderr |
| `postgres.execute failed: error serializing parameter N` | A parameter's column type isn't what the postgres port sends (it serializes everything as TEXT) | For UUID columns use `$N::text::uuid`; for TIMESTAMPTZ use `NOW() + INTERVAL '...'` in SQL rather than binding a `Date` value |
| `smtp.send_plain failed: STARTTLS is not supported` | Mailcatcher doesn't speak TLS | Set `SOMA_SMTP_STARTTLS=false` in `.env` |
| Browser shows "awaiting verification" forever | Magic-link email didn't arrive | Check http://localhost:1080 (Mailcatcher web UI) |
| `/api/me` returns 401 after successful verify via curl | Cookie not set because you read the JSON response instead of following the redirect | When curling, pass `Accept: application/json` and read the `session_token` from the body; send it as `Authorization: Bearer <token>` on subsequent calls |
| `OpenAI reasoning 400: Unsupported parameter: temperature` | Someone called `chatCompletion` against a reasoning model or passed `temperature` to `reasoningCompletion` | Keep the two wrappers in `backend/brain.mjs` separate. Reasoning models (`gpt-5-mini`, `o1`, `o3`) reject `temperature` and `max_tokens`; use `reasoning_effort` + `response_format` instead |
| `[ MIC UNSUPPORTED ]` button | Browser doesn't expose `MediaRecorder` + `navigator.mediaDevices.getUserMedia` | Use a recent Chrome/Edge/Firefox, served over localhost or HTTPS; the mic API is blocked on insecure origins |

## OpenAI parameter families

A load-bearing detail inherited from the wider SOMA repo and
re-stated here because mixing them is the most common way to
break this project:

| Parameter | Chat (`gpt-4o-mini`) | Reasoning (`gpt-5-mini`, `o1`, `o3`) |
|---|---|---|
| `temperature` | accepted | **rejected, API 400s** |
| `max_tokens` | accepted | rejected, use `max_completion_tokens` |
| `reasoning_effort` | rejected | accepted (`"low"` / `"medium"` / `"high"`) |
| `response_format: json_object` | accepted | accepted |

`backend/brain.mjs` exposes four distinct functions for this
exact reason: `chatCompletion`, `reasoningCompletion`,
`generatePackSpec`, `transcribeAudio`. Each one knows which
endpoint and which parameter set to use.

## What's next

The browser body currently only runs the three in-tab ports that
ship with `soma-project-web`'s wasm build (`dom`, `audio`, `voice`)
plus whatever skills the generated PackSpec declares on top of
them. A future expansion would let the context pack pull in more
browser-side ports (storage, fetch, canvas, webgl) or delegate
skills back to the native `soma-next` via MCP for things the
browser can't do alone.

The reasoning brain currently produces a MINIMAL pack shape that
`expandToFullPackSpec` inflates into full PackSpec JSON. As the
browser runtime grows more demanding, the minimal shape can grow
new optional fields (policies, resources, routines seed) without
touching the LLM prompt — the expansion layer absorbs the
complexity.

Episodes still flow from the API directly rather than from live
wasm runtime activity: the wasm runtime has no `soma_export_*`
entry point yet, and the "organic multi-step episode from one
goal" problem from the main `CLAUDE.md` still blocks natural
episode production on the browser path. Memory **storage** is
per-context and isolated; memory **production** from the browser
runtime is a separate open problem.
