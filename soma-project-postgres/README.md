# soma-project-postgres

Self-contained PostgreSQL project for SOMA MCP.

Query, insert, update, delete, and manage tables in PostgreSQL through the SOMA runtime via MCP `invoke_port` calls. 15 capabilities: `query`, `execute`, `find`, `find_many`, `count`, `aggregate`, `insert`, `update`, `delete`, `create_table`, `drop_table`, `alter_table`, `begin_transaction`, `commit`, `rollback`.

Uses the HelperBook database (19 tables, seeded test data) from `soma-project-helperbook/docker-compose.yml`.

## Setup

### 1. Start PostgreSQL

```bash
cd ../soma-project-helperbook
docker compose up -d postgres
```

### 2. Seed the database (if not already done)

```bash
cd ../soma-project-helperbook
psql -h localhost -U soma -d helperbook -f schema.sql
psql -h localhost -U soma -d helperbook -f seed.sql
```

Password: `soma`

### 3. Run the smoke test

```bash
./scripts/test-all.sh
```

The smoke test queries real HelperBook data: users, appointments, reviews, messages.

## Node Client Commands

```bash
node mcp-client.mjs skills
node mcp-client.mjs query --sql "SELECT name, role FROM users LIMIT 5"
node mcp-client.mjs count --table appointments
node mcp-client.mjs find --table users --id 00000000-0000-0000-0000-000000000001
node mcp-client.mjs find-many --table messages --limit 3
node mcp-client.mjs smoke
```

## Run SOMA MCP Directly

```bash
./scripts/run-mcp.sh
```

Register `./scripts/run-mcp.sh` as the stdio MCP server command in Claude Code or any MCP client.

## Environment

| Variable | Default | Description |
|---|---|---|
| `SOMA_POSTGRES_URL` | `host=localhost user=soma password=soma dbname=helperbook` | libpq connection string |
