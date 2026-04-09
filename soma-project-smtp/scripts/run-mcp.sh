#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [ ! -x "$PROJECT_ROOT/bin/soma" ]; then
  printf 'Missing %s\n' "$PROJECT_ROOT/bin/soma" >&2
  exit 1
fi

if [ ! -f "$PROJECT_ROOT/packs/smtp/libsoma_port_smtp.dylib" ] && [ ! -f "$PROJECT_ROOT/packs/smtp/libsoma_port_smtp.so" ]; then
  printf 'Missing SMTP port library in %s\n' "$PROJECT_ROOT/packs/smtp" >&2
  exit 1
fi

if [ -f "$PROJECT_ROOT/.env" ]; then
  set -a
  . "$PROJECT_ROOT/.env"
  set +a
fi

export SOMA_PORTS_PLUGIN_PATH="$PROJECT_ROOT/packs/smtp"
export SOMA_PORTS_REQUIRE_SIGNATURES="${SOMA_PORTS_REQUIRE_SIGNATURES:-false}"

exec "$PROJECT_ROOT/bin/soma" --mcp --pack "$PROJECT_ROOT/packs/smtp/manifest.json"
