//! soma-dump — Synaptic Protocol signal capture tool (Whitepaper Section 11.5).
//!
//! Connects to a SOMA's Synaptic Protocol server and captures signals
//! for debugging, monitoring, and analysis. Outputs signals as JSON lines.
//!
//! Usage:
//!   soma-dump <address>                  # Capture all signals
//!   soma-dump <address> --type intent    # Filter by signal type
//!   soma-dump <address> --channel 5      # Filter by channel

use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Parser;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

// Re-use protocol types from soma crate
// Since this is a separate binary, we inline the minimal codec needed.

#[derive(Parser)]
#[command(
    name = "soma-dump",
    about = "Synaptic Protocol signal capture tool"
)]
struct Cli {
    /// Address to connect to (e.g. 127.0.0.1:9999)
    address: String,

    /// Filter by signal type name (e.g. intent, result, data, ping)
    #[arg(long, short = 't')]
    signal_type: Option<String>,

    /// Filter by channel ID
    #[arg(long, short = 'c')]
    channel: Option<u32>,

    /// Output raw bytes instead of JSON
    #[arg(long)]
    raw: bool,

    /// Maximum number of signals to capture (0 = unlimited)
    #[arg(long, default_value_t = 0)]
    count: u64,

    /// SOMA ID to use for handshake
    #[arg(long, default_value = "soma-dump")]
    id: String,
}

const MAGIC: [u8; 2] = [0x53, 0x4D]; // "SM"

const fn signal_type_name(byte: u8) -> &'static str {
    match byte {
        0x01 => "handshake",
        0x02 => "handshake_ack",
        0x03 => "close",
        0x10 => "intent",
        0x11 => "result",
        0x20 => "data",
        0x21 => "binary",
        0x22 => "stream_start",
        0x23 => "stream_data",
        0x24 => "stream_end",
        0x30 => "chunk_start",
        0x31 => "chunk_data",
        0x32 => "chunk_end",
        0x33 => "chunk_ack",
        0x40 => "discover",
        0x41 => "discover_ack",
        0x42 => "peer_query",
        0x43 => "peer_list",
        0x50 => "subscribe",
        0x51 => "unsubscribe",
        0xF0 => "ping",
        0xF1 => "pong",
        0xFE => "error",
        0xFF => "control",
        _ => "unknown",
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    eprintln!("soma-dump: connecting to {} ...", cli.address);

    let mut stream = TcpStream::connect(&cli.address).await?;
    eprintln!("soma-dump: connected. Capturing signals (Ctrl+C to stop)");

    let mut captured: u64 = 0;
    let mut buf = vec![0u8; 64 * 1024];

    loop {
        // Read magic bytes
        let mut magic = [0u8; 2];
        match stream.read_exact(&mut magic).await {
            Ok(_) => {}
            Err(e) => {
                eprintln!("soma-dump: connection closed: {e}");
                break;
            }
        }

        if magic != MAGIC {
            eprintln!(
                "soma-dump: invalid magic: {:02X}{:02X} (expected 534D)",
                magic[0], magic[1]
            );
            continue;
        }

        // Read version + flags + signal_type
        let mut header = [0u8; 3];
        stream.read_exact(&mut header).await?;
        // header[0] is version (currently unused)
        let flags = header[1];
        let signal_type_byte = header[2];
        let signal_type = signal_type_name(signal_type_byte);

        // Read channel_id (4 bytes) + sequence (4 bytes)
        let mut ids = [0u8; 8];
        stream.read_exact(&mut ids).await?;
        let channel_id = u32::from_be_bytes([ids[0], ids[1], ids[2], ids[3]]);
        let sequence = u32::from_be_bytes([ids[4], ids[5], ids[6], ids[7]]);

        // Read sender_id length (1 byte) + sender_id
        let mut sid_len_buf = [0u8; 1];
        stream.read_exact(&mut sid_len_buf).await?;
        let sid_len = sid_len_buf[0] as usize;
        let mut sid_buf = vec![0u8; sid_len];
        stream.read_exact(&mut sid_buf).await?;
        let sender_id = String::from_utf8_lossy(&sid_buf).to_string();

        // Read metadata length (4 bytes) + metadata
        let mut meta_len_buf = [0u8; 4];
        stream.read_exact(&mut meta_len_buf).await?;
        let meta_len = u32::from_be_bytes(meta_len_buf) as usize;
        let mut meta_buf = vec![0u8; meta_len];
        stream.read_exact(&mut meta_buf).await?;

        // Read payload length (4 bytes) + payload
        let mut payload_len_buf = [0u8; 4];
        stream.read_exact(&mut payload_len_buf).await?;
        let payload_len = u32::from_be_bytes(payload_len_buf) as usize;
        let payload_len = payload_len.min(buf.len());
        stream.read_exact(&mut buf[..payload_len]).await?;

        // Read CRC32 (4 bytes)
        let mut crc_buf = [0u8; 4];
        stream.read_exact(&mut crc_buf).await?;

        // Apply filters
        if let Some(ref filter_type) = cli.signal_type
            && signal_type != filter_type.as_str() {
                continue;
            }
        if let Some(filter_channel) = cli.channel
            && channel_id != filter_channel {
                continue;
            }

        // Output
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        if cli.raw {
            let _ = std::io::stdout().write_all(&magic);
            let _ = std::io::stdout().write_all(&header);
            let _ = std::io::stdout().write_all(&ids);
            let _ = std::io::stdout().flush();
        } else {
            let payload_preview = if payload_len > 0 {
                let text = String::from_utf8_lossy(&buf[..payload_len.min(200)]);
                if payload_len > 200 {
                    format!("{text}... ({payload_len} bytes)")
                } else {
                    text.to_string()
                }
            } else {
                String::new()
            };

            let json = serde_json::json!({
                "timestamp_ms": timestamp,
                "type": signal_type,
                "type_byte": format!("0x{:02X}", signal_type_byte),
                "flags": flags,
                "channel_id": channel_id,
                "sequence": sequence,
                "sender_id": sender_id,
                "metadata_size": meta_len,
                "payload_size": payload_len,
                "payload_preview": payload_preview,
            });
            println!("{}", serde_json::to_string(&json).unwrap_or_default());
        }

        captured += 1;
        if cli.count > 0 && captured >= cli.count {
            eprintln!("soma-dump: captured {captured} signals, stopping");
            break;
        }
    }

    eprintln!("soma-dump: total captured: {captured} signals");
    Ok(())
}
