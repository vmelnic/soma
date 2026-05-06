#!/bin/bash
set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

SOMA="$PROJECT_DIR/bin/soma"
PACKS="$PROJECT_DIR/packs"

if [ ! -x "$SOMA" ]; then
  echo "error: $SOMA not found" >&2
  exit 1
fi

# Load project .env (values can be overridden by the shell environment)
if [ -f "$PROJECT_DIR/.env" ]; then
  set -a
  source "$PROJECT_DIR/.env"
  set +a
fi

export SOMA_SOMA_DATA_DIR="${SOMA_SOMA_DATA_DIR:-$PROJECT_DIR/data}"
export SOMA_WS_URL="${SOMA_WS_URL:-ws://127.0.0.1:9090}"
export BRIDGE_PORT="${BRIDGE_PORT:-3000}"

# Kill stale processes from previous runs
lsof -ti :9090 | xargs kill 2>/dev/null || true
lsof -ti :"$BRIDGE_PORT" | xargs kill 2>/dev/null || true
sleep 0.5

cleanup() {
  kill $SOMA_PID $BRIDGE_PID 2>/dev/null || true
  wait $SOMA_PID $BRIDGE_PID 2>/dev/null || true
}
trap cleanup EXIT INT TERM

PACK_ARGS=""
for m in "$PACKS"/*/manifest.json; do
  [ -f "$m" ] || continue
  PACK_ARGS="$PACK_ARGS --pack $m"
done

"$SOMA" --mcp \
  $PACK_ARGS \
  --mcp-ws-listen 127.0.0.1:9090 \
  "$@" </dev/null &
SOMA_PID=$!

sleep 1

node "$PROJECT_DIR/bridge.mjs" &
BRIDGE_PID=$!

echo "helperbook: soma pid=$SOMA_PID, bridge pid=$BRIDGE_PID, http://localhost:$BRIDGE_PORT"
wait
