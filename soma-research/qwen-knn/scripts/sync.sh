#!/usr/bin/env bash
# Sync qwen-knn code to Windows machine.
# Code only — never sync index/, corpus/, *.faiss, *.npy. Those live on Windows.

set -euo pipefail
cd "$(dirname "$0")/.."

REMOTE_PATH='C:/Users/vladi/Projects/qwen-knn'

ssh win "if not exist C:\\Users\\vladi\\Projects\\qwen-knn mkdir C:\\Users\\vladi\\Projects\\qwen-knn" 2>/dev/null || true

for item in src bench scripts ingest; do
    [ -d "$item" ] || continue
    scp -q -r "$item" "win:${REMOTE_PATH}/"
done
for f in *.md *.txt requirements.txt; do
    [ -e "$f" ] || continue
    scp -q "$f" "win:${REMOTE_PATH}/"
done
echo "synced code -> win:${REMOTE_PATH} (excluded: index/, corpus/, *.faiss, *.npy)"
