#!/usr/bin/env bash
# boards.sh — list every connected ESP board with chip type, flash size,
# revision, MAC address, and the matching firmware/chips/<chip>.toml overlay.
#
# Usage:
#   ./scripts/boards.sh
#
# No args. Walks every /dev/cu.usb* device that isn't an obvious bluetooth
# or debug-console node and probes it with `espflash board-info`.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/lib.sh"

ensure_esp_env || true

hr
echo "[boards] connected ESP devices"
hr

# Gather candidate ports. macOS exposes USB-to-UART bridges as cu.usbserial-*
# and native USB Serial JTAG as cu.usbmodem*.
shopt -s nullglob
PORTS=(/dev/cu.usbserial-* /dev/cu.usbmodem*)
shopt -u nullglob

if [[ ${#PORTS[@]} -eq 0 ]]; then
    echo "no candidate serial ports found in /dev/cu.usb*"
    echo "make sure the board is plugged in with a data-capable cable"
    exit 1
fi

found=0
for port in "${PORTS[@]}"; do
    echo
    echo "Port: ${port}"
    if info=$(espflash board-info --port "${port}" 2>&1); then
        chip=$(echo "${info}" | awk '/^Chip type:/ {print $3}')
        flash=$(echo "${info}" | awk '/^Flash size:/ {print $3}')
        mac=$(echo "${info}" | awk '/^MAC address:/ {print $3}')
        rev=$(echo "${info}" | grep -oE 'revision v[0-9]+\.[0-9]+' | head -1)
        echo "  chip:     ${chip:-unknown} ${rev:-}"
        echo "  flash:    ${flash:-unknown}"
        echo "  mac:      ${mac:-unknown}"

        # Suggest the chip name our scripts use for this device.
        case "${chip}" in
            esp32)   suggested="esp32" ;;
            esp32s2) suggested="esp32s2" ;;
            esp32s3) suggested="esp32s3" ;;
            esp32c3) suggested="esp32c3" ;;
            esp32c6) suggested="esp32c6" ;;
            *)       suggested="" ;;
        esac
        if [[ -n "${suggested}" ]]; then
            upper="$(echo "${suggested}" | tr '[:lower:]' '[:upper:]')_PORT"
            echo "  suggest:  add '${upper}=${port}' to scripts/devices.env"
            echo "  build:    ./scripts/build.sh ${suggested}"
            echo "  flash:    ./scripts/flash.sh ${suggested}"
        fi
        found=$((found + 1))
    else
        echo "  not an ESP device (espflash board-info failed)"
    fi
done

echo
hr
echo "[boards] ${found} ESP device(s) detected"
hr
