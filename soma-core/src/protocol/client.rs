//! Synaptic Protocol v2 client — binary wire format with handshake
//! and auto-reconnect with exponential backoff (Spec Sections 12, 14).

use anyhow::{bail, Result};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;

use super::connection::SynapseConnection;
use super::router::SignalRouter;
use super::signal::{Signal, SignalType};

/// Backoff schedule for auto-reconnect (Spec Section 14.2):
/// 100ms, 500ms, 2s, 5s, then exponential up to 60s.
const BACKOFF_SCHEDULE: &[Duration] = &[
    Duration::from_millis(100),
    Duration::from_millis(500),
    Duration::from_secs(2),
    Duration::from_secs(5),
];
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Record of an active subscription, kept for replay on reconnect (Section 14.3).
#[derive(Clone)]
pub struct SubscriptionRecord {
    pub topic: String,
    pub channel_id: u32,
    pub durable: bool,
    pub last_seen_sequence: Option<u32>,
}

/// Client for sending signals to peer SOMA nodes using the binary
/// Synaptic Protocol v2 wire format.
pub struct SynapseClient {
    pub local_id: String,
    pub capabilities: Vec<String>,
    pub plugins: Vec<String>,
    pub max_signal_size: u32,
    /// Active subscriptions for replay on reconnect (Section 14.3).
    active_subscriptions: Vec<SubscriptionRecord>,
    /// Session token from last successful handshake, carried forward on reconnect (Section 14.5).
    session_token: Option<String>,
}

impl SynapseClient {
    /// Create a new client with the given SOMA ID.
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

    /// Connect to a peer, perform handshake, and return the connection.
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

        // Carry forward session token from previous connection (Section 14.5)
        if let Some(ref token) = self.session_token {
            *conn.session_token.lock().await = token.clone();
        }

        // Perform client-side handshake
        let cap_refs: Vec<&str> = self.capabilities.iter().map(|s| s.as_str()).collect();
        let plug_refs: Vec<&str> = self.plugins.iter().map(|s| s.as_str()).collect();
        let peer_id = conn.client_handshake(&cap_refs, &plug_refs).await?;

        // Store the session token received from the server
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

    /// Connect with auto-reconnect. Retries with exponential backoff
    /// per Spec Section 14.2. Returns the connection on success.
    pub async fn connect_with_retry(&mut self, addr: &str) -> Result<Arc<SynapseConnection>> {
        let mut attempt = 0u32;

        loop {
            match self.connect(addr).await {
                Ok(conn) => {
                    // Replay subscriptions on reconnect (Spec Section 14.3)
                    if !self.active_subscriptions.is_empty() {
                        self.replay_subscriptions(&conn).await?;
                    }
                    return Ok(conn);
                }
                Err(e) => {
                    let delay = if (attempt as usize) < BACKOFF_SCHEDULE.len() {
                        BACKOFF_SCHEDULE[attempt as usize]
                    } else {
                        // Exponential backoff: 5s * 2^(attempt - 3), capped at 60s
                        let extra = attempt as u32 - BACKOFF_SCHEDULE.len() as u32;
                        let secs = 5u64.saturating_mul(1u64 << extra.min(10));
                        Duration::from_secs(secs).min(MAX_BACKOFF)
                    };

                    tracing::warn!(
                        addr = %addr,
                        attempt = attempt + 1,
                        error = %e,
                        retry_in_ms = delay.as_millis() as u64,
                        "Connection failed, retrying"
                    );

                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
            }
        }
    }

    /// Send a single signal to a peer (connect, handshake, send, receive response, disconnect).
    /// This is the simple one-shot API for sending intents.
    pub async fn send(addr: &str, local_id: &str, signal: &Signal) -> Result<Option<Signal>> {
        let mut client = SynapseClient::new(local_id.to_string());
        let conn = client.connect(addr).await?;

        // Send the signal
        conn.send(signal).await?;

        // Try to read a response
        match conn.recv().await {
            Ok(response) => {
                // Send CLOSE
                let close = Signal::close(local_id);
                let _ = conn.send(&close).await;
                conn.mark_dead();
                Ok(Some(response))
            }
            Err(_) => {
                conn.mark_dead();
                Ok(None)
            }
        }
    }

    /// Send a ping to a peer and return whether it responded with a Pong.
    pub async fn ping(addr: &str, sender: &str) -> Result<bool> {
        let mut signal = Signal::ping(sender);
        signal.channel_id = 0;
        match Self::send(addr, sender, &signal).await? {
            Some(resp) => Ok(resp.signal_type == SignalType::Pong),
            None => Ok(false),
        }
    }

    /// Send an intent to a peer and wait for the response with timeout (Section 14.3).
    /// Uses the SignalRouter for request-response correlation by sequence number.
    pub async fn send_intent_and_wait(
        addr: &str,
        sender: &str,
        text: &str,
        router: &SignalRouter,
    ) -> Result<Signal> {
        let mut intent = Signal::new(SignalType::Intent, sender.to_string());
        intent.payload = text.as_bytes().to_vec();

        // Register pending before sending
        let sequence = intent.sequence;
        let rx = router.register_pending(sequence)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        // Send the intent and deliver the response to the router
        let response = Self::send(addr, sender, &intent).await?;
        if let Some(resp) = response {
            router.deliver_response(sequence, resp);
        }

        // Wait for correlated response
        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => bail!("Response channel closed"),
            Err(_) => {
                router.cancel(sequence);
                bail!("Intent response timed out after 30s")
            }
        }
    }

    /// Send an Intent signal and wait for the Result/Error response.
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

    /// Track a subscription for replay on reconnect.
    pub fn track_subscription(&mut self, topic: String, channel_id: u32, durable: bool) {
        self.active_subscriptions.push(SubscriptionRecord {
            topic,
            channel_id,
            durable,
            last_seen_sequence: None,
        });
    }

    /// Remove a tracked subscription.
    pub fn untrack_subscription(&mut self, topic: &str) {
        self.active_subscriptions.retain(|s| s.topic != topic);
    }

    /// Update the last seen sequence number for a tracked subscription.
    /// Should be called when a signal is received on a subscribed topic,
    /// so that replay on reconnect can resume from the correct point.
    pub fn update_subscription_sequence(&mut self, topic: &str, sequence: u32) {
        if let Some(sub) = self.active_subscriptions.iter_mut().find(|s| s.topic == topic) {
            sub.last_seen_sequence = Some(sequence);
        }
    }

    /// Replay all tracked subscriptions on a connection (after reconnect).
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
