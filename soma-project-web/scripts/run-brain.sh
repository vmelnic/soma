#!/usr/bin/env bash
# Launch the brain proxy with .env loaded.
#
# Looks for .env next to this script's parent directory. If it exists,
# uses Node's built-in --env-file flag (Node 20.6+) to inject the
# variables. If it doesn't exist, falls back to the current
# environment — useful for CI or when OPENAI_API_KEY is already
# exported.
#
# Any arguments to this script are passed through to brain-proxy.mjs:
#   ./scripts/run-brain.sh                 # plain launch
#   ./scripts/run-brain.sh --fake          # fake mode (no API key)
#   ./scripts/run-brain.sh --port 9090     # custom port
#   ./scripts/run-brain.sh --model gpt-5   # override model
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

if [ -f .env ]; then
  printf '[run-brain] using .env\n' >&2
  exec node --env-file=.env scripts/brain-proxy.mjs "$@"
else
  printf '[run-brain] no .env found — using current environment.\n' >&2
  printf '[run-brain] To set OPENAI_API_KEY persistently:\n' >&2
  printf '[run-brain]   cp .env.example .env\n' >&2
  printf '[run-brain]   $EDITOR .env\n' >&2
  exec node scripts/brain-proxy.mjs "$@"
fi
