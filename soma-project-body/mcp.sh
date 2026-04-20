#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

[ -f .env ] && { set -a; source .env; set +a; }

PACKS=""
for m in packs/*/manifest.json; do
  [ -f "$m" ] || continue
  PACKS="$PACKS --pack $m"
done

exec bin/soma --mcp $PACKS --discover-lan --peer 127.0.0.1:9100
