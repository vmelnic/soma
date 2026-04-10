#!/usr/bin/env bash
# monitor.sh — open an espflash serial monitor on the chip's port.
#
# Usage:
#   ./scripts/monitor.sh                # default chip + default port
#   ./scripts/monitor.sh esp32s3
#   ./scripts/monitor.sh esp32 /dev/cu.usbserial-XXXX   # explicit port
#
# Monitor consumes UART0 — the same channel the wire protocol uses. Don't
# leave a monitor open while running ./scripts/test.sh against the same
# board, the monitor will eat the responses.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/lib.sh"

CHIP="$(require_chip "${1:-}")"

if [[ -n "${2:-}" ]]; then
    PORT="$2"
else
    PORT="$(chip_port "${CHIP}")"
fi

hr
echo "[monitor] chip:  ${CHIP}"
echo "[monitor] port:  ${PORT}"
echo "[monitor] press Ctrl+C to exit"
hr

ensure_esp_env "${CHIP}" || true

espflash monitor --port "${PORT}"
