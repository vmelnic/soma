// soma-esp32-port-delay — blocking delay primitives for ESP32-C3.
//
// Primitives:
//   delay.ms { ms } -> { slept_ms }
//   delay.us { us } -> { slept_us }
//
// Wraps esp-hal's blocking Delay. No peripheral claiming required — Delay
// is built from CPU clock state.

#![no_std]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use esp_hal::delay::Delay;
use serde_json::{json, Value};
use soma_esp32_leaf::{CapabilityDescriptor, Effect, SomaEspPort};

pub struct DelayPort {
    delay: Delay,
}

impl DelayPort {
    pub fn new() -> Self {
        Self {
            delay: Delay::new(),
        }
    }
}

impl Default for DelayPort {
    fn default() -> Self {
        Self::new()
    }
}

impl SomaEspPort for DelayPort {
    fn port_id(&self) -> &'static str {
        "delay"
    }

    fn primitives(&self) -> Vec<CapabilityDescriptor> {
        vec![
            CapabilityDescriptor {
                skill_id: "delay.ms".to_string(),
                description: "Block for N milliseconds (esp-hal Delay::delay_millis)"
                    .to_string(),
                input_schema: r#"{"ms":"u32"}"#.to_string(),
                output_schema: r#"{"slept_ms":"u32"}"#.to_string(),
                effect: Effect::ReadOnly,
            },
            CapabilityDescriptor {
                skill_id: "delay.us".to_string(),
                description: "Block for N microseconds (esp-hal Delay::delay_micros)"
                    .to_string(),
                input_schema: r#"{"us":"u32"}"#.to_string(),
                output_schema: r#"{"slept_us":"u32"}"#.to_string(),
                effect: Effect::ReadOnly,
            },
        ]
    }

    fn invoke(&mut self, skill_id: &str, input: &Value) -> Result<Value, String> {
        match skill_id {
            "delay.ms" => {
                let ms = input["ms"].as_u64().ok_or("missing 'ms'")? as u32;
                self.delay.delay_millis(ms);
                Ok(json!({ "slept_ms": ms }))
            }
            "delay.us" => {
                let us = input["us"].as_u64().ok_or("missing 'us'")? as u32;
                self.delay.delay_micros(us);
                Ok(json!({ "slept_us": us }))
            }
            _ => Err(alloc::format!("unknown primitive: {}", skill_id)),
        }
    }
}
