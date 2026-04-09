# soma-project-smtp

Self-contained SMTP pack project for SOMA MCP.

This folder already includes the runtime binary and the port library in the places SOMA uses them:
- `bin/soma`
- `packs/smtp/manifest.json`
- `packs/smtp/libsoma_port_smtp.dylib`

It also includes:
- `mcp-client.mjs` for exact MCP calls
- sample payload files under `samples/`
- `scripts/run-mcp.sh`
- `scripts/list-skills.sh`
- `scripts/test-all.sh`

## Requirements

- Node.js 18+
- valid SMTP settings in `.env`
- an SMTP relay that works with the current port implementation

## Setup

Edit `.env` and set:
- `SOMA_SMTP_HOST`
- `SOMA_SMTP_PORT`
- `SOMA_SMTP_USERNAME`
- `SOMA_SMTP_PASSWORD`
- `SOMA_SMTP_FROM`
- `SOMA_SMTP_TO`

`SOMA_SMTP_TO` is the mailbox used by the smoke tests.

## Main Commands

List the loaded SMTP skills through MCP:

```bash
./scripts/list-skills.sh
```

Run the full SMTP smoke flow through SOMA MCP:

```bash
./scripts/test-all.sh
```

That smoke flow executes these exact port capabilities:
- `send_plain`
- `send_html`
- `send_attachment`

## Run SOMA MCP Directly

```bash
./scripts/run-mcp.sh
```

Equivalent direct command from this directory:

```bash
SOMA_PORTS_PLUGIN_PATH="$PWD/packs/smtp" ./bin/soma --mcp --pack packs/smtp/manifest.json
```

## Node Client Commands

```bash
node mcp-client.mjs skills
node mcp-client.mjs send-plain
node mcp-client.mjs send-html
node mcp-client.mjs send-attachment
node mcp-client.mjs smoke
```

The Node client starts `scripts/run-mcp.sh`, talks to SOMA over stdio MCP, and uses exact `invoke_port` requests.

## Sample Files

- `samples/plain.txt`
- `samples/body.html`
- `samples/attachment.txt`

## MCP Client Use

If you want Codex CLI or another MCP client to use this pack, register:

```bash
./scripts/run-mcp.sh
```

as the stdio MCP server command.
