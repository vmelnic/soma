# soma-project-helperbook

First real-world application built with SOMA — a service marketplace where clients find and book service providers (hairdressers, plumbers, tutors, cleaners, personal trainers, etc.).

## Architecture

```
Browser (JS + Tailwind) --> Express Bridge --> MCP JSON-RPC --> SOMA Binary --> Ports --> PostgreSQL / Redis
```

**Backend:** soma-next runtime loads three ports (postgres, redis, auth) via pack manifests. The Express server spawns SOMA in MCP mode and bridges HTTP to JSON-RPC.

**Frontend:** Plain JavaScript + Tailwind CSS (CDN). No React, no build step. Four tabs: Contacts, Chats, Calendar, Profile.

**Ports used:**

| Port | Capabilities | Purpose |
|------|-------------|---------|
| postgres | query, execute, find, find_many, count, aggregate, insert, update, delete | All data persistence (19 tables) |
| redis | get, set, del, hget, hset, publish, subscribe, keys, lists | Caching, sessions, real-time pub/sub |
| auth | otp_generate, otp_verify, session_create, session_validate, token_generate | OTP verification, session management |

## Quick Start

```bash
# 1. Start services
docker compose up -d --wait

# 2. Apply schema + seed data
scripts/setup-db.sh
scripts/seed-db.sh

# 3. Install frontend deps
cd frontend && npm install && cd ..

# 4. Start frontend (spawns SOMA in MCP mode)
cd frontend && node server.js
# Open http://localhost:8080
```

## Database

- `schema.sql` — 19 tables (users, connections, chats, messages, appointments, reviews, provider_profiles, service_categories, etc.)
- `seed.sql` — test data: 13 users, 6 connections, 4 chats, 19 messages, 7 appointments, 4 reviews

## Services

| Service | Port | Purpose |
|---|---|---|
| PostgreSQL 17 | 5432 | Database (user: `soma`, password: `soma`, db: `helperbook`) |
| Redis 7 | 6379 | Cache / pub-sub |
| Mailcatcher | 1025 (SMTP), 1080 (web UI) | Local email capture |

## Scripts

| Script | Purpose |
|---|---|
| `scripts/setup-db.sh` | Apply `schema.sql` to PostgreSQL |
| `scripts/seed-db.sh` | Insert test data (`--reset` to drop and recreate) |
| `scripts/clean-db.sh` | Truncate all tables |
| `scripts/start.sh` | Start SOMA in REPL mode (direct interaction) |
| `scripts/start-mcp.sh` | Start SOMA in MCP mode (for LLM or frontend connection) |

## Frontend Structure

```
frontend/
  index.html        -- App shell: header, main content area, bottom nav
  server.js         -- Express server, SOMA MCP bridge
  css/app.css       -- Custom styles
  js/
    api.js          -- MCP communication layer
    app.js          -- App initialization, routing
    components/     -- Contact list, chat, calendar, profile views
```

## How It Works

The Express server spawns `bin/soma --mcp --pack ...` as a child process. Frontend JS sends JSON-RPC requests to `/api/mcp`, Express pipes them to SOMA stdin, returns stdout responses.

All business logic lives in SOMA via port invocations. The frontend is a pure renderer — it decides HOW things look, SOMA decides WHAT to show.
