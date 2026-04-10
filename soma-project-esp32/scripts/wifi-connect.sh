#!/usr/bin/env bash
# wifi-connect.sh — connect a flashed SOMA ESP32 leaf to a WiFi network.
#
# Usage:
#   ./scripts/wifi-connect.sh <chip> <ssid> <password> [--scan]
#
# Examples:
#   ./scripts/wifi-connect.sh esp32 MyNetwork MyPassword
#   ./scripts/wifi-connect.sh esp32s3 MyNetwork MyPassword --scan
#   ./scripts/wifi-connect.sh esp32 'SSID with spaces' 'pass with $pecial'
#
# Prerequisites:
#   1. Firmware must be flashed WITH the wifi feature:
#        ./scripts/cycle.sh <chip> wifi
#      (or ./scripts/build.sh + ./scripts/flash.sh with wifi variant)
#   2. The chip's serial port must be in scripts/devices.env
#   3. The board must be booted and in the "Body alive" state
#
# What it does:
#   1. Opens the chip's serial port at 115200 8N1
#   2. Drains the boot banner
#   3. (if --scan) lists visible APs over wifi.scan
#   4. Calls wifi.configure {ssid, password}
#   5. Polls wifi.status until the chip reports connected + IP assigned
#
# Credentials persist in SPI flash via FlashKvStore — the chip will
# auto-reconnect on every boot after this, until you call wifi.forget.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/lib.sh"

if [[ $# -lt 3 ]]; then
    echo "usage: wifi-connect.sh <chip> <ssid> <password> [--scan]" >&2
    echo "       <chip> = esp32s3 | esp32 | ..." >&2
    exit 3
fi

CHIP="$(require_chip "$1")"
SSID="$2"
PASSWORD="$3"
SCAN_FLAG="${4:-}"

PORT="$(chip_port "${CHIP}")"

ensure_python_venv

hr
echo "[wifi-connect] chip:     ${CHIP}"
echo "[wifi-connect] port:     ${PORT}"
echo "[wifi-connect] ssid:     ${SSID}"
echo "[wifi-connect] password: $(printf '%*s' "${#PASSWORD}" '' | tr ' ' '*')"
if [[ "${SCAN_FLAG}" == "--scan" ]]; then
    echo "[wifi-connect] scan:     yes (will list APs before configure)"
fi
hr

if [[ "${SCAN_FLAG}" == "--scan" ]]; then
    "${PYTHON}" "${SCRIPT_DIR}/wifi-connect.py" "${PORT}" "${SSID}" "${PASSWORD}" --scan
else
    "${PYTHON}" "${SCRIPT_DIR}/wifi-connect.py" "${PORT}" "${SSID}" "${PASSWORD}"
fi
