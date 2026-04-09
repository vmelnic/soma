#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [ -f "$PROJECT_ROOT/.env" ]; then
  set -a
  . "$PROJECT_ROOT/.env"
  set +a
fi

OLLAMA_HOST="${OLLAMA_HOST:-http://localhost:11434}"
OLLAMA_MODEL="${OLLAMA_MODEL:-gemma4:e2b}"

printf 'Pulling model %s from %s ...\n' "$OLLAMA_MODEL" "$OLLAMA_HOST"

curl -sf "$OLLAMA_HOST/api/pull" \
  -d "{\"name\": \"$OLLAMA_MODEL\"}" \
  --no-buffer | while IFS= read -r line; do
    status=$(printf '%s' "$line" | grep -o '"status":"[^"]*"' | head -1 | cut -d'"' -f4)
    if [ -n "$status" ]; then
      printf '\r\033[K%s' "$status"
    fi
  done

printf '\nDone.\n'
