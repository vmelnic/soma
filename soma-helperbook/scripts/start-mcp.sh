#!/bin/bash
# Start HelperBook SOMA in MCP mode
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

if [ ! -x "$PROJECT_DIR/bin/soma" ]; then
  printf 'Missing %s\n' "$PROJECT_DIR/bin/soma" >&2
  exit 1
fi

if [ -f "$PROJECT_DIR/.env" ]; then
  set -a
  . "$PROJECT_DIR/.env"
  set +a
fi

export SOMA_PORTS_PLUGIN_PATH="$PROJECT_DIR/packs/postgres:$PROJECT_DIR/packs/redis:$PROJECT_DIR/packs/auth"
export SOMA_PORTS_REQUIRE_SIGNATURES="${SOMA_PORTS_REQUIRE_SIGNATURES:-false}"

exec "$PROJECT_DIR/bin/soma" --mcp \
  --pack "$PROJECT_DIR/packs/postgres/manifest.json" \
  --pack "$PROJECT_DIR/packs/redis/manifest.json" \
  --pack "$PROJECT_DIR/packs/auth/manifest.json" \
  "$@"
