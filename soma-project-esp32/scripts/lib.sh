#!/usr/bin/env bash
# lib.sh — shared helpers sourced by every script in this directory.
#
# Defines:
#   REPO_ROOT       absolute path to soma-project-esp32
#   FIRMWARE_DIR    absolute path to firmware/
#   TARGET_DIR      absolute path to target/
#   chip_target     map a chip name to its rust target triple
#   chip_features   map a chip name to its full cargo feature list
#   chip_port       map a chip name to its serial port (from devices.env)
#   chip_config     map a chip name to its --config overlay path
#   require_chip    validate chip arg, default to esp32s3
#   ensure_esp_env  source ~/export-esp.sh if Xtensa toolchain is needed
#   ensure_python_venv  create or reuse a pyserial venv at ~/somavenv

set -euo pipefail

# Resolve REPO_ROOT relative to this script no matter where it's invoked from.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
FIRMWARE_DIR="${REPO_ROOT}/firmware"
TARGET_DIR="${REPO_ROOT}/target"
DEVICES_ENV="${SCRIPT_DIR}/devices.env"

# Load per-machine port assignments. The user is expected to edit
# scripts/devices.env once after cloning.
if [[ -f "${DEVICES_ENV}" ]]; then
    # shellcheck disable=SC1090
    source "${DEVICES_ENV}"
fi

# Default chip if none supplied. Match the firmware crate's default feature.
DEFAULT_CHIP="esp32s3"

# All ports + the chip feature, joined as a single feature list. The chip
# feature is required because the firmware's default feature set includes
# `chip-esp32s3` and `--no-default-features` strips it.
ALL_PORTS="gpio delay uart i2c spi adc pwm storage thermistor board display"

# Ports including wifi. Opt-in per build.
ALL_PORTS_WIFI="${ALL_PORTS} wifi"

# ---------------------------------------------------------------------------
# chip_* lookup helpers — single source of truth for the chip → cargo and
# chip → device mapping. Adding a new chip means adding a case here, in
# scripts/devices.env, and in firmware/chips/<chip>.toml + chip/<chip>.rs.
# ---------------------------------------------------------------------------

chip_target() {
    case "$1" in
        esp32s3) echo "xtensa-esp32s3-none-elf" ;;
        esp32)   echo "xtensa-esp32-none-elf" ;;
        esp32s2) echo "xtensa-esp32s2-none-elf" ;;
        esp32c3) echo "riscv32imc-unknown-none-elf" ;;
        esp32c6) echo "riscv32imac-unknown-none-elf" ;;
        *) echo "unsupported chip: $1 (add a case in scripts/lib.sh::chip_target)" >&2; return 1 ;;
    esac
}

chip_features() {
    # Returns the full cargo feature list including the chip-* feature.
    # Optional $2: pass "wifi" to enable the wifi feature and the wifi port.
    local ports
    case "${2:-}" in
        wifi) ports="${ALL_PORTS_WIFI}" ;;
        "")   ports="${ALL_PORTS}" ;;
        *)    echo "unsupported variant: $2 (pass 'wifi' or nothing)" >&2; return 1 ;;
    esac
    case "$1" in
        esp32s3) echo "chip-esp32s3 ${ports}" ;;
        esp32)   echo "chip-esp32 ${ports}" ;;
        esp32s2) echo "chip-esp32s2 ${ports}" ;;
        esp32c3) echo "chip-esp32c3 ${ports}" ;;
        esp32c6) echo "chip-esp32c6 ${ports}" ;;
        *) echo "unsupported chip: $1 (add a case in scripts/lib.sh::chip_features)" >&2; return 1 ;;
    esac
}

chip_config() {
    # Path to the cargo --config overlay TOML for a chip.
    local cfg="${FIRMWARE_DIR}/chips/$1.toml"
    if [[ ! -f "${cfg}" ]]; then
        echo "missing config overlay: ${cfg}" >&2
        echo "create one (see firmware/chips/esp32s3.toml as a template) before building." >&2
        return 1
    fi
    echo "${cfg}"
}

chip_port() {
    # Resolves the serial port for a chip from scripts/devices.env. The
    # variable name is upper-cased: ESP32_PORT, ESP32S3_PORT, etc.
    local upper
    upper="$(echo "$1" | tr '[:lower:]' '[:upper:]')_PORT"
    local port="${!upper:-}"
    if [[ -z "${port}" ]]; then
        echo "no serial port configured for chip '$1'." >&2
        echo "edit scripts/devices.env and set ${upper}=/dev/cu.usbserial-XXXX" >&2
        return 1
    fi
    echo "${port}"
}

chip_arch() {
    # Returns 'xtensa' or 'riscv' — used to decide whether to source the
    # Xtensa toolchain env script.
    case "$1" in
        esp32|esp32s2|esp32s3) echo "xtensa" ;;
        esp32c3|esp32c6|esp32h2|esp32c2|esp32p4) echo "riscv" ;;
        *) echo "unknown arch for chip: $1" >&2; return 1 ;;
    esac
}

require_chip() {
    # Reads the chip arg from $1, defaults to DEFAULT_CHIP. Validates it's
    # a known chip by calling chip_target.
    local chip="${1:-${DEFAULT_CHIP}}"
    chip_target "${chip}" >/dev/null
    echo "${chip}"
}

ensure_esp_env() {
    # Sources the Xtensa toolchain env script if it exists. Required for
    # Xtensa builds; harmless for RISC-V.
    if [[ -f "${HOME}/export-esp.sh" ]]; then
        # shellcheck disable=SC1091
        source "${HOME}/export-esp.sh"
    elif [[ "$(chip_arch "${1:-esp32s3}")" == "xtensa" ]]; then
        echo "warning: ~/export-esp.sh not found." >&2
        echo "  Install the Espressif Rust toolchain with: cargo install espup && espup install" >&2
        echo "  Then run this script again." >&2
        return 1
    fi
}

ensure_python_venv() {
    # Creates ~/somavenv with pyserial if it doesn't exist. The wire protocol
    # test scripts use this venv so the user doesn't need to install pyserial
    # globally (and macOS PEP-668 blocks pip install outside venvs anyway).
    local venv="${HOME}/somavenv"
    if [[ ! -x "${venv}/bin/python3" ]]; then
        echo "[setup] creating Python venv at ${venv}"
        python3 -m venv "${venv}"
        "${venv}/bin/pip" install --quiet pyserial
    fi
    PYTHON="${venv}/bin/python3"
    export PYTHON
}

# Pretty-printed banner for script output.
hr() {
    echo "---------------------------------------------------------------------"
}
