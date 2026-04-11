#!/usr/bin/env bash
# Run the Playwright browser proofs against the current pkg/ build.
#
# Preconditions: ./scripts/build.sh has been run at least once to
# populate pkg/. Playwright's webServer starts python3 -m http.server
# automatically, so a separate ./scripts/serve.sh is not required.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [ ! -d "$PROJECT_ROOT/pkg" ]; then
  printf '[test-browser] pkg/ not found — run ./scripts/build.sh first\n' >&2
  exit 1
fi

cd "$PROJECT_ROOT"
exec npx playwright test "$@"
