#!/usr/bin/env bash
# Check if Ollama is running and which models are loaded.
set -euo pipefail

HOST="${OLLAMA_HOST:-http://10.10.88.4:11434}"

echo "checking $HOST..."
if curl -sf "$HOST/api/tags" > /dev/null 2>&1; then
  echo "  Ollama is running"
  curl -sf "$HOST/api/tags" | python3 -c "
import sys, json
data = json.load(sys.stdin)
for m in data.get('models', []):
    size_gb = m.get('size', 0) / 1e9
    print(f\"  {m['name']:30s} {size_gb:.1f}GB\")
" 2>/dev/null || curl -sf "$HOST/api/tags"
else
  echo "  Ollama not responding at $HOST"
  echo "  Run: ./scripts/start-ollama.sh"
  exit 1
fi
