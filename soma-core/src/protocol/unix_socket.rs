//! Unix Domain Socket transport for Synaptic Protocol (Section 3.1).
//!
//! Same-host SOMAs communicate via UDS for zero network overhead.
//! The protocol frames are identical to TCP — only the transport changes.
//!
//! Signal routing: after reading and decoding a Synaptic Protocol frame,
//! the handler responds to PING (with PONG) and logs all other signal
//! types. Full SignalRouter integration is deferred.

#[cfg(unix)]
use anyhow::Result;
#[cfg(unix)]
use tokio::io::AsyncReadExt;
#[cfg(unix)]
use tokio::net::UnixListener;

use super::codec;
use super::signal::{Signal, SignalType};

/// Start a Unix Domain Socket listener for Synaptic Protocol.
///
/// On startup, removes any stale socket file at `path`. On shutdown
/// (when the accept loop ends or the task is cancelled), the caller
/// should invoke [`cleanup_socket`] to remove the file.
#[cfg(unix)]
pub async fn start_unix_server(path: &str) -> Result<()> {
    // Remove stale socket file if exists
    let _ = std::fs::remove_file(path);

    let listener = UnixListener::bind(path)?;
    tracing::info!(path = %path, "Unix Domain Socket transport started");

    loop {
        let (stream, _addr) = listener.accept().await?;
        tokio::spawn(async move {
            tracing::debug!("UDS connection accepted");
            handle_uds_connection(stream).await;
            tracing::debug!("UDS connection handler exiting");
        });
    }
}

/// Handle a single UDS connection: read frames, decode signals, respond.
#[cfg(unix)]
async fn handle_uds_connection(stream: tokio::net::UnixStream) {
    let (reader, writer) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(reader);
    let mut writer = writer;

    // Maximum frame size consistent with the default (10 MB + overhead)
    let max_frame_size = codec::DEFAULT_MAX_FRAME_SIZE;

    loop {
        // Read a complete Synaptic Protocol frame
        let frame = match codec::read_frame(&mut reader, max_frame_size).await {
            Ok(f) => f,
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("unexpected eof")
                    || msg.contains("connection reset")
                    || msg.contains("broken pipe")
                {
                    tracing::debug!("UDS peer disconnected");
                } else {
                    tracing::warn!(error = %e, "UDS frame read error");
                }
                break;
            }
        };

        // Decode the frame into a Signal
        match codec::decode_frame(&frame, None) {
            Ok(Some(signal)) => {
                tracing::debug!(
                    signal_type = ?signal.signal_type,
                    sender = %signal.sender_id,
                    seq = signal.sequence,
                    "UDS signal received"
                );

                let response = match signal.signal_type {
                    SignalType::Ping => {
                        Some(Signal::pong("soma-uds", signal.sequence))
                    }
                    SignalType::Close => {
                        tracing::info!(sender = %signal.sender_id, "UDS peer sent CLOSE");
                        break;
                    }
                    _ => {
                        tracing::debug!(
                            signal_type = ?signal.signal_type,
                            "UDS signal received (no handler)"
                        );
                        None
                    }
                };

                if let Some(resp) = response {
                    if let Err(e) = codec::write_frame(&mut writer, &resp).await {
                        tracing::warn!(error = %e, "UDS failed to send response");
                        break;
                    }
                }
            }
            Ok(None) => {
                tracing::debug!("UDS frame with unknown signal type, ignoring");
            }
            Err(e) => {
                tracing::warn!(error = %e, "UDS invalid frame");
            }
        }
    }
}

/// Remove the socket file on shutdown.
#[cfg(unix)]
pub fn cleanup_socket(path: &str) {
    if let Err(e) = std::fs::remove_file(path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(path = %path, error = %e, "Failed to remove UDS socket file");
        }
    } else {
        tracing::debug!(path = %path, "UDS socket file removed");
    }
}

/// Path for the Unix Domain Socket (default: /tmp/soma-{id}.sock).
pub fn default_socket_path(soma_id: &str) -> String {
    format!("/tmp/soma-{}.sock", soma_id)
}
