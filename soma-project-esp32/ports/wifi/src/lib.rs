// soma-esp32-port-wifi — WiFi configuration and status primitives.
//
// Primitives:
//   wifi.scan        { }                              -> { networks: [...] }
//   wifi.configure   { ssid, password }               -> { stored: bool, ssid }
//   wifi.status      { }                              -> { connected, ssid, ip, rssi }
//   wifi.disconnect  { }                              -> { ok: bool }
//   wifi.forget      { }                              -> { ok: bool }    (clears NVS creds)
//
// This port is type-erased and does NOT depend on esp-wifi. The firmware
// implements WifiOps with the real radio code and passes it in. The port
// just routes primitive calls through the trait.

#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use soma_esp32_leaf::{CapabilityDescriptor, Effect, SomaEspPort};

/// One nearby WiFi network discovered by wifi.scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiNetwork {
    pub ssid: String,
    pub rssi: i32,
    pub security: String,
    pub channel: u8,
}

/// Current WiFi connection state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiState {
    pub connected: bool,
    pub ssid: Option<String>,
    pub ip: Option<String>,
    pub rssi: Option<i32>,
    pub mac: Option<String>,
}

/// Errors from the WiFi backend.
#[derive(Debug, Clone)]
pub enum WifiError {
    NotInitialized,
    AuthFailed,
    NoApFound,
    HardwareError(String),
    StorageError(String),
}

/// Trait the firmware implements with the real esp-wifi-backed code.
/// The port crate stores a Box<dyn WifiOps> and never imports esp-wifi.
pub trait WifiOps {
    fn scan(&mut self) -> Result<Vec<WifiNetwork>, WifiError>;
    /// Store credentials in NVS and reboot. Returning Ok means the
    /// credentials were persisted; the device may not have rebooted yet.
    fn configure(&mut self, ssid: &str, password: &str) -> Result<(), WifiError>;
    fn status(&self) -> Result<WifiState, WifiError>;
    fn disconnect(&mut self) -> Result<(), WifiError>;
    fn forget(&mut self) -> Result<(), WifiError>;
}

pub struct WifiPort {
    ops: Box<dyn WifiOps>,
}

impl WifiPort {
    pub fn new(ops: Box<dyn WifiOps>) -> Self {
        Self { ops }
    }
}

impl SomaEspPort for WifiPort {
    fn port_id(&self) -> &'static str {
        "wifi"
    }

    fn primitives(&self) -> Vec<CapabilityDescriptor> {
        vec![
            CapabilityDescriptor {
                skill_id: "wifi.scan".to_string(),
                description:
                    "Scan for nearby WiFi networks. Returns SSID, RSSI, security, channel."
                        .to_string(),
                input_schema: "{}".to_string(),
                output_schema:
                    r#"{"networks":[{"ssid":"str","rssi":"i32","security":"str","channel":"u8"}]}"#
                        .to_string(),
                effect: Effect::ReadOnly,
            },
            CapabilityDescriptor {
                skill_id: "wifi.configure".to_string(),
                description:
                    "Store WiFi credentials in NVS. Device reboots after storage so the new \
                     config takes effect on next boot."
                        .to_string(),
                input_schema: r#"{"ssid":"str","password":"str"}"#.to_string(),
                output_schema: r#"{"stored":"bool","ssid":"str"}"#.to_string(),
                effect: Effect::StateMutation,
            },
            CapabilityDescriptor {
                skill_id: "wifi.status".to_string(),
                description:
                    "Report current WiFi connection state: SSID, IP, RSSI, MAC, connected flag"
                        .to_string(),
                input_schema: "{}".to_string(),
                output_schema:
                    r#"{"connected":"bool","ssid":"str?","ip":"str?","rssi":"i32?","mac":"str?"}"#
                        .to_string(),
                effect: Effect::ReadOnly,
            },
            CapabilityDescriptor {
                skill_id: "wifi.disconnect".to_string(),
                description:
                    "Disconnect from the current AP. Credentials remain stored in NVS."
                        .to_string(),
                input_schema: "{}".to_string(),
                output_schema: r#"{"ok":"bool"}"#.to_string(),
                effect: Effect::StateMutation,
            },
            CapabilityDescriptor {
                skill_id: "wifi.forget".to_string(),
                description: "Clear stored WiFi credentials from NVS and disconnect"
                    .to_string(),
                input_schema: "{}".to_string(),
                output_schema: r#"{"ok":"bool"}"#.to_string(),
                effect: Effect::StateMutation,
            },
        ]
    }

    fn invoke(&mut self, skill_id: &str, input: &Value) -> Result<Value, String> {
        match skill_id {
            "wifi.scan" => {
                let networks = self.ops.scan().map_err(map_err)?;
                let json_networks: Vec<Value> = networks
                    .iter()
                    .map(|n| {
                        json!({
                            "ssid": n.ssid,
                            "rssi": n.rssi,
                            "security": n.security,
                            "channel": n.channel,
                        })
                    })
                    .collect();
                Ok(json!({ "networks": json_networks }))
            }
            "wifi.configure" => {
                let ssid = input["ssid"]
                    .as_str()
                    .ok_or_else(|| "missing 'ssid'".to_string())?;
                let password = input["password"]
                    .as_str()
                    .ok_or_else(|| "missing 'password'".to_string())?;
                self.ops
                    .configure(ssid, password)
                    .map_err(map_err)?;
                Ok(json!({ "stored": true, "ssid": ssid }))
            }
            "wifi.status" => {
                let state = self.ops.status().map_err(map_err)?;
                Ok(json!({
                    "connected": state.connected,
                    "ssid": state.ssid,
                    "ip": state.ip,
                    "rssi": state.rssi,
                    "mac": state.mac,
                }))
            }
            "wifi.disconnect" => {
                self.ops.disconnect().map_err(map_err)?;
                Ok(json!({ "ok": true }))
            }
            "wifi.forget" => {
                self.ops.forget().map_err(map_err)?;
                Ok(json!({ "ok": true }))
            }
            _ => Err(alloc::format!("unknown primitive: {}", skill_id)),
        }
    }
}

fn map_err(e: WifiError) -> String {
    match e {
        WifiError::NotInitialized => "wifi not initialized".to_string(),
        WifiError::AuthFailed => "wifi authentication failed".to_string(),
        WifiError::NoApFound => "wifi access point not found".to_string(),
        WifiError::HardwareError(s) => alloc::format!("wifi hardware error: {}", s),
        WifiError::StorageError(s) => alloc::format!("wifi storage error: {}", s),
    }
}
