#!/usr/bin/env bash
# Attach soma-narrator to a running-or-spawned SOMA MCP server.
#
# Arg 1: path to a run-mcp.sh script (e.g. ../soma-project-mcp-bridge/scripts/run-mcp.sh).
# Extra args forwarded to narrator.mjs.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [ "$#" -lt 1 ]; then
  echo "usage: $0 <path-to-run-mcp.sh> [--speak] [--raw]" >&2
  exit 2
fi

if [ -f "$PROJECT_ROOT/.env" ]; then
  set -a
  . "$PROJECT_ROOT/.env"
  set +a
fi

SERVER="$1"; shift
exec node "$PROJECT_ROOT/narrator.mjs" --server "$SERVER" "$@"
