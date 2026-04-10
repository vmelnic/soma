#!/usr/bin/env bash
# test.sh — run the wire-protocol exercise against a flashed board.
#
# Usage:
#   ./scripts/test.sh                       # default chip + default port
#   ./scripts/test.sh esp32s3
#   ./scripts/test.sh esp32
#   ./scripts/test.sh esp32s3 "" wifi       # include wifi.scan / wifi.status
#   ./scripts/test.sh esp32 /dev/cu.x wifi  # explicit port + wifi variant
#
# Runs scripts/wire-test.py through ~/somavenv (auto-created on first run).
# The chip module's TEST_LED_PIN is hardcoded per chip below — keep this
# table in sync with the constants in firmware/src/chip/<chip>.rs.

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

VARIANT="${3:-}"

# Mirror of chip::active::TEST_LED_PIN. Update when chip/<chip>.rs changes.
case "${CHIP}" in
    esp32s3) TEST_PIN=15 ;;
    esp32)   TEST_PIN=13 ;;
    esp32s2) TEST_PIN=15 ;;
    esp32c3) TEST_PIN=7 ;;
    esp32c6) TEST_PIN=8 ;;
    *) echo "no test pin defined for chip '${CHIP}' — add a case in scripts/test.sh" >&2; exit 1 ;;
esac

ensure_python_venv

hr
echo "[test] chip:     ${CHIP}"
echo "[test] variant:  ${VARIANT:-(default)}"
echo "[test] port:     ${PORT}"
echo "[test] test pin: GPIO${TEST_PIN}"
hr

if [[ "${VARIANT}" == "wifi" ]]; then
    "${PYTHON}" "${SCRIPT_DIR}/wire-test.py" "${PORT}" "${TEST_PIN}" --wifi
else
    "${PYTHON}" "${SCRIPT_DIR}/wire-test.py" "${PORT}" "${TEST_PIN}"
fi
