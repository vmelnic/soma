//! Synaptic Protocol v2 — binary wire protocol for inter-SOMA communication.
//!
//! Provides TCP-based connectivity between SOMA instances with binary framing,
//! version-negotiated handshakes, per-connection heartbeat, multiplexed channels
//! with flow control, and pluggable transports (TCP, WebSocket, Unix socket).
//!

/// Signal types and flags — the fundamental unit of protocol communication.
pub mod signal;
/// Binary wire codec: frame encoding/decoding with CRC32, zstd, and AEAD encryption.
pub mod codec;
/// Managed TCP connections with handshake, heartbeat, flow control, and session tokens.
pub mod connection;
/// TCP listener that accepts connections and dispatches signals to handlers.
pub mod server;
/// TCP client with auto-reconnect and retry logic.
pub mod client;
/// Peer discovery and registry for multi-SOMA networks.
pub mod discovery;
/// Multi-hop signal relay with loop prevention.
pub mod relay;
/// Resumable chunked file transfer with SHA-256 verification.
pub mod chunked;
/// Topic-based publish/subscribe with wildcard matching and durable subscriptions.
pub mod pubsub;
/// Stream lifecycle management (start/data/end) with frame counting.
pub mod streaming;
/// Graduated rate limiting with CONTROL signal back-pressure.
pub mod rate_limit;
/// Priority queue for signals destined for offline peers, with expiry.
pub mod offline_queue;
/// ChaCha20-Poly1305 AEAD encryption and X25519/Ed25519 key exchange.
pub mod encryption;
/// Signal routing with pending-request tracking and timeout.
pub mod router;
/// WebSocket transport adapter for browser-based SOMA clients.
pub mod websocket;
/// Unix Domain Socket transport for local inter-process communication.
pub mod unix_socket;
