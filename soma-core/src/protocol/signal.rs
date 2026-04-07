//! Synaptic Protocol v2 signal types (Spec Sections 4.2, 4.3).

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

/// All signal types defined by the Synaptic Protocol v2 spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SignalType {
    // Connection lifecycle
    Handshake = 0x01,
    HandshakeAck = 0x02,
    Close = 0x03,

    // Intent / result
    Intent = 0x10,
    Result = 0x11,

    // Data transfer
    Data = 0x20,
    Binary = 0x21,
    StreamStart = 0x22,
    StreamData = 0x23,
    StreamEnd = 0x24,

    // Chunked transfer
    ChunkStart = 0x30,
    ChunkData = 0x31,
    ChunkEnd = 0x32,
    ChunkAck = 0x33,

    // Discovery
    Discover = 0x40,
    DiscoverAck = 0x41,
    PeerQuery = 0x42,
    PeerList = 0x43,

    // Pub/sub
    Subscribe = 0x50,
    Unsubscribe = 0x51,

    // Keepalive
    Ping = 0xF0,
    Pong = 0xF1,

    // Protocol-level
    Error = 0xFE,
    Control = 0xFF,
}

impl SignalType {
    /// Convert a raw byte to a SignalType, returning None for unknown types.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Self::Handshake),
            0x02 => Some(Self::HandshakeAck),
            0x03 => Some(Self::Close),
            0x10 => Some(Self::Intent),
            0x11 => Some(Self::Result),
            0x20 => Some(Self::Data),
            0x21 => Some(Self::Binary),
            0x22 => Some(Self::StreamStart),
            0x23 => Some(Self::StreamData),
            0x24 => Some(Self::StreamEnd),
            0x30 => Some(Self::ChunkStart),
            0x31 => Some(Self::ChunkData),
            0x32 => Some(Self::ChunkEnd),
            0x33 => Some(Self::ChunkAck),
            0x40 => Some(Self::Discover),
            0x41 => Some(Self::DiscoverAck),
            0x42 => Some(Self::PeerQuery),
            0x43 => Some(Self::PeerList),
            0x50 => Some(Self::Subscribe),
            0x51 => Some(Self::Unsubscribe),
            0xF0 => Some(Self::Ping),
            0xF1 => Some(Self::Pong),
            0xFE => Some(Self::Error),
            0xFF => Some(Self::Control),
            _ => None,
        }
    }

    /// Convert to the wire byte representation.
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// Whether this signal type is a control signal (must use channel 0).
    pub fn is_control(&self) -> bool {
        matches!(self, Self::Handshake | Self::HandshakeAck | Self::Close |
                 Self::Ping | Self::Pong | Self::Error | Self::Control |
                 Self::Discover | Self::DiscoverAck | Self::PeerQuery | Self::PeerList)
    }
}

bitflags! {
    /// Signal flags byte (Spec Section 4.2).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct SignalFlags: u8 {
        const COMPRESSED    = 0b0000_0001;
        const ENCRYPTED     = 0b0000_0010;
        const CHUNKED       = 0b0000_0100;
        const FINAL_CHUNK   = 0b0000_1000;
        const ACK_REQUESTED = 0b0001_0000;
        const PRIORITY      = 0b0010_0000;
    }
}

// Manual Serialize/Deserialize for SignalFlags since bitflags doesn't
// support derive(Serialize, Deserialize) on its internal type.
impl Serialize for SignalFlags {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_u8(self.bits())
    }
}

impl<'de> Deserialize<'de> for SignalFlags {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let bits = u8::deserialize(deserializer)?;
        Ok(SignalFlags::from_bits_truncate(bits))
    }
}

/// A Synaptic Protocol v2 signal. The fundamental unit of communication
/// between SOMAs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    pub signal_type: SignalType,
    pub flags: SignalFlags,
    pub channel_id: u32,
    pub sequence: u32,
    pub sender_id: String,
    pub metadata: serde_json::Value,
    pub payload: Vec<u8>,
    /// Trace ID for distributed tracing. Stored in metadata on the wire,
    /// but kept as a top-level field for convenience.
    #[serde(default)]
    pub trace_id: String,
}

impl Signal {
    /// Create a new signal with sensible defaults.
    pub fn new(signal_type: SignalType, sender_id: String) -> Self {
        let trace_id = uuid::Uuid::new_v4().to_string()[..12].to_string();
        Self {
            signal_type,
            flags: SignalFlags::empty(),
            channel_id: 0,
            sequence: 0,
            sender_id,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
            payload: Vec::new(),
            trace_id,
        }
    }

    /// Create a Ping signal.
    pub fn ping(sender: &str) -> Self {
        Self::new(SignalType::Ping, sender.to_string())
    }

    /// Create a Pong response matching a Ping sequence.
    pub fn pong(sender: &str, sequence: u32) -> Self {
        let mut s = Self::new(SignalType::Pong, sender.to_string());
        s.sequence = sequence;
        s
    }

    /// Create an Error signal with a reason string in the payload.
    pub fn error(sender: &str, reason: &str) -> Self {
        let mut s = Self::new(SignalType::Error, sender.to_string());
        s.payload = reason.as_bytes().to_vec();
        s
    }

    /// Create a Handshake signal with negotiation metadata.
    /// Includes a session_token for reconnect identification (Spec Section 14.5).
    pub fn handshake(soma_id: &str, capabilities: &[&str], plugins: &[&str]) -> Self {
        let mut s = Self::new(SignalType::Handshake, soma_id.to_string());
        s.channel_id = 0; // control channel
        let session_token = uuid::Uuid::new_v4().to_string();
        let meta = serde_json::json!({
            "protocol_version": "2.0",
            "supported_versions": ["2.0"],
            "soma_id": soma_id,
            "soma_core_version": "0.1.0",
            "capabilities": capabilities,
            "plugins": plugins,
            "max_signal_size": 10_485_760u32,
            "max_channels": 256u32,
            "session_token": session_token,
        });
        s.metadata = meta;
        s
    }

    /// Create a HandshakeAck signal.
    pub fn handshake_ack(
        soma_id: &str,
        negotiated_caps: &[String],
        max_signal_size: u32,
    ) -> Self {
        let mut s = Self::new(SignalType::HandshakeAck, soma_id.to_string());
        s.channel_id = 0;
        let meta = serde_json::json!({
            "protocol_version": "2.0",
            "negotiated_version": "2.0",
            "soma_id": soma_id,
            "negotiated_capabilities": negotiated_caps,
            "max_signal_size": max_signal_size,
        });
        s.metadata = meta;
        s
    }

    /// Create a Close signal.
    pub fn close(sender: &str) -> Self {
        Self::new(SignalType::Close, sender.to_string())
    }

    /// Convenience: read trace_id from metadata if the field is empty.
    pub fn effective_trace_id(&self) -> String {
        if !self.trace_id.is_empty() {
            return self.trace_id.clone();
        }
        self.metadata
            .get("trace_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }
}
