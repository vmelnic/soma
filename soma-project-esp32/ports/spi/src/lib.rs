// soma-esp32-port-spi — SPI master primitives for ESP32-C3.
//
// Primitives:
//   spi.write    { bytes: [u8] }    -> { written: u32 }
//   spi.read     { len: u32 }       -> { bytes: [u8] }
//   spi.transfer { tx_bytes: [u8] } -> { rx_bytes: [u8] }
//
// The port owns a single esp-hal Spi<Blocking> master configured by the
// firmware at boot with SCK, MOSI, MISO pins. Multiple SPI buses become
// multiple SpiPort instances with different port_ids.

#![no_std]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use esp_hal::spi::master::Spi;
use esp_hal::Blocking;
use serde_json::{json, Value};
use soma_esp32_leaf::{CapabilityDescriptor, Effect, SomaEspPort};

pub struct SpiPort<'d> {
    spi: Spi<'d, Blocking>,
}

impl<'d> SpiPort<'d> {
    pub fn new(spi: Spi<'d, Blocking>) -> Self {
        Self { spi }
    }
}

impl<'d> SomaEspPort for SpiPort<'d> {
    fn port_id(&self) -> &'static str {
        "spi"
    }

    fn primitives(&self) -> Vec<CapabilityDescriptor> {
        vec![
            CapabilityDescriptor {
                skill_id: "spi.write".to_string(),
                description: "Write bytes to the SPI bus (TX only)".to_string(),
                input_schema: r#"{"bytes":"[u8]"}"#.to_string(),
                output_schema: r#"{"written":"u32"}"#.to_string(),
                effect: Effect::StateMutation,
            },
            CapabilityDescriptor {
                skill_id: "spi.read".to_string(),
                description: "Read N bytes from the SPI bus (RX only)".to_string(),
                input_schema: r#"{"len":"u32"}"#.to_string(),
                output_schema: r#"{"bytes":"[u8]"}"#.to_string(),
                effect: Effect::ReadOnly,
            },
            CapabilityDescriptor {
                skill_id: "spi.transfer".to_string(),
                description:
                    "Full-duplex SPI transfer: send tx_bytes and receive the same number"
                        .to_string(),
                input_schema: r#"{"tx_bytes":"[u8]"}"#.to_string(),
                output_schema: r#"{"rx_bytes":"[u8]"}"#.to_string(),
                effect: Effect::StateMutation,
            },
        ]
    }

    fn invoke(&mut self, skill_id: &str, input: &Value) -> Result<Value, String> {
        match skill_id {
            "spi.write" => {
                let bytes = parse_byte_array(input, "bytes")?;
                self.spi
                    .write_bytes(&bytes)
                    .map_err(|e| alloc::format!("spi write error: {:?}", e))?;
                Ok(json!({ "written": bytes.len() as u32 }))
            }
            "spi.read" => {
                let len = input["len"]
                    .as_u64()
                    .ok_or_else(|| "missing 'len'".to_string())?
                    as usize;
                let cap = len.min(256);
                let mut buf = vec![0u8; cap];
                self.spi
                    .read_bytes(&mut buf)
                    .map_err(|e| alloc::format!("spi read error: {:?}", e))?;
                Ok(json!({ "bytes": bytes_to_json(&buf) }))
            }
            "spi.transfer" => {
                let tx_bytes = parse_byte_array(input, "tx_bytes")?;
                let mut buf = tx_bytes.clone();
                let rx = self
                    .spi
                    .transfer(&mut buf)
                    .map_err(|e| alloc::format!("spi transfer error: {:?}", e))?;
                Ok(json!({ "rx_bytes": bytes_to_json(rx) }))
            }
            _ => Err(alloc::format!("unknown primitive: {}", skill_id)),
        }
    }
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
