#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"

if [ -f "$ROOT/.env" ]; then
    set -a; source "$ROOT/.env"; set +a
fi

PACKS=""
for m in "$ROOT"/packs/*/manifest.json; do
    [ -f "$m" ] || continue
    PACKS="$PACKS --pack $m"
done

exec "$ROOT/bin/soma" --mcp $PACKS
