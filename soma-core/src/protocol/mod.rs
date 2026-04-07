//! Synaptic Protocol v2 — inter-SOMA communication (Spec Section 14).
//! Binary wire format over TCP with handshake, heartbeat, and multiplexed channels.

pub mod signal;
pub mod codec;
pub mod connection;
pub mod server;
pub mod client;
pub mod discovery;
pub mod relay;
pub mod chunked;
pub mod pubsub;
pub mod streaming;
pub mod rate_limit;
pub mod offline_queue;
pub mod encryption;
pub mod router;
pub mod websocket;
pub mod unix_socket;
