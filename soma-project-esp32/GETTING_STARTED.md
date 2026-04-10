# SOMA ESP32 — Getting Started

End-to-end guide: prerequisites → build → flash → wire-protocol exchange against real hardware. **Same source tree, two chip families today, more chips via a four-step recipe.**

This document is the practical companion to [README.md](README.md) — the README explains the architecture, this file walks through making it work on a physical board. **Every command and capture in this guide was executed against real hardware.**

---

## What's proven

| Path | Status | Hardware | Notes |
|---|---|---|---|
| **ESP32-S3** (Xtensa LX7) over UART0 | **PROVEN** | Sunton ESP32-S3-1732S019 (16 MB flash) | 14/14 wire-protocol tests, all primitives + routine round-trips |
| **ESP32** (Xtensa LX6, WROOM-32D) over UART0 | **PROVEN** | ESP-WROOM-32D 18650/OLED dev board (4 MB flash) | same source tree, `chip-esp32` feature, `chips/esp32.toml` overlay, 14/14 wire-protocol tests |
| **ESP32-S3 with `--features wifi`** (radio + smoltcp + TCP listener) | **PROVEN** | Sunton 1732S019 | 16/16 tests, real `wifi.scan` returns 7-8 live APs, `wifi.status` works, heap usage ~46 KB after init |
| **ESP32 (LX6) with `--features wifi`** (radio + smoltcp + TCP listener) | **PROVEN** | WROOM-32D | 16/16 tests, `wifi.scan` returns live APs when run before heavy storage writes (test script orders it first); `wifi.status` always works |
| ESP32-C3 / ESP32-C6 (RISC-V) | not proven on this branch | n/a | drop a new `chip/esp32c3.rs` module + cargo feature; instructions in §11 |
| ESP32-S2 (Xtensa LX7, no wifi5) | not proven on this branch | n/a | drop a new `chip/esp32s2.rs` module + cargo feature; instructions in §11 |

The two-chip layout is **single source tree, single firmware crate**, picking the chip with cargo features + a per-chip cargo config overlay. Adding chips means dropping one new file per chip and one new cargo feature — main.rs never changes.

### Known ESP32 LX6 wifi quirks

- **Heap size matters.** esp-wifi 0.12 + smoltcp + TCP rx/tx buffers + our Vecs need at least ~50 KB. This firmware allocates 96 KB (`esp_alloc::heap_allocator!(96 * 1024)`) — plenty of headroom. With 64 KB the dispatch loop died with a garbage-looking "memory allocation of N bytes failed" panic within milliseconds of entering run_dual_transport. Don't reduce the heap below 80 KB for wifi builds.
- **Run `wifi.scan` early on ESP32 LX6.** After heavy SPI flash writes (multiple `storage.set` calls in the same boot cycle), ESP32 LX6 esp-wifi 0.12 sometimes crashes on the next `wifi.scan` with an illegal-instruction exception from xtensa-lx-rt. The fix is to run wifi operations before storage-heavy sequences in the same boot cycle. `scripts/wire-test.py --wifi` orders the wifi tests first for this reason. ESP32-S3 is unaffected.

---

## 0. Quick start with the helper scripts

Every step in this guide is wrapped as a shell script under `scripts/`. If you trust the defaults, this is the entire flow from a fresh clone to a running, wire-protocol-tested board:

```bash
cd soma-project-esp32

# 0a. One-shot install: rustup, espup + Xtensa toolchain, espflash, Python venv
./scripts/setup.sh

# 0b. Source the Xtensa env (once per shell)
. ~/export-esp.sh

# 0c. Detect connected ESP boards and tell you which `chip` arg to use
./scripts/boards.sh

# 0d. Edit scripts/devices.env so each chip's *_PORT matches your /dev/cu.usbserial-*
$EDITOR scripts/devices.env

# 0e. Build, flash, and run the wire-protocol exercise — all at once
./scripts/cycle.sh esp32s3        # for the ESP32-S3
./scripts/cycle.sh esp32          # for the ESP32 (LX6 / WROOM-32D)

# With wifi (requires esp-wifi, smoltcp, TCP listener on port 9100):
./scripts/cycle.sh esp32s3 wifi   # 16 tests including wifi.scan
./scripts/cycle.sh esp32 wifi
```

Every step in `cycle.sh` can also be run individually:

| Script | What it does |
|---|---|
| `./scripts/setup.sh` | One-shot install of toolchain + espflash + Python venv |
| `./scripts/boards.sh` | Probe every USB serial port with `espflash board-info`, suggest the chip name |
| `./scripts/build.sh <chip> [wifi]` | Build firmware for `<chip>` with all default ports (add `wifi` for radio support) |
| `./scripts/flash.sh <chip> [port]` | Flash the built binary to the chip's port |
| `./scripts/monitor.sh <chip> [port]` | Open `espflash monitor` on the chip's port |
| `./scripts/test.sh <chip> [port] [wifi]` | Run the wire-protocol exercise via Python (14 tests, +2 for wifi) |
| `./scripts/cycle.sh <chip> [wifi]` | `build.sh` + `flash.sh` + `test.sh` in one shot |

The chip → port mapping is in `scripts/devices.env`, sourced by every script. The chip → cargo target / features / config-overlay mapping is in `scripts/lib.sh`. To support a new chip in the scripts: add a case in `lib.sh::chip_target`, `lib.sh::chip_features`, `test.sh`'s test-pin table, and a new `*_PORT` line in `devices.env`.

The rest of this document explains what each step does in detail — useful when something fails, or when you want to do anything off the happy path.

---

## 1. Prerequisites

### 1.1 Software

The ESP32-S3 (and the original ESP32) is a **Tensilica Xtensa** chip. Xtensa is not in upstream LLVM, so stock `rustc` cannot target it. You need the Espressif Rust fork installed via `espup`.

```bash
# Stock Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# espup installs the Xtensa-enabled Rust fork as the `+esp` toolchain
cargo install espup
espup install        # downloads ~1 GB: rustc/llvm/clang for Xtensa, gcc, etc.

# Source the env vars in every shell that builds firmware
. ~/export-esp.sh
# (or add this line to ~/.zshrc / ~/.bashrc)

# espflash — flashes the firmware over USB and provides a serial monitor
cargo install espflash
```

The repo's `rust-toolchain.toml` pins the channel to `esp`, so `cargo build` inside the workspace automatically picks the Xtensa toolchain. You only need to run `. ~/export-esp.sh` once per shell session — it puts `xtensa-esp-elf-gcc` (the linker) on `PATH`.

Verify:

```bash
rustup toolchain list | grep esp        # should show: esp
rustc +esp --print target-list | grep xtensa-esp32
# xtensa-esp32-none-elf
# xtensa-esp32s2-none-elf
# xtensa-esp32s3-none-elf
# (...)
espflash --version                       # 4.3+
```

### 1.2 Hardware — what's been proven

**ESP32-S3** path tested against:
- **Sunton ESP32-S3-1732S019** — ESP32-S3-WROOM-1, 16 MB flash, 8 MB PSRAM, on-board CH340 USB-to-UART bridge, 1.9″ ST7789 LCD on SPI2 (firmware leaves the LCD in reset)
- pin claims: GPIO15 (gpio test), GPIO8/9 (I²C), GPIO35/36 (SPI3), GPIO2 (ADC1_CH1), GPIO16 (PWM), GPIO17/18 (UART1)

**ESP32** path tested against:
- **ESP-WROOM-32D** "18650/OLED" dev board — 4 MB flash, dual-core 240 MHz, on-board CP2102 USB-to-UART bridge
- pin claims: GPIO13 (gpio test), GPIO21/22 (I²C, default ESP32 pins), GPIO18/23 (SPI3 / VSPI), GPIO34 (ADC1_CH6, input-only), GPIO25 (PWM), GPIO16/17 (UART1)
- **NEVER touched**: GPIO 6-11 (wired to internal QSPI flash on the WROOM-32 module — touching them bricks boot) and the strapping pins 0/2/5/12/15

Any other ESP32-S3 or ESP32 dev board should work with no code changes as long as the USB connector goes to a USB-to-UART bridge wired to UART0 and the pins listed above are free. If your board's pinout differs, edit the per-chip module under `firmware/src/chip/<chip>.rs` — that's the only place pin numbers live.

### 1.3 USB cable

A **data-capable** USB-A-to-USB-C (or USB-A-to-microUSB for older WROOM-32 boards) cable. Many cables that ship in retail boxes are power-only and the chip will not enumerate. If `ls /dev/cu.usb*` shows nothing after plugging the board in, swap cables before debugging anything else.

---

## 2. Workspace overview

```
soma-project-esp32/
├── Cargo.toml                workspace root, 14 members
│                               + per-package profile override for esp-storage
│                               (ESP32 LX6 needs opt-level >= 2 for flash timing)
├── rust-toolchain.toml       pins the +esp Xtensa toolchain
├── leaf/                     no_std wire protocol library (host-testable)
├── ports/                    chip-agnostic hardware port crates — esp-hal dep
│   │                          has NO chip feature; the firmware crate selects it
│   ├── gpio/                 gpio.write / read / toggle
│   ├── delay/                delay.ms / delay.us
│   ├── uart/                 uart.write / read  (UART1, downstream peripheral)
│   ├── i2c/                  i2c.write / read / write_read / scan
│   ├── spi/                  spi.write / read / transfer
│   ├── adc/                  adc.read / read_voltage
│   ├── pwm/                  pwm.set_duty / get_status
│   ├── wifi/                 wifi.scan / configure / status / disconnect / forget  (gated)
│   ├── storage/              storage.get / set / delete / list / clear  (SPI flash NVS)
│   ├── thermistor/           example sensor port (simulated, replace with real)
│   ├── board/                chip_info / pin_map / configure_pin / probe_i2c_buses / reboot
│   │                           (chip-agnostic — firmware injects closures at port construction)
│   └── display/              info / clear / draw_text / draw_text_xy / fill_rect /
│                               set_contrast / flush (chip-agnostic — firmware owns ssd1306,
│                               shares I2C0 with the i2c port via embedded-hal-bus)
└── firmware/                 the flashable binary
    ├── Cargo.toml            chip-esp32 / chip-esp32s3 cargo features select
    │                           the esp-* dep features (mutually exclusive)
    ├── .cargo/config.toml    default target = xtensa-esp32s3-none-elf
    │                           + per-target espflash runner config
    ├── chips/                cargo --config overlays, one per chip
    │   ├── esp32s3.toml      target + runner port for ESP32-S3
    │   └── esp32.toml        target + runner port for ESP32 (LX6)
    ├── build.rs              generates a custom rwtext.x in OUT_DIR that
    │                           injects .flash.appdesc at the start of drom_seg
    │                           (chip-agnostic — works for both ESP32 and S3)
    └── src/
        ├── main.rs           CHIP-AGNOSTIC: heap init, app descriptor,
        │                       FlashKvStore, dispatch loop, self-test
        │                       (calls chip::active::* for everything chip-specific)
        └── chip/
            ├── mod.rs        cfg-gated `pub use ... as active`,
            │                   compile_error! for missing/double chip features
            ├── esp32s3.rs    S3 pin map + register_all_ports()
            └── esp32.rs      ESP32 (LX6) pin map + register_all_ports()
```

The build produces, in `firmware/target/<target>/release/build/soma-esp32-firmware-*/out/rwtext.x`, an overridden copy of esp-hal's linker fragment that injects the ESP-IDF application descriptor at the very start of the DROM segment. Without this the chip's stage-2 bootloader rejects the image with `Image requires efuse blk rev >= v237.62` (it reads garbage from where it expects the descriptor). Same override works for both chips since the linker section names are shared. Details in [§12 Troubleshooting](#12-troubleshooting).

**Adding ports does not require touching `chip/`.** The chip module only owns peripheral wiring (pin numbers, peripheral instances) — port crates and main.rs are chip-agnostic.

### 2.1 Design rules — load-bearing, do not break

These constraints are what make the dual-chip support work. Any contributor adding a port, a chip, or a workspace setting needs to honor them.

| Rule | Where it lives | Why |
|---|---|---|
| **Port crates' esp-hal dep has NO chip feature.** Use `esp-hal = { version = "0.23" }`, never `features = ["esp32s3"]`. | `ports/*/Cargo.toml` | esp-hal's chip features are mutually exclusive at the dep level. If a port crate hardcodes one chip, building for any other chip fails with `expected exactly one enabled feature: ["esp32", "esp32c3", ..., "esp32s3"]`. The chip is selected exclusively by the firmware crate. |
| **Workspace `[profile.release.package.esp-storage]` overrides `opt-level = 3`.** | `Cargo.toml` (workspace root) | esp-storage's flash-write loops are timing-sensitive on the original ESP32 (LX6). Its build script refuses `opt-level = "s"` and aborts with `Building esp-storage for ESP32 needs optimization level 2 or 3`. The override applies only to `esp-storage`; the rest of the workspace stays at `opt-level = "s"` for size. |
| **Default pin numbers and peripheral instances live in `firmware/src/chip/<chip>.rs`.** main.rs and port crates never reference a specific GPIO. Actual pin assignments at boot come from `FlashKvStore` with the `DEFAULT_*` constants in the chip module as fallbacks. | `firmware/src/chip/` | Adding a chip is changing one file. Default pinout for a new board is changing one file. Reconfiguring pins on a deployed leaf is an MCP call to `board.configure_pin` — no reflash needed. |
| **Exactly one `chip-*` feature must be enabled at a time.** | `firmware/src/chip/mod.rs` | esp-hal asserts mutual exclusion in its build script. The chip module enforces it earlier with `compile_error!` so the failure is local and readable. |
| **The ESP-IDF app descriptor must be the FIRST 256 bytes of the first loadable flash data segment.** | `firmware/build.rs` (generates `rwtext.x`) + `firmware/src/main.rs` (the `esp_app_desc` static) | Without the override, the bootloader reads garbage from where it expects the descriptor and refuses to boot with `Image requires efuse blk rev >= v237.62`. Same override works for both ESP32 and ESP32-S3 since esp-hal's section names are shared. |

---

## 3. Build

The default build targets **ESP32-S3**. ESP32 (LX6) builds use the per-chip cargo config overlay in `firmware/chips/esp32.toml`. Both produce a Tensilica Xtensa ELF; the difference is the target triple, the esp-* feature flags, and the GPIO map (which lives in `firmware/src/chip/<chip>.rs`).

### 3.1 ESP32-S3 (default)

```bash
cd soma-project-esp32
. ~/export-esp.sh

# 9 ports, no wifi, UART0 transport. The proven path.
cargo +esp build --release -p soma-esp32-firmware
```

Output binary: `target/xtensa-esp32s3-none-elf/release/soma-esp32-firmware`

Sample build (~7s after first compile, sub-3s incremental):

```
Compiling soma-esp32-leaf v0.1.0
Compiling soma-esp32-port-gpio v0.1.0
... (9 port crates) ...
Compiling soma-esp32-firmware v0.1.0
Finished `release` profile [optimized + debuginfo] target(s) in 7.18s
```

### 3.2 ESP32 / WROOM-32D

```bash
cd soma-project-esp32
. ~/export-esp.sh

cargo +esp build --release \
    --config $(pwd)/firmware/chips/esp32.toml \
    --no-default-features \
    --features "chip-esp32 gpio delay uart i2c spi adc pwm storage thermistor" \
    -p soma-esp32-firmware
```

Output binary: `target/xtensa-esp32-none-elf/release/soma-esp32-firmware`

Why `--no-default-features`: the default feature set includes `chip-esp32s3`, and the two `chip-*` features are mutually exclusive at the dep level (esp-hal will fail to compile if both are enabled). `--no-default-features` strips them; the explicit feature list re-enables only the chip + ports you want.

Why `--config $(pwd)/firmware/chips/esp32.toml`: the overlay sets `[build] target = "xtensa-esp32-none-elf"` and the espflash runner port. The default `firmware/.cargo/config.toml` is for S3, this overlay temporarily overrides it for ESP32 builds. Cargo's `--config` flag requires a path that's parseable as TOML — pass an absolute path (or `./firmware/chips/esp32.toml`) to disambiguate it from a `key=value` expression.

**Why the workspace `Cargo.toml` overrides `[profile.release.package.esp-storage] opt-level = 3`:** the original ESP32 (LX6) has timing-sensitive flash-write loops in `esp-storage`. Its build script aborts with `Building esp-storage for ESP32 needs optimization level 2 or 3 - yours is s` if it sees `opt-level = "s"` (the workspace default for size). The override applies *only* to the `esp-storage` package — every other crate stays at `opt-level = "s"`. ESP32-S3 doesn't need this but the override is harmless there. **Do not remove this profile override** or ESP32 builds will break.

### 3.3 Inspect either binary

```bash
file target/xtensa-esp32s3-none-elf/release/soma-esp32-firmware
# ELF 32-bit LSB executable, Tensilica Xtensa, version 1 (SYSV),
# statically linked, with debug_info, not stripped

ls -lh target/xtensa-esp32s3-none-elf/release/soma-esp32-firmware
# 8.4M (with debug info — actual flashed image is ~250 KB)

ls -lh target/xtensa-esp32-none-elf/release/soma-esp32-firmware
# 4.5M (with debug info — actual flashed image is ~190 KB)

# Confirm the ESP-IDF app descriptor landed at the start of drom_seg
xtensa-esp-elf-objdump -t target/xtensa-esp32s3-none-elf/release/soma-esp32-firmware | grep esp_app_desc
# 3c000020 g     O .flash.appdesc  00000100 esp_app_desc

xtensa-esp-elf-objdump -t target/xtensa-esp32-none-elf/release/soma-esp32-firmware | grep esp_app_desc
# 3f400020 g     O .flash.appdesc  00000100 esp_app_desc
```

The address differs because each chip's drom_seg origin differs (S3 = `0x3C000020`, ESP32 = `0x3F400020`).

### 3.4 Build variants — port subset

Cargo features pick which ports compile in. Same syntax for both chips, just keep the chip feature included.

```bash
# S3, minimal: gpio + delay only (~150 KB image)
cargo +esp build --release -p soma-esp32-firmware \
    --no-default-features --features "chip-esp32s3 gpio delay"

# ESP32, sensors only (drop actuators)
cargo +esp build --release \
    --config $(pwd)/firmware/chips/esp32.toml \
    --no-default-features \
    --features "chip-esp32 delay i2c spi adc thermistor storage" \
    -p soma-esp32-firmware

# With wifi (NOT yet proven — will compile but the dispatch loop panics on
# 0-byte allocs from esp-wifi background tasks; see §12)
cargo +esp build --release -p soma-esp32-firmware --features wifi
```

Default features (S3 path):

```
chip-esp32s3 + gpio + delay + uart + i2c + spi + adc + pwm + storage + thermistor
```

`wifi` is intentionally off in defaults until the esp-alloc 0.6 zero-byte issue is worked around.

---

## 4. Flash

The serial port for each chip lives in **two places** that you must keep in sync (or just edit the script-side one and use the helper scripts everywhere):

1. **`scripts/devices.env`** — used by `./scripts/flash.sh`, `./scripts/test.sh`, `./scripts/monitor.sh`, `./scripts/cycle.sh`. The single source of truth for the helper scripts.
2. **`firmware/chips/<chip>.toml`** — used by `cargo run` (the espflash runner Cargo invokes when you `cargo run --release`). The cargo-side path.

### 4.1 Identify your boards

```bash
# Both boards can be plugged in at once
./scripts/boards.sh
# [boards] connected ESP devices
# Port: /dev/cu.usbserial-0001
#   chip:     esp32 revision v3.0
#   flash:    4MB
#   suggest:  add 'ESP32_PORT=/dev/cu.usbserial-0001' to scripts/devices.env
# Port: /dev/cu.usbserial-1120
#   chip:     esp32s3 revision v0.2
#   flash:    16MB
#   suggest:  add 'ESP32S3_PORT=/dev/cu.usbserial-1120' to scripts/devices.env
```

Or manually:

```bash
ls /dev/cu.* | grep -v -i 'bluetooth\|debug-console'
espflash board-info --port /dev/cu.usbserial-XXXX
```

### 4.2 Update the script-side port

Edit `scripts/devices.env` and set the right values for your machine:

```diff
- ESP32S3_PORT=/dev/cu.usbserial-1120
- ESP32_PORT=/dev/cu.usbserial-0001
+ ESP32S3_PORT=/dev/cu.usbserial-XXXX
+ ESP32_PORT=/dev/cu.usbserial-YYYY
```

### 4.3 Update the cargo-runner-side port (only if you use `cargo run`)

If you only use the helper scripts, you can skip this. If you use `cargo run --release` directly (or `cargo run` from inside an IDE), update `firmware/chips/<chip>.toml`:

```diff
  [target.xtensa-esp32s3-none-elf]
- runner = "espflash flash --monitor --port /dev/cu.usbserial-1120"
+ runner = "espflash flash --monitor --port /dev/cu.usbserial-XXXX"
```

```diff
  [target.xtensa-esp32-none-elf]
- runner = "espflash flash --monitor --port /dev/cu.usbserial-0001"
+ runner = "espflash flash --monitor --port /dev/cu.usbserial-YYYY"
```

### 4.4 Flash the S3 (script)

```bash
./scripts/flash.sh esp32s3
```

Or directly:

```bash
. ~/export-esp.sh
espflash flash --port /dev/cu.usbserial-XXXX \
    target/xtensa-esp32s3-none-elf/release/soma-esp32-firmware
# App/part. size: 254,640 / 16,384,000 bytes, 1.55%
```

### 4.5 Flash the ESP32 / WROOM-32D (script)

```bash
./scripts/flash.sh esp32
```

Or directly:

```bash
. ~/export-esp.sh
espflash flash --port /dev/cu.usbserial-YYYY \
    target/xtensa-esp32-none-elf/release/soma-esp32-firmware
# App/part. size: 188,672 / 4,128,768 bytes, 4.57%
```

Add `-M` to either direct invocation if you want espflash to attach a serial monitor after flashing — or use `./scripts/monitor.sh <chip>` afterward.

### 4.1 Expected boot output

After reset, UART0 at 115200 8N1 emits this banner before the dispatch loop takes over:

```
ESP-ROM:esp32s3-20210327
... (ROM bootloader) ...
I (xx) boot: ESP-IDF v5.5.1 2nd stage bootloader
I (xx) boot: Loaded app from partition at offset 0x10000

=========================================
SOMA ESP32-S3 leaf firmware booted
Free heap: 65536 bytes
=========================================
[port] registered: gpio (3 primitives, GPIO15 claimed)
[port] registered: delay (2 primitives)
[port] registered: uart (UART1 on GPIO17/GPIO18)
[port] registered: i2c (I2C0 on GPIO8/GPIO9)
[port] registered: spi (SPI3 on GPIO35/GPIO36)
[port] registered: adc (ADC1 channel 1 on GPIO2)
[port] registered: pwm (LEDC channel 0 on GPIO16, 1kHz)
[port] registered: storage (FlashKvStore on SPI flash sector 0x9f000)
[port] registered: thermistor (simulated)
[port] composite has 9 ports, 25 primitives total
```

The ESP32 / WROOM-32D banner is identical except the chip name and the GPIO pins (which come from `firmware/src/chip/esp32.rs`):

```
SOMA ESP32 leaf firmware booted
[port] registered: gpio (3 primitives, GPIO13 claimed)
[port] registered: delay (2 primitives)
[port] registered: uart (UART1 on GPIO16/GPIO17)
[port] registered: i2c (I2C0 on GPIO21/GPIO22)
[port] registered: spi (SPI3 on GPIO18/GPIO23)
[port] registered: adc (ADC1 channel 6 on GPIO34)
[port] registered: pwm (LEDC channel 0 on GPIO25, 1kHz)
[port] registered: storage (FlashKvStore on SPI flash sector 0x9f000)
[port] registered: thermistor (simulated)
[port] composite has 9 ports, 25 primitives total

=========================================
Body self-model
  primitives: 25
  routines:   0
=========================================
  [SM] gpio.write
  [RO] gpio.read
  [SM] gpio.toggle
  [RO] delay.ms
  [RO] delay.us
  [EX] uart.write
  [RO] uart.read
  ... (25 lines total) ...

=========================================
Self-test: brain composes a cross-port routine
=========================================
[1] Brain transfers routine 'demo_pulse' with 4 steps
    leaf acknowledged: stored 'demo_pulse' (4 steps)
[2] Brain invokes 'demo_pulse'

=========================================
Leaf transports active
  UART0 (host wire frames): yes (115200 8N1)
  TCP over WiFi:           disabled (built without --features wifi)
Body alive, awaiting brain messages
=========================================
```

The boot output is plain ASCII printed via `esp-println`. After the last `===` line the firmware enters its dispatch loop and stops emitting `println!` so the host's frame parser can read raw wire frames without log noise.

---

## 5. Wire protocol reference

The leaf wire protocol is **4-byte big-endian length prefix + JSON body**. Max frame size: 16 MB. The encoding is identical for UART0 and (when enabled) TCP/9100 over WiFi.

### 5.1 Frame format

```
+----+----+----+----+--------------------------------+
|       length (u32 BE)       |       JSON body     |
+----+----+----+----+--------------------------------+
0    1    2    3    4                              4+length
```

### 5.2 Request messages (host → leaf)

All requests are JSON objects with a `"type"` discriminator. The enum is `serde(rename_all = "snake_case")` so variants are `ping`, `list_capabilities`, `invoke_skill`, `transfer_routine`, `remove_routine`.

**Ping** — liveness check.
```json
{ "type": "ping", "nonce": 42 }
```

**ListCapabilities** — ask the body to enumerate every primitive and stored routine.
```json
{ "type": "list_capabilities" }
```

**InvokeSkill** — call a primitive or a previously transferred routine.
```json
{
  "type": "invoke_skill",
  "peer_id": "host",
  "skill_id": "gpio.toggle",
  "input": { "pin": 15 }
}
```

**TransferRoutine** — push a multi-step routine to the body for storage.
```json
{
  "type": "transfer_routine",
  "routine": {
    "routine_id": "blink_x3",
    "description": "Toggle GPIO15 three times with 100ms delays",
    "steps": [
      { "skill_id": "gpio.toggle", "input": { "pin": 15 } },
      { "skill_id": "delay.ms",    "input": { "ms": 100 } },
      { "skill_id": "gpio.toggle", "input": { "pin": 15 } },
      { "skill_id": "delay.ms",    "input": { "ms": 100 } },
      { "skill_id": "gpio.toggle", "input": { "pin": 15 } },
      { "skill_id": "delay.ms",    "input": { "ms": 100 } },
      { "skill_id": "gpio.toggle", "input": { "pin": 15 } }
    ]
  }
}
```

**RemoveRoutine** — drop a stored routine.
```json
{ "type": "remove_routine", "routine_id": "blink_x3" }
```

### 5.3 Response messages (leaf → host)

**Pong** — for `Ping`.
```json
{ "type": "pong", "nonce": 42, "load": 0.0 }
```

**Capabilities** — for `ListCapabilities`.
```json
{
  "type": "capabilities",
  "primitives": [
    {
      "skill_id": "gpio.write",
      "description": "Set a claimed GPIO pin high or low",
      "input_schema": "{\"pin\":\"u32\",\"value\":\"bool\"}",
      "output_schema": "{\"pin\":\"u32\",\"value\":\"bool\"}",
      "effect": "state_mutation"
    },
    /* 24 more */
  ],
  "routines": [
    {
      "routine_id": "demo_pulse",
      "description": "GPIO toggle, delay, gpio toggle — demonstrates routine walking",
      "step_count": 4
    }
  ]
}
```

**SkillResult** — for `InvokeSkill` (primitives and routines).
```json
{
  "type": "skill_result",
  "response": {
    "skill_id": "gpio.toggle",
    "success": true,
    "structured_result": { "pin": 15, "value": true },
    "failure_message": null,
    "latency_ms": 0,
    "steps_executed": 1
  }
}
```

For routines, `structured_result` is an array of per-step records:
```json
{
  "type": "skill_result",
  "response": {
    "skill_id": "blink_x3",
    "success": true,
    "structured_result": [
      { "ok": true, "result": {"pin": 15, "value": false}, "skill_id": "gpio.toggle", "step": 0 },
      { "ok": true, "result": {"slept_ms": 100},           "skill_id": "delay.ms",    "step": 1 },
      /* ... */
    ],
    "failure_message": null,
    "latency_ms": 0,
    "steps_executed": 7
  }
}
```

**RoutineStored** — for `TransferRoutine`.
```json
{ "type": "routine_stored", "routine_id": "blink_x3", "step_count": 7 }
```

**RoutineRemoved** — for `RemoveRoutine`.
```json
{ "type": "routine_removed", "routine_id": "blink_x3" }
```

**Error** — when the request cannot be processed.
```json
{ "type": "error", "details": "json decode failed inside frame" }
```

### 5.4 Effect classes

Every primitive declares one of three effect classes — the brain uses these to decide what's safe to do without user confirmation:

| Effect | Code | Meaning |
|---|---|---|
| `read_only` | `RO` | No state change. Safe to call freely. (`gpio.read`, `delay.ms`, `i2c.scan`, `storage.get`) |
| `state_mutation` | `SM` | Mutates body or device state. (`gpio.write`, `pwm.set_duty`, `storage.set`) |
| `external_effect` | `EX` | Visible outside the body — sends bytes, network packets, etc. (`uart.write`) |

---

## 6. Python client — full working example

This is the exact script used to verify the firmware. It exercises every message type and every effect class against real hardware.

```bash
# pyserial is the only dep
python3 -m venv ~/somavenv
~/somavenv/bin/pip install pyserial
```

Save as `client.py`:

```python
import serial, struct, json, time

PORT = '/dev/cu.usbserial-XXXX'   # update for your board
BAUD = 115200

def encode_frame(msg):
    body = json.dumps(msg).encode('utf-8')
    return struct.pack('>I', len(body)) + body

def read_frame(ser, timeout=3.0):
    """Scan stream for a 4-byte length prefix followed by valid JSON.

    The leaf firmware shares UART0 with esp-println, so the host parser
    must skip log lines until it finds bytes that decode as a valid frame.
    """
    buf = bytearray()
    end = time.time() + timeout
    while time.time() < end:
        chunk = ser.read(4096)
        if chunk:
            buf.extend(chunk)
        i = 0
        while i + 4 <= len(buf):
            length = struct.unpack('>I', bytes(buf[i:i+4]))[0]
            if 0 < length < 16384 and i + 4 + length <= len(buf):
                body = bytes(buf[i+4:i+4+length])
                try:
                    return json.loads(body)
                except json.JSONDecodeError:
                    pass
            i += 1
        time.sleep(0.01)
    return None

def call(s, label, msg):
    s.write(encode_frame(msg))
    s.flush()
    r = read_frame(s)
    print(f"{label}")
    print(f"  -> {json.dumps(r) if r else 'TIMEOUT'}")
    print()
    return r

s = serial.Serial(PORT, BAUD, timeout=0.05)
time.sleep(0.3)
s.read(8192)  # drain whatever's in the boot output buffer

# 1. Liveness
call(s, "Ping", {"type": "ping", "nonce": 42})

# 2. Self-model
r = call(s, "ListCapabilities", {"type": "list_capabilities"})
print(f"  primitives: {len(r['primitives'])}, routines: {len(r['routines'])}")

# 3. GPIO round-trip
call(s, "gpio.write pin=15 value=true",
     {"type": "invoke_skill", "peer_id": "host",
      "skill_id": "gpio.write", "input": {"pin": 15, "value": True}})
call(s, "gpio.read pin=15",
     {"type": "invoke_skill", "peer_id": "host",
      "skill_id": "gpio.read", "input": {"pin": 15}})
call(s, "gpio.toggle pin=15",
     {"type": "invoke_skill", "peer_id": "host",
      "skill_id": "gpio.toggle", "input": {"pin": 15}})

# 4. Delay
call(s, "delay.ms 50",
     {"type": "invoke_skill", "peer_id": "host",
      "skill_id": "delay.ms", "input": {"ms": 50}})

# 5. Sensor read
call(s, "thermistor.read_temp",
     {"type": "invoke_skill", "peer_id": "host",
      "skill_id": "thermistor.read_temp", "input": {"channel": 0}})

# 6. Persistent storage round-trip — survives reboots
call(s, "storage.set",
     {"type": "invoke_skill", "peer_id": "host",
      "skill_id": "storage.set",
      "input": {"key": "test_key", "value": "hello from host"}})
call(s, "storage.get",
     {"type": "invoke_skill", "peer_id": "host",
      "skill_id": "storage.get", "input": {"key": "test_key"}})
call(s, "storage.list",
     {"type": "invoke_skill", "peer_id": "host",
      "skill_id": "storage.list", "input": {}})

# 7. Multi-step routine: transfer + invoke
call(s, "TransferRoutine blink_x3", {
    "type": "transfer_routine",
    "routine": {
        "routine_id": "blink_x3",
        "description": "Toggle GPIO15 three times with 100ms delays",
        "steps": [
            {"skill_id": "gpio.toggle", "input": {"pin": 15}},
            {"skill_id": "delay.ms",    "input": {"ms": 100}},
            {"skill_id": "gpio.toggle", "input": {"pin": 15}},
            {"skill_id": "delay.ms",    "input": {"ms": 100}},
            {"skill_id": "gpio.toggle", "input": {"pin": 15}},
            {"skill_id": "delay.ms",    "input": {"ms": 100}},
            {"skill_id": "gpio.toggle", "input": {"pin": 15}}
        ]
    }
})
call(s, "invoke routine blink_x3",
     {"type": "invoke_skill", "peer_id": "host",
      "skill_id": "blink_x3", "input": {}})

# 8. Cleanup
call(s, "RemoveRoutine blink_x3",
     {"type": "remove_routine", "routine_id": "blink_x3"})

s.close()
```

### 6.1 Captured output (real hardware)

This is the actual output from the test board. **Every line below was emitted by a Sunton ESP32-S3-1732S019 connected via CH340 USB-to-UART.**

```
Ping
  -> {"type": "pong", "nonce": 42, "load": 0.0}

ListCapabilities
  -> {"type": "capabilities", "primitives": [...], "routines": [...]}
  primitives: 25, routines: 1

gpio.write pin=15 value=true
  -> {"type": "skill_result", "response": {"skill_id": "gpio.write",
      "success": true, "structured_result": {"pin": 15, "value": true},
      "failure_message": null, "latency_ms": 0, "steps_executed": 1}}

gpio.read pin=15
  -> {"type": "skill_result", "response": {"skill_id": "gpio.read",
      "success": true, "structured_result": {"pin": 15, "value": true},
      "failure_message": null, "latency_ms": 0, "steps_executed": 1}}

gpio.toggle pin=15
  -> {"type": "skill_result", "response": {"skill_id": "gpio.toggle",
      "success": true, "structured_result": {"pin": 15, "value": false},
      "failure_message": null, "latency_ms": 0, "steps_executed": 1}}

delay.ms 50
  -> {"type": "skill_result", "response": {"skill_id": "delay.ms",
      "success": true, "structured_result": {"slept_ms": 50},
      "failure_message": null, "latency_ms": 0, "steps_executed": 1}}

thermistor.read_temp
  -> {"type": "skill_result", "response": {"skill_id": "thermistor.read_temp",
      "success": true, "structured_result": {"channel": 0, "temp_c": 20.5},
      "failure_message": null, "latency_ms": 0, "steps_executed": 1}}

storage.set
  -> {"type": "skill_result", "response": {"skill_id": "storage.set",
      "success": true, "structured_result": {"key": "test_key", "stored": true},
      "failure_message": null, "latency_ms": 0, "steps_executed": 1}}

storage.get
  -> {"type": "skill_result", "response": {"skill_id": "storage.get",
      "success": true, "structured_result": {"found": true, "key": "test_key",
      "value": "hello from host"}, "failure_message": null, "latency_ms": 0,
      "steps_executed": 1}}

TransferRoutine blink_x3
  -> {"type": "routine_stored", "routine_id": "blink_x3", "step_count": 7}

invoke routine blink_x3
  -> {"type": "skill_result", "response": {"skill_id": "blink_x3",
      "success": true, "structured_result": [
        {"ok": true, "result": {"pin": 15, "value": false}, "skill_id": "gpio.toggle", "step": 0},
        {"ok": true, "result": {"slept_ms": 100},           "skill_id": "delay.ms",    "step": 1},
        {"ok": true, "result": {"pin": 15, "value": true},  "skill_id": "gpio.toggle", "step": 2},
        {"ok": true, "result": {"slept_ms": 100},           "skill_id": "delay.ms",    "step": 3},
        {"ok": true, "result": {"pin": 15, "value": false}, "skill_id": "gpio.toggle", "step": 4},
        {"ok": true, "result": {"slept_ms": 100},           "skill_id": "delay.ms",    "step": 5},
        {"ok": true, "result": {"pin": 15, "value": true},  "skill_id": "gpio.toggle", "step": 6}
      ], "failure_message": null, "latency_ms": 0, "steps_executed": 7}}

RemoveRoutine blink_x3
  -> {"type": "routine_removed", "routine_id": "blink_x3"}
```

The body validates input schemas. Sending `gpio.write` with an integer instead of a bool returns `{"success": false, "failure_message": "missing 'value'"}` rather than crashing — the brain learns from the failure.

---

## 7. Adding a new sensor port

The `thermistor` port is a worked example. To add a new sensor (say, BME280 pressure + humidity over I²C), follow these four steps. **No changes to leaf, main.rs, or other ports.** The new port plugs into the existing chip modules via the `register_all_ports` block.

> **Critical design rule:** port crates' `esp-hal` dep MUST NOT specify a chip feature. Use `esp-hal = { version = "0.23" }`, never `features = ["esp32s3"]`. The chip is selected exclusively by the firmware crate. Hardcoding a chip feature in a port crate breaks the dual-chip build with `expected exactly one enabled feature: ["esp32", ..., "esp32s3"]`. See §2.1 "Design rules — load-bearing".

### 7.1 Create the port crate

```bash
cd ports
cargo new --lib bme280
```

`ports/bme280/Cargo.toml`:
```toml
[package]
name = "soma-esp32-port-bme280"
version = "0.1.0"
edition = "2021"

[lib]
name = "soma_esp32_port_bme280"
crate-type = ["rlib"]

[dependencies]
soma-esp32-leaf = { path = "../../leaf" }
serde_json = { version = "1", default-features = false, features = ["alloc"] }
# OPTION A — type-erased backend (recommended):
#   The port stores a Box<dyn FnMut...> that the firmware injects at boot.
#   The port crate stays free of any esp-hal dep. See ports/adc and ports/pwm
#   for the pattern.
#
# OPTION B — direct esp-hal calls:
#   The port talks to esp-hal::i2c directly. Add the dep WITHOUT a chip feature:
#     esp-hal = { version = "0.23" }
#   The chip is selected by the firmware crate, NEVER here.
```

`ports/bme280/src/lib.rs`: implement the `SomaEspPort` trait, declare the primitives (`bme280.read_temp`, `bme280.read_pressure`, `bme280.read_humidity`), and implement them. Prefer Option A — the firmware injects an I²C-read closure that hides the chip-specific peripheral type behind `Box<dyn FnMut(u8, &mut [u8]) -> Result<...>>`, the same trick `ports/adc` and `ports/pwm` use.

### 7.2 Register in firmware Cargo.toml

Edit `firmware/Cargo.toml`:
```toml
[features]
default = [..., "bme280"]
bme280 = ["dep:soma-esp32-port-bme280"]

[dependencies]
soma-esp32-port-bme280 = { path = "../ports/bme280", optional = true }
```

### 7.3 Wire it in EVERY chip module

The port lives in `firmware/src/chip/<chip>.rs`'s `register_all_ports` function. Add a block for each chip you want to support:

```rust
// firmware/src/chip/esp32s3.rs (and esp32.rs, and any future chip module)
#[cfg(feature = "bme280")]
{
    // If you used OPTION A: build the closure here, capturing the chip's
    // I2C peripheral, and pass it to Bme280Port::new.
    //
    // If you used OPTION B: pass the typed I2C peripheral directly. The
    // port crate's esp-hal types resolve to whatever chip the firmware
    // crate selected.
    let bme280_port = soma_esp32_port_bme280::Bme280Port::new(/* ... */);
    composite.register(Box::new(bme280_port));
    println!("[port] registered: bme280");
}
```

main.rs is **not** edited — it calls `chip::active::register_all_ports`, which dispatches to the per-chip module.

Add to the workspace `Cargo.toml`:
```toml
members = [..., "ports/bme280", ...]
```

Rebuild and reflash. The new primitives appear automatically in `ListCapabilities`. The brain doesn't need to be rebuilt — it learns about the new skills the next time it asks.

---

## 8. Add a routine without rebuilding

The whole point of the wire protocol is that the brain can teach the body new high-level skills at runtime. Once you've built `client.py`:

```python
# Define a sensor-trigger routine: read thermistor, then toggle GPIO15 if hot.
# This is just an example of TransferRoutine — the body will execute the
# steps in order on every InvokeSkill call.
call(s, "TransferRoutine sample_and_blink", {
    "type": "transfer_routine",
    "routine": {
        "routine_id": "sample_and_blink",
        "description": "Read thermistor then toggle GPIO15",
        "steps": [
            {"skill_id": "thermistor.read_temp", "input": {"channel": 0}},
            {"skill_id": "gpio.toggle",          "input": {"pin": 15}},
            {"skill_id": "delay.ms",             "input": {"ms": 250}},
            {"skill_id": "gpio.toggle",          "input": {"pin": 15}}
        ]
    }
})

# Routines are persisted in RAM only — they vanish on reset. To make a
# routine survive reboots, store the JSON in storage and have the firmware
# re-transfer it on boot. (Future work: add a `RoutinePersist` message type.)
```

---

## 9. WiFi mode (proven on both chips)

`--features wifi` turns on esp-wifi 0.12, smoltcp with DHCP, the wifi port (5 primitives: scan / configure / status / disconnect / forget), and a TCP listener on port 9100 that accepts the same wire protocol as UART0. Proven on both ESP32-S3 and ESP32 LX6.

### 9.1 Build + flash + test

```bash
# ESP32-S3
./scripts/cycle.sh esp32s3 wifi         # 16/16 tests including wifi.scan

# ESP32 / WROOM-32D
./scripts/cycle.sh esp32 wifi           # 16/16 tests including wifi.scan
```

Or manually:

```bash
. ~/export-esp.sh
cargo +esp build --release \
    --config "$(pwd)/firmware/chips/esp32s3.toml" \
    --no-default-features \
    --features "chip-esp32s3 gpio delay uart i2c spi adc pwm storage thermistor wifi" \
    -p soma-esp32-firmware
./scripts/flash.sh esp32s3
./scripts/test.sh esp32s3 "" wifi
```

Expected output after boot:

```
[wifi] esp-wifi initialized
[wifi] smoltcp Interface created
[port] registered: wifi (RealWifiOps via esp-wifi station mode)
...
Free heap before dispatch: 50104 bytes    ← headroom after wifi init
UART0 (host wire frames): yes (115200 8N1)
TCP over WiFi:           yes (port 9100, when WiFi connected)
Body alive, awaiting brain messages
[net] DHCP lease lost                      ← smoltcp polling DHCP, no creds yet
```

### 9.2 Configuring WiFi from the host

Once booted, the host drives the wifi port over UART0 to get the radio onto a network:

```python
# List networks the radio sees
call(s, "wifi.scan",
     {"type": "invoke_skill", "peer_id": "host",
      "skill_id": "wifi.scan", "input": {}})
# -> {"networks": [{"ssid": "AP3-1", "channel": 6, "rssi": -54, ...}, ...]}

# Connect — credentials persist in SPI flash via FlashKvStore
call(s, "wifi.configure",
     {"type": "invoke_skill", "peer_id": "host",
      "skill_id": "wifi.configure",
      "input": {"ssid": "MyNetwork", "password": "MyPass"}})

# Check connection state (connected, IP, RSSI, MAC)
call(s, "wifi.status",
     {"type": "invoke_skill", "peer_id": "host",
      "skill_id": "wifi.status", "input": {}})
```

After the radio associates, the firmware prints `[net] DHCP assigned: x.x.x.x` and the TCP listener starts accepting connections on port 9100. From that point the **same wire protocol** works over a TCP socket — swap pyserial for a regular `socket.socket(AF_INET, SOCK_STREAM)` and the client code is otherwise identical.

### 9.3 Heap sizing

The firmware allocates **96 KB of heap** (`esp_alloc::heap_allocator!(96 * 1024)` in main.rs). esp-wifi 0.12 + smoltcp + TCP rx/tx buffers + our Vecs need at least ~50 KB. 96 KB leaves comfortable headroom. **Do not reduce below 80 KB for wifi builds** — below that the dispatch loop dies with a garbage-looking "memory allocation of N bytes failed" panic within milliseconds of entering run_dual_transport (the alloc error handler's size argument reads uninitialized stack bytes when the layout actually fails, which is why the number looks random).

### 9.4 ESP32 LX6 wifi.scan quirk

On ESP32 LX6 (original, WROOM-32), esp-wifi 0.12 sometimes crashes the chip with an illegal-instruction exception on `wifi.scan` **after** heavy SPI flash writes in the same boot cycle. ESP32-S3 is unaffected. The symptom is:

```
PANIC
panicked at xtensa-lx-rt-0.18.0/src/exception/context.rs:148:5:
Exception: Illegal...
```

The radio is still functional — it's a state-interaction bug between esp-wifi and esp-storage. **Workaround: run wifi operations before storage-heavy ones in the same boot cycle.** `scripts/wire-test.py --wifi` orders the wifi tests first for exactly this reason. If you're writing your own client, do `wifi.scan` and `wifi.configure` immediately after boot, then do your storage work after.

### 9.5 The vendored esp-alloc patch

`vendor/esp-alloc/` contains a patched copy of `esp-alloc 0.6.0` wired in via `[patch.crates-io]` in the workspace root. The patch adds a zero-byte-allocation guard to `GlobalAlloc::alloc` and `EspHeap::alloc_caps` that returns a dangling pointer for `layout.size() == 0` (matching what `alloc::alloc::Global::alloc` does).

**Historical note**: this patch was added while diagnosing the original "wifi panics at boot" issue. The real root cause turned out to be heap size, not zero-byte allocs — a 64 KB heap was too small and the panic format string was reading garbage stack bytes. The patch is still present because it's a correct defensive measure (stock esp-alloc 0.6 really does mishandle zero-byte allocs per its own docs) and removing it risks regressing something subtle. When the stack is eventually upgraded to esp-alloc 0.8+ (which has the fix upstream), the patch can be dropped.

---

## 10. Board port — runtime diagnostics and pin configuration

The `board` port is enabled by default and exposes five chip-agnostic primitives that let an LLM (or any host) inspect and reconfigure a running leaf without a reflash. All skills are reachable over the normal wire protocol — UART, TCP, or MCP `invoke_remote_skill`.

| Skill | Effect | Purpose |
|---|---|---|
| `board.chip_info` | ReadOnly | Returns `{chip, mac, free_heap, uptime_ms, firmware_version}`. First call after any deploy — confirms who you're talking to. |
| `board.pin_map` | ReadOnly | Returns the current `(pins.* key, gpio)` assignments. Reads fresh from FlashKvStore each call, so it always reflects what will be loaded on the next boot. |
| `board.configure_pin` | StateMutation | Persists a `pins.*` key to flash. Takes effect on next boot. Refuses any key that doesn't start with `pins.` so it can't accidentally overwrite wifi or other config. |
| `board.probe_i2c_buses` | StateMutation | Takes a list of `[sda, scl]` pairs, tears down the live I²C peripheral, re-initializes it on each pair, and scans 0x08..0x77 for ACKs. **Destroys the active I²C state** — after probing, call `board.reboot` before using `i2c.*` again. |
| `board.reboot` | ExternalEffect | Soft-resets the chip via `esp_hal::reset::software_reset()`. Used after `configure_pin` to apply new pin assignments. The caller's connection will be reset mid-send — that's expected. |

### 10.1 How runtime pin configuration works

At boot, each chip module (`firmware/src/chip/<chip>.rs`) calls `PinConfig::load(&FlashKvStore::new())`, which reads every `pins.*` key from the SOMA config sector and falls back to `DEFAULT_*` constants for missing keys. The resolved config is then used to construct the GPIO/I²C/SPI/ADC/PWM/UART peripherals via `AnyPin::steal(n)` (for everything except ADC — see next paragraph).

ADC is the one exception. `esp-hal`'s `AdcConfig::enable_pin` requires a statically-known `GpioPin<N>` because the `AdcChannel` trait is only implemented for concrete pin types. The chip module handles this with a `match` over every ADC1-capable pin: each arm constructs its own typed `Adc` instance inside a closure. At runtime exactly one arm runs, so `peripherals.ADC1` is moved exactly once. Adding a new ADC1 pin means adding one line to the match.

The valid GPIO list is **per chip**: ESP32 excludes GPIOs 6-11 (QSPI flash), ESP32-S3 excludes GPIOs 26-32 (QSPI flash + PSRAM). Configuring a pin outside the valid list logs a warning and silently falls back to the default — the firmware never tries to drive a flash pin, even if an LLM misconfigures one. Strapping pins (0, 2, 5, 12, 15 on ESP32; 0, 45, 46 on ESP32-S3) are allowed because they work after boot; the caller is responsible for not breaking the reset sequence.

### 10.2 End-to-end example: discover an OLED and reconfigure I²C

Suppose a new board has an SSD1306 OLED wired to GPIO 21/22 but the firmware defaults to 5/4. The full discovery-and-reconfigure cycle runs through MCP `invoke_remote_skill` with zero rebuilds:

```bash
# 1. Discover what's actually on the bus. Try a few common pairs.
invoke_remote_skill board.probe_i2c_buses \
    { "candidates": [[5, 4], [21, 22], [33, 32]] }
# -> {"probes": [
#      {"sda": 5,  "scl": 4,  "addresses": [],   "error": null},
#      {"sda": 21, "scl": 22, "addresses": [60], "error": null},   # ← found 0x3C
#      {"sda": 33, "scl": 32, "addresses": [],   "error": null}
#    ]}

# 2. Persist the working pair to flash.
invoke_remote_skill board.configure_pin \
    { "key": "pins.i2c0.sda", "value": "21" }
invoke_remote_skill board.configure_pin \
    { "key": "pins.i2c0.scl", "value": "22" }
# -> {"key": "pins.i2c0.sda", "value": "21", "stored": true, "reboot_required": true}

# 3. Reboot to apply. The connection drops mid-send — that's expected.
invoke_remote_skill board.reboot {}
# -> TransportFailure: Connection reset by peer   ← the chip is rebooting

# 4. Re-discover via mDNS (the firmware re-announces after DHCP) and verify.
invoke_remote_skill board.pin_map {}
# -> {"pins": [..., {"key": "pins.i2c0.sda", "gpio": 21},
#                   {"key": "pins.i2c0.scl", "gpio": 22}, ...]}

# 5. Normal i2c.* skills now work on the new pins.
invoke_remote_skill i2c.scan {}
# -> {"addresses": [60]}
```

This is proven end-to-end against a WROOM-32D + OLED dev board. The same cycle works on ESP32-S3 with no code changes.

### 10.3 The `pins.*` keys each chip honors

Every chip exposes the same nine keys. Defaults come from `DEFAULT_*` constants in the chip module; override any of them with `board.configure_pin`.

| Key | ESP32 default (WROOM-32D) | ESP32-S3 default (Sunton 1732S019) | Notes |
|---|---|---|---|
| `pins.gpio.test` | 13 | 15 | Pin claimed by the `gpio` port for `gpio.write/read/toggle`. |
| `pins.i2c0.sda` | 5  | 8  | I²C0 SDA. |
| `pins.i2c0.scl` | 4  | 9  | I²C0 SCL. |
| `pins.spi3.sck` | 18 | 35 | SPI3 (VSPI on ESP32) clock. SPI2 is reserved for the on-board display on 1732S019. |
| `pins.spi3.mosi`| 23 | 36 | SPI3 MOSI. |
| `pins.adc.pin`  | 34 | 2  | ADC1 input. Must be an ADC1-capable GPIO for that chip (see chip module's `adc_channel_for_pin` match). |
| `pins.pwm.pin`  | 25 | 16 | LEDC channel 0 output. |
| `pins.uart1.tx` | 16 | 17 | UART1 TX. UART0 is reserved for host wire-protocol transport. |
| `pins.uart1.rx` | 17 | 18 | UART1 RX. |

The FlashKvStore sector lives at flash offset `0x3F_F000` — the last 4 KB of the factory partition on both the 4 MB (ESP32) and 16 MB (ESP32-S3) default layouts. The sector is rewritten in its entirety on every `board.configure_pin` (erase-then-write), so configuration changes are rare-write by design.

### 10.4 Display port — SSD1306 OLED over shared I²C

The firmware ships with a `display` port that drives an SSD1306-class 128×64 monochrome OLED over the same I²C0 bus the `i2c` port uses. Both ports receive their own `embedded_hal_bus::i2c::RefCellDevice` handle into a single leaked `&'static RefCell<I2c>` — no stealing, no tearing down the bus, no contention for the other ports that ride I²C via sensors.

Skills:

| Skill | Effect | What it does |
|---|---|---|
| `display.info` | ReadOnly | `{width, height, driver, i2c_addr}`. Defaults: 128×64, ssd1306, 0x3C. |
| `display.clear` | StateMutation | Clear framebuffer + flush. |
| `display.draw_text {text, line?, column?, invert?}` | StateMutation | Draw on text row `line` (0..5 for a 128×64 panel at 6x10 font), starting at character column `column`. Clears the row first, so successive calls on the same line replace the content. Flushes. |
| `display.draw_text_xy {text, x, y, invert?}` | StateMutation | Draw at absolute pixel coordinates. Does NOT clear under the glyphs. Flushes. |
| `display.fill_rect {x, y, width, height, on}` | StateMutation | Fill a rectangle with on/off pixels. Flushes. |
| `display.set_contrast {value}` | StateMutation | Map a u8 (0-255) to the nearest `Brightness` preset and push to the panel. |
| `display.flush` | StateMutation | Force a framebuffer flush. Rarely useful — the draw skills all flush implicitly. |

**Architecture**: the `ports/display/` crate has no `esp-hal` / `ssd1306` / `embedded-graphics` dep. It stores seven type-erased `Box<dyn FnMut(...)>` closures and calls into them when a skill arrives. The firmware constructs the real `Ssd1306` driver inside `chip/<chip>.rs::register_i2c_and_display`, wraps it in a leaked `&'static RefCell`, and closes over it in each closure. Same pattern as `ports/adc` and `ports/board`. The chip module owns the hardware knowledge; the port crate owns the wire-protocol surface.

**Shared bus wiring** (in both `chip/esp32.rs` and `chip/esp32s3.rs`):

```rust
let bus_static: &'static RefCell<I2c<'static, Blocking>> =
    Box::leak(Box::new(RefCell::new(i2c)));

// i2c port gets its own handle
let i2c_device = RefCellDevice::new(bus_static);
composite.register(Box::new(I2cPort::new(i2c_device)));

// display port builds its Ssd1306 on a second handle
let display_device = RefCellDevice::new(bus_static);
let ssd = Ssd1306::new(I2CDisplayInterface::new(display_device), ...)
    .into_buffered_graphics_mode();
```

Because `I2cPort` is generic over any `embedded_hal::i2c::I2c` implementor, this swap is a one-line change — the refactor to make `I2cPort<B>` generic is what unlocks the shared-bus story. Handing in a raw `esp_hal::I2c` still works too (the `display` cargo feature off path in `register_all_ports`).

**Tracing the loop**: `scripts/thermistor-to-display.py` opens a direct TCP connection to the leaf's wire-protocol listener on port 9100 and every N seconds invokes `thermistor.read_temp` + `display.draw_text`. Typical output:

```
$ ./scripts/thermistor-to-display.py --host 192.168.100.203 --interval 5 --ticks 3
[init] connecting to leaf at 192.168.100.203:9100
[init] display cleared
[tick   1] temp_c=21.50  roundtrip=  150ms  -> displayed
[tick   2] temp_c=21.75  roundtrip=  407ms  -> displayed
[tick   3] temp_c=22.00  roundtrip=  186ms  -> displayed
```

The same loop runs via soma-next MCP — an LLM asks `invoke_remote_skill thermistor.read_temp` then `invoke_remote_skill display.draw_text` in a 5-second cadence. No firmware changes required; the brain is the scheduler.

**Turn it off**: drop `display` from the feature list. The firmware falls back to handing the raw `I2c<'_, Blocking>` straight to `I2cPort` (no RefCellDevice wrapper, no leaked bus). That path saves ~30 KB of flash and removes the `ssd1306` + `embedded-graphics` + `embedded-hal-bus` dependency chain.

### 10.5 Why not just set pins at compile time?

The original design hardcoded pins in each chip module. That made pin changes a reflash, which is unusable for:

- **Bringing up a new board** where you don't know which pins the pre-wired peripherals are on.
- **LLM-driven diagnosis** where the brain inspects the body, notices an I²C bus has no devices, probes alternatives, and reconfigures — all without human intervention.
- **Multi-board fleets** where each unit has slightly different wiring but runs the same firmware binary.

The runtime approach keeps the defaults in the chip module (first boot "just works") but makes everything changeable via the wire protocol. The cost is one extra `unsafe { AnyPin::steal(n) }` per peripheral at boot, and a per-chip ADC `match`. The benefit is that pin configuration becomes a skill, indistinguishable from `wifi.configure` or `storage.set`.

---

## 11. Adding a new chip

Two chips are proven today (ESP32-S3 and ESP32). Adding any other Espressif chip is **a four-step recipe with no main.rs changes**.

| Chip | Architecture | Toolchain | Status |
|---|---|---|---|
| ESP32-S3 | Xtensa LX7 | `+esp` (espup) | **proven** (`chip-esp32s3`, `chips/esp32s3.toml`) |
| ESP32 / WROOM-32D | Xtensa LX6 | `+esp` (espup) | **proven** (`chip-esp32`, `chips/esp32.toml`) |
| ESP32-S2 | Xtensa LX7 | `+esp` (espup) | not proven — use the recipe below; no native USB on most boards |
| ESP32-C3 | RISC-V `imc` | stock rustc | not proven — use the recipe below; native USB Serial JTAG available |
| ESP32-C6 | RISC-V `imac` | stock rustc | not proven — use the recipe below; native USB Serial JTAG available |
| ESP32-H2 | RISC-V `imac` | stock rustc | not proven — use the recipe below; no Wi-Fi (BLE/802.15.4 only) |

### 10.1 The four-step recipe

#### Step 1 — copy a chip module

```bash
# Pick the closest existing chip as your starting point.
# Xtensa LX6/LX7 ⇒ start from esp32s3.rs.  RISC-V (C3/C6/H2) ⇒ also start from esp32s3.rs
# but be ready to drop the xtensa-lx-rt dep in Step 3.
cp firmware/src/chip/esp32s3.rs firmware/src/chip/<chip>.rs
```

Edit the new file:
- `pub const NAME: &str` — friendly chip name in the boot banner
- `pub const TEST_LED_PIN: u32` — pin claimed by the gpio port + used by the boot self-test
- Pin numbers passed to `peripherals.GPIOxx` for I²C, SPI, ADC, PWM, UART1
- Peripheral instance names if they differ (`peripherals.SPI3` vs `peripherals.SPI2`, etc.)

The function signatures and the `register_all_ports` body structure stay identical — only the chip-specific bits inside change. The `chip::esp32.rs` module is the worked example for "this chip has fewer GPIOs and reserves some pins for flash".

#### Step 2 — register the chip in `chip/mod.rs`

```rust
// firmware/src/chip/mod.rs
#[cfg(feature = "chip-<chip>")]
pub mod <chip>;
#[cfg(feature = "chip-<chip>")]
pub use <chip> as active;
```

The `compile_error!` macros at the bottom enforce that exactly one chip feature is enabled at a time. Update them to mention your new chip if you want a more helpful error.

#### Step 3 — add the cargo feature in `firmware/Cargo.toml`

```toml
[features]
chip-<chip> = [
    "esp-hal/<chip>",
    "esp-backtrace/<chip>",
    "esp-println/<chip>",
    "esp-storage/<chip>",
    # Xtensa chips:
    "xtensa-lx-rt/<chip>",
    # RISC-V chips: no xtensa-lx-rt; you'll also want to add `riscv-rt`
    # as a direct dep gated on cfg(target_arch = "riscv32")
]
```

For **RISC-V chips** (C3/C6/H2):
- Drop `xtensa-lx`/`xtensa-lx-rt` from the chip feature list
- Add `riscv-rt = "0.13"` (or whichever version esp-hal expects) to `[target.'cfg(target_arch = "riscv32")'.dependencies]`
- Remove the `rust-toolchain.toml`'s `esp` channel pin OR ignore it for RISC-V builds (stock `rustc` handles RISC-V targets natively, you don't need `+esp`)

#### Step 4 — add a cargo config overlay in `firmware/chips/<chip>.toml`

```toml
[build]
target = "<chip-target-triple>"

[target.<chip-target-triple>]
runner = "espflash flash --monitor --port /dev/cu.usbserial-XXXX"
rustflags = [
  "-C", "link-arg=-Tlinkall.x",
  "-C", "force-frame-pointers",
]

[unstable]
build-std = ["core", "alloc"]
```

Common target triples:
- `xtensa-esp32-none-elf` (ESP32 LX6)
- `xtensa-esp32s2-none-elf` (ESP32-S2 LX7)
- `xtensa-esp32s3-none-elf` (ESP32-S3 LX7)
- `riscv32imc-unknown-none-elf` (ESP32-C3, ESP32-C2)
- `riscv32imac-unknown-none-elf` (ESP32-C6, ESP32-H2)
- `riscv32imafc-unknown-none-elf` (ESP32-P4)

### 10.2 Build and flash your new chip

```bash
. ~/export-esp.sh   # only needed for Xtensa chips
cargo +esp build --release \
    --config $(pwd)/firmware/chips/<chip>.toml \
    --no-default-features \
    --features "chip-<chip> gpio delay uart i2c spi adc pwm storage thermistor" \
    -p soma-esp32-firmware

espflash flash --port /dev/cu.usbserial-XXXX \
    target/<target-triple>/release/soma-esp32-firmware
```

For RISC-V chips drop the `+esp` toolchain selector — stock `cargo build` works.

**main.rs and the port crates do not change.** The only files you touch are the four listed above. The chip-agnostic parts of the firmware (boot banner, app descriptor, FlashKvStore, dispatch loop, self-test) work as-is via `chip::active::*`.

---

## 12. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `cargo build` says `unresolved import xtensa_lx` | esp-hal 0.23 needs `xtensa-lx`/`xtensa-lx-rt` as direct deps when targeting Xtensa | already in `firmware/Cargo.toml`; if you removed them, add back: `xtensa-lx = "0.10"`, `xtensa-lx-rt = "0.18"` (the chip feature comes from `chip-*` cargo features, not the dep declaration) |
| `cargo build` says `Linking globals named 'rtc_clk_xtal_freq_get': symbol multiply defined` | esp-rom-sys is being pulled in (typically by `esp-bootloader-esp-idf` with the chip feature) and collides with esp-hal 0.23 | this firmware does NOT use `esp-bootloader-esp-idf` — it provides its own `esp_app_desc` struct in `main.rs`. Don't add the bootloader crate. |
| `error: expected exactly one enabled feature from feature group: ["esp32", "esp32c2", ..., "esp32s3"]` from esp-hal's build.rs | Either zero or two `chip-*` features are enabled in the firmware crate, OR a port crate hardcodes a chip feature (`features = ["esp32s3"]`) that conflicts with the firmware's selection | Pass exactly one chip via `--features "chip-esp32s3 ..."` (or `chip-esp32`, etc.). If a port crate hardcodes a chip, fix it: `esp-hal = { version = "0.23" }` with NO chip feature. See §2.1. |
| `error: Building esp-storage for ESP32 needs optimization level 2 or 3 - yours is s` | Workspace `[profile.release]` uses `opt-level = "s"` and the per-package override for esp-storage was removed | Restore the override in workspace `Cargo.toml`: `[profile.release.package.esp-storage]\nopt-level = 3`. ESP32 LX6 needs this for esp-storage's flash-write timing. ESP32-S3 doesn't but it's harmless there. See §2.1 and §3.2. |
| Boot loop with `Image requires efuse blk rev >= v237.62` (or any other huge number) | The ESP-IDF application descriptor is missing or in the wrong section | Make sure `firmware/build.rs` runs (it generates `rwtext.x` in `OUT_DIR` to inject `.flash.appdesc` ahead of `.rwtext.wifi`). Confirm with `xtensa-esp-elf-objdump -t target/<target>/release/soma-esp32-firmware \| grep esp_app_desc` — the symbol must live in `.flash.appdesc` at vma `0x3c000020` (S3) or `0x3f400020` (ESP32). |
| Boot loop with `ESP-IDF App Descriptor missing in your esp-hal application` | espflash refuses to build the image because the descriptor isn't found | Check that `core::hint::black_box(&esp_app_desc);` is the FIRST line of `main()`. Without it LTO drops the static. Verify with the same objdump command above. |
| `panicked at memory allocation of 0 bytes failed` after boot | esp-alloc 0.6 doesn't handle 0-byte allocations gracefully; some background task (often esp-wifi) requests one | Build without `--features wifi`. The wifi path needs an allocator wrap or stack upgrade (see §9). |
| `error: failed to parse value from --config argument` when invoking cargo with `--config firmware/chips/esp32.toml` | Cargo's `--config` flag expects either a `key=value` expression or an absolute path; relative paths get parsed as expressions | Use an absolute path: `--config $(pwd)/firmware/chips/esp32.toml`. The `scripts/build.sh` wrapper does this for you. |
| `error[E0277]: the trait bound 'GpioPin<35>: PeripheralOutput' is not satisfied` (or similar trait bound on a high-numbered GPIO) when building for ESP32 | A pin number from the ESP32-S3 chip module (which has GPIO 0-48) leaked into the ESP32 build (which only has GPIO 0-39, with 34-39 input-only) | Each chip module owns its own pin numbers. The pin in question is from the wrong file. Make sure you only edited `chip/esp32.rs` for ESP32 and `chip/esp32s3.rs` for S3. |
| Boot succeeds but `gpio.toggle pin=N` returns `failure_message: "pin N not claimed"` | The chip module's `register_all_ports` claimed a different pin than the host is trying to use | The claimed pin is logged on boot: `[port] registered: gpio (3 primitives, GPIOXX claimed)`. Use that pin number in your wire-protocol calls. The `scripts/test.sh` table maps each chip to its test pin — keep it in sync with `chip::active::TEST_LED_PIN`. |
| `No serial ports could be detected` | Cable is power-only or chip not in known boot state | Swap to a known data cable. For ESP32-S3 boards, hold BOOT and tap RESET (or unplug+replug while holding BOOT) to force ROM-bootloader mode. |
| `cu.usbserial-XXXX` shows up but `espflash board-info` hangs | Wrong port, or another process is holding it | `lsof /dev/cu.usbserial-XXXX` to find who, kill it, retry. A common culprit is a leftover `espflash monitor` from a previous run. |
| `Image contains multiple DROM segments. Only the last one will be mapped.` warning at boot | Linker created multiple sections going into drom_seg | This is a benign warning when the appdesc is in its own `.flash.appdesc` section. The bootloader still reads the descriptor at boot from flash; runtime accesses to the descriptor would fault, but the firmware only references it once via `black_box` at boot. Ignore the warning. |
| Garbage on the host parser even though boot output looks fine | Frame scanner started reading mid-frame, mid-log-line | `scripts/wire-test.py` handles this — it skips bytes until it finds a valid 4-byte length followed by parseable JSON. If you wrote your own parser, drain the boot output first (`ser.read(8192)` after a 0.3s sleep) before sending the first frame. |
| `./scripts/test.sh` reports `[FAIL] gpio.write ... missing 'value'` | The host sent `1` instead of `true` (or `0` instead of `false`) for the gpio value | The body validates input schemas. `gpio.write` requires `{"pin": u32, "value": bool}` — Python `True`/`False`, JSON `true`/`false`. Numbers are not coerced. |

---

## 13. Next steps

**Done in this branch:**
- ✅ ESP32-S3 (Xtensa LX7) — full wire protocol against Sunton 1732S019, 14/14 tests pass via `./scripts/cycle.sh esp32s3`
- ✅ ESP32 (Xtensa LX6, WROOM-32D) — full wire protocol against the 18650/OLED dev board, 14/14 tests pass via `./scripts/cycle.sh esp32`
- ✅ **WiFi on ESP32-S3** — 16/16 tests including real `wifi.scan` + `wifi.status`, TCP listener on port 9100 ready, via `./scripts/cycle.sh esp32s3 wifi`
- ✅ **WiFi on ESP32 (LX6)** — 16/16 tests via `./scripts/cycle.sh esp32 wifi` (wifi tests ordered first to avoid LX6 scan-after-storage-write quirk)
- ✅ Single source tree, single firmware crate, chip selection via cargo features + per-chip config overlays
- ✅ Helper scripts under `scripts/` for setup, board detection, build, flash, monitor, test, end-to-end cycle — all with optional `wifi` variant
- ✅ `firmware/src/chip/<chip>.rs` module split — main.rs is chip-agnostic, adding a chip is dropping a single file (see §11)
- ✅ **`board` port** — runtime diagnostics + pin reconfiguration, 5 primitives (`chip_info`, `pin_map`, `configure_pin`, `probe_i2c_buses`, `reboot`). I²C discovery → configure → reboot cycle proven end-to-end over MCP (see §10)
- ✅ **`display` port (SSD1306)** — 7 primitives sharing the I²C0 bus with the `i2c` port via `embedded-hal-bus::RefCellDevice`. Draw text, rectangles, set contrast, flush. Proven: MCP `invoke_remote_skill display.draw_text` + `scripts/thermistor-to-display.py` 5-second periodic sensor update against a real OLED on the WROOM-32D. (see §10.4)
- ✅ Vendored + patched `esp-alloc 0.6.0` with zero-byte guard, 96 KB heap, resolves the 64 KB OOM panic in wifi builds

**Open work:**
- **ESP32-C3 / C6 / S2 chip modules** — copy `chip/esp32s3.rs`, retarget pins, drop xtensa-lx-rt for the C3/C6 RISC-V chips (see §11 recipe).
- **Connect wifi to an actual network end-to-end** — `wifi.configure` → DHCP → TCP accept on port 9100 → exercise wire protocol over TCP. The code path is wired; needs a working SSID/password and an over-the-air smoke test.
- **Investigate ESP32 LX6 `wifi.scan` after storage writes** — esp-wifi 0.12 bug, workaround in place (wifi tests run first). Proper fix would be in esp-wifi upstream.
- **Display port** — the 1732S019 has an ST7789 1.9″ panel sitting unused on SPI2. A new `ports/display/` crate would expose `display.draw_text`, `display.fill_rect`, etc., usable from routines just like GPIO. Each chip module would wire it via `register_all_ports`.
- **Connect to soma-next** — start a server SOMA with `--listen` and have the ESP32 firmware connect as a peer (wifi now works, so the only missing piece is configuring credentials and walking the TCP handshake). The wire protocol additions in soma-next (`ListCapabilities`, `RemoveRoutine`, `RoutineStored`, `RoutineRemoved`) are already on the server side, so the leaf and the runtime speak the same language.
- **MCP integration** — once an ESP32 is reachable from a soma-next peer, the same MCP tools (`invoke_remote_skill`, `transfer_routine`) work transparently from Claude Code or any other LLM driving SOMA.
