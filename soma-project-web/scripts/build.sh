#!/usr/bin/env bash
# Build soma-next as a browser-loadable wasm module and regenerate the JS
# glue in `pkg/`. Meant to be run from anywhere — the script resolves its
# own location so the relative paths work from any directory.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SOMA_NEXT_DIR="$(cd "$PROJECT_ROOT/../soma-next" && pwd)"

if ! command -v wasm-bindgen >/dev/null 2>&1; then
  printf 'wasm-bindgen CLI not found on PATH. Install with:\n' >&2
  printf '    cargo install wasm-bindgen-cli --version 0.2.117\n' >&2
  exit 1
fi

# The CLI version must match the wasm-bindgen crate version compiled into the
# .wasm. Mismatches produce a confusing empty pkg/ directory with no error.
CLI_VERSION="$(wasm-bindgen --version | awk '{print $2}')"
printf '[build] wasm-bindgen CLI version: %s\n' "$CLI_VERSION"

printf '[build] compiling soma-next for wasm32-unknown-unknown (release, --no-default-features)\n'
(
  cd "$SOMA_NEXT_DIR"
  cargo build \
    --no-default-features \
    --lib \
    --target wasm32-unknown-unknown \
    --release
)

WASM_INPUT="$SOMA_NEXT_DIR/target/wasm32-unknown-unknown/release/soma_next.wasm"
if [ ! -f "$WASM_INPUT" ]; then
  printf '[build] expected %s, not found\n' "$WASM_INPUT" >&2
  exit 1
fi

printf '[build] running wasm-bindgen (target = web) into %s/pkg\n' "$PROJECT_ROOT"
wasm-bindgen \
  --target web \
  --out-dir "$PROJECT_ROOT/pkg" \
  "$WASM_INPUT"

printf '[build] pkg contents:\n'
ls -lh "$PROJECT_ROOT/pkg/"

printf '[build] done. To serve locally run: ./scripts/serve.sh\n'
