#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [ -f "$PROJECT_ROOT/.env" ]; then
  set -a
  . "$PROJECT_ROOT/.env"
  set +a
fi

echo "=== SOMA-to-SOMA Test Suite ==="
echo ""

FAILED=0

echo "--- Level 1: Transport ---"
if node "$PROJECT_ROOT/test-level1.js"; then
  echo "Level 1: OK"
else
  echo "Level 1: FAILED"
  FAILED=1
fi

echo ""
echo "--- Level 2: Delegation ---"
if node "$PROJECT_ROOT/test-level2.js"; then
  echo "Level 2: OK"
else
  echo "Level 2: FAILED"
  FAILED=1
fi

echo ""
echo "--- Level 3: Transfer ---"
if node "$PROJECT_ROOT/test-level3.js"; then
  echo "Level 3: OK"
else
  echo "Level 3: FAILED"
  FAILED=1
fi

echo ""
if [ "$FAILED" -eq 0 ]; then
  echo "=== All levels passed ==="
else
  echo "=== Some levels failed ==="
  exit 1
fi
