// soma-esp32-port-pwm — PWM (LEDC) primitives for ESP32-C3.
//
// Primitives:
//   pwm.set_duty { duty_percent: u32 } -> { duty_percent: u32 }
//
// LEDC channels in esp-hal have lifetimes tied to the LEDC instance and the
// associated Timer. To keep this port type-erased, the firmware constructs
// and configures everything (Ledc, Timer, Channel) at boot, then provides a
// closure that performs the duty update. The port stores the closure.
//
// Frequency is fixed at boot via the Timer config — runtime frequency
// changes would require borrowing the Timer mutably and reconfiguring,
// which conflicts with Channel borrows. To change PWM frequency at
// runtime, reflash with a different Timer config.

#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use serde_json::{json, Value};
use soma_esp32_leaf::{CapabilityDescriptor, Effect, SomaEspPort};

/// Set-duty function provided by the firmware. Closes over an esp-hal
/// LEDC Channel to perform the actual duty update.
pub type PwmSetDutyFn = Box<dyn FnMut(u8) -> Result<(), PwmError>>;

#[derive(Debug, Clone, Copy)]
pub enum PwmError {
    InvalidDuty,
    HardwareError,
}

pub struct PwmPort {
    channel_id: u32,
    set_duty_fn: PwmSetDutyFn,
    /// Currently configured duty cycle (0-100). Last successfully set value.
    current_duty: u8,
    /// Fixed frequency in Hz set at firmware boot.
    fixed_frequency_hz: u32,
}

impl PwmPort {
    pub fn new(channel_id: u32, fixed_frequency_hz: u32, set_duty_fn: PwmSetDutyFn) -> Self {
        Self {
            channel_id,
            set_duty_fn,
            current_duty: 0,
            fixed_frequency_hz,
        }
    }
}

impl SomaEspPort for PwmPort {
    fn port_id(&self) -> &'static str {
        "pwm"
    }

    fn primitives(&self) -> Vec<CapabilityDescriptor> {
        vec![
            CapabilityDescriptor {
                skill_id: "pwm.set_duty".to_string(),
                description:
                    "Set the duty cycle (0-100%) of the configured LEDC channel"
                        .to_string(),
                input_schema: r#"{"duty_percent":"u32"}"#.to_string(),
                output_schema: r#"{"channel":"u32","duty_percent":"u32"}"#.to_string(),
                effect: Effect::StateMutation,
            },
            CapabilityDescriptor {
                skill_id: "pwm.get_status".to_string(),
                description: "Report current PWM duty and the fixed frequency".to_string(),
                input_schema: "{}".to_string(),
                output_schema:
                    r#"{"channel":"u32","duty_percent":"u32","frequency_hz":"u32"}"#
                        .to_string(),
                effect: Effect::ReadOnly,
            },
        ]
    }

    fn invoke(&mut self, skill_id: &str, input: &Value) -> Result<Value, String> {
        match skill_id {
            "pwm.set_duty" => {
                let duty = input["duty_percent"]
                    .as_u64()
                    .ok_or_else(|| "missing 'duty_percent'".to_string())?;
                if duty > 100 {
                    return Err(alloc::format!(
                        "duty_percent {} out of range 0-100",
                        duty
                    ));
                }
                let duty_u8 = duty as u8;
                (self.set_duty_fn)(duty_u8)
                    .map_err(|e| alloc::format!("pwm set_duty error: {:?}", e))?;
                self.current_duty = duty_u8;
                Ok(json!({
                    "channel": self.channel_id,
                    "duty_percent": duty_u8 as u32,
                }))
            }
            "pwm.get_status" => Ok(json!({
                "channel": self.channel_id,
                "duty_percent": self.current_duty as u32,
                "frequency_hz": self.fixed_frequency_hz,
            })),
            _ => Err(alloc::format!("unknown primitive: {}", skill_id)),
        }
    }
}
