#!/usr/bin/env bash
# Run a Python script on Windows RTX 3090 and stream output back.
# Usage: ./scripts/run.sh src/build_index.py --corpus corpus/rust-book --out index/rust-book

set -euo pipefail
cd "$(dirname "$0")/.."

REMOTE_PATH='C:\Users\vladi\Projects\qwen-knn'
VENV_PYTHON='C:\Users\vladi\Projects\qwen-knn\.venv\Scripts\python.exe'

./scripts/sync.sh > /dev/null

QUOTED=""
for arg in "$@"; do
    inner=$(printf '%s' "$arg" | sed 's/"/\\"/g')
    QUOTED="$QUOTED \"$inner\""
done
ssh win "chcp 65001 >nul & set PYTHONIOENCODING=utf-8& cd /d ${REMOTE_PATH} & set PYTHONPATH=${REMOTE_PATH}\\src& ${VENV_PYTHON}${QUOTED}"
