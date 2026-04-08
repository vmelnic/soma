//! Synaptic Protocol v2 client (Spec Sections 12, 14).
//!
//! Provides both one-shot and persistent connection modes for
//! inter-SOMA communication over the binary wire format:
//!
//! - **One-shot**: [`SynapseClient::send`] connects, handshakes, sends a single
//!   signal, reads the response, and disconnects.
//! - **Persistent**: [`SynapseClient::connect`] / [`SynapseClient::connect_with_retry`]
//!   establish a long-lived connection with auto-reconnect and subscription replay.
//!
//! Auto-reconnect uses a graduated backoff schedule (100 ms, 500 ms, 2 s, 5 s,
//! then exponential doubling capped at 60 s). On successful reconnect, all
//! tracked subscriptions are automatically replayed with their last-seen
//! sequence numbers so the peer can resume delivery without data loss.

use anyhow::{bail, Result};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;

use super::connection::SynapseConnection;
use super::router::SignalRouter;
use super::signal::{Signal, SignalType};

#[allow(dead_code)] // Spec Section 14.2 — used by connect_with_retry
/// Fixed portion of the backoff schedule; attempts beyond this use exponential doubling.
const BACKOFF_SCHEDULE: &[Duration] = &[
    Duration::from_millis(100),
    Duration::from_millis(500),
    Duration::from_secs(2),
    Duration::from_secs(5),
];
#[allow(dead_code)] // Spec Section 14.2 — used by connect_with_retry
/// Upper bound for the exponential portion of the backoff.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Snapshot of an active subscription, kept so it can be replayed on reconnect (Spec Section 14.3).
///
/// `last_seen_sequence` is updated as signals arrive, allowing the server
/// to resume delivery from the correct point after a reconnection.
#[derive(Clone)]
pub struct SubscriptionRecord {
    pub topic: String,
    pub channel_id: u32,
    /// If `true`, the server retains messages while the client is disconnected.
    pub durable: bool,
    /// Sequence number of the last signal received on this topic.
    pub last_seen_sequence: Option<u32>,
}

/// Client for sending signals to peer SOMA nodes over the Synaptic Protocol v2 wire format.
///
/// Holds connection-independent state (identity, capabilities, subscription
/// tracking, session token) that persists across reconnections.
pub struct SynapseClient {
    /// This node's SOMA identifier, sent during handshake.
    pub local_id: String,
    /// Protocol capabilities advertised during handshake (e.g. "streaming", "chunked").
    pub capabilities: Vec<String>,
    /// Plugin names advertised during handshake.
    pub plugins: Vec<String>,
    /// Maximum signal payload size in bytes (default 10 MiB).
    pub max_signal_size: u32,
    /// Subscriptions to replay after reconnect.
    active_subscriptions: Vec<SubscriptionRecord>,
    /// Session token from the last successful handshake, carried forward on reconnect
    /// to allow the server to restore session state without a full re-handshake.
    session_token: Option<String>,
}

impl SynapseClient {
    /// Create a new client with the given SOMA node ID and default capabilities.
    pub fn new(local_id: String) -> Self {
        Self {
            local_id,
            capabilities: vec!["streaming".into(), "chunked".into()],
            plugins: vec!["posix".into()],
            max_signal_size: 10_485_760,
            active_subscriptions: Vec::new(),
            session_token: None,
        }
    }

    /// Connect to a peer via TCP, perform the binary handshake, and return the connection.
    ///
    /// If a session token exists from a prior connection, it is sent to
    /// the server during handshake so session state can be restored.
    pub async fn connect(&mut self, addr: &str) -> Result<Arc<SynapseConnection>> {
        let stream = TcpStream::connect(addr).await?;
        let (reader, writer) = stream.into_split();

        let conn = Arc::new(SynapseConnection::new(
            self.local_id.clone(),
            String::new(),
            reader,
            writer,
            self.max_signal_size,
        ));

        // Carry forward session token so the server can restore prior state
        if let Some(ref token) = self.session_token {
            *conn.session_token.lock().await = token.clone();
        }

        // Exchange capabilities and establish session
        let cap_refs: Vec<&str> = self.capabilities.iter().map(std::string::String::as_str).collect();
        let plug_refs: Vec<&str> = self.plugins.iter().map(std::string::String::as_str).collect();
        let peer_id = conn.client_handshake(&cap_refs, &plug_refs).await?;

        // Persist the server-issued token for future reconnects
        let new_token = conn.session_token.lock().await.clone();
        if !new_token.is_empty() {
            self.session_token = Some(new_token);
        }

        tracing::info!(
            peer = %addr,
            peer_id = %peer_id,
            "Connected and handshake complete"
        );

        Ok(conn)
    }

    #[allow(dead_code)] // Spec Section 14.2 — auto-reconnect with backoff
    /// Connect with unlimited retries using graduated exponential backoff.
    ///
    /// The first four attempts use the fixed [`BACKOFF_SCHEDULE`]; subsequent
    /// attempts double from 5 s up to [`MAX_BACKOFF`] (60 s). On success,
    /// any tracked subscriptions are automatically replayed.
    pub async fn connect_with_retry(&mut self, addr: &str) -> Result<Arc<SynapseConnection>> {
        let mut attempt = 0u32;

        loop {
            match self.connect(addr).await {
                Ok(conn) => {
                    if !self.active_subscriptions.is_empty() {
                        self.replay_subscriptions(&conn).await?;
                    }
                    return Ok(conn);
                }
                Err(e) => {
                    #[allow(clippy::cast_possible_truncation)] // attempt count is small
                    let delay = if (attempt as usize) < BACKOFF_SCHEDULE.len() {
                        BACKOFF_SCHEDULE[attempt as usize]
                    } else {
                        // Exponential: 5 s * 2^(attempt - schedule_len), capped at MAX_BACKOFF
                        #[allow(clippy::cast_possible_truncation)] // schedule len fits in u32
                        let extra = attempt - BACKOFF_SCHEDULE.len() as u32;
                        let secs = 5u64.saturating_mul(1u64 << extra.min(10));
                        Duration::from_secs(secs).min(MAX_BACKOFF)
                    };

                    #[allow(clippy::cast_possible_truncation)] // delay ≤ 60s, fits in u64
                    let retry_in_ms = delay.as_millis() as u64;
                    tracing::warn!(
                        addr = %addr,
                        attempt = attempt + 1,
                        error = %e,
                        retry_in_ms,
                        "Connection failed, retrying"
                    );

                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
            }
        }
    }

    /// One-shot send: connect, handshake, send a signal, read one response, disconnect.
    pub async fn send(addr: &str, local_id: &str, signal: &Signal) -> Result<Option<Signal>> {
        let mut client = Self::new(local_id.to_string());
        let conn = client.connect(addr).await?;

        conn.send(signal).await?;

        if let Ok(response) = conn.recv().await {
            let close = Signal::close(local_id);
            let _ = conn.send(&close).await;
            conn.mark_dead();
            Ok(Some(response))
        } else {
            conn.mark_dead();
            Ok(None)
        }
    }

    /// Send a Ping and return `true` if the peer responds with a Pong.
    #[allow(dead_code)] // Spec feature for peer health checking
    pub async fn ping(addr: &str, sender: &str) -> Result<bool> {
        let mut signal = Signal::ping(sender);
        signal.channel_id = 0;
        match Self::send(addr, sender, &signal).await? {
            Some(resp) => Ok(resp.signal_type == SignalType::Pong),
            None => Ok(false),
        }
    }

    /// Send an intent and wait up to 30 s for a correlated response via the [`SignalRouter`].
    #[allow(dead_code)] // Spec feature for correlated intent-result
    pub async fn send_intent_and_wait(
        addr: &str,
        sender: &str,
        text: &str,
        router: &SignalRouter,
    ) -> Result<Signal> {
        let mut intent = Signal::new(SignalType::Intent, sender.to_string());
        intent.payload = text.as_bytes().to_vec();

        // Register the expected response before sending to avoid races
        let sequence = intent.sequence;
        let rx = router.register_pending(sequence)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        // One-shot send; response is routed by sequence number
        let response = Self::send(addr, sender, &intent).await?;
        if let Some(resp) = response {
            router.deliver_response(sequence, resp);
        }

        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => bail!("Response channel closed"),
            Err(_) => {
                router.cancel(sequence);
                bail!("Intent response timed out after 30s")
            }
        }
    }

    /// One-shot intent send without router-based correlation.
    #[allow(dead_code)] // Spec feature for inter-SOMA communication
    pub async fn send_intent(
        addr: &str,
        sender: &str,
        intent_text: &str,
    ) -> Result<Signal> {
        let mut signal = Signal::new(SignalType::Intent, sender.to_string());
        signal.channel_id = 1; // default data channel
        signal.payload = intent_text.as_bytes().to_vec();

        match Self::send(addr, sender, &signal).await? {
            Some(resp) => Ok(resp),
            None => bail!("No response received from peer"),
        }
    }

    /// Register a subscription so it will be replayed on reconnect.
    #[allow(dead_code)] // Spec feature for subscription replay
    pub fn track_subscription(&mut self, topic: String, channel_id: u32, durable: bool) {
        self.active_subscriptions.push(SubscriptionRecord {
            topic,
            channel_id,
            durable,
            last_seen_sequence: None,
        });
    }

    /// Stop tracking a subscription (it will not be replayed on reconnect).
    #[allow(dead_code)] // Spec feature for subscription replay
    pub fn untrack_subscription(&mut self, topic: &str) {
        self.active_subscriptions.retain(|s| s.topic != topic);
    }

    /// Advance the last-seen sequence for a subscription so reconnect replay resumes correctly.
    #[allow(dead_code)] // Spec feature for subscription replay
    pub fn update_subscription_sequence(&mut self, topic: &str, sequence: u32) {
        if let Some(sub) = self.active_subscriptions.iter_mut().find(|s| s.topic == topic) {
            sub.last_seen_sequence = Some(sequence);
        }
    }

    /// Re-subscribe to all tracked topics on a freshly established connection.
    ///
    /// Each subscription signal includes `last_seen_sequence` (when available)
    /// so the server can catch up the client from the correct point.
    #[allow(dead_code)] // Spec feature for subscription replay
    pub async fn replay_subscriptions(&self, conn: &Arc<SynapseConnection>) -> Result<()> {
        for sub in &self.active_subscriptions {
            let mut signal = Signal::new(SignalType::Subscribe, self.local_id.clone());
            signal.payload = sub.topic.as_bytes().to_vec();
            if let serde_json::Value::Object(ref mut map) = signal.metadata {
                map.insert("topic".to_string(), serde_json::json!(sub.topic));
                map.insert("durable".to_string(), serde_json::json!(sub.durable));
                if let Some(seq) = sub.last_seen_sequence {
                    map.insert("last_seen_sequence".to_string(), serde_json::json!(seq));
                }
            }
            signal.channel_id = sub.channel_id;
            conn.send(&signal).await?;
            tracing::debug!(topic = %sub.topic, "Replayed subscription on reconnect");
        }
        Ok(())
    }
}
