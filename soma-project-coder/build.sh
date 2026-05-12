#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

SOMA_NEXT="../soma-next"
SOMA_PORTS="../soma-ports"

CODE_PORTS=(git search runner patch)

mkdir -p bin

build_soma() {
  echo "building soma-next..."
  (cd "$SOMA_NEXT" && cargo build --release --bin soma)
  cp "$SOMA_NEXT/target/release/soma" bin/soma
  xattr -cr bin/soma 2>/dev/null || true
  codesign -fs - bin/soma 2>/dev/null || true
  echo "  bin/soma updated"
}

build_port() {
  local port="$1"
  local crate="soma-port-${port}"
  local libname="soma_port_${port//-/_}"
  echo "building $port..."
  (cd "$SOMA_PORTS" && cargo build --release -p "$crate")
  local dylib="$SOMA_PORTS/target/release/lib${libname}.dylib"
  [[ -f "$dylib" ]] || dylib="${dylib%.dylib}.so"
  mkdir -p "packs/$port"
  cp "$dylib" "packs/$port/"
  codesign -fs - "packs/$port/$(basename "$dylib")" 2>/dev/null || true
  echo "  packs/$port/$(basename "$dylib")"
}

copy_manifests() {
  echo "copying manifests..."
  for port in "${CODE_PORTS[@]}"; do
    local src="$SOMA_PORTS/$port/manifest.json"
    if [[ -f "$src" ]]; then
      mkdir -p "packs/$port"
      cp "$src" "packs/$port/manifest.json"
    fi
  done
  echo "  done"
}

if [[ $# -eq 0 ]]; then
  build_soma
  for port in "${CODE_PORTS[@]}"; do build_port "$port"; done
  copy_manifests
  echo "full build complete"
  exit 0
fi

for target in "$@"; do
  case "$target" in
    soma)      build_soma ;;
    all-ports) for port in "${CODE_PORTS[@]}"; do build_port "$port"; done; copy_manifests ;;
    copy)      copy_manifests ;;
    *)         build_port "$target"; copy_manifests ;;
  esac
done
