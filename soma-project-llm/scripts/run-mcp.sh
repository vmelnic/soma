#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [ ! -x "$PROJECT_ROOT/bin/soma" ]; then
  printf 'Missing %s\n' "$PROJECT_ROOT/bin/soma" >&2
  exit 1
fi

if [ ! -f "$PROJECT_ROOT/packs/postgres/libsoma_port_postgres.dylib" ] && [ ! -f "$PROJECT_ROOT/packs/postgres/libsoma_port_postgres.so" ]; then
  printf 'Missing postgres port library in %s\n' "$PROJECT_ROOT/packs/postgres" >&2
  exit 1
fi

if [ -f "$PROJECT_ROOT/.env" ]; then
  set -a
  . "$PROJECT_ROOT/.env"
  set +a
fi

export SOMA_PORTS_PLUGIN_PATH="$PROJECT_ROOT/packs/postgres"
export SOMA_PORTS_REQUIRE_SIGNATURES="${SOMA_PORTS_REQUIRE_SIGNATURES:-false}"

exec "$PROJECT_ROOT/bin/soma" --mcp --pack "$PROJECT_ROOT/packs/postgres/manifest.json"
