//! Synaptic Protocol signal types (Spec Section 14).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SignalType {
    Ping,
    Pong,
    Discover,
    DiscoverAck,
    Handshake,
    Close,
    Intent,
    Data,
    Result,
    Error,
    StreamStart,
    StreamData,
    StreamEnd,
    Subscribe,
    Unsubscribe,
    Control,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    pub signal_type: SignalType,
    pub sender: String,
    pub recipient: String,
    pub sequence: u64,
    pub channel_id: u32,
    pub payload: Vec<u8>,
    pub timestamp: u64,
}

impl Signal {
    /// Create a new signal with the given type and sender.
    pub fn new(signal_type: SignalType, sender: String, recipient: String) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            signal_type,
            sender,
            recipient,
            sequence: 0,
            channel_id: 0,
            payload: Vec::new(),
            timestamp: now,
        }
    }

    /// Create a Ping signal.
    pub fn ping(sender: &str) -> Self {
        Self::new(SignalType::Ping, sender.to_string(), String::new())
    }

    /// Create a Pong response.
    pub fn pong(sender: &str, recipient: &str) -> Self {
        Self::new(SignalType::Pong, sender.to_string(), recipient.to_string())
    }

    /// Serialize to length-prefixed bytes for wire transmission.
    pub fn to_bytes(&self) -> Vec<u8> {
        let json = serde_json::to_vec(self).unwrap_or_default();
        let len = json.len() as u32;
        let mut buf = len.to_be_bytes().to_vec();
        buf.extend(json);
        buf
    }

    /// Deserialize from JSON bytes.
    pub fn from_bytes(data: &[u8]) -> anyhow::Result<Self> {
        let signal: Signal = serde_json::from_slice(data)?;
        Ok(signal)
    }
}
