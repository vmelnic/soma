#!/usr/bin/env bash
set -euo pipefail

SOMA_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PORTS_DIR="$SOMA_ROOT/soma-ports"
SOMA_BIN="$SOMA_ROOT/soma-next/target/release/soma"

if [ $# -lt 2 ]; then
  echo "usage: $0 <project-dir> <port> [<port> ...]"
  echo "example: $0 projects/helperbook postgres redis auth"
  exit 1
fi

PROJECT="$1"; shift

if [ ! -d "$PROJECT" ]; then
  echo "error: project directory '$PROJECT' not found"
  exit 1
fi

# Copy soma binary
mkdir -p "$PROJECT/bin"
if [ -f "$SOMA_BIN" ]; then
  cp "$SOMA_BIN" "$PROJECT/bin/soma"
  xattr -d com.apple.quarantine "$PROJECT/bin/soma" 2>/dev/null || true
  codesign -fs - "$PROJECT/bin/soma" 2>/dev/null || true
  echo "  bin/soma copied"
else
  echo "  WARN: soma binary not found at $SOMA_BIN"
fi

# Copy each port's manifest + dylib
for PORT in "$@"; do
  MANIFEST="$PORTS_DIR/$PORT/manifest.json"
  # dylib can be in port's own target or shared target
  DYLIB="$PORTS_DIR/$PORT/target/release/libsoma_port_${PORT}.dylib"
  if [ ! -f "$DYLIB" ]; then
    DYLIB="$PORTS_DIR/target/release/libsoma_port_${PORT}.dylib"
  fi

  mkdir -p "$PROJECT/packs/$PORT"

  if [ -f "$MANIFEST" ]; then
    cp "$MANIFEST" "$PROJECT/packs/$PORT/manifest.json"
  else
    echo "  WARN: manifest not found for $PORT"
  fi

  if [ -f "$DYLIB" ]; then
    cp "$DYLIB" "$PROJECT/packs/$PORT/"
  else
    echo "  WARN: dylib not found for $PORT"
  fi

  echo "  packs/$PORT synced"
done

echo "done"
