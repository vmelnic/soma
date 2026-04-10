// soma-esp32-port-thermistor — example sensor port.
//
// Demonstrates the SOMA port pattern for adding new sensors. Each new
// sensor model is its own crate with this exact shape: a small no_std lib
// that exposes a SomaEspPort impl. The firmware Cargo.toml gates inclusion
// behind a cargo feature flag.
//
// Real sensor ports (DHT22, BME280, MPU6050, ...) follow the same shape.
// Production thermistor reading would use the core port's adc.read primitive
// (or take an esp-hal AdcPin handle at construction). This crate stubs the
// reading with simulated data so it builds without claiming hardware.

#![no_std]

extern crate alloc;

use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

use serde_json::{json, Value};
use soma_esp32_leaf::{CapabilityDescriptor, Effect, SomaEspPort};

pub struct ThermistorPort {
    /// Simulated temperature drift state. Production would query real ADC.
    last_reading_c: f64,
}

impl ThermistorPort {
    pub fn new() -> Self {
        Self {
            last_reading_c: 20.0,
        }
    }
}

impl Default for ThermistorPort {
    fn default() -> Self {
        Self::new()
    }
}

impl SomaEspPort for ThermistorPort {
    fn port_id(&self) -> &'static str {
        "thermistor"
    }

    fn primitives(&self) -> Vec<CapabilityDescriptor> {
        vec![
            CapabilityDescriptor {
                skill_id: "thermistor.read_temp".to_string(),
                description: "Read temperature in Celsius from a thermistor on a given ADC channel"
                    .to_string(),
                input_schema: r#"{"channel":"u32"}"#.to_string(),
                output_schema: r#"{"channel":"u32","temp_c":"f64"}"#.to_string(),
                effect: Effect::ReadOnly,
            },
            CapabilityDescriptor {
                skill_id: "thermistor.read_temp_calibrated".to_string(),
                description:
                    "Read temperature with optional offset/scale calibration parameters"
                        .to_string(),
                input_schema: r#"{"channel":"u32","offset_c":"f64","scale":"f64"}"#.to_string(),
                output_schema: r#"{"channel":"u32","temp_c":"f64","raw_c":"f64"}"#.to_string(),
                effect: Effect::ReadOnly,
            },
        ]
    }

    fn invoke(&mut self, skill_id: &str, input: &Value) -> Result<Value, alloc::string::String> {
        match skill_id {
            "thermistor.read_temp" => {
                let channel = input["channel"]
                    .as_u64()
                    .ok_or_else(|| "missing 'channel'".to_string())?;
                self.last_reading_c += 0.25;
                if self.last_reading_c > 30.0 {
                    self.last_reading_c = 20.0;
                }
                Ok(json!({ "channel": channel, "temp_c": self.last_reading_c }))
            }
            "thermistor.read_temp_calibrated" => {
                let channel = input["channel"]
                    .as_u64()
                    .ok_or_else(|| "missing 'channel'".to_string())?;
                let offset = input["offset_c"].as_f64().unwrap_or(0.0);
                let scale = input["scale"].as_f64().unwrap_or(1.0);
                let raw = self.last_reading_c;
                let calibrated = raw * scale + offset;
                Ok(
                    json!({ "channel": channel, "temp_c": calibrated, "raw_c": raw }),
                )
            }
            _ => Err(alloc::format!("unknown primitive: {}", skill_id)),
        }
    }
}
