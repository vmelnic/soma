// soma-esp32-port-adc — ADC primitives for ESP32-C3.
//
// Primitives:
//   adc.read         { channel: u32 } -> { channel: u32, raw: u32 }
//   adc.read_voltage { channel: u32 } -> { channel: u32, mv: u32 }
//
// ADC reads in esp-hal use typed AdcPin<PIN, ADCI> handles, which makes
// storing multiple pins of different types in a single port impractical.
// Instead, this port takes a closure at construction time that performs
// the actual ADC read. The firmware captures the Adc + AdcPin in the
// closure and the port stays type-erased.
//
// To support multiple ADC channels, the firmware can register multiple
// AdcPort instances each with a different `channel_id` and closure. They
// can then be exposed via the composite dispatcher under names like
// adc.read (first registered) or via separate port_ids.

#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use serde_json::{json, Value};
use soma_esp32_leaf::{CapabilityDescriptor, Effect, SomaEspPort};

/// Read function provided by the firmware. Closes over an esp-hal Adc and
/// AdcPin to actually perform the read. Returns the raw 12-bit ADC value.
pub type AdcReadFn = Box<dyn FnMut() -> Result<u16, AdcError>>;

#[derive(Debug, Clone, Copy)]
pub enum AdcError {
    HardwareError,
}

pub struct AdcPort {
    /// Logical channel identifier the brain references in routine inputs.
    channel_id: u32,
    /// Closure that reads the ADC. Captures the Adc and AdcPin from the
    /// firmware so this struct stays type-erased.
    read_fn: AdcReadFn,
    /// ADC reference voltage in millivolts (typically 3300 with 11dB attenuation).
    /// Used by adc.read_voltage to convert raw to mV.
    vref_mv: u32,
    /// ADC resolution in bits (12 for ESP32-C3 ADC1).
    resolution_bits: u32,
}

impl AdcPort {
    pub fn new(channel_id: u32, read_fn: AdcReadFn) -> Self {
        Self {
            channel_id,
            read_fn,
            vref_mv: 3300,
            resolution_bits: 12,
        }
    }

    pub fn with_vref(mut self, vref_mv: u32) -> Self {
        self.vref_mv = vref_mv;
        self
    }
}

impl SomaEspPort for AdcPort {
    fn port_id(&self) -> &'static str {
        "adc"
    }

    fn primitives(&self) -> Vec<CapabilityDescriptor> {
        vec![
            CapabilityDescriptor {
                skill_id: "adc.read".to_string(),
                description: "Read raw 12-bit ADC value from the configured channel"
                    .to_string(),
                input_schema: r#"{"channel":"u32"}"#.to_string(),
                output_schema: r#"{"channel":"u32","raw":"u32"}"#.to_string(),
                effect: Effect::ReadOnly,
            },
            CapabilityDescriptor {
                skill_id: "adc.read_voltage".to_string(),
                description:
                    "Read ADC value converted to millivolts using configured Vref".to_string(),
                input_schema: r#"{"channel":"u32"}"#.to_string(),
                output_schema: r#"{"channel":"u32","mv":"u32"}"#.to_string(),
                effect: Effect::ReadOnly,
            },
        ]
    }

    fn invoke(&mut self, skill_id: &str, _input: &Value) -> Result<Value, String> {
        match skill_id {
            "adc.read" => {
                let raw = (self.read_fn)()
                    .map_err(|e| alloc::format!("adc read error: {:?}", e))?;
                Ok(json!({ "channel": self.channel_id, "raw": raw as u32 }))
            }
            "adc.read_voltage" => {
                let raw = (self.read_fn)()
                    .map_err(|e| alloc::format!("adc read error: {:?}", e))?;
                // Convert raw to mV: mv = (raw / max_raw) * vref_mv
                let max_raw = (1u32 << self.resolution_bits) - 1;
                let mv = (raw as u32 * self.vref_mv) / max_raw;
                Ok(json!({ "channel": self.channel_id, "mv": mv }))
            }
            _ => Err(alloc::format!("unknown primitive: {}", skill_id)),
        }
    }
}
