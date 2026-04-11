#!/usr/bin/env bash
# Launch soma-next in MCP mode with the hello pack loaded.
#
# The Python MCP server is spawned as a child of soma-next via the
# PortBackend::McpClient stdio transport. Relative paths in the manifest
# (servers/hello_py/server.py) resolve against CWD, so we cd into the
# project root before execing the binary.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [ ! -x "$PROJECT_ROOT/bin/soma" ]; then
  printf 'Missing %s — run: cp soma-next/target/release/soma %s/bin/soma\n' \
    "$PROJECT_ROOT/bin/soma" "$PROJECT_ROOT" >&2
  exit 1
fi

if ! command -v python3 >/dev/null 2>&1; then
  printf 'python3 not found on PATH — the hello MCP server needs it\n' >&2
  exit 1
fi

cd "$PROJECT_ROOT"
exec "$PROJECT_ROOT/bin/soma" --mcp --pack "$PROJECT_ROOT/packs/hello/manifest.json"
