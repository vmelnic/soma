#!/usr/bin/env bash
# wifi-scan.sh — list WiFi networks visible to a flashed SOMA ESP32 leaf.
#
# Usage:
#   ./scripts/wifi-scan.sh                    # default chip (esp32s3)
#   ./scripts/wifi-scan.sh esp32s3
#   ./scripts/wifi-scan.sh esp32              # ESP32 WROOM-32D
#
# Prerequisites:
#   1. Firmware must be flashed WITH the wifi feature:
#        ./scripts/cycle.sh <chip> wifi
#   2. The chip's serial port must be in scripts/devices.env
#
# Best practice for ESP32 LX6: run this as the FIRST wifi operation after
# boot. esp-wifi 0.12 has a known bug where wifi.scan crashes with an
# illegal-instruction exception on ESP32 LX6 if called after any SPI flash
# writes in the same boot cycle. ESP32-S3 is unaffected. If you hit the
# crash, just reset the board and run scan again before doing anything else.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/lib.sh"

CHIP="$(require_chip "${1:-}")"
PORT="$(chip_port "${CHIP}")"

ensure_python_venv

hr
echo "[wifi-scan] chip: ${CHIP}"
echo "[wifi-scan] port: ${PORT}"
hr

"${PYTHON}" "${SCRIPT_DIR}/wifi-scan.py" "${PORT}"
