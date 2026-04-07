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

use super::connection::{
    SynapseConnection, DEFAULT_KEEPALIVE_INTERVAL, DEFAULT_MAX_MISSED_PONGS,
    DEFAULT_PONG_TIMEOUT,
};
use super::pubsub::PubSubManager;
use super::signal::{Signal, SignalFlags, SignalType};

/// Handler trait for incoming signals. Implementations decide how to
/// respond to each signal type.
pub trait SignalHandler: Send + Sync {
    fn handle(&self, signal: Signal) -> Option<Signal>;
}

/// A simple handler that responds to Ping with Pong and logs everything else.
pub struct DefaultHandler {
    pub name: String,
}

impl SignalHandler for DefaultHandler {
    fn handle(&self, signal: Signal) -> Option<Signal> {
        match signal.signal_type {
            SignalType::Ping => Some(Signal::pong(&self.name, signal.sequence)),
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
                let intent_text = match String::from_utf8(signal.payload.clone()) {
                    Ok(text) => text,
                    Err(_) => {
                        return Some(Signal::error(
                            &self.name,
                            "Invalid UTF-8 in intent payload",
                        ));
                    }
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
                            let output_str = match &result.output {
                                Some(val) => format!("{}", val),
                                None => "Done.".to_string(),
                            };
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
                            &format!("Inference error: {}", e),
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

/// Configuration for the SynapseServer.
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
}

impl SynapseServer {
    pub fn new(name: String, bind_addr: String) -> Self {
        Self {
            name,
            bind_addr,
            config: ServerConfig::default(),
            metrics: None,
        }
    }

    pub fn with_config(name: String, bind_addr: String, config: ServerConfig) -> Self {
        Self {
            name,
            bind_addr,
            config,
            metrics: None,
        }
    }

    pub fn with_metrics(mut self, metrics: Arc<crate::metrics::SomaMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Start listening for incoming connections. Runs until the task is cancelled.
    pub async fn start(
        &self,
        handler: impl SignalHandler + Send + Sync + 'static,
    ) -> Result<()> {
        let listener = TcpListener::bind(&self.bind_addr).await?;
        tracing::info!(
            bind = %self.bind_addr,
            name = %self.name,
            "Synaptic Protocol v2 server started (binary wire format)"
        );

        let handler = Arc::new(handler);
        let pubsub = Arc::new(tokio::sync::Mutex::new(PubSubManager::new()));
        let active_connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let connection_counter = Arc::new(std::sync::atomic::AtomicU64::new(1));
        let max_conn_limit: usize = self.config.max_connections;

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
            let active_conns = active_connections.clone();
            let conn_id = connection_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let metrics = self.metrics.clone();

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
                let cap_refs: Vec<&str> = capabilities.iter().map(|s| s.as_str()).collect();
                let plug_refs: Vec<&str> = plugins.iter().map(|s| s.as_str()).collect();
                let peer_id = match conn.server_handshake(&cap_refs, &plug_refs).await {
                    Ok(id) => id,
                    Err(e) => {
                        tracing::warn!(peer = %addr, error = %e, "Handshake failed");
                        active_conns.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                        return;
                    }
                };

                tracing::info!(peer = %addr, peer_id = %peer_id, "Handshake successful");

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

                // Main signal processing loop
                loop {
                    if !conn.is_alive() {
                        tracing::info!(peer = %peer_id, "Connection marked dead, closing");
                        break;
                    }

                    let signal = match conn.recv().await {
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
                    };

                    tracing::debug!(
                        signal_type = ?signal.signal_type,
                        sender = %signal.sender_id,
                        seq = signal.sequence,
                        channel = signal.channel_id,
                        "Received signal"
                    );

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
                                .and_then(|v| v.as_bool()).unwrap_or(false);
                            let mut ps = pubsub.lock().await;
                            let last_seen = signal.metadata.get("last_seen_sequence")
                                .and_then(|v| v.as_u64()).map(|v| v as u32);
                            ps.subscribe(
                                &topic,
                                signal.channel_id,
                                conn_id,
                                last_seen,
                                durable,
                            );
                            tracing::info!(peer = %peer_id, topic = %topic, conn_id, "Subscribed");
                            continue;
                        }
                        SignalType::Unsubscribe => {
                            let topic = String::from_utf8_lossy(&signal.payload).to_string();
                            let mut ps = pubsub.lock().await;
                            ps.unsubscribe(&topic, conn_id);
                            tracing::info!(peer = %peer_id, topic = %topic, "Unsubscribed");
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

                    // Relay capability gating (Sec 12.4)
                    if super::relay::should_relay(&signal, &server_name) {
                        if !conn.is_capability_negotiated("relay").await {
                            tracing::warn!(peer = %peer_id, "Relay rejected: capability not negotiated");
                            continue;
                        }
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
