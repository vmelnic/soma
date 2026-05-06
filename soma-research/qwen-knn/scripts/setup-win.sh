#!/usr/bin/env bash
# One-time Windows setup: create venv, install requirements.
# Usage: ./scripts/setup-win.sh

set -euo pipefail
cd "$(dirname "$0")/.."

REMOTE_PATH='C:\Users\vladi\Projects\qwen-knn'

./scripts/sync.sh > /dev/null

ssh win "cd /d ${REMOTE_PATH} & python -m venv .venv & .venv\\Scripts\\python.exe -m pip install --upgrade pip & .venv\\Scripts\\pip.exe install -r requirements.txt"
echo "venv + deps installed at win:${REMOTE_PATH}\\.venv"
