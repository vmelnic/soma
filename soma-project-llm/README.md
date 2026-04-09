# soma-project-llm

LLM-driven SOMA proof — Ollama generates SQL from natural language questions, SOMA executes against the HelperBook database via the postgres port, Ollama interprets results back to the user.

Demonstrates the LLM-driven path: **LLM is the brain, SOMA is the body.**

## Architecture

```
User question → Ollama (gemma4:e2b) → SQL → SOMA invoke_port("postgres","query") → PostgreSQL → rows → Ollama → answer
```

## Capabilities

- **Consumer role** — questions from a client's perspective (upcoming appointments, spending, provider ratings)
- **Provider role** — questions from a provider's perspective (client count, earnings, reviews, availability)
- **Free-form ask** — any natural language question against the HelperBook schema

## Prerequisites

- Docker (for Ollama)
- HelperBook database running (`cd ../soma-helperbook && docker compose up -d postgres && scripts/setup-db.sh && scripts/seed-db.sh`)
- Node.js 18+

## Setup

```bash
# 1. Start Ollama.
docker compose up -d --wait

# 2. Pull the model (first time only).
scripts/pull-model.sh

# 3. Verify everything works.
node ollama.js smoke
```

## Usage

```bash
# Run consumer role scenarios (Alexandru P. asking questions as a client).
node ollama.js consumer

# Run provider role scenarios (Ana M. asking questions as a hair stylist).
node ollama.js provider

# Run both roles.
node ollama.js both

# Ask a free-form question.
node ollama.js ask --question "Which providers speak French?"

# Ask as a specific role.
node ollama.js ask --question "Show my upcoming bookings" --role provider
```

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `OLLAMA_HOST` | `http://localhost:11434` | Ollama API endpoint |
| `OLLAMA_PORT` | `11434` | Docker-exposed port for Ollama |
| `OLLAMA_MODEL` | `gemma4:e2b` | Model to use for generation |
| `SOMA_POSTGRES_URL` | (see .env) | PostgreSQL connection string (HelperBook DB) |
| `SOMA_PORTS_PLUGIN_PATH` | `./packs/postgres` | Path to postgres port library |
| `SOMA_PORTS_REQUIRE_SIGNATURES` | `false` | Port signature verification |

## MCP Direct

```bash
# Start SOMA in MCP mode (for external LLM integration).
scripts/run-mcp.sh
```
