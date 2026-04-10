#!/usr/bin/env bash
# build.sh — build the firmware for a given chip.
#
# Usage:
#   ./scripts/build.sh                  # default chip (esp32s3), no wifi
#   ./scripts/build.sh esp32s3
#   ./scripts/build.sh esp32
#   ./scripts/build.sh esp32s3 wifi     # with esp-wifi + TCP listener
#   ./scripts/build.sh esp32 wifi
#
# What it does:
#   1. Sources the Xtensa env (if needed for the chip's architecture)
#   2. Looks up the chip's cargo --config overlay (firmware/chips/<chip>.toml)
#   3. Looks up the chip's full feature list (chip-<chip> + all hardware ports
#      + optional wifi)
#   4. Runs `cargo +esp build --release` with the right config + features
#   5. Prints the output binary path and size

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/lib.sh"

CHIP="$(require_chip "${1:-}")"
VARIANT="${2:-}"
CONFIG="$(chip_config "${CHIP}")"
FEATURES="$(chip_features "${CHIP}" "${VARIANT}")"
TARGET="$(chip_target "${CHIP}")"
ARCH="$(chip_arch "${CHIP}")"

hr
echo "[build] chip:     ${CHIP}"
echo "[build] variant:  ${VARIANT:-(default)}"
echo "[build] arch:     ${ARCH}"
echo "[build] target:   ${TARGET}"
echo "[build] config:   ${CONFIG}"
echo "[build] features: ${FEATURES}"
hr

ensure_esp_env "${CHIP}" || true

# RISC-V chips use stock cargo; Xtensa chips need the +esp toolchain.
if [[ "${ARCH}" == "xtensa" ]]; then
    CARGO_TOOLCHAIN="+esp"
else
    CARGO_TOOLCHAIN=""
fi

cd "${REPO_ROOT}"

# shellcheck disable=SC2086
cargo ${CARGO_TOOLCHAIN} build --release \
    --config "${CONFIG}" \
    --no-default-features \
    --features "${FEATURES}" \
    -p soma-esp32-firmware

BINARY="${TARGET_DIR}/${TARGET}/release/soma-esp32-firmware"
if [[ ! -x "${BINARY}" ]]; then
    echo "[build] expected binary not produced: ${BINARY}" >&2
    exit 1
fi

hr
echo "[build] done"
echo "[build] binary:   ${BINARY}"
echo "[build] size:     $(ls -lh "${BINARY}" | awk '{print $5}') (with debug info)"
echo "[build] flash:    ./scripts/flash.sh ${CHIP}"
hr
