//! Synaptic Protocol v2 binary wire codec (Spec Sections 4, 17).
//!
//! Frame layout:
//!   magic:          0x53 0x4D (2 bytes, "SM")
//!   version:        0x20 (1 byte, v2.0)
//!   flags:          u8 (1 byte)
//!   signal_type:    u8 (1 byte)
//!   channel_id:     u32 BE (4 bytes)
//!   sequence:       u32 BE (4 bytes)
//!   sender_id_len:  u8 (1 byte)
//!   sender_id:      [u8; sender_id_len]
//!   metadata_len:   u32 BE (4 bytes)
//!   metadata:       [u8; metadata_len] (MessagePack-encoded per spec)
//!   payload_len:    u32 BE (4 bytes)
//!   payload:        [u8; payload_len]
//!   checksum:       u32 BE CRC32 (4 bytes)

use anyhow::{bail, Result};
use crc32fast::Hasher;

use super::signal::{Signal, SignalFlags, SignalType};

/// Conditionally compress a payload using zstd level 3.
///
/// Skips compression if the payload is smaller than 256 bytes or if the
/// signal is `StreamData` (latency-sensitive). If the compressed output
/// is not smaller than the original, the original is returned unchanged.
fn maybe_compress(payload: &[u8], signal_type: SignalType) -> (Vec<u8>, bool) {
    if payload.len() < 256 {
        return (payload.to_vec(), false);
    }
    if signal_type == SignalType::StreamData {
        return (payload.to_vec(), false);
    }

    match zstd::encode_all(std::io::Cursor::new(payload), 3) {
        Ok(compressed) if compressed.len() < payload.len() => (compressed, true),
        _ => (payload.to_vec(), false),
    }
}

/// Decompress a payload that was compressed with zstd.
fn maybe_decompress(payload: &[u8], compressed: bool) -> Result<Vec<u8>> {
    if !compressed {
        return Ok(payload.to_vec());
    }
    zstd::decode_all(std::io::Cursor::new(payload))
        .map_err(|e| anyhow::anyhow!("zstd decompress failed: {}", e))
}

/// Magic bytes identifying a Synaptic Protocol frame.
pub const MAGIC: [u8; 2] = [0x53, 0x4D];

/// Protocol version byte: v2.0 = 0x20.
pub const VERSION: u8 = 0x20;

/// Minimum frame size: header(13) + sender_id_len(1) + meta_len(4) + payload_len(4) + checksum(4) = 26 bytes
/// with 0-length sender_id, metadata, and payload.
pub const MIN_FRAME_SIZE: usize = 26;

/// Default maximum frame size (10 MB + overhead). Negotiated via handshake.
pub const DEFAULT_MAX_FRAME_SIZE: usize = 10 * 1024 * 1024 + 1024;

/// Encode a Signal into binary wire format. Returns the complete frame bytes.
pub fn encode_frame(signal: &Signal) -> Vec<u8> {
    let sender_bytes = signal.sender_id.as_bytes();
    let sender_len = sender_bytes.len().min(255) as u8;

    // Serialize metadata: merge trace_id into metadata for wire
    let metadata_value = {
        let mut meta = signal.metadata.clone();
        if !signal.trace_id.is_empty() {
            if let serde_json::Value::Object(ref mut map) = meta {
                map.insert(
                    "trace_id".to_string(),
                    serde_json::Value::String(signal.trace_id.clone()),
                );
            }
        }
        meta
    };
    let metadata_bytes = rmp_serde::to_vec(&metadata_value).unwrap_or_default();

    // Compression: apply if COMPRESSED flag is set OR payload > 1024 bytes,
    // but only when payload >= 256 bytes and not StreamData (Spec Section 19).
    let should_try_compress =
        signal.flags.contains(SignalFlags::COMPRESSED) || signal.payload.len() > 1024;
    let (payload_bytes, was_compressed) = if should_try_compress {
        maybe_compress(&signal.payload, signal.signal_type)
    } else {
        (signal.payload.clone(), false)
    };

    let mut flags = signal.flags;
    if was_compressed {
        flags |= SignalFlags::COMPRESSED;
    } else {
        flags -= SignalFlags::COMPRESSED;
    }

    // Encryption: if ENCRYPTED flag is set, encrypt payload (Section 9.4)
    let payload_bytes = if flags.contains(SignalFlags::ENCRYPTED) {
        // Use a fixed key/nonce for now (real: session-derived keys)
        let key = [0x42u8; 32];
        let nonce = [0u8; 12];
        let aad = signal.sender_id.as_bytes();
        match super::encryption::encrypt_payload(&key, &nonce, &payload_bytes, aad) {
            Ok(encrypted) => encrypted,
            Err(e) => {
                tracing::warn!(error = %e, "Encryption failed — sending unencrypted");
                flags -= SignalFlags::ENCRYPTED;
                payload_bytes
            }
        }
    } else {
        payload_bytes
    };

    // Pre-allocate: header(13) + sender + meta_len(4) + meta + payload_len(4) + payload + checksum(4)
    let total = 13
        + (sender_len as usize)
        + 4
        + metadata_bytes.len()
        + 4
        + payload_bytes.len()
        + 4;
    let mut buf = Vec::with_capacity(total);

    // Magic
    buf.extend_from_slice(&MAGIC);
    // Version
    buf.push(VERSION);
    // Flags
    buf.push(flags.bits());
    // Signal type
    buf.push(signal.signal_type.to_u8());
    // Channel ID (BE)
    buf.extend_from_slice(&signal.channel_id.to_be_bytes());
    // Sequence (BE)
    buf.extend_from_slice(&signal.sequence.to_be_bytes());
    // Sender ID length + sender ID
    buf.push(sender_len);
    buf.extend_from_slice(&sender_bytes[..sender_len as usize]);
    // Metadata length (BE) + metadata
    buf.extend_from_slice(&(metadata_bytes.len() as u32).to_be_bytes());
    buf.extend_from_slice(&metadata_bytes);
    // Payload length (BE) + payload
    buf.extend_from_slice(&(payload_bytes.len() as u32).to_be_bytes());
    buf.extend_from_slice(&payload_bytes);

    // CRC32 over everything so far
    let mut hasher = Hasher::new();
    hasher.update(&buf);
    let crc = hasher.finalize();
    buf.extend_from_slice(&crc.to_be_bytes());

    buf
}

/// Decode a complete binary frame into a Signal.
/// `data` must contain the entire frame including checksum.
pub fn decode_frame(data: &[u8]) -> Result<Signal> {
    if data.len() < MIN_FRAME_SIZE {
        bail!(
            "Frame too small: {} bytes (minimum {})",
            data.len(),
            MIN_FRAME_SIZE
        );
    }

    // Check magic
    if data[0] != MAGIC[0] || data[1] != MAGIC[1] {
        bail!(
            "Invalid magic bytes: 0x{:02X} 0x{:02X} (expected 0x53 0x4D)",
            data[0],
            data[1]
        );
    }

    // Check version
    let version = data[2];
    if version != VERSION {
        bail!(
            "Unsupported protocol version: 0x{:02X} (expected 0x{:02X})",
            version,
            VERSION
        );
    }

    // Parse flags
    let flags = SignalFlags::from_bits_truncate(data[3]);

    // Parse signal type
    let signal_type = SignalType::from_u8(data[4])
        .ok_or_else(|| anyhow::anyhow!("Unknown signal type: 0x{:02X}", data[4]))?;

    // Channel ID
    let channel_id = u32::from_be_bytes([data[5], data[6], data[7], data[8]]);
    // Sequence
    let sequence = u32::from_be_bytes([data[9], data[10], data[11], data[12]]);

    // Sender ID
    let sender_len = data[13] as usize;
    let mut offset = 14;
    if offset + sender_len > data.len() {
        bail!("Frame truncated at sender_id");
    }
    let sender_id = String::from_utf8_lossy(&data[offset..offset + sender_len]).to_string();
    offset += sender_len;

    // Metadata
    if offset + 4 > data.len() {
        bail!("Frame truncated at metadata_length");
    }
    let meta_len = u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]) as usize;
    offset += 4;
    if offset + meta_len > data.len() {
        bail!("Frame truncated at metadata");
    }
    let metadata: serde_json::Value = if meta_len > 0 {
        let meta_bytes = &data[offset..offset + meta_len];
        // Primary: MessagePack (per spec)
        rmp_serde::from_slice(meta_bytes).unwrap_or_else(|_| {
            // Fallback in debug mode: try JSON for backward compat during development
            if cfg!(debug_assertions) {
                serde_json::from_slice(meta_bytes)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()))
            } else {
                serde_json::Value::Object(serde_json::Map::new())
            }
        })
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    };
    offset += meta_len;

    // Payload
    if offset + 4 > data.len() {
        bail!("Frame truncated at payload_length");
    }
    let payload_len = u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]) as usize;
    offset += 4;
    if offset + payload_len > data.len() {
        bail!("Frame truncated at payload");
    }
    let raw_payload = data[offset..offset + payload_len].to_vec();
    offset += payload_len;

    // Decrypt payload if ENCRYPTED flag is set (Section 9.4)
    let raw_payload = if flags.contains(SignalFlags::ENCRYPTED) {
        let key = [0x42u8; 32]; // Must match encode key (real: session-derived)
        let nonce = [0u8; 12];
        let aad = sender_id.as_bytes();
        match super::encryption::decrypt_payload(&key, &nonce, &raw_payload, aad) {
            Ok(decrypted) => decrypted,
            Err(e) => {
                tracing::warn!(error = %e, "Decryption failed — using raw payload");
                raw_payload
            }
        }
    } else {
        raw_payload
    };

    // Decompress payload if COMPRESSED flag is set (Spec Section 19)
    let payload = maybe_decompress(&raw_payload, flags.contains(SignalFlags::COMPRESSED))?;

    // Checksum
    if offset + 4 > data.len() {
        bail!("Frame truncated at checksum");
    }
    let received_crc = u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]);

    // Verify CRC32 over all bytes before the checksum
    let mut hasher = Hasher::new();
    hasher.update(&data[..offset]);
    let computed_crc = hasher.finalize();
    if received_crc != computed_crc {
        bail!(
            "Checksum mismatch: received 0x{:08X}, computed 0x{:08X}",
            received_crc,
            computed_crc
        );
    }

    // Extract trace_id from metadata if present
    let trace_id = metadata
        .get("trace_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Ok(Signal {
        signal_type,
        flags,
        channel_id,
        sequence,
        sender_id,
        metadata,
        payload,
        trace_id,
    })
}

/// Read a complete frame from an async reader.
/// Reads length-prefixed: first reads the 13-byte fixed header to determine
/// sender_id_len, then reads enough to get metadata_len, then payload_len,
/// then the checksum.
///
/// Returns the full frame bytes (ready for decode_frame).
pub async fn read_frame(
    reader: &mut (impl tokio::io::AsyncReadExt + Unpin),
    max_frame_size: usize,
) -> Result<Vec<u8>> {
    // Read fixed header: magic(2) + version(1) + flags(1) + signal_type(1)
    //                   + channel_id(4) + sequence(4) + sender_id_len(1) = 14 bytes
    let mut header = [0u8; 14];
    reader.read_exact(&mut header).await?;

    // Validate magic early
    if header[0] != MAGIC[0] || header[1] != MAGIC[1] {
        bail!(
            "Invalid magic bytes: 0x{:02X} 0x{:02X}",
            header[0],
            header[1]
        );
    }

    let sender_len = header[13] as usize;

    // Read sender_id + metadata_len(4)
    let mut sender_and_meta_len = vec![0u8; sender_len + 4];
    reader.read_exact(&mut sender_and_meta_len).await?;

    let meta_len_offset = sender_len;
    let meta_len = u32::from_be_bytes([
        sender_and_meta_len[meta_len_offset],
        sender_and_meta_len[meta_len_offset + 1],
        sender_and_meta_len[meta_len_offset + 2],
        sender_and_meta_len[meta_len_offset + 3],
    ]) as usize;

    // Read metadata + payload_len(4)
    let mut meta_and_payload_len = vec![0u8; meta_len + 4];
    reader.read_exact(&mut meta_and_payload_len).await?;

    let payload_len_offset = meta_len;
    let payload_len = u32::from_be_bytes([
        meta_and_payload_len[payload_len_offset],
        meta_and_payload_len[payload_len_offset + 1],
        meta_and_payload_len[payload_len_offset + 2],
        meta_and_payload_len[payload_len_offset + 3],
    ]) as usize;

    // Check total frame size
    let total = 14 + sender_len + 4 + meta_len + 4 + payload_len + 4;
    if total > max_frame_size {
        bail!(
            "Frame exceeds max size: {} bytes (max {})",
            total,
            max_frame_size
        );
    }

    // Read payload + checksum(4)
    let mut payload_and_crc = vec![0u8; payload_len + 4];
    reader.read_exact(&mut payload_and_crc).await?;

    // Assemble the full frame
    let mut frame = Vec::with_capacity(total);
    frame.extend_from_slice(&header);
    frame.extend_from_slice(&sender_and_meta_len);
    frame.extend_from_slice(&meta_and_payload_len);
    frame.extend_from_slice(&payload_and_crc);

    Ok(frame)
}

/// Write a signal as a binary frame to an async writer.
pub async fn write_frame(
    writer: &mut (impl tokio::io::AsyncWriteExt + Unpin),
    signal: &Signal,
) -> Result<()> {
    let frame = encode_frame(signal);
    writer.write_all(&frame).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let mut signal = Signal::new(SignalType::Intent, "test-soma".to_string());
        signal.channel_id = 1;
        signal.sequence = 42;
        signal.payload = b"list files in /tmp".to_vec();
        signal.metadata = serde_json::json!({"content_type": "text/plain"});
        signal.trace_id = "abc123".to_string();

        let encoded = encode_frame(&signal);
        let decoded = decode_frame(&encoded).expect("decode should succeed");

        assert_eq!(decoded.signal_type, SignalType::Intent);
        assert_eq!(decoded.channel_id, 1);
        assert_eq!(decoded.sequence, 42);
        assert_eq!(decoded.sender_id, "test-soma");
        assert_eq!(decoded.payload, b"list files in /tmp");
        assert_eq!(decoded.trace_id, "abc123");
    }

    #[test]
    fn test_empty_signal() {
        let signal = Signal::new(SignalType::Ping, "s".to_string());
        let encoded = encode_frame(&signal);
        let decoded = decode_frame(&encoded).expect("decode should succeed");
        assert_eq!(decoded.signal_type, SignalType::Ping);
        assert_eq!(decoded.sender_id, "s");
        assert!(decoded.payload.is_empty());
    }

    #[test]
    fn test_checksum_corruption() {
        let signal = Signal::new(SignalType::Ping, "s".to_string());
        let mut encoded = encode_frame(&signal);
        // Corrupt the last byte (part of checksum)
        let last = encoded.len() - 1;
        encoded[last] ^= 0xFF;
        assert!(decode_frame(&encoded).is_err());
    }

    #[test]
    fn test_invalid_magic() {
        let signal = Signal::new(SignalType::Ping, "s".to_string());
        let mut encoded = encode_frame(&signal);
        encoded[0] = 0x00;
        assert!(decode_frame(&encoded).is_err());
    }

    #[test]
    fn test_all_signal_types_roundtrip() {
        let types = [
            SignalType::Handshake,
            SignalType::HandshakeAck,
            SignalType::Close,
            SignalType::Intent,
            SignalType::Result,
            SignalType::Data,
            SignalType::Binary,
            SignalType::StreamStart,
            SignalType::StreamData,
            SignalType::StreamEnd,
            SignalType::ChunkStart,
            SignalType::ChunkData,
            SignalType::ChunkEnd,
            SignalType::ChunkAck,
            SignalType::Discover,
            SignalType::DiscoverAck,
            SignalType::PeerQuery,
            SignalType::PeerList,
            SignalType::Subscribe,
            SignalType::Unsubscribe,
            SignalType::Ping,
            SignalType::Pong,
            SignalType::Error,
            SignalType::Control,
        ];
        for st in types {
            let signal = Signal::new(st, "test".to_string());
            let encoded = encode_frame(&signal);
            let decoded = decode_frame(&encoded).unwrap();
            assert_eq!(decoded.signal_type, st);
        }
    }

    #[test]
    fn test_flags_roundtrip() {
        let mut signal = Signal::new(SignalType::Data, "test".to_string());
        signal.flags = SignalFlags::COMPRESSED | SignalFlags::PRIORITY;
        // Set a large enough payload so compression actually engages
        signal.payload = vec![0x42; 512];
        let encoded = encode_frame(&signal);
        let decoded = decode_frame(&encoded).unwrap();
        assert!(decoded.flags.contains(SignalFlags::COMPRESSED));
        assert!(decoded.flags.contains(SignalFlags::PRIORITY));
        assert!(!decoded.flags.contains(SignalFlags::ENCRYPTED));
        assert_eq!(decoded.payload, vec![0x42; 512]);
    }

    #[test]
    fn test_compression_large_payload_roundtrip() {
        // Generate a large, compressible payload (> 1024 bytes)
        let payload: Vec<u8> = (0..4096)
            .map(|i| (i % 256) as u8)
            .collect();

        let mut signal = Signal::new(SignalType::Data, "compress-test".to_string());
        signal.payload = payload.clone();
        signal.metadata = serde_json::json!({"content_type": "application/octet-stream"});

        let encoded = encode_frame(&signal);
        // The encoded frame should be smaller than raw payload + overhead
        // because compression kicks in for payloads > 1024 bytes
        assert!(encoded.len() < payload.len() + 100,
            "Encoded frame ({} bytes) should be noticeably smaller than raw payload ({} bytes)",
            encoded.len(), payload.len());

        let decoded = decode_frame(&encoded).unwrap();
        assert_eq!(decoded.payload, payload, "Payload must survive compression roundtrip");
        assert_eq!(decoded.signal_type, SignalType::Data);
        assert_eq!(decoded.sender_id, "compress-test");
        // COMPRESSED flag should be set on the decoded frame
        assert!(decoded.flags.contains(SignalFlags::COMPRESSED));
    }

    #[test]
    fn test_compression_skipped_for_small_payload() {
        let mut signal = Signal::new(SignalType::Data, "small".to_string());
        signal.payload = b"tiny".to_vec(); // < 256 bytes
        signal.flags = SignalFlags::COMPRESSED; // request compression

        let encoded = encode_frame(&signal);
        let decoded = decode_frame(&encoded).unwrap();
        // Compression should be cleared because payload is too small
        assert!(!decoded.flags.contains(SignalFlags::COMPRESSED));
        assert_eq!(decoded.payload, b"tiny");
    }

    #[test]
    fn test_compression_skipped_for_stream_data() {
        let mut signal = Signal::new(SignalType::StreamData, "streamer".to_string());
        signal.payload = vec![0xAB; 2048]; // large payload but StreamData
        signal.flags = SignalFlags::COMPRESSED;

        let encoded = encode_frame(&signal);
        let decoded = decode_frame(&encoded).unwrap();
        // StreamData should never be compressed
        assert!(!decoded.flags.contains(SignalFlags::COMPRESSED));
        assert_eq!(decoded.payload, vec![0xAB; 2048]);
    }
}
