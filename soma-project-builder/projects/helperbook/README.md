# HelperBook

Local services marketplace — book appointments, message providers, leave reviews. Runs on SOMA runtime with no application source code; behavior is defined entirely by routines, screens, and port manifests.

## Prerequisites

- Docker (for Postgres, Redis, Mailcatcher)
- Node.js 20+
- `soma-next` binary built at `../../soma-next/` (`cargo build --release`)
- Ports built at `../../soma-ports/` (`cargo build --workspace --release`)

## Setup

```bash
# 1. Start Postgres + Redis + Mailcatcher
docker compose up -d --wait

# 2. Create schema and seed data
bash scripts/setup.sh

# 3. Copy soma binary (from repo root)
cp ../../soma-next/target/release/soma bin/soma
codesign -fs - bin/soma   # macOS only

# 4. Copy port dylibs into packs
cp ../../soma-ports/target/release/libsoma_port_postgres.dylib packs/postgres/
cp ../../soma-ports/target/release/libsoma_port_redis.dylib packs/redis/
cp ../../soma-ports/target/release/libsoma_port_auth.dylib packs/auth/
cp ../../soma-ports/target/release/libsoma_port_smtp.dylib packs/smtp/

# 5. Compile routines
node ../../engine/compile.mjs .

# 6. Install bridge dependencies
npm install

# 7. Install frontend dependencies
cd app && npm install && cd ..
```

## Configuration

All config lives in `.env` (sourced by `scripts/start.sh`):

```
SOMA_POSTGRES_URL    Postgres connection string
SOMA_REDIS_URL       Redis URL
SOMA_SMTP_HOST       SMTP host (localhost for mailcatcher)
SOMA_SMTP_PORT       SMTP port (1025 for mailcatcher)
SOMA_SMTP_FROM       Sender address
SOMA_SMTP_STARTTLS   false for mailcatcher
BRIDGE_PORT          HTTP bridge port (default 3000)
```

## Run

```bash
# Start soma + bridge (single command, reads .env)
bash scripts/start.sh

# In another terminal — start the frontend dev server
cd app && npm run dev
```

- Frontend: http://localhost:5173
- Bridge API: http://localhost:3000
- Mailcatcher UI: http://localhost:1080
- SOMA WebSocket: ws://127.0.0.1:9090

## Project structure

```
bin/              soma binary
bridge.mjs        HTTP→WebSocket bridge (translates REST calls to MCP)
packs/            port adapters (postgres, redis, auth, smtp) + manifests
routines/         routine definitions (.md) — compiled to data/routines.json
screens/          UI screen definitions (JSON) — consumed by the Vue renderer
app/              Vue 3 frontend (Vite + Tailwind)
scripts/          setup.sh, start.sh, reset.sh, seed.sh
schema.sql        Postgres schema
seed.sql          Test data
.env              Runtime configuration
docker-compose.yml
```

## Routines

| Routine | Steps | Description |
|---------|-------|-------------|
| login_otp | auth.otp_generate → smtp.send_plain | Generate OTP, email code |
| verify_otp | auth.otp_verify → auth.session_create | Verify code, create session |
| list_providers | postgres.query | List service providers |
| get_user_profile | auth.session_validate → postgres.find | Get user by ID |
| list_appointments | auth.session_validate → postgres.query | User's appointments |
| list_contacts | auth.session_validate → postgres.query | User's accepted connections |
| book_appointment | auth → postgres.find → postgres.count → postgres.insert | Book with conflict check |
| cancel_appointment | auth → postgres.find → postgres.update | Cancel by ID |
| send_message | auth → postgres.insert | Send chat message |
| submit_review | auth → postgres.find → postgres.count → postgres.insert | Review with duplicate check |

## Test

```bash
# Requires soma + bridge running
node test_routines.mjs
```

## Reset database

```bash
bash scripts/reset.sh
```
