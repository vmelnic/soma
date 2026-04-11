#!/usr/bin/env bash
# Serve the soma-project-web directory over HTTP on localhost:8080.
# The browser needs the wasm module to be fetched with the
# `application/wasm` MIME type (not `file://`), so a local HTTP server
# is required.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

PORT="${PORT:-8080}"
printf '[serve] http://localhost:%s/index.html\n' "$PORT"
printf '[serve] Ctrl-C to stop\n'

cd "$PROJECT_ROOT"
exec python3 -m http.server "$PORT"
