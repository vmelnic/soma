#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

SOMA_NEXT="../soma-next"
SOMA_PORTS="../soma-ports"

# All port crate names (workspace members + redis which is excluded)
ALL_PORTS=(assemblyai auth calendar crypto deepgram elasticsearch geo google-calendar google-drive google-mail image mongodb mysql pdf postgres push redis s3 slack smtp sqlite stripe timer twilio youtube)
REDIS_SEPARATE=true

usage() {
  cat <<EOF
Usage: $(basename "$0") [target...]

Targets:
  soma          Build the soma-next binary
  all-ports     Build all port crates
  <port-name>   Build a specific port (e.g. postgres, redis, slack)
  copy          Copy manifests + dylibs to packs/ (no build)

No arguments = build everything (soma + all ports) and copy.

Examples:
  ./build.sh                    # full rebuild
  ./build.sh soma               # rebuild runtime only
  ./build.sh postgres redis     # rebuild two ports
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
  if [[ "$port" == "redis" && "$REDIS_SEPARATE" == "true" ]]; then
    echo "building $port (separate manifest)..."
    (cd "$SOMA_PORTS/redis" && cargo build --release)
    local dylib="$SOMA_PORTS/redis/target/release/lib${libname}.dylib"
  else
    echo "building $port..."
    (cd "$SOMA_PORTS" && cargo build --release -p "$crate")
    local dylib="$SOMA_PORTS/target/release/lib${libname}.dylib"
  fi
  # .so for linux
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
  for port in "${ALL_PORTS[@]}"; do
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
  for port in "${ALL_PORTS[@]}"; do
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
      for port in "${ALL_PORTS[@]}"; do
        build_port "$port"
      done
      copy_manifests
      ;;
    copy)      copy_manifests ;;
    *)
      found=false
      for port in "${ALL_PORTS[@]}"; do
        if [[ "$port" == "$target" ]]; then
          build_port "$port"
          copy_manifests
          found=true
          break
        fi
      done
      if [[ "$found" == "false" ]]; then
        echo "unknown target: $target"
        echo "valid ports: ${ALL_PORTS[*]}"
        exit 1
      fi
      ;;
  esac
done
