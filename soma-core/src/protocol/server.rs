//! Synaptic Protocol server — TCP listener for inter-SOMA communication.

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use std::sync::Arc;

use super::signal::Signal;

/// Handler trait for incoming signals.
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
            super::signal::SignalType::Ping => {
                Some(Signal::pong(&self.name, &signal.sender))
            }
            _ => {
                tracing::debug!(
                    signal_type = ?signal.signal_type,
                    sender = %signal.sender,
                    "Received signal (no handler)"
                );
                None
            }
        }
    }
}

/// TCP server that accepts connections and dispatches signals to a handler.
pub struct SynapseServer {
    name: String,
    bind_addr: String,
}

impl SynapseServer {
    pub fn new(name: String, bind_addr: String) -> Self {
        Self { name, bind_addr }
    }

    /// Start listening for incoming connections. This runs until the task is cancelled.
    pub async fn start(&self, handler: impl SignalHandler + Send + Sync + 'static) -> Result<()> {
        let listener = TcpListener::bind(&self.bind_addr).await?;
        tracing::info!(
            bind = %self.bind_addr,
            name = %self.name,
            "Synaptic Protocol server started"
        );

        let handler = Arc::new(handler);

        loop {
            let (mut stream, addr) = listener.accept().await?;
            let handler = handler.clone();
            let server_name = self.name.clone();

            tokio::spawn(async move {
                tracing::debug!(peer = %addr, "Connection accepted");

                loop {
                    // Read 4-byte length prefix
                    let mut len_buf = [0u8; 4];
                    match stream.read_exact(&mut len_buf).await {
                        Ok(_) => {}
                        Err(_) => break, // connection closed
                    }
                    let msg_len = u32::from_be_bytes(len_buf) as usize;

                    // Sanity check on message size (max 16 MB)
                    if msg_len > 16 * 1024 * 1024 {
                        tracing::warn!(len = msg_len, "Signal too large, dropping connection");
                        break;
                    }

                    // Read the message body
                    let mut msg_buf = vec![0u8; msg_len];
                    match stream.read_exact(&mut msg_buf).await {
                        Ok(_) => {}
                        Err(_) => break,
                    }

                    // Parse and handle
                    match Signal::from_bytes(&msg_buf) {
                        Ok(signal) => {
                            if let Some(response) = handler.handle(signal) {
                                let resp_bytes = response.to_bytes();
                                if stream.write_all(&resp_bytes).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "Failed to parse signal");
                        }
                    }
                }

                tracing::debug!(peer = %addr, "Connection closed");
                let _ = server_name; // suppress unused warning
            });
        }
    }
}
