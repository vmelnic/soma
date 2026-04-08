//! Unix Domain Socket transport for the Synaptic Protocol.
//!
//! Provides zero-network-overhead IPC for SOMA instances on the same host.
//! The wire format is identical to TCP — only the transport layer differs.
//! Socket files are placed in `/tmp` by convention and cleaned up on
//! shutdown via [`cleanup_socket`].
//!
//! Signal routing is minimal: only `PING` (responded with `PONG`) and
//! `CLOSE` (terminates the connection) are handled. Full `SignalRouter`
//! integration is not yet wired.

#[cfg(unix)]
use anyhow::Result;
#[cfg(unix)]
use tokio::net::UnixListener;

use super::codec;
use super::signal::{Signal, SignalType};

/// Binds a Unix Domain Socket listener at `path` and spawns a task per connection.
///
/// Removes any stale socket file at `path` before binding. The caller should
/// invoke [`cleanup_socket`] on shutdown to remove the file.
#[cfg(unix)]
#[allow(dead_code)]
pub async fn start_unix_server(path: &str) -> Result<()> {
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

/// Reads Synaptic Protocol frames from a UDS connection in a loop,
/// responding to `PING` and terminating on `CLOSE` or I/O errors.
///
/// Disconnection errors (EOF, reset, broken pipe) are logged at debug
/// level since they represent normal peer departure.
#[cfg(unix)]
#[allow(dead_code)] // Called by start_unix_server
async fn handle_uds_connection(stream: tokio::net::UnixStream) {
    let (reader, writer) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(reader);
    let mut writer = writer;

    let max_frame_size = codec::DEFAULT_MAX_FRAME_SIZE;

    loop {
        let frame = match codec::read_frame(&mut reader, max_frame_size).await {
            Ok(f) => f,
            Err(e) => {
                let msg = e.to_string();
                // Normal disconnection patterns — not worth warning about
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

                if let Some(resp) = response
                    && let Err(e) = codec::write_frame(&mut writer, &resp).await {
                        tracing::warn!(error = %e, "UDS failed to send response");
                        break;
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

/// Removes the socket file at `path`, logging a warning on failure
/// (except `NotFound`, which is silently ignored).
#[cfg(unix)]
#[allow(dead_code)]
pub fn cleanup_socket(path: &str) {
    if let Err(e) = std::fs::remove_file(path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(path = %path, error = %e, "Failed to remove UDS socket file");
        }
    } else {
        tracing::debug!(path = %path, "UDS socket file removed");
    }
}

/// Returns the conventional socket path for a SOMA instance: `/tmp/soma-{id}.sock`.
#[allow(dead_code)]
pub fn default_socket_path(soma_id: &str) -> String {
    format!("/tmp/soma-{soma_id}.sock")
}
