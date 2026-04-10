#!/usr/bin/env bash
# flash.sh — flash the firmware for a given chip to its assigned serial port.
#
# Usage:
#   ./scripts/flash.sh                    # default chip + default port
#   ./scripts/flash.sh esp32s3
#   ./scripts/flash.sh esp32
#   ./scripts/flash.sh esp32 /dev/cu.usbserial-XXXX   # explicit port override
#
# The serial port comes from scripts/devices.env (ESP32S3_PORT, ESP32_PORT,
# etc.). Override with the optional second arg or by exporting the matching
# *_PORT env var.
#
# What it does:
#   1. Verifies the binary exists (won't auto-build — call build.sh first)
#   2. Resolves the chip's serial port
#   3. Runs `espflash flash` with the right binary + port
#   4. Suggests the next step (monitor or test)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/lib.sh"

CHIP="$(require_chip "${1:-}")"
TARGET="$(chip_target "${CHIP}")"
BINARY="${TARGET_DIR}/${TARGET}/release/soma-esp32-firmware"

if [[ ! -x "${BINARY}" ]]; then
    echo "[flash] no built binary at ${BINARY}" >&2
    echo "[flash] run ./scripts/build.sh ${CHIP} first" >&2
    exit 1
fi

# Port: explicit second arg > env var > devices.env default
if [[ -n "${2:-}" ]]; then
    PORT="$2"
else
    PORT="$(chip_port "${CHIP}")"
fi

hr
echo "[flash] chip:    ${CHIP}"
echo "[flash] binary:  ${BINARY}"
echo "[flash] port:    ${PORT}"
hr

ensure_esp_env "${CHIP}" || true

espflash flash --port "${PORT}" "${BINARY}"

hr
echo "[flash] done"
echo "[flash] monitor:   ./scripts/monitor.sh ${CHIP}"
echo "[flash] test wire: ./scripts/test.sh ${CHIP}"
hr
