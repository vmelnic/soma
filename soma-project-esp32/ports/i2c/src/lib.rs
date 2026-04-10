// soma-esp32-port-i2c — I2C master primitives.
//
// Primitives:
//   i2c.write     { addr: u8, bytes: [u8] }              -> { written: u32 }
//   i2c.read      { addr: u8, len: u32 }                 -> { bytes: [u8] }
//   i2c.write_read{ addr: u8, write_bytes: [u8], read_len: u32 } -> { bytes: [u8] }
//   i2c.scan      {}                                     -> { addresses: [u8] }
//
// The port is generic over any type implementing `embedded_hal::i2c::I2c`.
// The firmware decides at boot what to pass in:
//
//   - Stand-alone: hand in a raw esp-hal `I2c<'static, Blocking>` when
//     nothing else needs the bus.
//
//   - Shared with the display port: hand in an
//     `embedded_hal_bus::i2c::RefCellDevice<'static, I2c<...>>` so each
//     consumer (i2c port, display port) locks the bus per-transaction.
//
// Both shapes satisfy the `embedded_hal::i2c::I2c` trait, so `I2cPort`
// doesn't care.

#![no_std]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use embedded_hal::i2c::I2c as EhI2c;
use serde_json::{json, Value};
use soma_esp32_leaf::{CapabilityDescriptor, Effect, SomaEspPort};

/// I²C port generic over the bus implementation.
///
/// `B` must implement `embedded_hal::i2c::I2c` with any concrete error
/// type — the port converts errors to strings via `{:?}`, so the caller
/// sees a chip-agnostic message.
pub struct I2cPort<B> {
    bus: B,
}

impl<B> I2cPort<B>
where
    B: EhI2c,
{
    pub fn new(bus: B) -> Self {
        Self { bus }
    }
}

impl<B> SomaEspPort for I2cPort<B>
where
    B: EhI2c,
    B::Error: core::fmt::Debug,
{
    fn port_id(&self) -> &'static str {
        "i2c"
    }

    fn primitives(&self) -> Vec<CapabilityDescriptor> {
        vec![
            CapabilityDescriptor {
                skill_id: "i2c.write".to_string(),
                description: "Write bytes to an I2C device address".to_string(),
                input_schema: r#"{"addr":"u8","bytes":"[u8]"}"#.to_string(),
                output_schema: r#"{"written":"u32"}"#.to_string(),
                effect: Effect::StateMutation,
            },
            CapabilityDescriptor {
                skill_id: "i2c.read".to_string(),
                description: "Read N bytes from an I2C device address".to_string(),
                input_schema: r#"{"addr":"u8","len":"u32"}"#.to_string(),
                output_schema: r#"{"bytes":"[u8]"}"#.to_string(),
                effect: Effect::ReadOnly,
            },
            CapabilityDescriptor {
                skill_id: "i2c.write_read".to_string(),
                description:
                    "Write bytes then read N bytes from an I2C device in a single transaction"
                        .to_string(),
                input_schema:
                    r#"{"addr":"u8","write_bytes":"[u8]","read_len":"u32"}"#.to_string(),
                output_schema: r#"{"bytes":"[u8]"}"#.to_string(),
                effect: Effect::StateMutation,
            },
            CapabilityDescriptor {
                skill_id: "i2c.scan".to_string(),
                description: "Scan the bus and return responding 7-bit addresses".to_string(),
                input_schema: "{}".to_string(),
                output_schema: r#"{"addresses":"[u8]"}"#.to_string(),
                effect: Effect::ReadOnly,
            },
        ]
    }

    fn invoke(&mut self, skill_id: &str, input: &Value) -> Result<Value, String> {
        match skill_id {
            "i2c.write" => {
                let addr = parse_u8(input, "addr")?;
                let bytes = parse_byte_array(input, "bytes")?;
                self.bus
                    .write(addr, &bytes)
                    .map_err(|e| alloc::format!("i2c write error: {:?}", e))?;
                Ok(json!({ "written": bytes.len() as u32 }))
            }
            "i2c.read" => {
                let addr = parse_u8(input, "addr")?;
                let len = input["len"]
                    .as_u64()
                    .ok_or_else(|| "missing 'len'".to_string())?
                    as usize;
                let cap = len.min(256);
                let mut buf = vec![0u8; cap];
                self.bus
                    .read(addr, &mut buf)
                    .map_err(|e| alloc::format!("i2c read error: {:?}", e))?;
                Ok(json!({ "bytes": bytes_to_json(&buf) }))
            }
            "i2c.write_read" => {
                let addr = parse_u8(input, "addr")?;
                let write_bytes = parse_byte_array(input, "write_bytes")?;
                let read_len = input["read_len"]
                    .as_u64()
                    .ok_or_else(|| "missing 'read_len'".to_string())?
                    as usize;
                let cap = read_len.min(256);
                let mut buf = vec![0u8; cap];
                self.bus
                    .write_read(addr, &write_bytes, &mut buf)
                    .map_err(|e| alloc::format!("i2c write_read error: {:?}", e))?;
                Ok(json!({ "bytes": bytes_to_json(&buf) }))
            }
            "i2c.scan" => {
                // Probe each 7-bit address with a zero-length write. Devices
                // that ACK their address respond; non-existent addresses error.
                let mut found: Vec<u8> = Vec::new();
                for addr in 0x08u8..=0x77 {
                    if self.bus.write(addr, &[]).is_ok() {
                        found.push(addr);
                    }
                }
                Ok(json!({ "addresses": bytes_to_json(&found) }))
            }
            _ => Err(alloc::format!("unknown primitive: {}", skill_id)),
        }
    }
}

fn parse_u8(input: &Value, key: &str) -> Result<u8, String> {
    let v = input[key]
        .as_u64()
        .ok_or_else(|| alloc::format!("missing '{}'", key))?;
    if v > 255 {
        return Err(alloc::format!("'{}' out of u8 range: {}", key, v));
    }
    Ok(v as u8)
}

fn parse_byte_array(input: &Value, key: &str) -> Result<Vec<u8>, String> {
    let arr = input[key]
        .as_array()
        .ok_or_else(|| alloc::format!("missing '{}' array", key))?;
    let mut out = Vec::with_capacity(arr.len());
    for v in arr {
        let b = v
            .as_u64()
            .ok_or_else(|| "byte value must be integer".to_string())?;
        if b > 255 {
            return Err(alloc::format!("byte out of range: {}", b));
        }
        out.push(b as u8);
    }
    Ok(out)
}

fn bytes_to_json(bytes: &[u8]) -> Vec<Value> {
    bytes.iter().map(|b| json!(*b)).collect()
}
