# soma-project-body

Vue 3 + Capacitor PWA. Turns any device into a SOMA limb.

Operator speaks a goal → decider LLM picks the action → SOMA executes via ports → narrator LLM reports back.

## Views

- **Talk** — natural language input, decider routes to `invoke_port` or `create_goal_async`, narrator speaks results. Voice input/output via Web Speech API.
- **Work** — live sessions, event stream, inter-device handoff.
- **Apps** — routines (mined or authored). Export/import as `.somapp.json`.
- **Devices** — port inventory, auth token, per-role LLM config (provider, model, API key, narrator mode).

## Architecture

Browser ports (camera, geo, haptics, clipboard, filesystem, mic, notifications, nfc, ocr) register via `reverse/register_ports` over WebSocket. When SOMA needs a device capability, it sends `reverse/invoke_port` back over the same connection.

Runtime ports (postgres, redis, s3, smtp, etc.) run server-side as dynamic libraries loaded from `packs/`.

## Setup

```bash
cp .env.example .env           # edit credentials
./build.sh                     # build soma + all ports, copy to bin/ and packs/
docker compose up -d           # postgres, redis, mongo, mysql, minio, mailcatcher
npm install
npm run dev                    # vite on :5173
npm run soma                   # soma runtime on ws://:7890
```

## Build script

```bash
./build.sh                     # full rebuild (soma + all ports)
./build.sh soma                # runtime only
./build.sh postgres redis      # specific ports
./build.sh all-ports           # all ports, skip runtime
./build.sh copy                # re-copy manifests without building
```

Requires `../soma-next` and `../soma-ports` as siblings.

## Ports (21 packs)

| Pack | Kind | Docker | Credentials |
|------|------|--------|-------------|
| postgres | database | `postgres:17` | `SOMA_POSTGRES_URL` |
| redis | cache | `redis:7` | `SOMA_REDIS_URL` |
| mongodb | database | `mongo:7` | `SOMA_MONGODB_URL` |
| mysql | database | `mysql:8` | `SOMA_MYSQL_URL` |
| s3 | storage | `minio` | `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` |
| smtp | email | `mailcatcher` | `SOMA_SMTP_HOST`, `SOMA_SMTP_FROM` |
| elasticsearch | search | optional | `SOMA_ELASTICSEARCH_URL` |
| google-mail | email | — | `SOMA_GOOGLE_ACCESS_TOKEN` |
| google-calendar | calendar | — | `SOMA_GOOGLE_ACCESS_TOKEN` |
| google-drive | storage | — | `SOMA_GOOGLE_ACCESS_TOKEN` |
| slack | messaging | — | `SOMA_SLACK_BOT_TOKEN` |
| twilio | messaging | — | `SOMA_TWILIO_ACCOUNT_SID`, `SOMA_TWILIO_AUTH_TOKEN` |
| stripe | payment | — | `SOMA_STRIPE_SECRET_KEY` |
| auth, crypto, timer, calendar, geo, image, pdf, push | local | — | none |

## Brains

Decider and narrator configured independently per-role. Built-in providers: OpenAI, Anthropic, Groq, GLM, Together, Ollama, LM Studio. Any OpenAI-compatible endpoint works.

Narrator modes: `terse` (one sentence), `debug` (with port IDs), `alarm` (failures only).

## Mobile

```bash
npm run build
npx cap add ios && npm run cap:sync && npm run cap:ios
npx cap add android && npm run cap:sync && npm run cap:android
```

## Tests

```bash
npm test                       # JS unit tests (node:test)
cd ../soma-next && cargo test  # runtime tests
```
