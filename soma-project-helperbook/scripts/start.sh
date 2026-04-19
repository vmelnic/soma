#!/bin/bash
# Start HelperBook SOMA in REPL mode
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

echo "Starting services..."
cd "$PROJECT_DIR"
docker compose up -d --wait

if [ -f "$PROJECT_DIR/.env" ]; then
  set -a
  . "$PROJECT_DIR/.env"
  set +a
fi

export SOMA_PORTS_PLUGIN_PATH="$PROJECT_DIR/packs/postgres:$PROJECT_DIR/packs/redis:$PROJECT_DIR/packs/auth"
export SOMA_PORTS_REQUIRE_SIGNATURES="${SOMA_PORTS_REQUIRE_SIGNATURES:-false}"

echo "Starting SOMA HelperBook..."
exec "$PROJECT_DIR/bin/soma" \
  --pack "$PROJECT_DIR/packs/postgres/manifest.json" \
  --pack "$PROJECT_DIR/packs/redis/manifest.json" \
  --pack "$PROJECT_DIR/packs/auth/manifest.json" \
  repl "$@"
