# SOMA HelperBook

First real-world application built with SOMA -- a service marketplace where clients find and book service providers.

## Architecture

Browser (JS) -> Express Bridge -> MCP JSON-RPC -> SOMA Binary -> 7 Plugins -> PostgreSQL/Redis

## Quick Start

```bash
# 1. Start services
docker compose up -d --wait

# 2. Apply schema + seed data
scripts/setup-db.sh
scripts/seed-db.sh

# 3. Build plugins
cd ../soma-plugins && cargo build --release

# 4. Start frontend + SOMA
cd frontend && npm install && node server.js
# Open http://localhost:8080
```

## Database

- schema.sql: 19 tables (users, connections, messages, chats, appointments, reviews, etc.)
- seed.sql: test data (13 users, 4 chats, 19 messages, 7 appointments, 4 reviews)

## Scripts

- scripts/setup-db.sh: Apply schema
- scripts/seed-db.sh [--reset]: Seed test data
- scripts/clean-db.sh: Truncate all tables
- scripts/start.sh: Start SOMA in REPL mode
- scripts/start-mcp.sh: Start SOMA in MCP mode
- scripts/synthesize.sh: Train Mind with all plugins

## Frontend

Plain JavaScript + Tailwind CSS + Google Fonts + Lucide Icons (all from CDN).
No build step. Express server bridges HTTP to SOMA MCP.

## Configuration

soma.toml -- HelperBook-specific SOMA config with all plugin settings.
