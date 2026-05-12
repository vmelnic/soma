#!/usr/bin/env bash
# Start Ollama on Windows RTX 3090 and pull the model.
# Usage: ./scripts/start-ollama.sh [model]
#   Default model: qwen3:32b

set -euo pipefail
cd "$(dirname "$0")/.."

HOST="vladi@10.10.88.4"
MODEL="${1:-qwen3:32b}"

echo "starting Ollama on $HOST..."
# Start Ollama serve in background (nohup equivalent on Windows)
ssh "$HOST" "start /b ollama serve" 2>/dev/null &
sleep 3

echo "checking Ollama status..."
if ssh "$HOST" "ollama list" 2>/dev/null; then
  echo "  Ollama is running"
else
  echo "  waiting for Ollama to start..."
  sleep 5
  ssh "$HOST" "ollama list" || { echo "FAILED: Ollama not responding"; exit 1; }
fi

echo "pulling $MODEL (this may take a while on first run)..."
ssh "$HOST" "ollama pull $MODEL"
echo "  $MODEL ready"

echo ""
echo "Ollama API available at http://10.10.88.4:11434"
echo "Test: curl http://10.10.88.4:11434/api/tags"
