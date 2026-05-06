#!/usr/bin/env bash
# Fetch a file or directory from Windows back to local.
# Usage: ./scripts/fetch.sh logs/bench.txt

set -euo pipefail
cd "$(dirname "$0")/.."

REMOTE='C:/Users/vladi/Projects/qwen-knn'
TARGET="${1:?usage: fetch.sh <relative-path>}"

mkdir -p "$(dirname "$TARGET")"
scp -q -r "win:${REMOTE}/${TARGET}" "$(dirname "$TARGET")/"
echo "fetched -> $TARGET"
