#!/usr/bin/env bash
# Sync soma-engram code (only) to Windows machine.
# Never syncs .pt checkpoint or data files — those live only on Windows.

set -euo pipefail
cd "$(dirname "$0")/.."

REMOTE_PATH='C:/Users/vladi/Projects/soma-engram'

ssh win "if not exist C:\\Users\\vladi\\Projects\\soma-engram mkdir C:\\Users\\vladi\\Projects\\soma-engram" 2>/dev/null || true

# Sync only code: .py, .sh, .md, .txt; recursive subdirs (extract, ltc, tests, scripts)
# Excludes data/, checkpoints/, *.pt explicitly — large files stay on Windows
for item in extract ltc tests scripts; do
    [ -d "$item" ] || continue
    scp -q -r "$item" "win:${REMOTE_PATH}/"
done
for f in *.py *.md *.txt requirements.txt; do
    [ -e "$f" ] || continue
    scp -q "$f" "win:${REMOTE_PATH}/"
done
echo "synced code -> win:${REMOTE_PATH} (excluded: data/, checkpoints/, *.pt)"
