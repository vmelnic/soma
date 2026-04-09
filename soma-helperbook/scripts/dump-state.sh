#!/usr/bin/env bash
# Dump the full SOMA runtime state to data/dump.json.
# This captures everything: sessions, episodes, schemas, routines, ports, packs,
# skills, beliefs, metrics, and self-model — a complete portable snapshot.
#
# Usage:
#   ./scripts/dump-state.sh                  # dump to data/dump.json
#   ./scripts/dump-state.sh ./my-backup.json # dump to custom path
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
OUTPUT="${1:-$PROJECT_ROOT/data/dump.json}"

if [ -f "$PROJECT_ROOT/.env" ]; then
  set -a
  . "$PROJECT_ROOT/.env"
  set +a
fi

export SOMA_PORTS_PLUGIN_PATH="$PROJECT_ROOT/packs/postgres:$PROJECT_ROOT/packs/redis:$PROJECT_ROOT/packs/auth"
export SOMA_PORTS_REQUIRE_SIGNATURES="${SOMA_PORTS_REQUIRE_SIGNATURES:-false}"
export SOMA_SOMA_DATA_DIR="${SOMA_SOMA_DATA_DIR:-$PROJECT_ROOT/data}"

mkdir -p "$(dirname "$OUTPUT")"

printf '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}\n{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"dump_state","arguments":{"sections":["full"]}}}\n' \
  | "$PROJECT_ROOT/bin/soma" --mcp \
      --pack "$PROJECT_ROOT/packs/postgres/manifest.json" \
      --pack "$PROJECT_ROOT/packs/redis/manifest.json" \
      --pack "$PROJECT_ROOT/packs/auth/manifest.json" \
      2>/dev/null \
  | while IFS= read -r line; do
      # Take the second JSON-RPC response (id=2) which is the dump_state result
      id=$(printf '%s' "$line" | python3 -c "import sys,json; print(json.load(sys.stdin).get('id',''))" 2>/dev/null || true)
      if [ "$id" = "2" ]; then
        printf '%s' "$line" | python3 -c "
import sys, json
resp = json.load(sys.stdin)
result = resp.get('result', {})
json.dump(result, sys.stdout, indent=2)
print()
"
        break
      fi
    done > "$OUTPUT"

SIZE=$(wc -c < "$OUTPUT" | tr -d ' ')
SECTIONS=$(python3 -c "import json; d=json.load(open('$OUTPUT')); print(', '.join(sorted(d.keys())))" 2>/dev/null || echo "?")

echo "State dumped to: $OUTPUT"
echo "Size: ${SIZE} bytes"
echo "Sections: $SECTIONS"
echo ""
echo "This file contains the complete SOMA runtime state."
echo "An LLM can load this as context instead of reading source code."
