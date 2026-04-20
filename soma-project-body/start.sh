#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

# Load .env if present
if [ -f .env ]; then
  set -a; source .env; set +a
fi

# Collect all pack manifests (skip mongodb — its cdylib crashes at dlopen
# due to signal_hook_registry conflict with the host tokio runtime)
PACKS=""
for m in packs/*/manifest.json; do
  [ -f "$m" ] || continue
  case "$m" in *mongodb*) continue ;; esac
  PACKS="$PACKS --pack $m"
done

TOKEN_ARG=""
if [ -n "${SOMA_WS_TOKEN:-}" ]; then
  TOKEN_ARG="--mcp-ws-token $SOMA_WS_TOKEN"
fi

echo "starting soma runtime (ws :7890)..."
exec bin/soma --mcp --mcp-ws-listen 0.0.0.0:7890 $TOKEN_ARG $PACKS
