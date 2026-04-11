#!/usr/bin/env bash
# Copy soma-next binary + the 3 port dylibs we need (crypto, postgres,
# smtp) into bin/. Run this after a clean checkout or after rebuilding
# soma-next / soma-ports.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SOMA_NEXT="$(cd "$PROJECT_ROOT/../soma-next" && pwd)"
SOMA_PORTS="$(cd "$PROJECT_ROOT/../soma-ports" && pwd)"

mkdir -p "$PROJECT_ROOT/bin"

# soma-next binary
SOMA_BIN="$SOMA_NEXT/target/release/soma"
if [ ! -x "$SOMA_BIN" ]; then
  printf 'error: %s not found (or not executable)\n' "$SOMA_BIN" >&2
  printf 'build soma-next first: (cd %s && cargo build --release)\n' "$SOMA_NEXT" >&2
  exit 1
fi
cp "$SOMA_BIN" "$PROJECT_ROOT/bin/soma"
printf '[copy-binaries] soma binary: %d bytes\n' "$(stat -f%z "$PROJECT_ROOT/bin/soma" 2>/dev/null || stat -c%s "$PROJECT_ROOT/bin/soma")"

# Port dylibs
for port in crypto postgres smtp; do
  if [[ "$OSTYPE" == "darwin"* ]]; then
    ext="dylib"
  else
    ext="so"
  fi
  src="$SOMA_PORTS/target/release/libsoma_port_${port}.${ext}"
  if [ ! -f "$src" ]; then
    printf 'error: %s not found\n' "$src" >&2
    printf 'build soma-ports first: (cd %s && cargo build --workspace --release)\n' "$SOMA_PORTS" >&2
    exit 1
  fi
  dst="$PROJECT_ROOT/bin/libsoma_port_${port}.${ext}"
  cp "$src" "$dst"
  printf '[copy-binaries] %s port: %d bytes\n' "$port" "$(stat -f%z "$dst" 2>/dev/null || stat -c%s "$dst")"
done

# macOS gatekeeper — strip quarantine + re-sign so the dylibs can
# load. These are ad-hoc signatures; for distribution you'd sign with
# a real identity.
if [[ "$OSTYPE" == "darwin"* ]]; then
  xattr -d com.apple.quarantine "$PROJECT_ROOT/bin/soma" 2>/dev/null || true
  codesign -fs - "$PROJECT_ROOT/bin/soma" 2>/dev/null || true
  for port in crypto postgres smtp; do
    xattr -d com.apple.quarantine "$PROJECT_ROOT/bin/libsoma_port_${port}.dylib" 2>/dev/null || true
    codesign -fs - "$PROJECT_ROOT/bin/libsoma_port_${port}.dylib" 2>/dev/null || true
  done
fi

printf '[copy-binaries] bin/ ready:\n'
ls -lh "$PROJECT_ROOT/bin/"
