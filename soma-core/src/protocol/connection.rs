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

/// State for a multiplexed channel on a connection.
#[derive(Debug, Clone)]
pub struct ChannelState {
    pub id: u32,
    pub active: bool,
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
        let frame =
            codec::read_frame(&mut *reader, self.negotiated_max_signal_size as usize).await?;
        let signal = codec::decode_frame(&frame)?;

        // Any received signal resets the heartbeat counter (Spec Section 18.3)
        *self.last_received.lock().await = Instant::now();
        self.missed_pong_count.store(0, Ordering::Relaxed);

        Ok(signal)
    }

    /// Get a clone of the Arc'd writer for shared access (e.g., heartbeat task).
    pub fn writer_handle(&self) -> Arc<Mutex<OwnedWriteHalf>> {
        self.writer.clone()
    }

    /// Register a channel as active.
    pub async fn open_channel(&self, id: u32) {
        let mut channels = self.channels.lock().await;
        channels.insert(id, ChannelState { id, active: true });
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

        // Send HANDSHAKE_ACK
        let mut ack = Signal::handshake_ack(&self.local_id, &negotiated, negotiated_size);
        // Include our plugins in the ack metadata
        if let serde_json::Value::Object(ref mut map) = ack.metadata {
            map.insert(
                "plugins".to_string(),
                serde_json::json!(our_plugins),
            );
        }
        ack.sequence = self.next_sequence();
        self.send(&ack).await?;

        tracing::info!(
            peer = %peer_soma_id,
            capabilities = ?negotiated,
            max_signal_size = negotiated_size,
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
        // Send HANDSHAKE
        let mut hs = Signal::handshake(&self.local_id, our_capabilities, our_plugins);
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

        tracing::info!(
            peer = %peer_soma_id,
            capabilities = ?negotiated,
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
                // Send PING
                let mut ping = Signal::ping(&conn.local_id);
                ping.sequence = conn.next_sequence();
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
