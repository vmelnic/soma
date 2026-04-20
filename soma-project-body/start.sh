#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

if [ -f .env ]; then
  set -a; source .env; set +a
fi

TOKEN_ARG=""
if [ -n "${SOMA_WS_TOKEN:-}" ]; then
  TOKEN_ARG="--mcp-ws-token $SOMA_WS_TOKEN"
fi

PACKS=""
for m in packs/*/manifest.json; do
  [ -f "$m" ] || continue
  PACKS="$PACKS --pack $m"
done

echo "starting soma runtime (ws :7890)..."
exec bin/soma --mcp --mcp-ws-listen 127.0.0.1:7890 $TOKEN_ARG $PACKS
