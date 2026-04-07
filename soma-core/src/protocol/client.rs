//! Synaptic Protocol v2 client — binary wire format with handshake
//! and auto-reconnect with exponential backoff (Spec Sections 12, 14).

use anyhow::{bail, Result};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;

use super::connection::SynapseConnection;
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

/// Client for sending signals to peer SOMA nodes using the binary
/// Synaptic Protocol v2 wire format.
pub struct SynapseClient {
    pub local_id: String,
    pub capabilities: Vec<String>,
    pub plugins: Vec<String>,
    pub max_signal_size: u32,
}

impl SynapseClient {
    /// Create a new client with the given SOMA ID.
    pub fn new(local_id: String) -> Self {
        Self {
            local_id,
            capabilities: vec!["streaming".into(), "chunked".into()],
            plugins: vec!["posix".into()],
            max_signal_size: 10_485_760,
        }
    }

    /// Connect to a peer, perform handshake, and return the connection.
    pub async fn connect(&self, addr: &str) -> Result<Arc<SynapseConnection>> {
        let stream = TcpStream::connect(addr).await?;
        let (reader, writer) = stream.into_split();

        let conn = Arc::new(SynapseConnection::new(
            self.local_id.clone(),
            String::new(),
            reader,
            writer,
            self.max_signal_size,
        ));

        // Perform client-side handshake
        let cap_refs: Vec<&str> = self.capabilities.iter().map(|s| s.as_str()).collect();
        let plug_refs: Vec<&str> = self.plugins.iter().map(|s| s.as_str()).collect();
        let peer_id = conn.client_handshake(&cap_refs, &plug_refs).await?;

        tracing::info!(
            peer = %addr,
            peer_id = %peer_id,
            "Connected and handshake complete"
        );

        Ok(conn)
    }

    /// Connect with auto-reconnect. Retries with exponential backoff
    /// per Spec Section 14.2. Returns the connection on success.
    pub async fn connect_with_retry(&self, addr: &str) -> Result<Arc<SynapseConnection>> {
        let mut attempt = 0u32;

        loop {
            match self.connect(addr).await {
                Ok(conn) => return Ok(conn),
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
        let client = SynapseClient::new(local_id.to_string());
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
}
