#!/usr/bin/env bash
# setup.sh — one-shot install of every prerequisite the firmware build needs.
#
# Idempotent: safe to run multiple times. Skips anything already installed.
#
# Installs:
#   - Stock Rust toolchain (via rustup, if missing)
#   - espup + Xtensa-enabled Rust fork (~1 GB download)
#   - espflash 4.x (the flasher and serial monitor)
#   - Python venv at ~/somavenv with pyserial (for wire-protocol tests)
#
# After running: `. ~/export-esp.sh` once per shell to put the Xtensa
# linker on PATH, then `./scripts/build.sh esp32s3` to build the firmware.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/lib.sh"

hr
echo "[setup] SOMA ESP32 firmware prerequisites"
hr

# 1. rustup
if ! command -v rustup >/dev/null 2>&1; then
    echo "[setup] installing rustup"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    # shellcheck disable=SC1091
    source "${HOME}/.cargo/env"
else
    echo "[setup] rustup already installed: $(rustup --version)"
fi

# 2. espup + Xtensa toolchain
if ! command -v espup >/dev/null 2>&1; then
    echo "[setup] installing espup"
    cargo install espup
fi

if [[ ! -f "${HOME}/export-esp.sh" ]]; then
    echo "[setup] running espup install (downloads ~1 GB Xtensa toolchain)"
    espup install
else
    echo "[setup] Xtensa toolchain already present (${HOME}/export-esp.sh exists)"
fi

# 3. espflash
if ! command -v espflash >/dev/null 2>&1; then
    echo "[setup] installing espflash"
    cargo install espflash
else
    echo "[setup] espflash already installed: $(espflash --version)"
fi

# 4. Python venv with pyserial (for wire-protocol tests)
ensure_python_venv

hr
echo "[setup] done."
hr
echo
echo "Next steps:"
echo "  1. Source the Xtensa env in your shell:   . ~/export-esp.sh"
echo "  2. Edit scripts/devices.env to point at your serial ports"
echo "  3. Build for ESP32-S3:                    ./scripts/build.sh esp32s3"
echo "  4. Build for ESP32 (LX6 / WROOM-32D):     ./scripts/build.sh esp32"
echo "  5. Flash:                                  ./scripts/flash.sh <chip>"
echo "  6. Run the wire-protocol test:             ./scripts/test.sh <chip>"
