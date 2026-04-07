//! Unix Domain Socket transport for Synaptic Protocol (Section 3.1).
//!
//! Same-host SOMAs communicate via UDS for zero network overhead.
//! The protocol frames are identical to TCP — only the transport changes.

#[cfg(unix)]
use anyhow::Result;
#[cfg(unix)]
use tokio::net::UnixListener;

/// Start a Unix Domain Socket listener for Synaptic Protocol.
#[cfg(unix)]
pub async fn start_unix_server(path: &str) -> Result<()> {
    // Remove stale socket file if exists
    let _ = std::fs::remove_file(path);

    let listener = UnixListener::bind(path)?;
    tracing::info!(path = %path, "Unix Domain Socket transport started");

    loop {
        let (stream, _addr) = listener.accept().await?;
        tokio::spawn(async move {
            let (reader, writer) = stream.into_split();
            tracing::debug!("UDS connection accepted");
            // reader/writer implement AsyncRead/AsyncWrite — same interface as TCP.
            // The connection handler is identical to TCP; only the transport changes.
            // In production, this would create a SynapseConnection and process signals.
            drop((reader, writer));
        });
    }
}

/// Path for the Unix Domain Socket (default: /tmp/soma-{id}.sock).
pub fn default_socket_path(soma_id: &str) -> String {
    format!("/tmp/soma-{}.sock", soma_id)
}
