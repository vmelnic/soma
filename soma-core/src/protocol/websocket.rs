//! WebSocket transport adapter for Synaptic Protocol (Section 3.1).
//!
//! Browser-based renderers cannot open raw TCP connections.
//! This adapter wraps/unwraps Synaptic Protocol frames inside
//! WebSocket binary messages.
//!
//! Signal routing: after decoding a Synaptic Protocol frame, the
//! handler responds to HANDSHAKE (with `HANDSHAKE_ACK`), PING (with
//! PONG), and INTENT (with an acknowledgment DATA signal). Other
//! signal types are logged but not routed — full `SignalRouter`
//! integration is deferred until the TCP server handler is
//! refactored into a shared trait.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;

use super::codec;
use super::signal::{Signal, SignalType};

/// Maximum number of concurrent WebSocket connections.
#[allow(dead_code)] // Used by start_ws_server
const DEFAULT_MAX_WS_CONNECTIONS: usize = 64;

/// Start a WebSocket server that wraps Synaptic Protocol.
/// Each incoming WS connection is bridged to a minimal signal handler.
#[allow(dead_code)] // Spec feature for browser-based renderers
pub async fn start_ws_server(bind_addr: &str) -> Result<()> {
    start_ws_server_with_limit(bind_addr, DEFAULT_MAX_WS_CONNECTIONS).await
}

/// Start a WebSocket server with an explicit connection limit.
#[allow(dead_code)] // Spec feature for browser-based renderers
pub async fn start_ws_server_with_limit(bind_addr: &str, max_connections: usize) -> Result<()> {
    let listener = TcpListener::bind(bind_addr).await?;
    tracing::info!(bind = %bind_addr, max_connections, "WebSocket transport started");

    let active_connections = Arc::new(AtomicUsize::new(0));

    loop {
        let (stream, addr) = listener.accept().await?;

        // Enforce connection limit
        let current = active_connections.load(Ordering::Relaxed);
        if current >= max_connections {
            tracing::warn!(
                peer = %addr,
                active = current,
                limit = max_connections,
                "WebSocket connection rejected: limit reached"
            );
            drop(stream);
            continue;
        }
        active_connections.fetch_add(1, Ordering::Relaxed);

        let active_conns = active_connections.clone();
        tokio::spawn(async move {
            match accept_async(stream).await {
                Ok(ws_stream) => {
                    tracing::info!(peer = %addr, "WebSocket connection accepted");
                    handle_ws_connection(ws_stream, addr).await;
                }
                Err(e) => {
                    tracing::warn!(peer = %addr, error = %e, "WebSocket handshake failed");
                }
            }
            active_conns.fetch_sub(1, Ordering::Relaxed);
        });
    }
}

#[allow(dead_code)] // Called by start_ws_server_with_limit
#[allow(clippy::too_many_lines)]
async fn handle_ws_connection(
    ws_stream: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    addr: std::net::SocketAddr,
) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let (mut write, mut read) = ws_stream.split();
    let server_id = "soma-ws";
    let mut handshake_done = false;

    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Binary(data)) => {
                // data is a complete Synaptic Protocol frame
                match codec::decode_frame(&data, None) {
                    Ok(Some(signal)) => {
                        tracing::debug!(
                            peer = %addr,
                            signal_type = ?signal.signal_type,
                            sender = %signal.sender_id,
                            "WS frame received"
                        );

                        // Handshake gate: first message must be HANDSHAKE
                        if !handshake_done {
                            if signal.signal_type != SignalType::Handshake {
                                tracing::warn!(
                                    peer = %addr,
                                    signal_type = ?signal.signal_type,
                                    "Expected HANDSHAKE as first signal, closing"
                                );
                                let err = Signal::error(server_id, "handshake_required");
                                let frame = codec::encode_frame(&err, None);
                                let _ = write.send(Message::Binary(frame)).await;
                                break;
                            }

                            // Send HANDSHAKE_ACK
                            let caps = vec!["streaming".to_string()];
                            let ack = Signal::handshake_ack(server_id, &caps, 10_485_760);
                            let frame = codec::encode_frame(&ack, None);
                            if let Err(e) = write.send(Message::Binary(frame)).await {
                                tracing::warn!(peer = %addr, error = %e, "Failed to send HANDSHAKE_ACK");
                                break;
                            }
                            tracing::info!(
                                peer = %addr,
                                remote_id = %signal.sender_id,
                                "WS handshake completed"
                            );
                            handshake_done = true;
                            continue;
                        }

                        // Route known signal types
                        let response = match signal.signal_type {
                            SignalType::Ping => {
                                Some(Signal::pong(server_id, signal.sequence))
                            }
                            SignalType::Intent => {
                                let intent_text =
                                    String::from_utf8_lossy(&signal.payload).to_string();
                                tracing::info!(
                                    peer = %addr,
                                    intent = %intent_text,
                                    "WS intent received"
                                );
                                // Acknowledge with a DATA signal containing the
                                // received intent. Full mind-engine routing is
                                // deferred until the handler trait is shared.
                                let mut ack =
                                    Signal::new(SignalType::Data, server_id.to_string());
                                ack.payload =
                                    format!("ack:intent:{intent_text}").into_bytes();
                                ack.channel_id = signal.channel_id;
                                ack.trace_id = signal.effective_trace_id();
                                Some(ack)
                            }
                            SignalType::Close => {
                                tracing::info!(peer = %addr, "Peer sent CLOSE");
                                break;
                            }
                            _ => {
                                tracing::debug!(
                                    peer = %addr,
                                    signal_type = ?signal.signal_type,
                                    "WS signal received (no handler)"
                                );
                                None
                            }
                        };

                        if let Some(resp) = response {
                            let frame = codec::encode_frame(&resp, None);
                            if let Err(e) = write.send(Message::Binary(frame)).await {
                                tracing::warn!(
                                    peer = %addr,
                                    error = %e,
                                    "Failed to send WS response"
                                );
                                break;
                            }
                        }
                    }
                    Ok(None) => {
                        tracing::debug!(
                            peer = %addr,
                            "WS frame with unknown signal type, ignoring"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(peer = %addr, error = %e, "Invalid WS frame");
                    }
                }
            }
            Ok(Message::Close(_)) => {
                tracing::info!(peer = %addr, "WebSocket closed");
                break;
            }
            Ok(Message::Ping(data)) => {
                // WebSocket-level ping/pong (not Synaptic Protocol level)
                let _ = write.send(Message::Pong(data)).await;
            }
            Err(e) => {
                tracing::warn!(peer = %addr, error = %e, "WebSocket error");
                break;
            }
            _ => {} // ignore text messages
        }
    }

    tracing::debug!(peer = %addr, "WebSocket connection handler exiting");
}
