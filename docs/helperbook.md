# HelperBook

## What It Is

HelperBook is the first real-world application built entirely with SOMA. It is a service marketplace where clients find and book service providers -- hairdressers, plumbers, babysitters, tutors, cleaners, personal trainers, and more.

The feature set resembles a Telegram-like messaging app with domain-specific additions: dual-role users (everyone can be both client and provider), connection requests, real-time chat with multiple message types, in-chat appointment cards, calendar scheduling, reviews and ratings, AI smart replies, push notifications, offline support, and multi-device sync.

HelperBook proves that a complex, multi-feature application can exist as a SOMA instance with plugins, built and maintained by an LLM driving SOMA via MCP. No application source code is written by hand.


## Architecture

```
Browser (JS + Tailwind) --> Express Bridge --> MCP JSON-RPC --> SOMA Binary --> Plugins --> PostgreSQL / Redis
```

**Backend SOMA** holds all business logic and state. It loads plugins for data persistence (PostgreSQL, Redis), authentication (OTP, sessions), cryptography, geolocation, and HTTP bridging. The Mind generates execution programs from structured intents; plugins carry them out.

**Frontend** is plain JavaScript with Tailwind CSS. An Express server (`frontend/server.js`) bridges HTTP POST requests to the SOMA binary's MCP stdio interface. The frontend sends JSON-RPC requests to `/api/mcp`, Express pipes them to the SOMA process's stdin, and returns stdout responses. No React, no build step, no bundler.

**Building** happens via an LLM connected to SOMA through MCP. The LLM installs plugins, creates the database schema, seeds data, configures business logic, and records decisions. The Interface SOMA (when it exists as a neural renderer) is a pure renderer -- it receives semantic signals and renders UI. It does not converse or understand natural language. All conversational intelligence lives in the external LLM.


## Quick Start

Prerequisites: Docker, Rust toolchain, Node.js.

```bash
# 1. Start PostgreSQL and Redis
cd soma-helperbook
docker compose up -d --wait

# 2. Apply schema (19 tables)
scripts/setup-db.sh

# 3. Seed test data (13 users, 4 chats, 19 messages, 7 appointments, 4 reviews)
scripts/seed-db.sh

# 4. Build SOMA plugins
cd ../soma-plugins && cargo build --release

# 5. Build SOMA core
cd ../soma-core && cargo build --release

# 6. Start frontend (spawns SOMA in MCP mode)
cd ../soma-helperbook/frontend && npm install && node server.js
# Open http://localhost:8080
```

The Express server spawns the SOMA binary as a child process with `--mcp` flag, initializes the MCP handshake, and proxies all requests through `/api/mcp`. A status endpoint at `/api/status` reports whether SOMA is running and MCP is initialized.

### Scripts

| Script | Purpose |
|--------|---------|
| `scripts/setup-db.sh` | Apply `schema.sql` to PostgreSQL |
| `scripts/seed-db.sh` | Insert test data from `seed.sql` (supports `--reset`) |
| `scripts/clean-db.sh` | Truncate all tables |
| `scripts/start.sh` | Start SOMA in REPL mode (direct interaction) |
| `scripts/start-mcp.sh` | Start SOMA in MCP mode (for LLM connection) |
| `scripts/synthesize.sh` | Train the Mind with all plugins + domain data |


## Data Model

The database schema lives in `schema.sql` -- 19 tables applied as a single transaction. All primary keys are UUIDs. Foreign keys enforce referential integrity. The tables, grouped by domain:

**Identity and social:**
- `users` -- phone, name, photo, bio, location (lat/lon), role (`client`/`provider`/`both`), subscription plan, verification status, locale, currency
- `connections` -- requester, recipient, status (`pending`/`accepted`/`declined`/`blocked`), optional message
- `blocked_users` -- blocker/blocked pairs

**Messaging:**
- `chats` -- type (`direct`/`group`), name, creator
- `chat_members` -- membership with role (`member`/`admin`) and mute control
- `messages` -- sender, type (`text`/`photo`/`video`/`voice`/`document`/`location`/`contact_card`/`appointment_card`/`service_card`), content, media URL, delivery status (`sent`/`delivered`/`read`), reply threading, edit/delete timestamps

**Appointments and services:**
- `appointments` -- client, provider, service, time range, location, rate (amount/currency/type), status lifecycle (`proposed` -> `confirmed` -> `in_progress` -> `completed` | `dismissed` | `cancelled` | `no_show`)
- `reviews` -- per-appointment, bidirectional (both parties review), rating 1-5, feedback text, tags, photos, provider response
- `services_history` -- completed service records with hours, rates, confirmation from both parties, dispute flag
- `service_categories` -- hierarchical (parent/child), trilingual (en/ro/ru), with icons
- `user_services` -- links users to service categories with per-user rates

**Provider profiles:**
- `provider_profiles` -- extended bio, certifications, working schedule (JSONB), gallery, service area radius, languages, response metrics

**Organization and settings:**
- `contact_notes` -- per-contact private notes
- `contact_folders` -- user-defined folder groupings
- `contact_folder_members` -- contacts in folders
- `user_settings` -- key/value per user
- `notifications` -- type, title, body, JSONB data, read status
- `devices` -- push tokens, device type, last active

**Infrastructure:**
- `_soma_migrations` -- tracks every schema change with SQL executed, rollback SQL, and the SOMA instance that made the change

Indexes cover the hot query paths: user phone lookups, connection status queries, message ordering within chats, appointment scheduling by client/provider/time, review lookups, and notification delivery.

The full SQL is in `soma-helperbook/schema.sql`. Seed data with realistic Bucharest-area test users is in `seed.sql`.


## Plugins Used

HelperBook loads seven plugins, configured in `soma.toml`:

| Plugin | Role | Key Conventions |
|--------|------|-----------------|
| **PostgreSQL** | All data persistence | `query`, `execute`, `query_one`, `find`, `count`, `aggregate` |
| **Redis** | Caching, sessions, real-time pub/sub | `get`, `set`, `publish`, `subscribe`, `hget`, `hset` |
| **Auth** | OTP verification, session management | `generate_otp`, `verify_otp`, `create_session`, `validate_session` |
| **Crypto** | Password hashing, token signing | `hash`, `verify_hash`, `sign`, `random_bytes` |
| **HTTP Bridge** | External API calls | `get`, `post`, `put`, `delete` |
| **Geo** | Distance calculations, radius search | `distance`, `radius_filter`, `geocode` |
| **Built-in (POSIX)** | Filesystem operations | File I/O, directory listing |

Plugin configuration in `soma.toml` includes database connection details for PostgreSQL (host, port, database, credentials via env var), Redis URL, and auth settings (session TTL, OTP TTL and length). The password is never stored in the config file -- `password_env = "SOMA_PG_PASSWORD"` reads from the environment.

Future plugins from the [catalog](plugin-catalog.md) can extend HelperBook without architectural changes: image processing, S3 storage, SMTP email, Twilio SMS, APNS/FCM push, AI inference for smart replies, text search, analytics, localization.


## Key Features

**Dual-role users.** Every account has a `role` field: `client`, `provider`, or `both`. A hairdresser who also books a plumber uses one account. The data model treats both roles symmetrically.

**Connection requests.** Users send connection requests with an optional message. Recipients accept, decline, or block. Accepted connections unlock direct chat. The `connections` table tracks the lifecycle.

**Real-time chat.** Messages support nine types: plain text, photos, videos, voice messages, documents, locations, contact cards, appointment cards, and service cards. Delivery status progresses from `sent` to `delivered` to `read`. Messages can be replies (threaded via `reply_to_id`), edited, or soft-deleted.

**Appointment scheduling.** Either party proposes an appointment in chat, which appears as an interactive card. The card shows service, date/time, duration, location, and rate. Recipients confirm, dismiss, or suggest changes. Appointments flow through a full lifecycle: proposed, confirmed, in progress, completed, dismissed, cancelled, or no-show.

**Reviews and ratings.** After a completed appointment, both client and provider can leave reviews (1-5 stars, text feedback, tags, photos). Providers can respond to reviews. Bidirectional reviews build trust for both sides.

**Calendar.** Appointments render on a calendar view. The frontend groups events by day and shows service, provider/client name, time, and status.

**Contact organization.** Users create folders (Home, Kids, VIP) and assign contacts to them. Private notes can be attached to any contact.

**AI smart replies (planned).** When implemented, an AI inference plugin analyzes conversation context and suggests quick replies ("Sounds good", "What time works?"). Intent detection in chat can auto-suggest appointment creation when scheduling language is detected.

**Offline support (planned).** The Interface SOMA's offline-cache plugin stores contacts, recent messages, calendar events, and profile data locally. Actions taken offline queue and sync on reconnect.

**Multi-device sync (planned).** Multiple Interface SOMAs (phone, browser, tablet) connect to the same Backend SOMA. All subscribe to the same real-time channels. Read receipts, typing indicators, and state changes propagate to all connected devices.


## Semantic Signals

The Backend SOMA sends semantic data to the frontend -- structured JSON describing what to display, not how to display it. The frontend (or a future Interface SOMA renderer) decides presentation.

**Contact list signal:**
```json
{
  "view": "contact_list",
  "sub_tab": "contacts",
  "data": [
    {
      "id": "user_abc",
      "name": "Ana M.",
      "photo_url": "soma://media/photos/ana.jpg",
      "role": "provider",
      "services": ["Hair Stylist", "Makeup"],
      "rating": 4.8,
      "review_count": 23,
      "distance_km": 2.3,
      "online": true,
      "badges": ["verified", "id_checked"],
      "favorited": true
    }
  ],
  "folders": ["Home", "Kids", "VIP"],
  "actions": ["search", "filter", "add_contact"],
  "filters_available": ["service", "location", "rating", "availability"]
}
```

**Chat signal with appointment card** (from Whitepaper Section 16.4):
```json
{
  "view": "chat",
  "peer": {"name": "Ana M.", "online": true},
  "messages": [
    {
      "from": "ana",
      "type": "text",
      "content": "Can you come Thursday at 3?",
      "status": "read"
    },
    {
      "type": "appointment_card",
      "data": {
        "service": "Hair Styling",
        "date": "2026-04-10",
        "time": "15:00",
        "rate": {"amount": 35, "currency": "EUR"},
        "status": "proposed"
      },
      "actions": ["confirm", "dismiss"]
    }
  ],
  "input": {
    "ai_suggestions": ["Sounds good", "What time works?", "I'm not available"]
  }
}
```

**Calendar signal:**
```json
{
  "view": "calendar",
  "month": "2026-04",
  "days_with_events": [7, 10, 12, 15, 20],
  "selected_day": 10,
  "events": [
    {
      "id": "apt_1",
      "time": "15:00-16:00",
      "service": "Hair Styling",
      "with": {"name": "Ana M.", "photo_url": "soma://media/photos/ana.jpg"},
      "status": "confirmed",
      "location": "123 Main St"
    }
  ]
}
```

The Interface SOMA renders these signals using its design knowledge (absorbed from pencil.dev `.pen` files). It controls HOW things look. The Backend SOMA (via the LLM) controls WHAT to show.

**Real-time updates** use Synaptic Protocol subscriptions. The Interface SOMA subscribes to channels (`chat:user_abc`, `presence`, `notifications`) and receives streamed updates -- new messages, status changes, appointment confirmations -- without polling or page reloads.


## How It Was Built

HelperBook is built by a human talking to an LLM (Claude, ChatGPT, or any other), with the LLM driving SOMA via MCP. The human never writes SQL, never configures plugins directly, never touches the database. The LLM translates natural language into structured MCP tool calls.

A typical build session:

```
Human opens Claude with SOMA connected via MCP.

Claude: -> soma.get_state()                        [sees fresh SOMA]
Claude: "I see a fresh SOMA. What are we building?"
Human:  "A service marketplace. Here's the spec."   [shares the product spec]

Claude: -> soma.install_plugin("postgres")
Claude: -> soma.install_plugin("redis")
Claude: -> soma.install_plugin("auth")
         ... installs all needed plugins

Claude: -> soma.postgres.execute("CREATE TABLE users (...)")
Claude: -> soma.record_decision({
             what: "users table with role ENUM(client,provider,both)",
             why: "every user has one account with dual role capability"
           })
         ... creates all 19 tables

Claude: "Data model ready. Setting up authentication..."
         ... configures auth, messaging, scheduling
```

Each structural change is recorded as a decision with reasoning. When a new LLM session starts (even with a different LLM), it calls `soma.get_state()` and has complete context -- every table, every plugin, every decision, every business rule. Zero context loss across sessions.

This is the key architectural revision from Spec 09: the building process happens via LLM + MCP, not via direct conversation with SOMA. SOMA is a pure executor. The LLM brings intelligence. SOMA brings state, memory, and execution.


## Frontend Architecture

The current frontend is a pragmatic web implementation:

**Technology:** Plain JavaScript, Tailwind CSS (CDN), Google Fonts (Inter + Playfair Display), Lucide Icons. No React, no Vue, no build step. The HTML loads component scripts directly.

**Structure:**
```
frontend/
  index.html          -- App shell: header, main content area, bottom nav
  server.js           -- Express server, SOMA MCP bridge
  css/app.css         -- Custom styles
  js/
    api.js            -- MCP communication layer
    app.js            -- App initialization, routing
    components/
      nav.js          -- Bottom tab navigation (Contacts, Chats, Calendar, Profile)
      contacts.js     -- Contact list view
      chat.js         -- Chat view with messages
      calendar.js     -- Calendar view
      profile.js      -- User profile view
      provider-card.js -- Provider card component
```

**MCP Bridge:** The Express server spawns the SOMA binary as a child process with stdio pipes. It sends JSON-RPC messages to SOMA's stdin and reads responses from stdout. The MCP handshake (`initialize` + `notifications/initialized`) runs on first request. All frontend API calls go through `POST /api/mcp`.

**App shell:** A mobile-first layout (`max-w-md mx-auto`) with a sticky header showing the HelperBook brand and SOMA connection status, a scrollable main content area, and a fixed bottom navigation bar with four tabs.


## Configuration

### soma.toml

HelperBook's SOMA configuration (`soma-helperbook/soma.toml`):

```toml
[soma]
id = "helperbook-backend"
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

[memory.consolidation]
enabled = true
trigger = "experience_count"
threshold = 3

[protocol]
bind = "127.0.0.1:9999"

[mcp]
transport = "stdio"
enabled = true

[plugins.postgres]
host = "localhost"
port = 5432
database = "helperbook"
username = "soma"
password_env = "SOMA_PG_PASSWORD"

[plugins.redis]
url = "redis://localhost:6379/0"

[plugins.auth]
session_ttl_hours = 720
otp_ttl_minutes = 5
otp_length = 6
```

### Docker Compose

Two services, both with health checks:

- **PostgreSQL 17** on port 5432 -- user `soma`, password `soma`, database `helperbook`, persistent volume `pgdata`
- **Redis 7 (Alpine)** on port 6379

```bash
docker compose up -d --wait    # Start with health check wait
docker compose down            # Stop
docker compose down -v         # Stop and delete data
```

### Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `SOMA_PG_PASSWORD` | PostgreSQL password | Set to `soma` in scripts |
| `PORT` | Express server port | `8080` |
| `SOMA_MCP_ADMIN_TOKEN` | MCP admin authentication | (optional) |
| `SOMA_MCP_BUILDER_TOKEN` | MCP builder authentication | (optional) |
| `SOMA_MCP_VIEWER_TOKEN` | MCP read-only authentication | (optional) |


## Domain Training Data

The `domain/` directory contains HelperBook-specific training examples that teach the Mind about this application's patterns. The file `helperbook-training.json` provides 25 examples covering:

- **User management** -- registration, profile updates, phone lookups
- **Authentication** -- OTP generation, verification, session creation, login flows
- **Connections** -- sending requests, accepting, listing pending
- **Messaging** -- sending messages, reading unread, counting per chat
- **Appointments** -- creating, confirming, cancelling, listing history
- **Reviews** -- submitting ratings and feedback
- **Geolocation** -- searching providers by radius
- **Caching** -- Redis-based view count tracking
- **Notifications** -- publishing events via Redis pub/sub

Each example includes multiple intent phrasings (5 per example), the expected program (sequence of plugin convention calls), parameter pools for data augmentation, and tags for categorization. This data is used by the Synthesizer to train or fine-tune (via LoRA) the Mind for HelperBook-specific patterns.

Training command:
```bash
scripts/synthesize.sh
# or directly:
soma-synthesize train --plugins ../soma-plugins --domain ./domain --output ../models
```

The domain LoRA gives the Mind knowledge of HelperBook's schema, business rules, and multi-step workflows (like login: verify OTP, then create session, then emit result). Combined with plugin-specific LoRAs from the six catalog plugins, the Mind can generate correct programs for the full range of HelperBook operations.
