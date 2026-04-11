#!/usr/bin/env bash
# Start the Node backend with .env loaded.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

if [ ! -f .env ]; then
  printf '[start-backend] .env not found. Copy .env.example:\n' >&2
  printf '    cp .env.example .env\n' >&2
  exit 1
fi

exec node --env-file=.env backend/server.mjs
