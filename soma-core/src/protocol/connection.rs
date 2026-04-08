//! Managed Synaptic Protocol v2 TCP connection.
//!
//! `SynapseConnection` owns a split TCP stream and provides:
//! - Version-negotiated handshake (client and server sides)
//! - Multiplexed channels with per-channel flow control (recv window)
//! - Heartbeat with RTT measurement and dead-peer detection
//! - Session tokens for reconnect identification (24h expiry)
//!

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::Mutex;

use super::codec;
use super::signal::{Signal, SignalType};

/// Interval between keepalive PINGs when idle.
pub const DEFAULT_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
/// How long to wait for a PONG before counting it as missed.
pub const DEFAULT_PONG_TIMEOUT: Duration = Duration::from_secs(10);
/// Consecutive missed PONGs before declaring the peer dead.
pub const DEFAULT_MAX_MISSED_PONGS: u32 = 3;

/// Connection quality metrics derived from PING/PONG RTT measurements.
///
/// Maintains a sliding window of up to 100 RTT samples and computes
/// smoothed RTT and jitter (standard deviation).
#[derive(Debug, Clone)]
pub struct ConnectionQuality {
    /// Smoothed round-trip time (average of recent samples).
    pub rtt_ms: f32,
    /// RTT jitter (standard deviation of recent samples).
    pub rtt_jitter_ms: f32,
    pub connection_age_secs: u64,
    #[allow(dead_code)]
    pub signal_loss_rate: f32,
    #[allow(dead_code)]
    pub bandwidth_bytes_sec: f64,
    /// Sliding window of recent RTT observations (max 100).
    rtt_samples: Vec<f32>,
}

impl Default for ConnectionQuality {
    fn default() -> Self {
        Self {
            rtt_ms: 0.0,
            rtt_jitter_ms: 0.0,
            connection_age_secs: 0,
            signal_loss_rate: 0.0,
            bandwidth_bytes_sec: 0.0,
            rtt_samples: Vec::new(),
        }
    }
}

impl ConnectionQuality {
    /// Record an RTT observation and recompute smoothed RTT and jitter.
    pub fn record_rtt(&mut self, rtt_ms: f32) {
        self.rtt_samples.push(rtt_ms);
        if self.rtt_samples.len() > 100 {
            self.rtt_samples.remove(0);
        }
        #[allow(clippy::cast_precision_loss)] // sample count <= 100, fits in f32
        let sample_count = self.rtt_samples.len() as f32;
        self.rtt_ms =
            self.rtt_samples.iter().sum::<f32>() / sample_count;
        // Jitter = stddev of RTT
        let mean = self.rtt_ms;
        let variance = self.rtt_samples.iter()
            .map(|x| (x - mean).powi(2))
            .sum::<f32>()
            / sample_count;
        self.rtt_jitter_ms = variance.sqrt();
    }
}

#[allow(dead_code)] // Used internally by channel flow control
/// Default receive window size per channel.
pub const DEFAULT_RECV_WINDOW: u64 = 1_048_576;

/// Per-channel flow control state for a multiplexed connection.
///
/// Tracks a receive window: the sender must not exceed `recv_window - bytes_in_flight`
/// bytes without an acknowledgment. When `bytes_in_flight` drops below 50% of
/// `recv_window_max`, the window is restored.
#[derive(Debug, Clone)]
pub struct ChannelState {
    pub id: u32,
    pub active: bool,
    pub recv_window: u64,
    pub recv_window_max: u64,
    pub bytes_in_flight: u64,
}

/// Default maximum number of multiplexed channels per connection.
pub const DEFAULT_MAX_CHANNELS: usize = 256;

/// Protocol versions supported by this implementation.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2.0"];

/// A managed Synaptic Protocol v2 connection.
///
/// Owns the split TCP stream halves and tracks all per-connection state:
/// peer identity, multiplexed channels, sequence numbering, heartbeat
/// timing, negotiated parameters, and session tokens.
///
/// The writer is `Arc`'d so the heartbeat task can send PINGs concurrently
/// with the main signal loop. All mutable state uses `Mutex` or atomics
/// for safe shared access.
pub struct SynapseConnection {
    /// Peer's SOMA ID, populated during handshake.
    pub peer_id: Mutex<String>,
    reader: Mutex<OwnedReadHalf>,
    /// `Arc`'d so the heartbeat task can share the write half.
    writer: Arc<Mutex<OwnedWriteHalf>>,
    channels: Mutex<HashMap<u32, ChannelState>>,
    sequence_counter: AtomicU32,
    /// Timestamp of last received data; any frame resets the heartbeat counter.
    pub last_received: Mutex<Instant>,
    pub last_sent: Mutex<Instant>,
    pub missed_pong_count: AtomicU32,
    /// Negotiated max frame size. Atomic so handshake can update through `&self`.
    pub negotiated_max_signal_size: AtomicU32,
    #[allow(dead_code)]
    pub max_channels: usize,
    pub negotiated_capabilities: Mutex<Vec<String>>,
    pub local_id: String,
    pub alive: std::sync::atomic::AtomicBool,
    /// Session token for reconnect identification (24h expiry).
    pub session_token: Mutex<String>,
    pub session_token_created: Mutex<Option<Instant>>,
    pub quality: Mutex<ConnectionQuality>,
    /// `(sequence, sent_at)` of the last PING, for RTT computation on PONG.
    pub ping_sent_at: Mutex<Option<(u32, Instant)>>,
    pub created_at: Instant,
    /// Heartbeat loop sends `peer_id` here when the peer is declared dead,
    /// allowing the server to clean up subscriptions, peer registry, etc.
    pub dead_peer_tx: Mutex<Option<tokio::sync::mpsc::Sender<String>>>,
}

impl SynapseConnection {
    /// Create a new `SynapseConnection` from TCP stream halves.
    pub fn new(
        local_id: String,
        peer_id: String,
        reader: OwnedReadHalf,
        writer: OwnedWriteHalf,
        max_signal_size: u32,
    ) -> Self {
        let now = Instant::now();
        Self {
            peer_id: Mutex::new(peer_id),
            reader: Mutex::new(reader),
            writer: Arc::new(Mutex::new(writer)),
            channels: Mutex::new(HashMap::new()),
            sequence_counter: AtomicU32::new(1),
            last_received: Mutex::new(now),
            last_sent: Mutex::new(now),
            missed_pong_count: AtomicU32::new(0),
            negotiated_max_signal_size: AtomicU32::new(max_signal_size),
            max_channels: DEFAULT_MAX_CHANNELS,
            negotiated_capabilities: Mutex::new(Vec::new()),
            local_id,
            alive: std::sync::atomic::AtomicBool::new(true),
            session_token: Mutex::new(String::new()),
            session_token_created: Mutex::new(None),
            quality: Mutex::new(ConnectionQuality::default()),
            ping_sent_at: Mutex::new(None),
            created_at: now,
            dead_peer_tx: Mutex::new(None),
        }
    }

    /// Get the next sequence number.
    pub fn next_sequence(&self) -> u32 {
        self.sequence_counter.fetch_add(1, Ordering::Relaxed)
    }

    /// Send a signal over this connection. Assigns the next sequence number
    /// and encodes to binary wire format.
    ///
    /// If the signal's channel has flow control state, checks whether the
    /// payload would exceed `recv_window - bytes_in_flight` and logs a
    /// warning if so. After a successful send,
    /// `bytes_in_flight` is incremented by the payload length.
    #[allow(clippy::significant_drop_tightening)] // lock guards are scoped appropriately
    pub async fn send(&self, signal: &Signal) -> Result<()> {
        if !self.alive.load(Ordering::Relaxed) {
            let peer = self.peer_id.lock().await;
            bail!("Connection to {} is dead", *peer);
        }

        let payload_len = signal.payload.len() as u64;

        {
            let channels = self.channels.lock().await;
            if let Some(ch) = channels.get(&signal.channel_id)
                && ch.active {
                    let available = ch.recv_window.saturating_sub(ch.bytes_in_flight);
                    if payload_len > available {
                        let peer = self.peer_id.lock().await;
                        tracing::warn!(
                            peer = %*peer,
                            channel = signal.channel_id,
                            payload_len,
                            bytes_in_flight = ch.bytes_in_flight,
                            recv_window = ch.recv_window,
                            available,
                            "Channel flow control: payload exceeds available window"
                        );
                        // Back-pressure: log and proceed
                    }
                }
        }

        let mut writer = self.writer.lock().await;
        let frame = codec::encode_frame(signal, None);
        writer.write_all(&frame).await?;
        writer.flush().await?;
        drop(writer);
        *self.last_sent.lock().await = Instant::now();

        {
            let mut channels = self.channels.lock().await;
            if let Some(ch) = channels.get_mut(&signal.channel_id)
                && ch.active {
                    ch.bytes_in_flight += payload_len;
                }
        }

        Ok(())
    }

    #[allow(dead_code)]
    /// Send a signal, assigning the next sequence number automatically.
    pub async fn send_auto_seq(&self, signal: &mut Signal) -> Result<()> {
        signal.sequence = self.next_sequence();
        self.send(signal).await
    }

    /// Receive the next signal from this connection. Blocks until a
    /// complete frame is available.
    ///
    /// After receiving a signal, updates the channel's `bytes_in_flight`
    /// (decremented by the payload size, clamped to 0). When
    /// `bytes_in_flight` drops below 50% of `recv_window_max`, a window
    /// update could be sent (currently just tracked).
    #[allow(clippy::significant_drop_tightening)] // reader lock held for frame loop, peer for bail
    pub async fn recv(&self) -> Result<Signal> {
        if !self.alive.load(Ordering::Relaxed) {
            let peer = self.peer_id.lock().await;
            bail!("Connection to {} is dead", *peer);
        }
        let mut reader = self.reader.lock().await;
        loop {
            let max_size = self.negotiated_max_signal_size.load(Ordering::Relaxed);
            let frame =
                codec::read_frame(&mut *reader, max_size as usize).await?;

            // Any received frame resets the missed-pong counter
            *self.last_received.lock().await = Instant::now();
            self.missed_pong_count.store(0, Ordering::Relaxed);

            if let Some(signal) = codec::decode_frame(&frame, None)? {
                let payload_len = signal.payload.len() as u64;
                if payload_len > 0 {
                    let mut channels = self.channels.lock().await;
                    if let Some(ch) = channels.get_mut(&signal.channel_id) {
                        ch.bytes_in_flight =
                            ch.bytes_in_flight.saturating_sub(payload_len);

                        // Restore window when in-flight drops below 50%
                        let half_window = ch.recv_window_max / 2;
                        if ch.bytes_in_flight < half_window
                            && ch.recv_window < ch.recv_window_max
                        {
                            ch.recv_window = ch.recv_window_max;
                            tracing::debug!(
                                channel = ch.id,
                                bytes_in_flight = ch.bytes_in_flight,
                                recv_window = ch.recv_window,
                                "Channel flow control: window restored"
                            );
                            // Send a WINDOW_UPDATE control signal to
                            // the peer when the protocol supports it.
                        }
                    }
                }
                return Ok(signal);
            }
            // Unknown signal type -- skip frame
            // (warning already logged by decode_frame)
        }
    }

    #[allow(dead_code)]
    /// Get a clone of the Arc'd writer for shared access (e.g., heartbeat task).
    pub fn writer_handle(&self) -> Arc<Mutex<OwnedWriteHalf>> {
        self.writer.clone()
    }

    #[allow(dead_code)]
    /// Register a channel as active with default flow-control window.
    /// Returns an error if the maximum number of channels has been reached.
    #[allow(clippy::significant_drop_tightening)] // channels lock needed for check-then-insert
    pub async fn open_channel(&self, id: u32) -> Result<()> {
        let mut channels = self.channels.lock().await;
        // Check max_channels limit before opening a new channel
        let active_count = channels.values().filter(|ch| ch.active).count();
        if active_count >= self.max_channels {
            bail!(
                "Maximum channel limit reached ({}/{})",
                active_count,
                self.max_channels
            );
        }
        channels.insert(
            id,
            ChannelState {
                id,
                active: true,
                recv_window: DEFAULT_RECV_WINDOW,
                recv_window_max: DEFAULT_RECV_WINDOW,
                bytes_in_flight: 0,
            },
        );
        Ok(())
    }

    #[allow(dead_code)]
    /// Close a channel.
    pub async fn close_channel(&self, id: u32) {
        let mut channels = self.channels.lock().await;
        if let Some(ch) = channels.get_mut(&id) {
            ch.active = false;
        }
    }

    #[allow(dead_code)]
    /// Update the channel's flow-control window by reducing `bytes_in_flight`
    /// by `acked_bytes`. This should be called when the peer acknowledges
    /// receipt of data.
    pub async fn update_channel_window(&self, channel_id: u32, acked_bytes: usize) {
        let mut channels = self.channels.lock().await;
        if let Some(ch) = channels.get_mut(&channel_id) {
            ch.bytes_in_flight = ch.bytes_in_flight.saturating_sub(acked_bytes as u64);
            tracing::debug!(
                channel = channel_id,
                acked_bytes,
                bytes_in_flight = ch.bytes_in_flight,
                recv_window = ch.recv_window,
                "Channel window updated after ack"
            );
        }
    }

    /// Set the dead-peer notification channel. When the heartbeat loop
    /// declares this peer dead, it sends the `peer_id` through this channel
    /// so that the server can perform cleanup.
    pub async fn set_dead_peer_tx(&self, tx: tokio::sync::mpsc::Sender<String>) {
        *self.dead_peer_tx.lock().await = Some(tx);
    }

    /// Mark the connection as dead.
    pub fn mark_dead(&self) {
        self.alive.store(false, Ordering::Relaxed);
    }

    /// Check if the connection is still alive.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    #[allow(dead_code)]
    /// Check if the session token is still valid (not expired).
    /// Default expiry: 24 hours.
    pub async fn is_session_valid(&self) -> bool {
        const SESSION_EXPIRY: Duration = Duration::from_secs(24 * 3600);
        let created = self.session_token_created.lock().await;
        created.is_some_and(|t| t.elapsed() < SESSION_EXPIRY)
    }

    /// Check whether a signal type is permitted by negotiated capabilities.
    /// Lifecycle, intent, data, and discovery signals are always allowed;
    /// streaming, chunked, and pubsub require their respective capability.
    pub async fn is_signal_allowed(&self, signal_type: SignalType) -> bool {
        match signal_type {
            SignalType::Handshake
            | SignalType::HandshakeAck
            | SignalType::Close
            | SignalType::Ping
            | SignalType::Pong
            | SignalType::Error
            | SignalType::Control
            | SignalType::Intent
            | SignalType::Result
            | SignalType::Data
            | SignalType::Discover
            | SignalType::DiscoverAck
            | SignalType::PeerQuery
            | SignalType::PeerList
            | SignalType::Binary => true,

            SignalType::StreamStart | SignalType::StreamData | SignalType::StreamEnd => {
                let caps = self.negotiated_capabilities.lock().await;
                caps.iter().any(|c| c == "streaming")
            }

            SignalType::ChunkStart
            | SignalType::ChunkData
            | SignalType::ChunkEnd
            | SignalType::ChunkAck => {
                let caps = self.negotiated_capabilities.lock().await;
                caps.iter().any(|c| c == "chunked")
            }

            SignalType::Subscribe | SignalType::Unsubscribe => {
                let caps = self.negotiated_capabilities.lock().await;
                caps.iter().any(|c| c == "pubsub")
            }
        }
    }

    /// Check if a specific capability was negotiated during handshake.
    pub async fn is_capability_negotiated(&self, cap: &str) -> bool {
        let caps = self.negotiated_capabilities.lock().await;
        caps.iter().any(|c| c == cap)
    }

    /// Match a PONG sequence to the outstanding PING and record the RTT.
    pub async fn record_pong_rtt(&self, pong_sequence: u32) {
        let mut ping_info = self.ping_sent_at.lock().await;
        if let Some((seq, sent_at)) = ping_info.take() {
            if seq == pong_sequence {
                let rtt = sent_at.elapsed().as_secs_f32() * 1000.0;
                let mut quality = self.quality.lock().await;
                quality.connection_age_secs = self.created_at.elapsed().as_secs();
                quality.record_rtt(rtt);
                let peer = self.peer_id.lock().await;
                tracing::debug!(
                    peer = %*peer,
                    rtt_ms = rtt,
                    avg_rtt_ms = quality.rtt_ms,
                    jitter_ms = quality.rtt_jitter_ms,
                    "RTT recorded"
                );
            } else {
                *ping_info = Some((seq, sent_at));
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    /// Perform the server side of the handshake: receive HANDSHAKE,
    /// negotiate, send `HANDSHAKE_ACK`. Returns the peer's SOMA ID.
    pub async fn server_handshake(
        &self,
        our_capabilities: &[&str],
        our_plugins: &[&str],
    ) -> Result<String> {
        // Receive HANDSHAKE
        let hs = self.recv().await?;
        if hs.signal_type != SignalType::Handshake {
            bail!(
                "Expected HANDSHAKE, got {:?} from peer",
                hs.signal_type
            );
        }

        let peer_soma_id = hs
            .metadata
            .get("soma_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        *self.peer_id.lock().await = peer_soma_id.clone();

        // Protocol version negotiation: find highest mutually supported version
        let peer_version = hs
            .metadata
            .get("protocol_version")
            .and_then(|v| v.as_str())
            .unwrap_or("2.0")
            .to_string();
        let peer_supported: Vec<String> = hs
            .metadata
            .get("supported_versions")
            .and_then(|v| v.as_array()).map_or_else(|| vec![peer_version.clone()], |arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            });

        let our_supported: std::collections::HashSet<&str> =
            SUPPORTED_PROTOCOL_VERSIONS.iter().copied().collect();
        let mut mutual_versions: Vec<&String> = peer_supported
            .iter()
            .filter(|v| our_supported.contains(v.as_str()))
            .collect();
        mutual_versions.sort();

        let negotiated_version = if let Some(v) = mutual_versions.last() { (*v).clone() } else {
            // No compatible version — send ERROR and bail
            let mut err_signal = Signal::error(
                &self.local_id,
                "incompatible_protocol",
            );
            if let serde_json::Value::Object(ref mut map) = err_signal.metadata {
                map.insert(
                    "our_supported".to_string(),
                    serde_json::json!(SUPPORTED_PROTOCOL_VERSIONS),
                );
                map.insert(
                    "peer_supported".to_string(),
                    serde_json::json!(peer_supported),
                );
            }
            err_signal.sequence = self.next_sequence();
            let _ = self.send(&err_signal).await;
            bail!(
                "Incompatible protocol versions: peer supports {peer_supported:?}, we support {SUPPORTED_PROTOCOL_VERSIONS:?}"
            );
        };

        // Capability negotiation: intersection of both sides' capabilities
        let peer_caps: Vec<String> = hs
            .metadata
            .get("capabilities")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let our_caps_set: std::collections::HashSet<&str> =
            our_capabilities.iter().copied().collect();
        let negotiated: Vec<String> = peer_caps
            .iter()
            .filter(|c| our_caps_set.contains(c.as_str()))
            .cloned()
            .collect();

        // Max signal size: use the minimum of both sides
        #[allow(clippy::cast_possible_truncation)] // signal sizes fit in u32 (max 10MB)
        let peer_max_size = hs
            .metadata
            .get("max_signal_size")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(10_485_760) as u32;
        let our_max_size = self.negotiated_max_signal_size.load(Ordering::Relaxed);
        let negotiated_size = peer_max_size.min(our_max_size);

        self.negotiated_max_signal_size.store(negotiated_size, Ordering::Relaxed);
        *self.negotiated_capabilities.lock().await = negotiated.clone();

        #[allow(clippy::cast_possible_truncation)] // channel count fits in usize
        let _peer_max_channels = hs
            .metadata
            .get("max_channels")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(DEFAULT_MAX_CHANNELS as u64) as usize;

        let session_token = uuid::Uuid::new_v4().to_string();
        *self.session_token.lock().await = session_token.clone();
        *self.session_token_created.lock().await = Some(Instant::now());

        let peer_session_token = hs
            .metadata
            .get("session_token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if !peer_session_token.is_empty() {
            tracing::info!(
                peer = %peer_soma_id,
                prev_session = %peer_session_token,
                "Peer reconnecting with previous session token"
            );
        }

        let mut ack = Signal::handshake_ack(&self.local_id, &negotiated, negotiated_size);
        if let serde_json::Value::Object(ref mut map) = ack.metadata {
            map.insert(
                "plugins".to_string(),
                serde_json::json!(our_plugins),
            );
            map.insert(
                "session_token".to_string(),
                serde_json::json!(session_token),
            );
            map.insert(
                "negotiated_version".to_string(),
                serde_json::json!(negotiated_version),
            );
        }
        ack.sequence = self.next_sequence();
        self.send(&ack).await?;

        let session_display = self.session_token.lock().await.clone();
        tracing::info!(
            peer = %peer_soma_id,
            capabilities = ?negotiated,
            max_signal_size = negotiated_size,
            negotiated_version = %negotiated_version,
            session = %session_display,
            "Handshake completed (server side)"
        );

        Ok(peer_soma_id)
    }

    /// Perform the client side of the handshake: send HANDSHAKE,
    /// receive `HANDSHAKE_ACK`. Returns the peer's SOMA ID.
    pub async fn client_handshake(
        &self,
        our_capabilities: &[&str],
        our_plugins: &[&str],
    ) -> Result<String> {
        let mut hs = Signal::handshake(&self.local_id, our_capabilities, our_plugins);
        let prev_token = self.session_token.lock().await.clone();
        if !prev_token.is_empty()
            && let serde_json::Value::Object(ref mut map) = hs.metadata {
                map.insert(
                    "session_token".to_string(),
                    serde_json::json!(prev_token),
                );
            }
        hs.sequence = self.next_sequence();
        self.send(&hs).await?;

        let ack = self.recv().await?;
        if ack.signal_type != SignalType::HandshakeAck {
            bail!(
                "Expected HANDSHAKE_ACK, got {:?}",
                ack.signal_type
            );
        }

        let peer_soma_id = ack
            .metadata
            .get("soma_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        *self.peer_id.lock().await = peer_soma_id.clone();

        let negotiated: Vec<String> = ack
            .metadata
            .get("negotiated_capabilities")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        *self.negotiated_capabilities.lock().await = negotiated.clone();

        #[allow(clippy::cast_possible_truncation)] // signal sizes fit in u32 (max 10MB)
        let ack_max_size = ack
            .metadata
            .get("max_signal_size")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(|| u64::from(self.negotiated_max_signal_size.load(Ordering::Relaxed))) as u32;
        let our_max_size = self.negotiated_max_signal_size.load(Ordering::Relaxed);
        let negotiated_size = ack_max_size.min(our_max_size);
        self.negotiated_max_signal_size.store(negotiated_size, Ordering::Relaxed);

        let server_session_token = ack
            .metadata
            .get("session_token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !server_session_token.is_empty() {
            *self.session_token.lock().await = server_session_token.clone();
        }

        let session_display = self.session_token.lock().await.clone();
        tracing::info!(
            peer = %peer_soma_id,
            capabilities = ?negotiated,
            max_signal_size = negotiated_size,
            session = %session_display,
            "Handshake completed (client side)"
        );

        Ok(peer_soma_id)
    }

    #[allow(dead_code)]
    /// Get a snapshot of the `peer_id` (convenience for logging outside async).
    pub async fn peer_id_snapshot(&self) -> String {
        self.peer_id.lock().await.clone()
    }

    /// Heartbeat loop: sends PING after `keepalive_interval` of idle time,
    /// waits `pong_timeout` for a response, and declares the peer dead after
    /// `max_missed` consecutive misses.
    ///
    /// Must be spawned as a separate tokio task. Terminates when the
    /// connection is marked dead or the peer is declared dead.
    pub async fn heartbeat_loop(
        conn: Arc<Self>,
        keepalive_interval: Duration,
        pong_timeout: Duration,
        max_missed: u32,
    ) {
        let mut check_interval = tokio::time::interval(keepalive_interval);

        loop {
            check_interval.tick().await;

            if !conn.is_alive() {
                break;
            }

            let elapsed_since_recv = {
                let last = conn.last_received.lock().await;
                last.elapsed()
            };

            if elapsed_since_recv >= keepalive_interval {
                let mut ping = Signal::ping(&conn.local_id);
                ping.sequence = conn.next_sequence();
                let ping_seq = ping.sequence;
                *conn.ping_sent_at.lock().await = Some((ping_seq, Instant::now()));
                let peer_display = conn.peer_id.lock().await.clone();
                if conn.send(&ping).await.is_err() {
                    tracing::warn!(peer = %peer_display, "Failed to send PING, marking dead");
                    conn.mark_dead();
                    break;
                }

                tokio::time::sleep(pong_timeout).await;

                let elapsed_after_ping = {
                    let last = conn.last_received.lock().await;
                    last.elapsed()
                };

                if elapsed_after_ping >= pong_timeout {
                    let missed = conn.missed_pong_count.fetch_add(1, Ordering::Relaxed) + 1;
                    let peer_display = conn.peer_id.lock().await.clone();
                    tracing::warn!(
                        peer = %peer_display,
                        missed_pongs = missed,
                        "PONG not received"
                    );

                    if missed >= max_missed {
                        tracing::error!(
                            peer = %peer_display,
                            missed_pongs = missed,
                            "Peer declared dead after {} missed pongs",
                            missed
                        );

                        conn.mark_dead();

                        // Notify the server handler via dead_peer_tx so it can
                        // clean up subscriptions, peer registry, etc.
                        {
                            let tx_guard = conn.dead_peer_tx.lock().await;
                            if let Some(ref tx) = *tx_guard {
                                if let Err(e) = tx.send(peer_display.clone()).await {
                                    tracing::warn!(
                                        peer = %peer_display,
                                        error = %e,
                                        "Failed to send dead-peer notification"
                                    );
                                } else {
                                    tracing::debug!(
                                        peer = %peer_display,
                                        "Dead-peer notification sent to server handler"
                                    );
                                }
                            } else {
                                tracing::debug!(peer = %peer_display, "No dead-peer notification channel; cleanup skipped");
                            }
                        }

                        // Server-side only logs; the peer's client reconnects via connect_with_retry
                        tracing::info!(
                            peer = %peer_display,
                            "Dead peer cleanup complete; client-side reconnect handled by connect_with_retry"
                        );

                        break;
                    }
                }
            }
        }
    }
}
