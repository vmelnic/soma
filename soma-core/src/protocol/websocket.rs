//! WebSocket transport adapter for Synaptic Protocol (Section 3.1).
//!
//! Browser-based renderers cannot open raw TCP connections.
//! This adapter wraps/unwraps Synaptic Protocol frames inside
//! WebSocket binary messages.

use anyhow::Result;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;

/// Start a WebSocket server that wraps Synaptic Protocol.
/// Each incoming WS connection is bridged to the signal handler.
pub async fn start_ws_server(bind_addr: &str) -> Result<()> {
    let listener = TcpListener::bind(bind_addr).await?;
    tracing::info!(bind = %bind_addr, "WebSocket transport started");

    loop {
        let (stream, addr) = listener.accept().await?;
        tokio::spawn(async move {
            match accept_async(stream).await {
                Ok(ws_stream) => {
                    tracing::info!(peer = %addr, "WebSocket connection accepted");
                    // Bridge: WS binary messages <-> Synaptic Protocol frames
                    // Each WS binary message contains exactly one Synaptic frame
                    handle_ws_connection(ws_stream, addr).await;
                }
                Err(e) => {
                    tracing::warn!(peer = %addr, error = %e, "WebSocket handshake failed");
                }
            }
        });
    }
}

async fn handle_ws_connection(
    ws_stream: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    addr: std::net::SocketAddr,
) {
    use futures_util::{StreamExt, SinkExt};
    use tokio_tungstenite::tungstenite::Message;

    let (mut write, mut read) = ws_stream.split();

    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Binary(data)) => {
                // data is a complete Synaptic Protocol frame
                match super::codec::decode_frame(&data) {
                    Ok(Some(signal)) => {
                        tracing::debug!(
                            peer = %addr,
                            signal_type = ?signal.signal_type,
                            "WS frame received"
                        );
                        // Process signal and optionally send response
                        // In production, this would route through SignalRouter
                    }
                    Ok(None) => {
                        tracing::debug!(peer = %addr, "WS frame with unknown signal type, ignoring");
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
                let _ = write.send(Message::Pong(data)).await;
            }
            Err(e) => {
                tracing::warn!(peer = %addr, error = %e, "WebSocket error");
                break;
            }
            _ => {} // ignore text messages
        }
    }
}
