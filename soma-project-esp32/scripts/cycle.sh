#!/usr/bin/env bash
# cycle.sh — convenience wrapper: build + flash + test for a chip.
#
# Usage:
#   ./scripts/cycle.sh                      # default: esp32s3, no wifi
#   ./scripts/cycle.sh esp32s3
#   ./scripts/cycle.sh esp32
#   ./scripts/cycle.sh esp32s3 wifi         # build + flash with wifi enabled
#   ./scripts/cycle.sh esp32 wifi
#
# Three steps in one command. Bails if any step fails. Useful when you've
# changed src/chip/<chip>.rs and want to confirm everything still works
# end-to-end on real hardware.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

CHIP="${1:-esp32s3}"
VARIANT="${2:-}"

"${SCRIPT_DIR}/build.sh" "${CHIP}" "${VARIANT}"
"${SCRIPT_DIR}/flash.sh" "${CHIP}"
# Give the chip a beat to come out of reset before we start sending frames.
sleep 1
"${SCRIPT_DIR}/test.sh" "${CHIP}" "" "${VARIANT}"
