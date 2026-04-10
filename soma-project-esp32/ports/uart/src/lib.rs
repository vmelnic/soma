// soma-esp32-port-uart — UART primitives for ESP32-C3.
//
// Primitives:
//   uart.write { bytes: [u8] }    -> { written: u32 }
//   uart.read  { max_bytes: u32 } -> { bytes: [u8], read: u32 }
//
// The port owns a single Uart instance configured by the firmware at boot.
// Multiple UART ports can be registered if the firmware needs more than one
// (UART0 / UART1) — each becomes a separate port_id (uart0, uart1) by
// constructing multiple instances of UartPort with different identifiers.
//
// Note: ESP32-C3 has UART0 (typically used for the serial monitor — esp-println
// holds it) and UART1 (free for application use). The firmware should claim
// UART1 unless serial logging is disabled.

#![no_std]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use esp_hal::uart::Uart;
use esp_hal::Blocking;
use serde_json::{json, Value};
use soma_esp32_leaf::{CapabilityDescriptor, Effect, SomaEspPort};

pub struct UartPort<'d> {
    uart: Uart<'d, Blocking>,
}

impl<'d> UartPort<'d> {
    pub fn new(uart: Uart<'d, Blocking>) -> Self {
        Self { uart }
    }
}

impl<'d> SomaEspPort for UartPort<'d> {
    fn port_id(&self) -> &'static str {
        "uart"
    }

    fn primitives(&self) -> Vec<CapabilityDescriptor> {
        vec![
            CapabilityDescriptor {
                skill_id: "uart.write".to_string(),
                description: "Write bytes to the UART (esp-hal Uart::write_bytes)"
                    .to_string(),
                input_schema: r#"{"bytes":"[u8]"}"#.to_string(),
                output_schema: r#"{"written":"u32"}"#.to_string(),
                effect: Effect::ExternalEffect,
            },
            CapabilityDescriptor {
                skill_id: "uart.read".to_string(),
                description: "Read up to max_bytes from the UART RX buffer (non-blocking)"
                    .to_string(),
                input_schema: r#"{"max_bytes":"u32"}"#.to_string(),
                output_schema: r#"{"bytes":"[u8]","read":"u32"}"#.to_string(),
                effect: Effect::ReadOnly,
            },
        ]
    }

    fn invoke(&mut self, skill_id: &str, input: &Value) -> Result<Value, String> {
        match skill_id {
            "uart.write" => {
                let bytes_array = input["bytes"]
                    .as_array()
                    .ok_or_else(|| "missing 'bytes' array".to_string())?;
                let mut bytes: Vec<u8> = Vec::with_capacity(bytes_array.len());
                for v in bytes_array {
                    let b = v
                        .as_u64()
                        .ok_or_else(|| "byte value must be integer".to_string())?;
                    if b > 255 {
                        return Err(alloc::format!("byte value out of range: {}", b));
                    }
                    bytes.push(b as u8);
                }
                let written = self
                    .uart
                    .write_bytes(&bytes)
                    .map_err(|e| alloc::format!("uart write error: {:?}", e))?;
                Ok(json!({ "written": written as u32 }))
            }
            "uart.read" => {
                let max_bytes = input["max_bytes"]
                    .as_u64()
                    .ok_or_else(|| "missing 'max_bytes'".to_string())?
                    as usize;
                let cap = max_bytes.min(256); // bound to a sane size
                let mut buf = vec![0u8; cap];
                let read = self
                    .uart
                    .read_buffered_bytes(&mut buf)
                    .map_err(|e| alloc::format!("uart read error: {:?}", e))?;
                buf.truncate(read);
                let json_bytes: Vec<Value> = buf.iter().map(|b| json!(*b)).collect();
                Ok(json!({ "bytes": json_bytes, "read": read as u32 }))
            }
            _ => Err(alloc::format!("unknown primitive: {}", skill_id)),
        }
    }
}
