# soma-project-esp32

SOMA leaf node for ESP32, built around modular **port crates** composed via cargo features. Mirrors the desktop SOMA `soma-ports/` model — just compile-time instead of runtime, because microcontrollers can't dynamically load shared libraries.

End-vision: multiple ESP32s on WiFi acting as SOMA peer leaf nodes. An LLM (Claude / Codex / Ollama) talks to a server SOMA via MCP. The server has the ESP32s registered as s2s peers. The LLM says "turn on the kitchen light" → server SOMA delegates via `invoke_remote_skill` → kitchen ESP32 invokes its local GPIO primitive → light turns on. Each ESP32 advertises whatever ports its firmware was built with.

## Architecture: body, brain, and ports

```
            ┌─────────┐  MCP   ┌──────────────┐
            │ Claude  │ ─────► │ Server SOMA  │
            │ Codex   │        │ (postgres,   │
            │ Ollama  │        │  s3, smtp)   │
            └─────────┘        └──────┬───────┘
                                      │ s2s wire protocol over WiFi
                          ┌───────────┼───────────┐
                          ▼           ▼           ▼
                    ┌─────────┐ ┌─────────┐ ┌─────────┐
                    │ esp32   │ │ esp32   │ │ esp32   │
                    │ kitchen │ │ bedroom │ │ garage  │
                    │ ports:  │ │ ports:  │ │ ports:  │
                    │  core   │ │  core   │ │  core   │
                    │  +ws2812│ │  +bme280│ │  +servo │
                    │  +relay │ │  +pir   │ │  +reed  │
                    └─────────┘ └─────────┘ └─────────┘
```

Three layers of skill abstraction:

```
Layer 3: Routines (transferred at runtime via TransferRoutine, NO rebuild)
   "blink_kitchen_light"  = [gpio.write(7,1), delay.ms(500), gpio.write(7,0)]
   "morning_briefing"     = [bme280.read, dht22.read, status]
   "intruder_check"       = [pir.read, vl53l0x.measure_distance]

Layer 2: Sensor / actuator port primitives (compiled in via cargo features)
   bme280.read, dht22.read, ws2812.set_colors, servo.set_angle, ...

Layer 1: Core hardware primitives (always present from the core port)
   gpio.write, gpio.read, gpio.toggle, i2c.read, i2c.write, spi.transfer,
   adc.read, pwm.set_duty, pwm.set_freq, uart.write, uart.read,
   delay.ms, delay.us, ...

Layer 0: ESP32-C3 hardware
```

**Adding a new high-level skill** (e.g. "blink the LED 5 times") = the brain composes a Routine from existing primitives and sends a `TransferRoutine` wire message. The leaf stores it. Next `InvokeSkill("blink5")` walks the stored routine. **NO firmware rebuild.**

**Adding a new sensor model** (e.g. supporting BME280 instead of just thermistor) = add a port crate under `ports/`, list it as an optional dependency in `firmware/Cargo.toml`, gate it behind a feature, register it in `main.rs`. **One firmware rebuild and reflash. The architecture and other ports are unchanged.**

**Adding a new bus** (e.g. CAN or I2S audio that the firmware doesn't speak) = add primitives to the core port. Rare. Same kind of effort as adding a sensor crate.

## Workspace structure

```
soma-project-esp32/
├── Cargo.toml              workspace root
├── leaf/                   no_std wire protocol, LeafState, routine storage,
│                           SomaEspPort trait, CompositeDispatcher
│   ├── Cargo.toml
│   └── src/lib.rs          (14 unit tests)
├── ports/                  modular port crates — each is independent
│   ├── core/               ALWAYS included; gpio, i2c, spi, adc, pwm, uart, delay
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   └── thermistor/         example sensor port — same shape as a real BME280/DHT22
│       ├── Cargo.toml
│       └── src/lib.rs
└── firmware/               binary that selects ports via cargo features
    ├── Cargo.toml          [features] gates per port
    ├── .cargo/config.toml
    └── src/main.rs
```

## Build variants

Different ESP32s can run different firmware variants from the **same source tree**, just with different feature combinations:

```bash
cd firmware

# Default — core + thermistor
cargo build --release
# Output: ~74 KB ELF code

# Minimal — core only, no sensor ports
cargo build --release --no-default-features --features core
# Output: ~69 KB ELF code (3 KB smaller)

# Explicit selection (after adding more port crates)
cargo build --release --no-default-features --features "core thermistor"
```

The 5 KB delta between minimal and full is the thermistor port crate. Each future sensor port adds a similar small delta.

## Adding a new sensor port (the 3-step recipe)

Say you want to add BME280 (temperature + pressure + humidity) support:

**1. Create the crate**

```
ports/bme280/
  Cargo.toml          # depends on soma-esp32-leaf and esp-hal
  src/lib.rs          # implements SomaEspPort with bme280.read primitive
```

The lib.rs is ~100 lines: a `BME280Port` struct that owns an I2C handle, a `SomaEspPort` impl that exposes `bme280.read` and `bme280.read_calibrated`, and the actual sensor protocol (calibration register parsing + measurement read sequence). Existing crates like `bme280` from crates.io can be wrapped.

**2. Add the optional dep + feature in `firmware/Cargo.toml`**

```toml
[features]
default = ["core", "thermistor", "bme280"]
bme280 = ["dep:soma-esp32-port-bme280"]

[dependencies]
soma-esp32-port-bme280 = { path = "../ports/bme280", optional = true }
```

**3. Register in `firmware/src/main.rs`**

```rust
#[cfg(feature = "bme280")]
{
    // Construct an I2C bus from peripherals (this is the firmware's job)
    let i2c = ...;
    let bme280 = soma_esp32_port_bme280::Bme280Port::new(i2c);
    composite.register(Box::new(bme280));
    println!("[port] registered: bme280");
}
```

That's it. Recompile, reflash. The brain discovers `bme280.read` via `ListCapabilities` automatically — no protocol changes anywhere.

## Core port primitives (5 — every one is real)

The core port declares ONLY the primitives whose hardware is actually wired against esp-hal. The body never lies about its capabilities — if a primitive isn't in `list_primitives()`, it doesn't exist on this firmware.

| Primitive | Purpose |
|---|---|
| `gpio.write` | Set a claimed GPIO pin high or low (esp-hal `Output::set_high/set_low`) |
| `gpio.read` | Read the last-known logical state of a claimed pin |
| `gpio.toggle` | Flip a claimed GPIO pin |
| `delay.ms` | Block for N milliseconds (esp-hal `Delay::delay_millis`) |
| `delay.us` | Block for N microseconds (esp-hal `Delay::delay_micros`) |

That's it. The other buses (I2C, SPI, ADC, PWM, UART) are **not** in this port — they belong in their own port crates. Each is a separate workspace member added when implemented. This mirrors how desktop SOMA only loads the ports a deployment actually has — `list_ports` reflects reality, not aspirations.

### Future bus port crates

Each bus is its own port crate following the same shape as `ports/core/`:

| Crate (planned) | Primitives | Hardware backing |
|---|---|---|
| `ports/i2c/` | `i2c.read`, `i2c.write`, `i2c.scan` | esp-hal `I2c<Master>` claimed at firmware boot |
| `ports/spi/` | `spi.transfer`, `spi.write` | esp-hal `Spi<Master>` |
| `ports/adc/` | `adc.read`, `adc.read_voltage` | esp-hal `Adc` + per-pin `AdcPin` registration |
| `ports/pwm/` | `pwm.set_duty`, `pwm.set_freq` | esp-hal LEDC channels |
| `ports/uart/` | `uart.write`, `uart.read` | esp-hal `Uart` |

Each is roughly 100-150 lines of esp-hal-specific code: a port struct that owns the peripheral handle, a `SomaEspPort` impl with the right primitives, an `invoke` match. They don't exist yet because the L1/L2 proof only needed GPIO + delay to demonstrate the architecture. Adding each one is a small, focused PR — no changes to the leaf, the firmware Cargo.toml, or other ports beyond the standard 3-step recipe (add dep, add feature, register in main.rs).

### Why the buses aren't in the core port

Two reasons, both architectural:

1. **Honesty.** A port should advertise only what it can deliver. If the core port declared `i2c.read` but the firmware never wired up I2C, the brain would call it and get a runtime error. Better: don't claim it.
2. **Composition.** Different ESP32s need different buses. A board with only sensors on I2C doesn't need SPI. A board talking to a UART display doesn't need PWM. By splitting per-bus, the firmware composes only what it uses, and unrelated changes don't affect each other.

The result: if you flash a firmware with `core + i2c + thermistor`, the brain sees `gpio.* + delay.* + i2c.* + thermistor.*`. If you flash one with just `core + ws2812`, the brain sees `gpio.* + delay.* + ws2812.*`. The same brain code (LLM) talks to both — it adapts based on `ListCapabilities`.

## Wire protocol (mirrors desktop SOMA s2s)

```rust
enum TransportMessage {
    InvokeSkill { peer_id, skill_id, input },   // primitive OR stored routine
    ListCapabilities,                            // discover body's vocabulary
    TransferRoutine { routine },                 // brain teaches body a new skill
    RemoveRoutine { routine_id },                // brain forgets a skill
    Ping { nonce },
}

enum TransportResponse {
    SkillResult { response },                    // single primitive or aggregated routine result
    Capabilities { primitives, routines },       // proprioception
    RoutineStored { routine_id, step_count },
    RoutineRemoved { routine_id },
    Pong { nonce, load },
    Error { details },
}
```

Frames: 4-byte big-endian length prefix + JSON payload. Max frame: 16 KB.

## Status by layer

| Layer | Status | What it proves |
|---|---|---|
| **L1: no_std wire protocol** | DONE | The leaf library compiles for `riscv32imc-unknown-none-elf`. 14/14 unit tests pass. |
| **L2: Real ESP32-C3 firmware** | DONE | The composite firmware compiles to ~74 KB ELF. Two port crates (core + thermistor) compose. The boot self-test transfers a cross-port routine and walks all steps via the dispatch path on real hardware. |
| **L3: WiFi s2s peer** | TODO | Add `esp-wifi`, `embedded-tls` (or plaintext on a trusted LAN), implement a TCP listener that calls `decode_frame → leaf.handle → encode_response`. Register the device with the server SOMA's `--peer` flag. |
| **Multi-device mesh** | TODO | Multiple ESP32s with different port combinations, each registered with the server SOMA. LLM picks the right device by capability via `invoke_remote_skill`. |

## Run the proof

```bash
# Leaf library tests (host-side)
cargo test -p soma-esp32-leaf --features std

# Embedded build for ESP32-C3 (default features: core + thermistor)
cd firmware
cargo build --release

# Or build with only the core port
cargo build --release --no-default-features --features core

# Flash to a real ESP32-C3-DevKitM-1 over USB
cargo install espflash
cargo run --release   # uses .cargo/config.toml runner
```

Expected serial output on boot:

```
=========================================
SOMA ESP32-C3 leaf firmware booted
Free heap: 49152 bytes
=========================================
[port] registered: core (gpio, delay, i2c, spi, adc, pwm, uart)
[port] registered: thermistor
[port] composite has 2 ports, 20 primitives total

=========================================
Body self-model
  primitives: 20
  routines:   0
=========================================
  [SM] gpio.write
  [RO] gpio.read
  [SM] gpio.toggle
  [SM] gpio.set_input
  [SM] gpio.set_output
  [RO] i2c.read
  ...
  [RO] thermistor.read_temp
  [RO] thermistor.read_temp_calibrated

=========================================
Self-test: brain composes a cross-port routine
=========================================
[1] Brain transfers routine 'monitor_temp_pulse_led' with 5 steps
    leaf acknowledged: stored 'monitor_temp_pulse_led' (5 steps)

[2] ListCapabilities now shows the new routine alongside primitives
...

[3] Brain invokes 'monitor_temp_pulse_led'
    response: success=true, steps_executed=5
```

The cross-port routine reads the thermistor port's primitive, toggles GPIO via the core port's primitive, waits, reads the thermistor again — proving the composite dispatcher routes per-skill to the right port and the leaf state walks the routine across ports.

## Why this matters

Without modular ports, the firmware would need to be rebuilt every time you wanted to support a new sensor model — and the leaf library would slowly accumulate domain knowledge about every sensor. With modular ports:

- The leaf library stays small and stable (just protocol + state machine)
- Each sensor crate is independently developed and maintained
- Different ESP32s can have different vocabularies — pick the ports you need
- A community can contribute port crates without touching the core
- The brain (LLM) learns what's available via `ListCapabilities` — same protocol regardless of which ports are loaded

This is the embedded equivalent of soma-ports' desktop pattern, with cargo features replacing dynamic loading. The architectural promise of SOMA — that the runtime is the program and behavior comes from outside — holds even on a $5 microcontroller.
