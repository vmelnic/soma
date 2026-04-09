#!/usr/bin/env bash
set -euo pipefail

# Usage: start-peer.sh <role> [extra-flags...]
#   role: "filesystem" or "postgres"
#
# Examples:
#   ./scripts/start-peer.sh filesystem --listen 127.0.0.1:9100
#   ./scripts/start-peer.sh postgres   --listen 127.0.0.1:9101 --peer 127.0.0.1:9100

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ROLE="${1:?Usage: start-peer.sh <filesystem|postgres> [extra-flags...]}"
shift

if [ ! -x "$PROJECT_ROOT/bin/soma" ]; then
  printf 'Missing %s\n' "$PROJECT_ROOT/bin/soma" >&2
  exit 1
fi

# Load .env
if [ -f "$PROJECT_ROOT/.env" ]; then
  set -a
  . "$PROJECT_ROOT/.env"
  set +a
fi

case "$ROLE" in
  filesystem)
    export SOMA_PORTS_PLUGIN_PATH=""
    exec "$PROJECT_ROOT/bin/soma" \
      --pack "$PROJECT_ROOT/packs/filesystem/manifest.json" \
      "$@"
    ;;
  postgres)
    export SOMA_PORTS_PLUGIN_PATH="$PROJECT_ROOT/packs/postgres"
    export SOMA_PORTS_REQUIRE_SIGNATURES="${SOMA_PORTS_REQUIRE_SIGNATURES:-false}"
    exec "$PROJECT_ROOT/bin/soma" \
      --pack "$PROJECT_ROOT/packs/postgres/manifest.json" \
      "$@"
    ;;
  *)
    printf 'Unknown role: %s (use "filesystem" or "postgres")\n' "$ROLE" >&2
    exit 1
    ;;
esac
