// soma-esp32-port-gpio — GPIO primitives for ESP32-C3.
//
// Primitives:
//   gpio.write   { pin, value }   -> { pin, value }
//   gpio.read    { pin }          -> { pin, value }    (last-known logical state)
//   gpio.toggle  { pin }          -> { pin, value }
//
// All primitives are real, backed by esp-hal Output handles claimed by the
// firmware at boot. The port maintains a BTreeMap of claimed pins; each pin
// number the brain references must have been claimed via claim_output_pin.
//
// Pin direction: this port currently exposes only Output pins. Input/pull-up
// and dynamic reconfiguration would require additional hardware-claim methods.

#![no_std]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use esp_hal::gpio::Output;
use serde_json::{json, Value};
use soma_esp32_leaf::{CapabilityDescriptor, Effect, SomaEspPort};

pub struct GpioPort<'d> {
    output_pins: BTreeMap<u32, Output<'d>>,
    pin_state: BTreeMap<u32, bool>,
}

impl<'d> GpioPort<'d> {
    pub fn new() -> Self {
        Self {
            output_pins: BTreeMap::new(),
            pin_state: BTreeMap::new(),
        }
    }

    /// Register an esp-hal Output pin built by the firmware. `pin_number`
    /// is what the brain references in routine inputs (e.g. 7 for GPIO7).
    pub fn claim_output_pin(&mut self, pin_number: u32, output: Output<'d>) {
        self.output_pins.insert(pin_number, output);
        self.pin_state.insert(pin_number, false);
    }
}

impl<'d> Default for GpioPort<'d> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'d> SomaEspPort for GpioPort<'d> {
    fn port_id(&self) -> &'static str {
        "gpio"
    }

    fn primitives(&self) -> Vec<CapabilityDescriptor> {
        vec![
            CapabilityDescriptor {
                skill_id: "gpio.write".to_string(),
                description: "Set a claimed GPIO pin high or low".to_string(),
                input_schema: r#"{"pin":"u32","value":"bool"}"#.to_string(),
                output_schema: r#"{"pin":"u32","value":"bool"}"#.to_string(),
                effect: Effect::StateMutation,
            },
            CapabilityDescriptor {
                skill_id: "gpio.read".to_string(),
                description: "Read the last-known logical state of a claimed GPIO pin"
                    .to_string(),
                input_schema: r#"{"pin":"u32"}"#.to_string(),
                output_schema: r#"{"pin":"u32","value":"bool"}"#.to_string(),
                effect: Effect::ReadOnly,
            },
            CapabilityDescriptor {
                skill_id: "gpio.toggle".to_string(),
                description: "Flip the state of a claimed GPIO pin".to_string(),
                input_schema: r#"{"pin":"u32"}"#.to_string(),
                output_schema: r#"{"pin":"u32","value":"bool"}"#.to_string(),
                effect: Effect::StateMutation,
            },
        ]
    }

    fn invoke(&mut self, skill_id: &str, input: &Value) -> Result<Value, String> {
        match skill_id {
            "gpio.write" => {
                let pin = input["pin"].as_u64().ok_or("missing 'pin'")? as u32;
                let value = input["value"].as_bool().ok_or("missing 'value'")?;
                let p = self
                    .output_pins
                    .get_mut(&pin)
                    .ok_or_else(|| alloc::format!("pin {} not claimed", pin))?;
                if value {
                    p.set_high();
                } else {
                    p.set_low();
                }
                self.pin_state.insert(pin, value);
                Ok(json!({ "pin": pin, "value": value }))
            }
            "gpio.read" => {
                let pin = input["pin"].as_u64().ok_or("missing 'pin'")? as u32;
                let value = self.pin_state.get(&pin).copied().unwrap_or(false);
                Ok(json!({ "pin": pin, "value": value }))
            }
            "gpio.toggle" => {
                let pin = input["pin"].as_u64().ok_or("missing 'pin'")? as u32;
                let new_value = !self.pin_state.get(&pin).copied().unwrap_or(false);
                let p = self
                    .output_pins
                    .get_mut(&pin)
                    .ok_or_else(|| alloc::format!("pin {} not claimed", pin))?;
                if new_value {
                    p.set_high();
                } else {
                    p.set_low();
                }
                self.pin_state.insert(pin, new_value);
                Ok(json!({ "pin": pin, "value": new_value }))
            }
            _ => Err(alloc::format!("unknown primitive: {}", skill_id)),
        }
    }
}
