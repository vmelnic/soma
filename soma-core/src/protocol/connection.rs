//! SynapseConnection — managed TCP connection with binary framing,
//! heartbeat, and handshake (Spec Sections 11.1, 12, 18).

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

/// Default keepalive interval (Spec Section 18.1).
pub const DEFAULT_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
/// Default pong timeout.
pub const DEFAULT_PONG_TIMEOUT: Duration = Duration::from_secs(10);
/// Max missed pongs before declaring peer dead.
pub const DEFAULT_MAX_MISSED_PONGS: u32 = 3;

/// Connection quality metrics derived from RTT measurements (Spec Section 12.4).
#[derive(Debug, Clone)]
pub struct ConnectionQuality {
    /// Smoothed round-trip time in milliseconds.
    pub rtt_ms: f32,
    /// RTT jitter (standard deviation) in milliseconds.
    pub rtt_jitter_ms: f32,
    /// How long this connection has been alive.
    pub connection_age_secs: u64,
    /// Recent RTT samples for computing averages.
    rtt_samples: Vec<f32>,
}

impl Default for ConnectionQuality {
    fn default() -> Self {
        Self {
            rtt_ms: 0.0,
            rtt_jitter_ms: 0.0,
            connection_age_secs: 0,
            rtt_samples: Vec::new(),
        }
    }
}

impl ConnectionQuality {
    /// Record a new RTT observation. Keeps the last 100 samples and
    /// recomputes the smoothed RTT and jitter (stddev).
    pub fn record_rtt(&mut self, rtt_ms: f32) {
        self.rtt_samples.push(rtt_ms);
        if self.rtt_samples.len() > 100 {
            self.rtt_samples.remove(0);
        }
        self.rtt_ms =
            self.rtt_samples.iter().sum::<f32>() / self.rtt_samples.len() as f32;
        // Jitter = stddev of RTT
        let mean = self.rtt_ms;
        let variance = self.rtt_samples.iter()
            .map(|x| (x - mean).powi(2))
            .sum::<f32>()
            / self.rtt_samples.len() as f32;
        self.rtt_jitter_ms = variance.sqrt();
    }
}

/// Default receive window size: 1 MB (Spec Section 9.2).
pub const DEFAULT_RECV_WINDOW: u64 = 1_048_576;

/// State for a multiplexed channel on a connection.
#[derive(Debug, Clone)]
pub struct ChannelState {
    pub id: u32,
    pub active: bool,
    /// Bytes remaining in the receive window (flow control).
    pub recv_window: u64,
    /// Maximum window size (default 1 MB).
    pub recv_window_max: u64,
    /// Unacknowledged bytes sent on this channel.
    pub bytes_in_flight: u64,
}

/// A managed Synaptic Protocol v2 connection. Owns the TCP read/write halves
/// and tracks per-connection state: channels, sequence counter, heartbeat,
/// and negotiated parameters.
pub struct SynapseConnection {
    /// Peer's SOMA ID (learned during handshake).
    pub peer_id: String,
    /// Read half of the TCP stream.
    reader: Mutex<OwnedReadHalf>,
    /// Write half, Arc'd so heartbeat task can share it.
    writer: Arc<Mutex<OwnedWriteHalf>>,
    /// Active channels on this connection.
    channels: Mutex<HashMap<u32, ChannelState>>,
    /// Monotonically increasing sequence number.
    sequence_counter: AtomicU32,
    /// When we last received ANY data from the peer (resets heartbeat).
    pub last_received: Mutex<Instant>,
    /// When we last sent data to the peer.
    pub last_sent: Mutex<Instant>,
    /// Missed PONG count for dead peer detection.
    pub missed_pong_count: AtomicU32,
    /// Negotiated max frame size (from handshake).
    pub negotiated_max_signal_size: u32,
    /// Negotiated capability set.
    pub negotiated_capabilities: Mutex<Vec<String>>,
    /// Our SOMA ID.
    pub local_id: String,
    /// Whether the connection is alive.
    pub alive: std::sync::atomic::AtomicBool,
    /// Session token for reconnect identification (Spec Section 14.5).
    pub session_token: Mutex<String>,
    /// When the current session token was created (for expiry checks, Spec Section 14.5).
    pub session_token_created: Mutex<Option<Instant>>,
    /// Connection quality metrics (RTT, jitter).
    pub quality: Mutex<ConnectionQuality>,
    /// Timestamp of the last PING we sent, keyed by sequence number,
    /// so we can compute RTT when the matching PONG arrives.
    pub ping_sent_at: Mutex<Option<(u32, Instant)>>,
    /// When this connection was established.
    pub created_at: Instant,
}

impl SynapseConnection {
    /// Create a new SynapseConnection from TCP stream halves.
    pub fn new(
        local_id: String,
        peer_id: String,
        reader: OwnedReadHalf,
        writer: OwnedWriteHalf,
        max_signal_size: u32,
    ) -> Self {
        let now = Instant::now();
        Self {
            peer_id,
            reader: Mutex::new(reader),
            writer: Arc::new(Mutex::new(writer)),
            channels: Mutex::new(HashMap::new()),
            sequence_counter: AtomicU32::new(1),
            last_received: Mutex::new(now),
            last_sent: Mutex::new(now),
            missed_pong_count: AtomicU32::new(0),
            negotiated_max_signal_size: max_signal_size,
            negotiated_capabilities: Mutex::new(Vec::new()),
            local_id,
            alive: std::sync::atomic::AtomicBool::new(true),
            session_token: Mutex::new(String::new()),
            session_token_created: Mutex::new(None),
            quality: Mutex::new(ConnectionQuality::default()),
            ping_sent_at: Mutex::new(None),
            created_at: now,
        }
    }

    /// Get the next sequence number.
    pub fn next_sequence(&self) -> u32 {
        self.sequence_counter.fetch_add(1, Ordering::Relaxed)
    }

    /// Send a signal over this connection. Assigns the next sequence number
    /// and encodes to binary wire format.
    pub async fn send(&self, signal: &Signal) -> Result<()> {
        if !self.alive.load(Ordering::Relaxed) {
            bail!("Connection to {} is dead", self.peer_id);
        }
        let mut writer = self.writer.lock().await;
        let frame = codec::encode_frame(signal);
        writer.write_all(&frame).await?;
        writer.flush().await?;
        *self.last_sent.lock().await = Instant::now();
        Ok(())
    }

    /// Send a signal, assigning the next sequence number automatically.
    pub async fn send_auto_seq(&self, signal: &mut Signal) -> Result<()> {
        signal.sequence = self.next_sequence();
        self.send(signal).await
    }

    /// Receive the next signal from this connection. Blocks until a
    /// complete frame is available.
    pub async fn recv(&self) -> Result<Signal> {
        if !self.alive.load(Ordering::Relaxed) {
            bail!("Connection to {} is dead", self.peer_id);
        }
        let mut reader = self.reader.lock().await;
        loop {
            let frame =
                codec::read_frame(&mut *reader, self.negotiated_max_signal_size as usize).await?;

            // Any received frame resets the heartbeat counter (Spec Section 18.3)
            *self.last_received.lock().await = Instant::now();
            self.missed_pong_count.store(0, Ordering::Relaxed);

            match codec::decode_frame(&frame)? {
                Some(signal) => return Ok(signal),
                None => {
                    // Unknown signal type — skip frame per Spec Sec 12.3
                    // (warning already logged by decode_frame)
                    continue;
                }
            }
        }
    }

    /// Get a clone of the Arc'd writer for shared access (e.g., heartbeat task).
    pub fn writer_handle(&self) -> Arc<Mutex<OwnedWriteHalf>> {
        self.writer.clone()
    }

    /// Register a channel as active with default flow-control window.
    pub async fn open_channel(&self, id: u32) {
        let mut channels = self.channels.lock().await;
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
    }

    /// Close a channel.
    pub async fn close_channel(&self, id: u32) {
        let mut channels = self.channels.lock().await;
        if let Some(ch) = channels.get_mut(&id) {
            ch.active = false;
        }
    }

    /// Mark the connection as dead.
    pub fn mark_dead(&self) {
        self.alive.store(false, Ordering::Relaxed);
    }

    /// Check if the connection is still alive.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// Check if the session token is still valid (not expired).
    /// Default expiry: 24 hours (Section 14.5).
    pub async fn is_session_valid(&self) -> bool {
        const SESSION_EXPIRY: Duration = Duration::from_secs(24 * 3600);
        let created = self.session_token_created.lock().await;
        match *created {
            Some(t) => t.elapsed() < SESSION_EXPIRY,
            None => false, // no session established
        }
    }

    /// Check if a signal type is allowed by negotiated capabilities
    /// (Spec Section 12.4). Returns `true` if the signal may proceed.
    pub async fn is_signal_allowed(&self, signal_type: SignalType) -> bool {
        match signal_type {
            // Always allowed (no capability needed)
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

            // Requires "streaming" capability
            SignalType::StreamStart | SignalType::StreamData | SignalType::StreamEnd => {
                let caps = self.negotiated_capabilities.lock().await;
                caps.iter().any(|c| c == "streaming")
            }

            // Requires "chunked" capability
            SignalType::ChunkStart
            | SignalType::ChunkData
            | SignalType::ChunkEnd
            | SignalType::ChunkAck => {
                let caps = self.negotiated_capabilities.lock().await;
                caps.iter().any(|c| c == "chunked")
            }

            // Requires "pubsub" capability
            SignalType::Subscribe | SignalType::Unsubscribe => {
                let caps = self.negotiated_capabilities.lock().await;
                caps.iter().any(|c| c == "pubsub")
            }
        }
    }

    /// Check if a specific capability was negotiated during handshake
    /// (Spec Section 12.4).
    pub async fn is_capability_negotiated(&self, cap: &str) -> bool {
        let caps = self.negotiated_capabilities.lock().await;
        caps.iter().any(|c| c == cap)
    }

    /// Record a PONG response and compute RTT if we have a matching
    /// PING timestamp for the given sequence number.
    pub async fn record_pong_rtt(&self, pong_sequence: u32) {
        let mut ping_info = self.ping_sent_at.lock().await;
        if let Some((seq, sent_at)) = ping_info.take() {
            if seq == pong_sequence {
                let rtt = sent_at.elapsed().as_secs_f32() * 1000.0;
                let mut quality = self.quality.lock().await;
                quality.connection_age_secs = self.created_at.elapsed().as_secs();
                quality.record_rtt(rtt);
                tracing::debug!(
                    peer = %self.peer_id,
                    rtt_ms = rtt,
                    avg_rtt_ms = quality.rtt_ms,
                    jitter_ms = quality.rtt_jitter_ms,
                    "RTT recorded"
                );
            } else {
                // Put it back if sequence doesn't match
                *ping_info = Some((seq, sent_at));
            }
        }
    }

    /// Perform the server side of the handshake: receive HANDSHAKE,
    /// negotiate, send HANDSHAKE_ACK. Returns the peer's SOMA ID.
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

        // Extract peer capabilities
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

        // Negotiate: intersection of capabilities
        let our_caps_set: std::collections::HashSet<&str> =
            our_capabilities.iter().copied().collect();
        let negotiated: Vec<String> = peer_caps
            .iter()
            .filter(|c| our_caps_set.contains(c.as_str()))
            .cloned()
            .collect();

        // Negotiate max_signal_size: minimum of both sides
        let peer_max_size = hs
            .metadata
            .get("max_signal_size")
            .and_then(|v| v.as_u64())
            .unwrap_or(10_485_760) as u32;
        let our_max_size = self.negotiated_max_signal_size;
        let negotiated_size = peer_max_size.min(our_max_size);

        // Store negotiated params
        *self.negotiated_capabilities.lock().await = negotiated.clone();

        // Generate session token (Spec Section 14.5)
        let session_token = uuid::Uuid::new_v4().to_string();
        *self.session_token.lock().await = session_token.clone();
        *self.session_token_created.lock().await = Some(Instant::now());

        // Check if the peer sent a previous session token (reconnect)
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

        // Send HANDSHAKE_ACK
        let mut ack = Signal::handshake_ack(&self.local_id, &negotiated, negotiated_size);
        // Include our plugins and session token in the ack metadata
        if let serde_json::Value::Object(ref mut map) = ack.metadata {
            map.insert(
                "plugins".to_string(),
                serde_json::json!(our_plugins),
            );
            map.insert(
                "session_token".to_string(),
                serde_json::json!(session_token),
            );
        }
        ack.sequence = self.next_sequence();
        self.send(&ack).await?;

        let session_display = self.session_token.lock().await.clone();
        tracing::info!(
            peer = %peer_soma_id,
            capabilities = ?negotiated,
            max_signal_size = negotiated_size,
            session = %session_display,
            "Handshake completed (server side)"
        );

        Ok(peer_soma_id)
    }

    /// Perform the client side of the handshake: send HANDSHAKE,
    /// receive HANDSHAKE_ACK. Returns the peer's SOMA ID.
    pub async fn client_handshake(
        &self,
        our_capabilities: &[&str],
        our_plugins: &[&str],
    ) -> Result<String> {
        // Send HANDSHAKE (include previous session token if reconnecting)
        let mut hs = Signal::handshake(&self.local_id, our_capabilities, our_plugins);
        let prev_token = self.session_token.lock().await.clone();
        if !prev_token.is_empty() {
            if let serde_json::Value::Object(ref mut map) = hs.metadata {
                map.insert(
                    "session_token".to_string(),
                    serde_json::json!(prev_token),
                );
            }
        }
        hs.sequence = self.next_sequence();
        self.send(&hs).await?;

        // Receive HANDSHAKE_ACK
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

        // Extract negotiated capabilities
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

        // Store session token from server (Spec Section 14.5)
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
            session = %session_display,
            "Handshake completed (client side)"
        );

        Ok(peer_soma_id)
    }

    /// Run the heartbeat loop. Sends PING when idle, tracks missed PONGs,
    /// and declares the peer dead after too many misses.
    ///
    /// This should be spawned as a separate task. It terminates when the
    /// connection is marked dead.
    pub async fn heartbeat_loop(
        conn: Arc<SynapseConnection>,
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

            // Check if we need to send a PING
            let elapsed_since_recv = {
                let last = conn.last_received.lock().await;
                last.elapsed()
            };

            if elapsed_since_recv >= keepalive_interval {
                // Send PING and record the send time for RTT measurement
                let mut ping = Signal::ping(&conn.local_id);
                ping.sequence = conn.next_sequence();
                let ping_seq = ping.sequence;
                // Store (sequence, Instant) so we can compute RTT when PONG arrives
                *conn.ping_sent_at.lock().await = Some((ping_seq, Instant::now()));
                if conn.send(&ping).await.is_err() {
                    tracing::warn!(peer = %conn.peer_id, "Failed to send PING, marking dead");
                    conn.mark_dead();
                    break;
                }

                // Wait for pong_timeout
                tokio::time::sleep(pong_timeout).await;

                // Check if we received anything since the PING
                let elapsed_after_ping = {
                    let last = conn.last_received.lock().await;
                    last.elapsed()
                };

                if elapsed_after_ping >= pong_timeout {
                    let missed = conn.missed_pong_count.fetch_add(1, Ordering::Relaxed) + 1;
                    tracing::warn!(
                        peer = %conn.peer_id,
                        missed_pongs = missed,
                        "PONG not received"
                    );

                    if missed >= max_missed {
                        tracing::error!(
                            peer = %conn.peer_id,
                            missed_pongs = missed,
                            "Peer declared dead after {} missed pongs",
                            missed
                        );
                        conn.mark_dead();
                        break;
                    }
                }
            }
        }
    }
}
