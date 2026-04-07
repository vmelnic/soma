//! Synaptic Protocol server — TCP listener for inter-SOMA communication.

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use std::sync::Arc;

use super::signal::{Signal, SignalType};
use crate::mind::MindEngine;

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
            SignalType::Ping => {
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

/// Handler that routes Intent signals to the mind engine and plugin manager.
/// This enables inter-SOMA intent-to-result communication.
pub struct SomaSignalHandler {
    pub name: String,
    pub mind: Arc<std::sync::RwLock<crate::mind::onnx_engine::OnnxMindEngine>>,
    pub plugins: Arc<crate::plugin::manager::PluginManager>,
    pub max_program_steps: usize,
}

impl SignalHandler for SomaSignalHandler {
    fn handle(&self, signal: Signal) -> Option<Signal> {
        match signal.signal_type {
            SignalType::Ping => {
                Some(Signal::pong(&self.name, &signal.sender))
            }
            SignalType::Intent => {
                // Extract intent text from payload
                let intent_text = match String::from_utf8(signal.payload.clone()) {
                    Ok(text) => text,
                    Err(_) => {
                        let mut resp = Signal::new(SignalType::Error, self.name.clone(), signal.sender.clone());
                        resp.payload = b"Invalid UTF-8 in intent payload".to_vec();
                        return Some(resp);
                    }
                };

                // Propagate trace_id from incoming signal (Section 18.1.2)
                let trace_id = if signal.trace_id.is_empty() {
                    uuid::Uuid::new_v4().to_string()[..12].to_string()
                } else {
                    signal.trace_id.clone()
                };

                tracing::info!(
                    component = "router",
                    trace_id = %trace_id,
                    sender = %signal.sender,
                    intent = %intent_text,
                    "Processing remote intent"
                );

                // Run inference via the mind engine
                let mind_guard = self.mind.read().unwrap();
                match mind_guard.infer(&intent_text) {
                    Ok(program) => {
                        let result = self.plugins.execute_program(&program.steps, self.max_program_steps);

                        tracing::info!(
                            component = "mind",
                            trace_id = %trace_id,
                            steps = program.steps.len(),
                            confidence = %program.confidence,
                            success = result.success,
                            "Remote intent processed"
                        );

                        let mut resp = Signal::new(SignalType::Data, self.name.clone(), signal.sender.clone());
                        // Propagate trace_id in response
                        resp.trace_id = trace_id;

                        if result.success {
                            // Serialize the output as the response payload
                            let output_str = match &result.output {
                                Some(val) => format!("{}", val),
                                None => "Done.".to_string(),
                            };
                            resp.payload = output_str.into_bytes();
                        } else {
                            resp.signal_type = SignalType::Error;
                            resp.payload = result.error.unwrap_or_else(|| "unknown error".into()).into_bytes();
                        }

                        Some(resp)
                    }
                    Err(e) => {
                        let mut resp = Signal::new(SignalType::Error, self.name.clone(), signal.sender.clone());
                        resp.payload = format!("Inference error: {}", e).into_bytes();
                        Some(resp)
                    }
                }
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
