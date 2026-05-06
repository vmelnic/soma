#!/usr/bin/env bash
# Run a Python script on Windows RTX 3090 and stream output back.
# Usage: ./scripts/run.sh extract.py --model qwen-3b
#        ./scripts/run.sh distill.py --steps 1000

set -euo pipefail
cd "$(dirname "$0")/.."

REMOTE_PATH='C:\Users\vladi\Projects\soma-engram'
VENV_PYTHON='.venv\Scripts\python.exe'

# Sync first
./scripts/sync.sh > /dev/null

# Use existing soma-brain venv (has torch+cuda+transformers)
VENV_PYTHON='C:\Users\vladi\Projects\soma-brain\.venv\Scripts\python.exe'

# Quote args for Windows cmd.exe — wrap in double quotes, escape inner ones
QUOTED=""
for arg in "$@"; do
    inner=$(printf '%s' "$arg" | sed 's/"/\\"/g')
    QUOTED="$QUOTED \"$inner\""
done
ssh win "chcp 65001 >nul & set PYTHONIOENCODING=utf-8& cd /d ${REMOTE_PATH} & ${VENV_PYTHON}${QUOTED}"
