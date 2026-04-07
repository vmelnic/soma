//! Synaptic Protocol client — sends signals to peer SOMA instances.

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::signal::Signal;

/// Client for sending signals to peer SOMA nodes.
pub struct SynapseClient;

impl SynapseClient {
    /// Send a signal to a peer at the given address.
    /// Returns the response signal if the peer sends one, or None if the
    /// connection closes without a response.
    pub async fn send(addr: &str, signal: &Signal) -> Result<Option<Signal>> {
        let mut stream = TcpStream::connect(addr).await?;

        // Write the signal as length-prefixed JSON
        let bytes = signal.to_bytes();
        stream.write_all(&bytes).await?;

        // Try to read a response (4-byte length prefix + body)
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(_) => return Ok(None), // no response
        }
        let msg_len = u32::from_be_bytes(len_buf) as usize;

        if msg_len > 16 * 1024 * 1024 {
            return Err(anyhow::anyhow!("Response too large: {} bytes", msg_len));
        }

        let mut msg_buf = vec![0u8; msg_len];
        stream.read_exact(&mut msg_buf).await?;

        let response = Signal::from_bytes(&msg_buf)?;
        Ok(Some(response))
    }

    /// Send a ping to a peer and return whether it responded with a Pong.
    pub async fn ping(addr: &str, sender: &str) -> Result<bool> {
        let signal = Signal::ping(sender);
        match Self::send(addr, &signal).await? {
            Some(resp) => Ok(resp.signal_type == super::signal::SignalType::Pong),
            None => Ok(false),
        }
    }
}
