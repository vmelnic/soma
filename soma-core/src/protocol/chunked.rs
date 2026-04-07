//! Chunked transfer support for large file uploads (Spec Section 6.3).
//!
//! Manages CHUNK_START / CHUNK_DATA / CHUNK_END / CHUNK_ACK lifecycle,
//! including resumption from the last acknowledged chunk and SHA-256
//! verification on reassembly.

use std::collections::HashMap;

use anyhow::{bail, Result};
use sha2::{Digest, Sha256};

use super::signal::{Signal, SignalType};

/// State for a single in-progress chunked transfer.
pub struct ChunkTransfer {
    pub channel_id: u32,
    pub filename: String,
    pub total_size: u64,
    pub chunk_size: u32,
    pub total_chunks: u32,
    pub checksum_sha256: String,
    pub received_chunks: HashMap<u32, Vec<u8>>,
    pub last_ack_seq: u32,
}

/// Manages all active chunked transfers, keyed by channel_id.
pub struct ChunkManager {
    active_transfers: HashMap<u32, ChunkTransfer>,
}

impl ChunkManager {
    pub fn new() -> Self {
        Self {
            active_transfers: HashMap::new(),
        }
    }

    /// Handle CHUNK_START: create a new transfer from the signal metadata.
    ///
    /// If metadata contains `resume_from`, the transfer starts expecting
    /// chunks from that sequence number onward (chunks below it are
    /// assumed already received in a prior session and are not required).
    pub fn start_transfer(
        &mut self,
        channel_id: u32,
        metadata: &serde_json::Value,
    ) -> Result<()> {
        let filename = metadata
            .get("filename")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let total_size = metadata
            .get("total_size")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let chunk_size = metadata
            .get("chunk_size")
            .and_then(|v| v.as_u64())
            .unwrap_or(65536) as u32;

        let total_chunks = metadata
            .get("total_chunks")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        let checksum_sha256 = metadata
            .get("checksum_sha256")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let resume_from = metadata
            .get("resume_from")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        let mut transfer = ChunkTransfer {
            channel_id,
            filename,
            total_size,
            chunk_size,
            total_chunks,
            checksum_sha256,
            received_chunks: HashMap::new(),
            last_ack_seq: 0,
        };

        // If resuming, mark all chunks before resume_from as implicitly received
        // (they won't actually be stored — finalize will only work if the caller
        // re-provides them or the transfer is truly complete).
        if resume_from > 0 {
            transfer.last_ack_seq = resume_from.saturating_sub(1);
        }

        self.active_transfers.insert(channel_id, transfer);
        Ok(())
    }

    /// Handle CHUNK_DATA: store chunk data and return a CHUNK_ACK signal.
    ///
    /// Returns `None` if the channel_id has no active transfer.
    pub fn receive_chunk(
        &mut self,
        channel_id: u32,
        sequence: u32,
        data: Vec<u8>,
    ) -> Option<Signal> {
        let transfer = self.active_transfers.get_mut(&channel_id)?;
        transfer.received_chunks.insert(sequence, data);
        if sequence > transfer.last_ack_seq {
            transfer.last_ack_seq = sequence;
        }

        let mut ack = Signal::new(SignalType::ChunkAck, String::new());
        ack.channel_id = channel_id;
        ack.sequence = sequence;
        ack.metadata = serde_json::json!({ "ack_seq": sequence });
        Some(ack)
    }

    /// Handle CHUNK_END: verify all chunks received, reassemble in sequence
    /// order, verify SHA-256 checksum (if provided), and return the complete
    /// payload.
    pub fn finalize_transfer(&mut self, channel_id: u32) -> Result<Vec<u8>> {
        let transfer = match self.active_transfers.remove(&channel_id) {
            Some(t) => t,
            None => bail!("No active transfer on channel {}", channel_id),
        };

        // Determine expected chunk count from the map itself if total_chunks
        // was not specified in the CHUNK_START metadata.
        let expected = if transfer.total_chunks > 0 {
            transfer.total_chunks
        } else if transfer.received_chunks.is_empty() {
            0
        } else {
            // Highest sequence + 1 (zero-based)
            transfer.received_chunks.keys().max().copied().unwrap_or(0) + 1
        };

        // Check for missing chunks
        for seq in 0..expected {
            if !transfer.received_chunks.contains_key(&seq) {
                bail!(
                    "Missing chunk {} of {} for file '{}'",
                    seq,
                    expected,
                    transfer.filename
                );
            }
        }

        // Reassemble in order
        let mut assembled = Vec::with_capacity(transfer.total_size as usize);
        for seq in 0..expected {
            if let Some(chunk) = transfer.received_chunks.get(&seq) {
                assembled.extend_from_slice(chunk);
            }
        }

        // Verify SHA-256 if a checksum was provided
        if !transfer.checksum_sha256.is_empty() {
            let mut hasher = Sha256::new();
            hasher.update(&assembled);
            let computed = format!("{:x}", hasher.finalize());
            if computed != transfer.checksum_sha256 {
                bail!(
                    "SHA-256 mismatch for '{}': expected {}, got {}",
                    transfer.filename,
                    transfer.checksum_sha256,
                    computed
                );
            }
        }

        Ok(assembled)
    }

    /// Return the next expected sequence number for a transfer (useful for
    /// communicating resume position to the sender).
    pub fn resume_from(&self, channel_id: u32) -> Option<u32> {
        let transfer = self.active_transfers.get(&channel_id)?;
        Some(transfer.last_ack_seq + 1)
    }

    /// Check whether a transfer is active on the given channel.
    pub fn has_transfer(&self, channel_id: u32) -> bool {
        self.active_transfers.contains_key(&channel_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_chunked_transfer() {
        let mut mgr = ChunkManager::new();

        let meta = serde_json::json!({
            "filename": "test.bin",
            "total_size": 10,
            "chunk_size": 5,
            "total_chunks": 2,
        });
        mgr.start_transfer(7, &meta).unwrap();

        let ack0 = mgr.receive_chunk(7, 0, vec![1, 2, 3, 4, 5]);
        assert!(ack0.is_some());
        assert_eq!(ack0.unwrap().sequence, 0);

        let ack1 = mgr.receive_chunk(7, 1, vec![6, 7, 8, 9, 10]);
        assert!(ack1.is_some());

        let data = mgr.finalize_transfer(7).unwrap();
        assert_eq!(data, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    }

    #[test]
    fn test_sha256_verification() {
        use sha2::{Digest, Sha256};

        let payload = b"hello world";
        let mut hasher = Sha256::new();
        hasher.update(payload);
        let checksum = format!("{:x}", hasher.finalize());

        let mut mgr = ChunkManager::new();
        let meta = serde_json::json!({
            "filename": "hello.txt",
            "total_size": 11,
            "chunk_size": 11,
            "total_chunks": 1,
            "checksum_sha256": checksum,
        });
        mgr.start_transfer(1, &meta).unwrap();
        mgr.receive_chunk(1, 0, payload.to_vec());
        let data = mgr.finalize_transfer(1).unwrap();
        assert_eq!(data, payload);
    }

    #[test]
    fn test_sha256_mismatch() {
        let mut mgr = ChunkManager::new();
        let meta = serde_json::json!({
            "filename": "bad.txt",
            "total_size": 5,
            "chunk_size": 5,
            "total_chunks": 1,
            "checksum_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
        });
        mgr.start_transfer(2, &meta).unwrap();
        mgr.receive_chunk(2, 0, vec![1, 2, 3, 4, 5]);
        assert!(mgr.finalize_transfer(2).is_err());
    }

    #[test]
    fn test_missing_chunk() {
        let mut mgr = ChunkManager::new();
        let meta = serde_json::json!({
            "filename": "gap.bin",
            "total_size": 15,
            "chunk_size": 5,
            "total_chunks": 3,
        });
        mgr.start_transfer(3, &meta).unwrap();
        mgr.receive_chunk(3, 0, vec![1, 2, 3, 4, 5]);
        // Skip chunk 1
        mgr.receive_chunk(3, 2, vec![11, 12, 13, 14, 15]);
        assert!(mgr.finalize_transfer(3).is_err());
    }

    #[test]
    fn test_resume_from() {
        let mut mgr = ChunkManager::new();
        let meta = serde_json::json!({
            "filename": "resume.bin",
            "total_size": 20,
            "chunk_size": 5,
            "total_chunks": 4,
            "resume_from": 2,
        });
        mgr.start_transfer(4, &meta).unwrap();
        assert_eq!(mgr.resume_from(4), Some(2));
    }

    #[test]
    fn test_no_transfer() {
        let mgr = ChunkManager::new();
        assert_eq!(mgr.resume_from(99), None);
        assert!(!mgr.has_transfer(99));
    }
}
