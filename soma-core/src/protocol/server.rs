//! Synaptic Protocol v2 server — TCP listener with binary wire format,
//! handshake negotiation, heartbeat, and signal routing.
//!
// Future: SignalRouter struct with DashMap for pending_requests,
// stream_handlers, chunk_buffers (Section 14.2). Currently routing
// is inline per-connection. Will be extracted when inter-SOMA
// request-response correlation is needed.

use anyhow::Result;
use std::sync::Arc;
use tokio::net::TcpListener;

use crate::mind::MindEngine;

use super::chunked::ChunkManager;
use super::connection::{
    SynapseConnection, DEFAULT_KEEPALIVE_INTERVAL, DEFAULT_MAX_MISSED_PONGS,
    DEFAULT_PONG_TIMEOUT,
};
use super::discovery::PeerRegistry;
use super::pubsub::PubSubManager;
use super::rate_limit::{self, RateLimiter, ViolationLevel};
use super::signal::{Signal, SignalFlags, SignalType};

/// Handler trait for incoming signals. Implementations decide how to
/// respond to each signal type.
pub trait SignalHandler: Send + Sync {
    fn handle(&self, signal: Signal) -> Option<Signal>;
}

#[allow(dead_code)] // Spec feature for basic protocol testing
/// A simple handler that responds to Ping with Pong and logs everything else.
pub struct DefaultHandler {
    pub name: String,
}

impl SignalHandler for DefaultHandler {
    fn handle(&self, signal: Signal) -> Option<Signal> {
        if signal.signal_type == SignalType::Ping { Some(Signal::pong(&self.name, signal.sequence)) } else {
            tracing::debug!(
                signal_type = ?signal.signal_type,
                sender = %signal.sender_id,
                "Received signal (no handler)"
            );
            None
        }
    }
}

/// Handler that routes Intent signals to the mind engine and plugin manager.
/// This enables inter-SOMA intent-to-result communication.
pub struct SomaSignalHandler {
    pub name: String,
    pub mind: Arc<std::sync::RwLock<crate::mind::onnx_engine::OnnxMindEngine>>,
    pub plugins: Arc<std::sync::RwLock<crate::plugin::manager::PluginManager>>,
    pub max_program_steps: usize,
}

impl SignalHandler for SomaSignalHandler {
    fn handle(&self, signal: Signal) -> Option<Signal> {
        match signal.signal_type {
            SignalType::Ping => Some(Signal::pong(&self.name, signal.sequence)),
            SignalType::Intent => {
                // Extract intent text from payload
                let Ok(intent_text) = String::from_utf8(signal.payload.clone()) else {
                    return Some(Signal::error(
                        &self.name,
                        "Invalid UTF-8 in intent payload",
                    ));
                };

                // Propagate trace_id from incoming signal
                let trace_id = if signal.trace_id.is_empty() {
                    uuid::Uuid::new_v4().to_string()[..12].to_string()
                } else {
                    signal.trace_id.clone()
                };

                tracing::info!(
                    component = "router",
                    trace_id = %trace_id,
                    sender = %signal.sender_id,
                    intent = %intent_text,
                    "Processing remote intent"
                );

                // Run inference via the mind engine
                let mind_guard = self.mind.read().unwrap();
                match mind_guard.infer(&intent_text) {
                    Ok(program) => {
                        let result = self
                            .plugins
                            .read()
                            .unwrap()
                            .execute_program(&program.steps, self.max_program_steps);

                        tracing::info!(
                            component = "mind",
                            trace_id = %trace_id,
                            steps = program.steps.len(),
                            confidence = %program.confidence,
                            success = result.success,
                            "Remote intent processed"
                        );

                        let mut resp = if result.success {
                            let output_str = result.output.as_ref().map_or_else(
                                || "Done.".to_string(),
                                |val| format!("{val}"),
                            );
                            let mut r =
                                Signal::new(SignalType::Result, self.name.clone());
                            r.payload = output_str.into_bytes();
                            r
                        } else {
                            let mut r =
                                Signal::new(SignalType::Error, self.name.clone());
                            r.payload = result
                                .error
                                .unwrap_or_else(|| "unknown error".into())
                                .into_bytes();
                            r
                        };
                        resp.trace_id = trace_id;
                        resp.channel_id = signal.channel_id;
                        Some(resp)
                    }
                    Err(e) => {
                        let mut resp = Signal::error(
                            &self.name,
                            &format!("Inference error: {e}"),
                        );
                        resp.trace_id = trace_id;
                        resp.channel_id = signal.channel_id;
                        Some(resp)
                    }
                }
            }
            _ => {
                tracing::debug!(
                    signal_type = ?signal.signal_type,
                    sender = %signal.sender_id,
                    "Received signal (no handler)"
                );
                None
            }
        }
    }
}

/// Configuration for the `SynapseServer`.
pub struct ServerConfig {
    pub max_signal_size: u32,
    pub max_connections: usize,
    pub keepalive_interval_secs: u64,
    pub pong_timeout_secs: u64,
    pub max_missed_pongs: u32,
    pub capabilities: Vec<String>,
    pub plugins: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            max_signal_size: 10_485_760,
            max_connections: 16,
            keepalive_interval_secs: DEFAULT_KEEPALIVE_INTERVAL.as_secs(),
            pong_timeout_secs: DEFAULT_PONG_TIMEOUT.as_secs(),
            max_missed_pongs: DEFAULT_MAX_MISSED_PONGS,
            capabilities: vec![
                "streaming".into(),
                "chunked".into(),
                "encryption".into(),
                "relay".into(),
            ],
            plugins: vec!["posix".into()],
        }
    }
}

/// TCP server that accepts connections, performs handshakes, manages
/// heartbeats, and dispatches signals to a handler using binary wire format.
pub struct SynapseServer {
    name: String,
    bind_addr: String,
    config: ServerConfig,
    metrics: Option<Arc<crate::metrics::SomaMetrics>>,
    peer_registry: Option<Arc<std::sync::RwLock<PeerRegistry>>>,
}

impl SynapseServer {
    pub fn new(name: String, bind_addr: String) -> Self {
        Self {
            name,
            bind_addr,
            config: ServerConfig::default(),
            metrics: None,
            peer_registry: None,
        }
    }

    #[allow(dead_code)] // Spec feature for custom server configuration
    pub const fn with_config(name: String, bind_addr: String, config: ServerConfig) -> Self {
        Self {
            name,
            bind_addr,
            config,
            metrics: None,
            peer_registry: None,
        }
    }

    pub fn with_metrics(mut self, metrics: Arc<crate::metrics::SomaMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    pub fn with_peer_registry(mut self, registry: Arc<std::sync::RwLock<PeerRegistry>>) -> Self {
        self.peer_registry = Some(registry);
        self
    }

    #[allow(clippy::too_many_lines)]
    /// Start listening for incoming connections. Runs until the task is cancelled.
    pub async fn start(
        &self,
        handler: impl SignalHandler + 'static,
    ) -> Result<()> {
        let listener = TcpListener::bind(&self.bind_addr).await?;
        tracing::info!(
            bind = %self.bind_addr,
            name = %self.name,
            "Synaptic Protocol v2 server started (binary wire format)"
        );

        let handler = Arc::new(handler);
        let pubsub = Arc::new(tokio::sync::Mutex::new(PubSubManager::new()));
        let chunk_mgr = Arc::new(tokio::sync::Mutex::new(ChunkManager::new()));
        let active_connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let connection_counter = Arc::new(std::sync::atomic::AtomicU64::new(1));
        let max_conn_limit: usize = self.config.max_connections;
        let peer_registry = self.peer_registry.clone();

        loop {
            let (stream, addr) = listener.accept().await?;

            // Enforce max connection limit (Section 11)
            let current = active_connections.load(std::sync::atomic::Ordering::Relaxed);
            if current >= max_conn_limit {
                tracing::warn!(
                    peer = %addr,
                    active = current,
                    limit = max_conn_limit,
                    "Connection rejected: max connections reached"
                );
                drop(stream);
                continue;
            }
            active_connections.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

            let handler = handler.clone();
            let server_name = self.name.clone();
            let max_signal_size = self.config.max_signal_size;
            let keepalive_secs = self.config.keepalive_interval_secs;
            let pong_timeout_secs = self.config.pong_timeout_secs;
            let max_missed = self.config.max_missed_pongs;
            let capabilities: Vec<String> = self.config.capabilities.clone();
            let plugins: Vec<String> = self.config.plugins.clone();
            let pubsub = pubsub.clone();
            let chunk_mgr = chunk_mgr.clone();
            let active_conns = active_connections.clone();
            let conn_id = connection_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let metrics = self.metrics.clone();
            let peer_registry = peer_registry.clone();

            tokio::spawn(async move {
                tracing::debug!(peer = %addr, "TCP connection accepted");

                let (reader, writer) = stream.into_split();
                let conn = Arc::new(SynapseConnection::new(
                    server_name.clone(),
                    String::new(), // peer_id filled after handshake
                    reader,
                    writer,
                    max_signal_size,
                ));

                // Perform server-side handshake
                let cap_refs: Vec<&str> = capabilities.iter().map(std::string::String::as_str).collect();
                let plug_refs: Vec<&str> = plugins.iter().map(std::string::String::as_str).collect();
                let peer_id = match conn.server_handshake(&cap_refs, &plug_refs).await {
                    Ok(id) => id,
                    Err(e) => {
                        tracing::warn!(peer = %addr, error = %e, "Handshake failed");
                        active_conns.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                        return;
                    }
                };

                tracing::info!(peer = %addr, peer_id = %peer_id, "Handshake successful");

                // Dead-peer notification channel (Spec Section 18.3).
                // The heartbeat loop sends the peer_id when the peer is
                // declared dead; we select on this in the main loop to
                // perform cleanup with access to PubSubManager, PeerRegistry, etc.
                let (dead_peer_tx, mut dead_peer_rx) =
                    tokio::sync::mpsc::channel::<String>(1);
                conn.set_dead_peer_tx(dead_peer_tx).await;

                // Start heartbeat task
                let conn_for_heartbeat = conn.clone();
                let _heartbeat_handle = tokio::spawn(async move {
                    SynapseConnection::heartbeat_loop(
                        conn_for_heartbeat,
                        std::time::Duration::from_secs(keepalive_secs),
                        std::time::Duration::from_secs(pong_timeout_secs),
                        max_missed,
                    )
                    .await;
                });

                // Per-connection rate limiter (Spec Section 20)
                let mut rate_limiter = RateLimiter::new(1000, 10_485_760);

                // Main signal processing loop
                loop {
                    if !conn.is_alive() {
                        tracing::info!(peer = %peer_id, "Connection marked dead, closing");
                        break;
                    }

                    // Select between receiving a signal and a dead-peer
                    // notification from the heartbeat loop.
                    let signal = tokio::select! {
                        dead_id = dead_peer_rx.recv() => {
                            if let Some(dead_id) = dead_id {
                                tracing::info!(
                                    peer = %dead_id,
                                    conn_id,
                                    "Dead peer notification received, performing cleanup"
                                );

                                // Step 4: Remove from active peer list (PeerRegistry)
                                if let Some(ref registry) = peer_registry {
                                    let removed = registry.write().unwrap().remove(&dead_id);
                                    if removed.is_some() {
                                        tracing::info!(
                                            peer = %dead_id,
                                            "Removed dead peer from PeerRegistry"
                                        );
                                    }
                                }

                                // Step 5: Cancel all subscriptions for this connection (PubSubManager)
                                {
                                    let mut ps = pubsub.lock().await;
                                    ps.remove_connection(conn_id);
                                    drop(ps);
                                    tracing::info!(
                                        peer = %dead_id,
                                        conn_id,
                                        "Cancelled subscriptions for dead peer"
                                    );
                                }

                                // Steps 2 & 3: SignalRouter.fail_all and StreamManager.interrupt_all
                                // are not yet accessible here; log for future implementation.
                                tracing::debug!(peer = %dead_id, "TODO: fail pending requests via SignalRouter");
                                tracing::debug!(peer = %dead_id, "TODO: interrupt active streams via StreamManager");
                            }
                            break;
                        }
                        recv_result = conn.recv() => {
                            match recv_result {
                                Ok(s) => {
                                    if let Some(ref m) = metrics {
                                        m.record_signal_received(s.payload.len() as u64);
                                    }
                                    s
                                }
                                Err(e) => {
                                    // Check if it's just a clean disconnect
                                    let msg = e.to_string();
                                    if msg.contains("unexpected eof")
                                        || msg.contains("connection reset")
                                        || msg.contains("broken pipe")
                                    {
                                        tracing::debug!(
                                            peer = %peer_id,
                                            "Peer disconnected"
                                        );
                                    } else {
                                        tracing::warn!(
                                            peer = %peer_id,
                                            error = %e,
                                            "Error reading signal"
                                        );
                                    }
                                    conn.mark_dead();
                                    break;
                                }
                            }
                        }
                    };

                    tracing::debug!(
                        signal_type = ?signal.signal_type,
                        sender = %signal.sender_id,
                        seq = signal.sequence,
                        channel = signal.channel_id,
                        "Received signal"
                    );

                    // Rate limiting (Spec Section 20): check before processing
                    if let Some(retry_after) = rate_limiter.check(signal.payload.len()) {
                        let level = rate_limiter.violation_level();
                        match level {
                            ViolationLevel::Severe => {
                                tracing::error!(
                                    peer = %peer_id,
                                    "Rate limit severe violation, closing connection"
                                );
                                let mut ctrl = rate_limit::create_rate_limit_signal(
                                    &server_name,
                                    retry_after,
                                );
                                ctrl.sequence = conn.next_sequence();
                                let _ = conn.send(&ctrl).await;
                                conn.mark_dead();
                                break;
                            }
                            ViolationLevel::Sustained => {
                                tracing::warn!(
                                    peer = %peer_id,
                                    retry_after_ms = retry_after,
                                    "Rate limit sustained violation"
                                );
                                continue;
                            }
                            _ => {
                                tracing::debug!(
                                    peer = %peer_id,
                                    retry_after_ms = retry_after,
                                    "Rate limit warning"
                                );
                                continue;
                            }
                        }
                    }

                    // Handle protocol-level signals internally
                    match signal.signal_type {
                        SignalType::Close => {
                            tracing::info!(peer = %peer_id, "Peer sent CLOSE");
                            conn.mark_dead();
                            break;
                        }
                        SignalType::Pong => {
                            conn.record_pong_rtt(signal.sequence).await;
                            tracing::debug!(peer = %peer_id, seq = signal.sequence, "PONG received");
                            continue;
                        }
                        SignalType::Subscribe => {
                            // Pub/Sub subscribe (Section 9.4)
                            let topic = String::from_utf8_lossy(&signal.payload).to_string();
                            let durable = signal.metadata.get("durable")
                                .and_then(serde_json::Value::as_bool).unwrap_or(false);
                            let mut ps = pubsub.lock().await;
                            #[allow(clippy::cast_possible_truncation)] // sequence numbers fit in u32
                            let last_seen = signal.metadata.get("last_seen_sequence")
                                .and_then(serde_json::Value::as_u64).map(|v| v as u32);
                            ps.subscribe(
                                &topic,
                                signal.channel_id,
                                conn_id,
                                last_seen,
                                durable,
                            );
                            tracing::info!(peer = %peer_id, topic = %topic, conn_id, "Subscribed");

                            // Catch-up: send buffered signals since last_seen (Spec Section 16)
                            if let Some(last) = last_seen {
                                let catchup_signals = ps.catch_up(&topic, last);
                                drop(ps); // release lock before sending
                                for mut catchup_signal in catchup_signals {
                                    catchup_signal.sequence = conn.next_sequence();
                                    catchup_signal.sender_id = server_name.clone();
                                    if let Err(e) = conn.send(&catchup_signal).await {
                                        tracing::warn!(
                                            peer = %peer_id,
                                            error = %e,
                                            "Failed to send catch-up signal"
                                        );
                                        conn.mark_dead();
                                        break;
                                    }
                                }
                                if !conn.is_alive() {
                                    break;
                                }
                            }
                            continue;
                        }
                        SignalType::Unsubscribe => {
                            let topic = String::from_utf8_lossy(&signal.payload).to_string();
                            let mut ps = pubsub.lock().await;
                            ps.unsubscribe(&topic, conn_id);
                            drop(ps);
                            tracing::info!(peer = %peer_id, topic = %topic, "Unsubscribed");
                            continue;
                        }

                        // Discovery signals (Spec Section 7)
                        SignalType::Discover => {
                            if let Some(ref registry) = peer_registry {
                                {
                                    let mut pr = registry.write().unwrap();
                                    pr.register_from_discover(&signal);
                                }
                                tracing::info!(
                                    peer = %peer_id,
                                    sender = %signal.sender_id,
                                    "Peer discovered, sending DISCOVER_ACK"
                                );
                                // Send DISCOVER_ACK back
                                let mut ack = Signal::new(
                                    SignalType::DiscoverAck,
                                    server_name.clone(),
                                );
                                ack.sequence = conn.next_sequence();
                                ack.channel_id = 0;
                                if let Err(e) = conn.send(&ack).await {
                                    tracing::warn!(
                                        peer = %peer_id,
                                        error = %e,
                                        "Failed to send DISCOVER_ACK"
                                    );
                                    conn.mark_dead();
                                    break;
                                }
                            }
                            continue;
                        }
                        SignalType::DiscoverAck => {
                            if let Some(ref registry) = peer_registry {
                                let mut pr = registry.write().unwrap();
                                pr.register_from_discover(&signal);
                                drop(pr);
                                tracing::info!(
                                    peer = %peer_id,
                                    sender = %signal.sender_id,
                                    "DISCOVER_ACK received, peer registered"
                                );
                            }
                            continue;
                        }
                        SignalType::PeerQuery => {
                            if let Some(ref registry) = peer_registry {
                                let response = {
                                    let pr = registry.read().unwrap();
                                    pr.handle_peer_query(&signal, &server_name)
                                };
                                let mut response = response;
                                response.sequence = conn.next_sequence();
                                if let Err(e) = conn.send(&response).await {
                                    tracing::warn!(
                                        peer = %peer_id,
                                        error = %e,
                                        "Failed to send PEER_LIST response"
                                    );
                                    conn.mark_dead();
                                    break;
                                }
                            }
                            continue;
                        }
                        SignalType::PeerList => {
                            if let Some(ref registry) = peer_registry {
                                let mut pr = registry.write().unwrap();
                                pr.register_from_discover(&signal);
                                drop(pr);
                                tracing::info!(
                                    peer = %peer_id,
                                    "PEER_LIST received, peers updated"
                                );
                            }
                            continue;
                        }

                        // Chunked transfer signals (Spec Section 6.3)
                        SignalType::ChunkStart => {
                            let mut cm = chunk_mgr.lock().await;
                            let result = cm.start_transfer(signal.channel_id, &signal.metadata);
                            drop(cm);
                            if let Err(e) = result {
                                tracing::warn!(
                                    peer = %peer_id,
                                    channel = signal.channel_id,
                                    error = %e,
                                    "Failed to start chunked transfer"
                                );
                                let mut err = Signal::error(
                                    &server_name,
                                    &format!("chunk_start_failed: {e}"),
                                );
                                err.sequence = conn.next_sequence();
                                err.channel_id = signal.channel_id;
                                let _ = conn.send(&err).await;
                            } else {
                                tracing::info!(
                                    peer = %peer_id,
                                    channel = signal.channel_id,
                                    "Chunked transfer started"
                                );
                            }
                            continue;
                        }
                        SignalType::ChunkData => {
                            let mut cm = chunk_mgr.lock().await;
                            let ack_opt = cm.receive_chunk(
                                signal.channel_id,
                                signal.sequence,
                                signal.payload.clone(),
                            );
                            drop(cm);
                            if let Some(mut ack) = ack_opt {
                                ack.sender_id = server_name.clone();
                                ack.sequence = conn.next_sequence();
                                if let Err(e) = conn.send(&ack).await {
                                    tracing::warn!(
                                        peer = %peer_id,
                                        error = %e,
                                        "Failed to send CHUNK_ACK"
                                    );
                                    conn.mark_dead();
                                    break;
                                }
                            }
                            continue;
                        }
                        SignalType::ChunkEnd => {
                            let mut cm = chunk_mgr.lock().await;
                            let result = cm.finalize_transfer(signal.channel_id);
                            drop(cm);
                            match result {
                                Ok(data) => {
                                    tracing::info!(
                                        peer = %peer_id,
                                        channel = signal.channel_id,
                                        size = data.len(),
                                        "Chunked transfer complete"
                                    );
                                    // Send final ACK with reassembled size
                                    let mut ack = Signal::new(
                                        SignalType::ChunkAck,
                                        server_name.clone(),
                                    );
                                    ack.channel_id = signal.channel_id;
                                    ack.sequence = conn.next_sequence();
                                    ack.metadata = serde_json::json!({
                                        "status": "complete",
                                        "total_bytes": data.len(),
                                    });
                                    let _ = conn.send(&ack).await;
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        peer = %peer_id,
                                        channel = signal.channel_id,
                                        error = %e,
                                        "Chunked transfer finalization failed"
                                    );
                                    let mut err = Signal::error(
                                        &server_name,
                                        &format!("chunk_finalize_failed: {e}"),
                                    );
                                    err.sequence = conn.next_sequence();
                                    err.channel_id = signal.channel_id;
                                    let _ = conn.send(&err).await;
                                }
                            }
                            continue;
                        }
                        SignalType::ChunkAck => {
                            // ChunkAck is typically sent by the receiver; if we get
                            // one it means we are the sender. Log and continue.
                            tracing::debug!(
                                peer = %peer_id,
                                channel = signal.channel_id,
                                seq = signal.sequence,
                                "CHUNK_ACK received"
                            );
                            continue;
                        }

                        _ => {}
                    }

                    // PRIORITY signals bypass capability checks (Sec 9.2)
                    let is_priority = signal.flags.contains(SignalFlags::PRIORITY);

                    // Capability enforcement (Spec Section 12.4):
                    // reject signals whose type requires a capability that
                    // was not negotiated during handshake.
                    if !is_priority && !conn.is_signal_allowed(signal.signal_type).await {
                        tracing::warn!(
                            peer = %peer_id,
                            signal_type = ?signal.signal_type,
                            "Signal rejected: capability_not_negotiated"
                        );
                        let mut err = Signal::error(
                            &server_name,
                            "capability_not_negotiated",
                        );
                        err.sequence = conn.next_sequence();
                        err.channel_id = signal.channel_id;
                        if let Err(e) = conn.send(&err).await {
                            tracing::warn!(
                                peer = %peer_id,
                                error = %e,
                                "Failed to send capability error"
                            );
                            conn.mark_dead();
                            break;
                        }
                        continue;
                    }

                    // PubSub fan-out: if DATA or STREAM_DATA has a topic, publish to subscribers
                    if matches!(signal.signal_type, SignalType::Data | SignalType::StreamData)
                        && let Some(topic) = signal.metadata.get("topic").and_then(|v| v.as_str()) {
                            let topic = topic.to_string();
                            let mut ps = pubsub.lock().await;
                            let fan_out = ps.publish(
                                &topic,
                                &signal.payload,
                                signal.channel_id,
                            );
                            drop(ps);
                            // Fan-out signals to matching subscribers on this connection
                            // (In a full implementation, fan-out would route to each
                            // connection by conn_id. Here we send signals destined
                            // for this connection.)
                            for (target_conn_id, mut pub_signal) in fan_out {
                                if target_conn_id == conn_id {
                                    pub_signal.sender_id = server_name.clone();
                                    pub_signal.sequence = conn.next_sequence();
                                    if let Err(e) = conn.send(&pub_signal).await {
                                        tracing::warn!(
                                            peer = %peer_id,
                                            error = %e,
                                            "Failed to send pub/sub fan-out signal"
                                        );
                                        conn.mark_dead();
                                        break;
                                    }
                                } else {
                                    tracing::debug!(
                                        target_conn = target_conn_id,
                                        topic = %topic,
                                        "PubSub fan-out to other connection (not yet routed)"
                                    );
                                }
                            }
                            if !conn.is_alive() {
                                break;
                            }
                        }

                    // Relay capability gating (Sec 12.4)
                    if super::relay::should_relay(&signal, &server_name) {
                        if !conn.is_capability_negotiated("relay").await {
                            tracing::warn!(peer = %peer_id, "Relay rejected: capability not negotiated");
                            continue;
                        }
                        // Prepare and forward the relayed signal (Spec Section 15)
                        let mut relay_signal = signal.clone();
                        match super::relay::prepare_relay(&mut relay_signal, &server_name) {
                            Ok(()) => {
                                let recipient = relay_signal.metadata
                                    .get("recipient")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown")
                                    .to_string();
                                if let Some(ref registry) = peer_registry {
                                    let peer_addr = {
                                        let pr = registry.read().unwrap();
                                        pr.get(&recipient).map(|p| p.addr.clone())
                                    };
                                    if let Some(addr) = peer_addr {
                                        tracing::info!(
                                            peer = %peer_id,
                                            recipient = %recipient,
                                            address = %addr,
                                            hop_count = relay_signal.metadata.get("hop_count")
                                                .and_then(serde_json::Value::as_u64).unwrap_or(0),
                                            "Relaying signal to peer (via SynapseClient)"
                                        );
                                        // Forward the signal to the target peer
                                        let target_addr = addr.clone();
                                        let sender_name = server_name.clone();
                                        let relay_signal = relay_signal.clone();
                                        tokio::spawn(async move {
                                            match crate::protocol::client::SynapseClient::send(
                                                &target_addr,
                                                &sender_name,
                                                &relay_signal,
                                            ).await {
                                                Ok(Some(response)) => {
                                                    tracing::info!(
                                                        target = %target_addr,
                                                        response_type = ?response.signal_type,
                                                        "Relay forwarded and response received"
                                                    );
                                                    // TODO: route response back to original sender
                                                }
                                                Ok(None) => {
                                                    tracing::info!(target = %target_addr, "Relay forwarded (no response)");
                                                }
                                                Err(e) => {
                                                    tracing::warn!(
                                                        target = %target_addr,
                                                        error = %e,
                                                        "Relay forwarding failed"
                                                    );
                                                }
                                            }
                                        });
                                    } else {
                                        tracing::warn!(
                                            peer = %peer_id,
                                            recipient = %recipient,
                                            "Relay target not found in peer registry"
                                        );
                                    }
                                } else {
                                    tracing::warn!(
                                        peer = %peer_id,
                                        recipient = %recipient,
                                        "Cannot relay: no peer registry configured"
                                    );
                                }
                            }
                            Err(reason) => {
                                tracing::warn!(
                                    peer = %peer_id,
                                    reason = %reason,
                                    "Relay preparation failed"
                                );
                            }
                        }
                        continue;
                    }

                    // Dispatch to handler
                    if let Some(mut response) = handler.handle(signal) {
                        response.sequence = conn.next_sequence();
                        if let Some(ref m) = metrics {
                            m.record_signal_sent(response.payload.len() as u64);
                        }
                        if let Err(e) = conn.send(&response).await {
                            tracing::warn!(
                                peer = %peer_id,
                                error = %e,
                                "Failed to send response"
                            );
                            conn.mark_dead();
                            break;
                        }
                    }
                }

                // Clean up pub/sub subscriptions for this connection
                {
                    let mut ps = pubsub.lock().await;
                    ps.remove_connection(conn_id);
                }

                active_conns.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                tracing::debug!(peer = %addr, peer_id = %peer_id, "Connection handler exiting");
            });
        }
    }
}
