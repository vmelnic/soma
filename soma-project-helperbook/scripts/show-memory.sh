#!/usr/bin/env bash
# Show contents of the SOMA persistent memory directory.
# Displays episodes, schemas, routines, and session checkpoints stored on disk.
#
# Usage: ./scripts/show-memory.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [ -f "$PROJECT_ROOT/.env" ]; then
  set -a
  . "$PROJECT_ROOT/.env"
  set +a
fi

DATA_DIR="${SOMA_SOMA_DATA_DIR:-$PROJECT_ROOT/data}"

echo "SOMA Memory: $DATA_DIR"
echo ""

if [ ! -d "$DATA_DIR" ]; then
  echo "  (empty — no data directory yet)"
  exit 0
fi

for f in episodes.json schemas.json routines.json; do
  path="$DATA_DIR/$f"
  if [ -f "$path" ]; then
    count=$(python3 -c "import json; print(len(json.load(open('$path'))))" 2>/dev/null || echo "?")
    size=$(wc -c < "$path" | tr -d ' ')
    echo "  $f: $count items (${size}B)"
  else
    echo "  $f: (not created yet)"
  fi
done

SESSIONS_DIR="$DATA_DIR/sessions"
if [ -d "$SESSIONS_DIR" ]; then
  count=$(ls "$SESSIONS_DIR"/*.json 2>/dev/null | wc -l | tr -d ' ')
  echo "  sessions/: $count checkpoint(s)"
else
  echo "  sessions/: (no checkpoints)"
fi

DUMP="$DATA_DIR/dump.json"
if [ -f "$DUMP" ]; then
  size=$(wc -c < "$DUMP" | tr -d ' ')
  echo "  dump.json: ${size}B (last full state dump)"
fi

echo ""
