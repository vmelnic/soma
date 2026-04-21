#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

SOMA_NEXT="../soma-next"
SOMA_PORTS="../soma-ports"

CODE_PORTS=(git search runner patch)

usage() {
  cat <<EOF
Usage: $(basename "$0") [target...]

Targets:
  soma          Build the soma-next binary
  all-ports     Build all coding port crates
  <port-name>   Build a specific port (git, search, runner, patch)
  copy          Copy manifests + dylibs to packs/ (no build)

No arguments = build everything (soma + all ports) and copy.

Examples:
  ./build.sh                    # full rebuild
  ./build.sh soma               # rebuild runtime only
  ./build.sh git runner         # rebuild two ports
  ./build.sh copy               # just re-copy artifacts
EOF
  exit 0
}

[[ "${1:-}" == "-h" || "${1:-}" == "--help" ]] && usage

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
  if [[ ! -f "$dylib" ]]; then
    dylib="${dylib%.dylib}.so"
  fi
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
    else
      echo "  warning: no manifest for $port"
    fi
  done
  echo "  done"
}

mkdir -p bin

if [[ $# -eq 0 ]]; then
  build_soma
  for port in "${CODE_PORTS[@]}"; do
    build_port "$port"
  done
  copy_manifests
  echo "full build complete"
  exit 0
fi

for target in "$@"; do
  case "$target" in
    soma)      build_soma ;;
    all-ports)
      for port in "${CODE_PORTS[@]}"; do
        build_port "$port"
      done
      copy_manifests
      ;;
    copy)      copy_manifests ;;
    *)
      found=false
      for port in "${CODE_PORTS[@]}"; do
        if [[ "$port" == "$target" ]]; then
          build_port "$port"
          copy_manifests
          found=true
          break
        fi
      done
      if [[ "$found" == "false" ]]; then
        echo "unknown target: $target"
        echo "valid ports: ${CODE_PORTS[*]}"
        exit 1
      fi
      ;;
  esac
done
