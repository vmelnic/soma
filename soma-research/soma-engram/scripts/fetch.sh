#!/usr/bin/env bash
# Fetch a file or directory from Windows back to local.
# Usage: ./scripts/fetch.sh checkpoints/sdm.bin
#        ./scripts/fetch.sh logs/extract.log

set -euo pipefail
cd "$(dirname "$0")/.."

REMOTE='C:/Users/vladi/Projects/soma-engram'
TARGET="${1:?usage: fetch.sh <relative-path>}"

mkdir -p "$(dirname "$TARGET")"
scp -q -r "win:${REMOTE}/${TARGET}" "$(dirname "$TARGET")/"
echo "fetched -> $TARGET"
