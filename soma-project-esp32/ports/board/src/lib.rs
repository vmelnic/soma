// soma-esp32-port-board — diagnostic + configuration skills for the leaf.
//
// Primitives:
//   board.chip_info      { }                         -> {chip, mac, free_heap, uptime_ms, firmware_version}
//   board.pin_map        { }                         -> {i2c0.sda, i2c0.scl, spi3.sck, spi3.mosi, adc.pin, pwm.pin, uart1.tx, uart1.rx, gpio.test}
//   board.configure_pin  {peripheral, key, value}    -> {stored, reboot_required}
//   board.probe_i2c_buses {candidates: [[sda, scl]]} -> {probes: [{sda, scl, addresses: [u8]}]}
//   board.reboot         {}                          -> never returns (triggers soft reset)
//
// The port is chip-agnostic. The firmware injects four closures at
// construction time:
//
//   chip_info_fn   : () -> ChipInfo            — read from the chip module
//   pin_map_fn     : () -> Vec<(&'static str, u8)>  — current config (flash-backed)
//   probe_i2c_fn   : &[(u8, u8)] -> Vec<ProbeResult> — tear down I2C, retry each pair
//   reboot_fn      : () -> !                    — esp_hal::reset::software_reset()
//
// Because probe_i2c_fn re-initializes the I2C peripheral, running it
// invalidates the currently-registered i2c port's state. The caller is
// expected to treat it as a one-shot discovery step followed by
// configure_pin → reboot.

#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use serde_json::{json, Value};
use soma_esp32_leaf::{CapabilityDescriptor, Effect, SomaEspPort};

/// Data returned by board.chip_info.
#[derive(Debug, Clone)]
pub struct ChipInfo {
    pub chip: &'static str,
    pub mac: [u8; 6],
    pub free_heap: u32,
    pub uptime_ms: u64,
    pub firmware_version: &'static str,
}

/// A single (sda, scl) probe result returned by board.probe_i2c_buses.
#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub sda: u8,
    pub scl: u8,
    /// 7-bit I²C addresses that ACK'd during the scan. Empty means
    /// either nothing on the bus or the pair is unreachable.
    pub addresses: Vec<u8>,
    /// Error string if the pair couldn't be initialized (e.g. pin
    /// conflict, strapping pin, flash-reserved pin).
    pub error: Option<String>,
}

/// Boxed closures the firmware injects at port construction.
///
/// All are `Box<dyn FnMut ... + Send>` because `SomaEspPort::invoke`
/// takes `&mut self` and the port runs in a single-threaded dispatch
/// loop, so no Sync bound is required.
pub type ChipInfoFn = Box<dyn FnMut() -> ChipInfo + Send>;
pub type PinMapFn = Box<dyn FnMut() -> Vec<(&'static str, u8)> + Send>;
pub type ProbeI2cFn = Box<dyn FnMut(&[(u8, u8)]) -> Vec<ProbeResult> + Send>;
/// The closure the firmware injects to trigger a soft reset. It never
/// returns in practice — the chip reboots — but we type it as `-> ()`
/// to stay on stable Rust (the `!` type is still unstable as of
/// rustc 1.93). The BoardPort::invoke handler calls this and then
/// loops forever to match the "never returns" semantic.
pub type RebootFn = Box<dyn FnMut() + Send>;
pub type ConfigureFn = Box<dyn FnMut(&str, &str) -> Result<(), String> + Send>;

pub struct BoardPort {
    chip_info_fn: ChipInfoFn,
    pin_map_fn: PinMapFn,
    probe_i2c_fn: ProbeI2cFn,
    reboot_fn: RebootFn,
    configure_fn: ConfigureFn,
}

impl BoardPort {
    pub fn new(
        chip_info_fn: ChipInfoFn,
        pin_map_fn: PinMapFn,
        probe_i2c_fn: ProbeI2cFn,
        reboot_fn: RebootFn,
        configure_fn: ConfigureFn,
    ) -> Self {
        Self {
            chip_info_fn,
            pin_map_fn,
            probe_i2c_fn,
            reboot_fn,
            configure_fn,
        }
    }

    fn format_mac(mac: &[u8; 6]) -> String {
        alloc::format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        )
    }
}

impl SomaEspPort for BoardPort {
    fn port_id(&self) -> &'static str {
        "board"
    }

    fn primitives(&self) -> Vec<CapabilityDescriptor> {
        vec![
            CapabilityDescriptor {
                skill_id: "board.chip_info".to_string(),
                description: "Report chip model, MAC, free heap, uptime, firmware version"
                    .to_string(),
                input_schema: "{}".to_string(),
                output_schema: r#"{"chip":"str","mac":"str","free_heap":"u32","uptime_ms":"u64","firmware_version":"str"}"#
                    .to_string(),
                effect: Effect::ReadOnly,
            },
            CapabilityDescriptor {
                skill_id: "board.pin_map".to_string(),
                description:
                    "Report the current pin assignments for every peripheral on this chip"
                        .to_string(),
                input_schema: "{}".to_string(),
                output_schema: r#"{"pins":[{"key":"str","gpio":"u8"}]}"#.to_string(),
                effect: Effect::ReadOnly,
            },
            CapabilityDescriptor {
                skill_id: "board.configure_pin".to_string(),
                description:
                    "Persist a pin assignment to flash (e.g. 'pins.i2c0.sda'='5'). Takes effect on next boot."
                        .to_string(),
                input_schema: r#"{"key":"str","value":"str"}"#.to_string(),
                output_schema: r#"{"key":"str","value":"str","stored":"bool","reboot_required":"bool"}"#
                    .to_string(),
                effect: Effect::StateMutation,
            },
            CapabilityDescriptor {
                skill_id: "board.probe_i2c_buses".to_string(),
                description:
                    "Try each (sda, scl) pin pair, initialize I2C0 on it, scan for devices, report what was found. WARNING: destroys the current I2C state — call board.reboot after if i2c.* skills are in use."
                        .to_string(),
                input_schema: r#"{"candidates":"[[u8,u8]]"}"#.to_string(),
                output_schema: r#"{"probes":[{"sda":"u8","scl":"u8","addresses":"[u8]","error":"str?"}]}"#
                    .to_string(),
                effect: Effect::StateMutation,
            },
            CapabilityDescriptor {
                skill_id: "board.reboot".to_string(),
                description: "Soft-reset the chip. Used after board.configure_pin to apply new pin settings."
                    .to_string(),
                input_schema: "{}".to_string(),
                output_schema: r#"{"rebooting":"bool"}"#.to_string(),
                effect: Effect::ExternalEffect,
            },
        ]
    }

    fn invoke(&mut self, skill_id: &str, input: &Value) -> Result<Value, String> {
        match skill_id {
            "board.chip_info" => {
                let info = (self.chip_info_fn)();
                Ok(json!({
                    "chip": info.chip,
                    "mac": Self::format_mac(&info.mac),
                    "free_heap": info.free_heap,
                    "uptime_ms": info.uptime_ms,
                    "firmware_version": info.firmware_version,
                }))
            }
            "board.pin_map" => {
                let pins = (self.pin_map_fn)();
                let entries: Vec<Value> = pins
                    .into_iter()
                    .map(|(k, g)| json!({"key": k, "gpio": g}))
                    .collect();
                Ok(json!({ "pins": entries }))
            }
            "board.configure_pin" => {
                let key = input["key"]
                    .as_str()
                    .ok_or_else(|| "missing 'key'".to_string())?;
                let value = input["value"]
                    .as_str()
                    .ok_or_else(|| "missing 'value'".to_string())?;
                // Validate the key starts with "pins." — protects
                // against accidental overwrite of wifi.ssid etc.
                if !key.starts_with("pins.") {
                    return Err(
                        "key must start with 'pins.' (e.g. 'pins.i2c0.sda')".to_string(),
                    );
                }
                // Validate value parses as u8.
                if value.parse::<u8>().is_err() {
                    return Err(alloc::format!(
                        "value '{}' is not a valid GPIO number (0-255)",
                        value
                    ));
                }
                (self.configure_fn)(key, value)?;
                Ok(json!({
                    "key": key,
                    "value": value,
                    "stored": true,
                    "reboot_required": true,
                }))
            }
            "board.probe_i2c_buses" => {
                let candidates_json = input["candidates"]
                    .as_array()
                    .ok_or_else(|| "missing 'candidates' array".to_string())?;
                let mut candidates: Vec<(u8, u8)> = Vec::new();
                for entry in candidates_json {
                    let pair = entry
                        .as_array()
                        .ok_or_else(|| "each candidate must be [sda, scl]".to_string())?;
                    if pair.len() != 2 {
                        return Err("each candidate must have exactly 2 elements".to_string());
                    }
                    let sda = pair[0]
                        .as_u64()
                        .ok_or_else(|| "sda must be a u8".to_string())?
                        as u8;
                    let scl = pair[1]
                        .as_u64()
                        .ok_or_else(|| "scl must be a u8".to_string())?
                        as u8;
                    candidates.push((sda, scl));
                }
                let results = (self.probe_i2c_fn)(&candidates);
                let probes: Vec<Value> = results
                    .into_iter()
                    .map(|r| {
                        json!({
                            "sda": r.sda,
                            "scl": r.scl,
                            "addresses": r.addresses,
                            "error": r.error,
                        })
                    })
                    .collect();
                Ok(json!({ "probes": probes }))
            }
            "board.reboot" => {
                // The caller's TCP connection will drop mid-send as
                // soon as the chip halts. This is expected behavior
                // for a reboot skill.
                //
                // We trigger the reboot, then loop forever in case the
                // underlying closure was a no-op (which would be a
                // firmware bug). Under normal conditions the reboot
                // happens before the loop matters.
                (self.reboot_fn)();
                #[allow(clippy::empty_loop)]
                loop {
                    core::hint::spin_loop();
                }
            }
            _ => Err(alloc::format!("unknown primitive: {}", skill_id)),
        }
    }
}
