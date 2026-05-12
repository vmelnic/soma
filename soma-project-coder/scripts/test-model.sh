#!/usr/bin/env bash
# Quick test: send a coding prompt to Ollama and measure latency.
# Usage: ./scripts/test-model.sh [model]
set -euo pipefail

HOST="${OLLAMA_HOST:-http://10.10.88.4:11434}"
MODEL="${1:-qwen3:32b}"

echo "testing $MODEL on $HOST..."
echo ""

START=$(date +%s%N)
RESPONSE=$(curl -sf "$HOST/api/generate" \
  -d "{
    \"model\": \"$MODEL\",
    \"prompt\": \"Write a JavaScript function that reverses a string. Return ONLY the function, no explanation.\",
    \"stream\": false,
    \"options\": { \"temperature\": 0.3, \"num_predict\": 256 }
  }")
END=$(date +%s%N)

ELAPSED=$(( (END - START) / 1000000 ))
echo "$RESPONSE" | python3 -c "
import sys, json
data = json.load(sys.stdin)
print(data.get('response', 'NO RESPONSE'))
print()
eval_count = data.get('eval_count', 0)
eval_duration = data.get('eval_duration', 1)
tok_per_sec = eval_count / (eval_duration / 1e9) if eval_duration > 0 else 0
print(f'tokens: {eval_count}, speed: {tok_per_sec:.1f} tok/s')
" 2>/dev/null || echo "$RESPONSE"

echo "wall time: ${ELAPSED}ms"
