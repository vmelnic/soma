#!/usr/bin/env bash
# Copy wasm build output and the hello pack from soma-project-web.
#
# Run this once after building soma-project-web (wasm-pack build).
# Dev flow from a fresh clone:
#   1. cd ../soma-project-web && ./scripts/build.sh
#   2. cd ../soma-project-terminal && ./scripts/copy-frontend-assets.sh
# The wasm bundle is large (~1.3 MB) and lives in frontend/pkg/ which
# is gitignored — same convention soma-project-web uses.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
WEB_ROOT="$(cd "$PROJECT_ROOT/../soma-project-web" && pwd)"

if [ ! -f "$WEB_ROOT/pkg/soma_next_bg.wasm" ]; then
  printf '[copy-frontend-assets] %s/pkg/soma_next_bg.wasm not found.\n' "$WEB_ROOT" >&2
  printf '  Build soma-project-web first:\n' >&2
  printf '    cd %s && ./scripts/build.sh\n' "$WEB_ROOT" >&2
  exit 1
fi

mkdir -p "$PROJECT_ROOT/frontend/pkg"
cp "$WEB_ROOT/pkg/soma_next_bg.wasm"      "$PROJECT_ROOT/frontend/pkg/"
cp "$WEB_ROOT/pkg/soma_next_bg.wasm.d.ts" "$PROJECT_ROOT/frontend/pkg/"
cp "$WEB_ROOT/pkg/soma_next.d.ts"         "$PROJECT_ROOT/frontend/pkg/"
cp "$WEB_ROOT/pkg/soma_next.js"           "$PROJECT_ROOT/frontend/pkg/"

mkdir -p "$PROJECT_ROOT/frontend/packs/hello"
cp "$WEB_ROOT/packs/hello/manifest.json"  "$PROJECT_ROOT/frontend/packs/hello/"

printf '[copy-frontend-assets] copied wasm pkg + hello pack from %s\n' "$WEB_ROOT"
