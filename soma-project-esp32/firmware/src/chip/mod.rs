// chip — chip-specific peripheral wiring lives here, one file per chip.
//
// main.rs stays chip-agnostic by calling `chip::active::*`. Adding a new
// Espressif chip is a four-step recipe with no main.rs changes:
//
//   1. Add a new file `src/chip/<chip>.rs` implementing the same surface
//      as `esp32s3.rs` and `esp32.rs` (NAME, TEST_LED_PIN, init_peripherals,
//      register_all_ports).
//   2. Add a new feature flag `chip-<chip>` in `firmware/Cargo.toml` that
//      fans out to every esp-* dep's chip feature.
//   3. Add a new cargo config overlay `firmware/chips/<chip>.toml` setting
//      the build target and the espflash runner port.
//   4. Add a new cfg-gated `pub mod` + `pub use ... as active` line below.
//
// Exactly one chip-* feature must be enabled at a time — both esp-hal and
// the host build's chip module declarations are mutually exclusive.

#[cfg(feature = "chip-esp32s3")]
pub mod esp32s3;
#[cfg(feature = "chip-esp32s3")]
pub use esp32s3 as active;

#[cfg(feature = "chip-esp32")]
pub mod esp32;
#[cfg(feature = "chip-esp32")]
pub use esp32 as active;

#[cfg(not(any(feature = "chip-esp32s3", feature = "chip-esp32")))]
compile_error!(
    "soma-esp32-firmware requires exactly one chip feature: \
     enable `chip-esp32s3` (default) or `chip-esp32`."
);

#[cfg(all(feature = "chip-esp32s3", feature = "chip-esp32"))]
compile_error!(
    "soma-esp32-firmware: chip-esp32s3 and chip-esp32 are mutually exclusive. \
     Use `--no-default-features` when selecting the non-default chip."
);
