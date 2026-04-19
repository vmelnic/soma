# HelperBook

## What It Is

Service marketplace where clients find and book service providers -- hairdressers, plumbers, tutors, cleaners, personal trainers, photographers, and more. Telegram-like messaging, appointments with scheduling, and post-service reviews. First real-world application built on SOMA.

## Architecture

```
Browser (JS + Tailwind) --> Express Bridge (server.js) --> MCP JSON-RPC --> soma-next binary --> Ports --> PostgreSQL / Redis
```

The Express server spawns `bin/soma --mcp --pack ...` as a child process. Frontend JS sends JSON-RPC requests to `/api/mcp`, Express pipes them to SOMA stdin and returns stdout responses. All business logic lives in SOMA via port invocations. The frontend is a pure renderer -- it decides how things look, SOMA decides what to show.

## Services (Docker)

Defined in `docker-compose.yml`:

| Service | Image | Ports | Purpose |
|---|---|---|---|
| PostgreSQL | postgres:17 | 5432 | Database (user: `soma`, password: `soma`, db: `helperbook`) |
| Redis | redis:7-alpine | 6379 | Caching, sessions, real-time pub/sub |
| Mailcatcher | schickling/mailcatcher | 1025 (SMTP), 1080 (web UI) | Local email capture for development |

Both PostgreSQL and Redis have health checks configured (5s interval, 5 retries). PostgreSQL data is persisted in a named volume `pgdata`.

## Ports Used

Three ports loaded via pack manifests:

**postgres** -- all data persistence:
`query`, `execute`, `find`, `find_many`, `count`, `aggregate`, `insert`, `update`, `delete`

**redis** -- caching and pub/sub:
`get`, `set`, `del`, `hget`, `hset`, `publish`, `subscribe`, `keys`, `lists`, and more

**auth** -- OTP verification and session management:
`otp_generate`, `otp_verify`, `session_create`, `session_validate`, `token_generate`, and more

## Data Model

Tables grouped by domain:

**Identity** -- users, connections, blocked_users

**Messaging** -- chats, chat_members, messages

**Appointments** -- appointments, reviews, services_history, service_categories, user_services

**Providers** -- provider_profiles

**Organization** -- contact_notes, contact_folders, contact_folder_members, user_settings, notifications, devices

**Infrastructure** -- _soma_migrations

Key design points:
- All primary keys are UUIDs (`gen_random_uuid()`)
- Users have a `role` field: `client`, `provider`, or `both`
- Connections track relationship status: `pending`, `accepted`, `declined`, `blocked`
- Appointments track lifecycle: `pending`, `confirmed`, `completed`, `cancelled`, `no_show`
- Reviews are linked to completed appointments with 1-5 ratings

**Seed data** (`seed.sql`): a small set of users (one client and a handful of providers across Bucharest), provider profiles, connections, chats, messages, appointments, and reviews — enough to exercise every flow without hand-crafting fixtures.

## Frontend

Plain JavaScript + Tailwind CSS (CDN). No React, no build step. Mobile-first layout (max-w-md centered).

```
frontend/
  index.html        -- App shell: header, main content area, bottom nav
  server.js         -- Express server, SOMA MCP bridge
  css/app.css       -- Custom styles
  js/
    api.js          -- MCP communication layer
    app.js          -- App initialization, routing
    components/
      nav.js        -- Bottom navigation (4 tabs)
      contacts.js   -- Contact list view
      chat.js       -- Chat/messaging view
      calendar.js   -- Appointment calendar view
      profile.js    -- User profile view
      provider-card.js -- Provider display component
```

Four tabs: Contacts, Chats, Calendar, Profile. Uses Inter + Playfair Display fonts (Google Fonts) and Lucide icons.

The Express server (`server.js`) spawns SOMA with three pack manifests and bridges HTTP POST `/api/mcp` to SOMA's stdin/stdout. Environment variables configure database and Redis URLs, with defaults pointing to localhost services.

## Quick Start

```bash
# Start services
docker compose up -d --wait

# Apply schema
scripts/setup-db.sh

# Seed test data
scripts/seed-db.sh

# Install frontend deps and start
cd frontend && npm install && node server.js
# Open http://localhost:8080
```

## Environment

Configuration lives in `.env`, loaded by scripts and server.js:

```
SOMA_POSTGRES_URL="host=localhost user=soma password=soma dbname=helperbook"
SOMA_REDIS_URL=redis://localhost:6379/0
SOMA_SMTP_HOST=localhost
SOMA_SMTP_PORT=1025
SOMA_SOMA_DATA_DIR=./data
SOMA_PORTS_PLUGIN_PATH=./packs/postgres:./packs/redis:./packs/auth
SOMA_PORTS_REQUIRE_SIGNATURES=false
```

## Scripts

| Script | Purpose |
|---|---|
| `scripts/setup-db.sh` | Apply `schema.sql` to PostgreSQL |
| `scripts/seed-db.sh` | Insert test data (`--reset` to drop and recreate) |
| `scripts/clean-db.sh` | Truncate all tables |
| `scripts/start.sh` | Start SOMA in REPL mode (direct interaction) |
| `scripts/start-mcp.sh` | Start SOMA in MCP mode (for LLM or frontend connection) |

`start-mcp.sh` loads `.env`, sets `SOMA_PORTS_PLUGIN_PATH` to all three pack directories, and runs `bin/soma --mcp` with all three pack manifests.

## Capabilities Checklist

Two automated harnesses verify the full runtime stack against a running HelperBook instance:

**`capabilities-checklist/run.mjs`** -- exercises the full runtime surface across seven layers:

1. **Port Invocation** -- postgres query/insert/update/delete, redis get/set, auth OTP/session/token flows, error handling for missing ports and bad capabilities
2. **State & Context** -- `dump_state` sections (ports, skills, packs, metrics, sessions, episodes, schemas, routines, belief), selective section queries
3. **Goal & Session Lifecycle** -- goal creation, session management, execution tracing, pause/resume
4. **Memory Persistence** -- episodes, schemas, and routines survive within a process; session counter increments
5. **Policy & Safety** -- SQL injection prevention, cross-tenant isolation, input sanitization
6. **Proprioception** -- self-reporting, metrics counters, resource tracking
7. **Observation & Tracing** -- execution traces, structured logging

Run: `node capabilities-checklist/run.mjs`

**`capabilities-checklist/persistence.mjs`** -- memory-persistence check:

Spawns separate SOMA processes sequentially, verifying that disk-backed stores (episodes, schemas, routines) survive process restarts while sessions correctly reset as ephemeral state. Also verifies external service persistence (Redis markers, Postgres data) across process lifetimes.

Run: `node capabilities-checklist/persistence.mjs`

Both require services running (`docker compose up -d --wait`).
