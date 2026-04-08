//! Resumable chunked transfer with SHA-256 integrity verification.
//!
//! Large payloads (schemas, routines, etc.) are split into fixed-size chunks,
//! each individually hashed. The sender transmits a manifest first, then
//! chunks. The receiver can accept chunks out of order, verify each one, and
//! request only missing chunks on resume. The reassembled payload is verified
//! against the manifest's overall SHA-256 hash.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::errors::{Result, SomaError};
use crate::types::peer::DistributedFailure;

/// Default chunk size: 64 KB.
pub const DEFAULT_CHUNK_SIZE: usize = 64 * 1024;

/// Maximum payload size: 256 MB. Prevents unbounded memory allocation.
const MAX_PAYLOAD_SIZE: u64 = 256 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// Manifest sent before any chunk data. Describes the transfer so the receiver
/// can allocate storage and know when all chunks have arrived.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferManifest {
    pub transfer_id: Uuid,
    pub total_bytes: u64,
    pub total_chunks: u32,
    /// Hex-encoded SHA-256 of the complete payload.
    pub sha256_hash: String,
    /// Per-chunk SHA-256 hashes, indexed by chunk_index. Allows the receiver
    /// to verify each chunk independently on arrival.
    pub chunk_hashes: Vec<String>,
}

/// A single chunk of a transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub transfer_id: Uuid,
    pub chunk_index: u32,
    pub data: Vec<u8>,
    /// Hex-encoded SHA-256 of this chunk's data.
    pub chunk_hash: String,
}

/// Sent by the receiver to report which chunks it already holds, so the
/// sender can skip them on a resumed transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeRequest {
    pub transfer_id: Uuid,
    /// Chunk indices the receiver already has and verified.
    pub received_indices: Vec<u32>,
}

// ---------------------------------------------------------------------------
// Sender
// ---------------------------------------------------------------------------

/// Splits a payload into chunks and produces a manifest with integrity hashes.
pub struct ChunkedSender {
    chunk_size: usize,
}

impl ChunkedSender {
    pub fn new(chunk_size: usize) -> Self {
        let size = if chunk_size == 0 {
            DEFAULT_CHUNK_SIZE
        } else {
            chunk_size
        };
        Self { chunk_size: size }
    }

    /// Compute the overall SHA-256 hash of the payload.
    fn hash_bytes(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        format!("{:x}", hasher.finalize())
    }

    /// Split `payload` into chunks and produce the manifest plus all chunk objects.
    pub fn prepare(&self, payload: &[u8]) -> (TransferManifest, Vec<Chunk>) {
        let transfer_id = Uuid::new_v4();
        let total_bytes = payload.len() as u64;
        let overall_hash = Self::hash_bytes(payload);

        let mut chunks = Vec::new();
        let mut chunk_hashes = Vec::new();
        let mut offset = 0usize;
        let mut index = 0u32;

        while offset < payload.len() {
            let end = (offset + self.chunk_size).min(payload.len());
            let data = payload[offset..end].to_vec();
            let chunk_hash = Self::hash_bytes(&data);
            chunk_hashes.push(chunk_hash.clone());
            chunks.push(Chunk {
                transfer_id,
                chunk_index: index,
                data,
                chunk_hash,
            });
            offset = end;
            index += 1;
        }

        // Handle empty payload: produce a single empty chunk so the manifest
        // is consistent (total_chunks = 1, one hash entry).
        if chunks.is_empty() {
            let empty_hash = Self::hash_bytes(&[]);
            chunk_hashes.push(empty_hash.clone());
            chunks.push(Chunk {
                transfer_id,
                chunk_index: 0,
                data: Vec::new(),
                chunk_hash: empty_hash,
            });
        }

        let manifest = TransferManifest {
            transfer_id,
            total_bytes,
            total_chunks: chunks.len() as u32,
            sha256_hash: overall_hash,
            chunk_hashes,
        };

        (manifest, chunks)
    }

    /// Given a set of already-received chunk indices, filter the chunk list to
    /// only those that still need to be sent. Used for resume.
    pub fn filter_missing(chunks: &[Chunk], already_received: &HashSet<u32>) -> Vec<Chunk> {
        chunks
            .iter()
            .filter(|c| !already_received.contains(&c.chunk_index))
            .cloned()
            .collect()
    }
}

impl Default for ChunkedSender {
    fn default() -> Self {
        Self::new(DEFAULT_CHUNK_SIZE)
    }
}

// ---------------------------------------------------------------------------
// Receiver
// ---------------------------------------------------------------------------

/// Accumulates chunks for a single transfer, verifying each one on arrival,
/// and reassembles the complete payload once all chunks are present.
pub struct ChunkedReceiver {
    transfer_id: Uuid,
    manifest: TransferManifest,
    /// Received chunk data, keyed by chunk_index. Only chunks that passed
    /// per-chunk hash verification are stored here.
    received: HashMap<u32, Vec<u8>>,
}

impl ChunkedReceiver {
    /// Start receiving a transfer described by `manifest`.
    pub fn new(manifest: TransferManifest) -> Result<Self> {
        if manifest.total_bytes > MAX_PAYLOAD_SIZE {
            return Err(SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!(
                    "transfer payload too large: {} bytes (max {})",
                    manifest.total_bytes, MAX_PAYLOAD_SIZE
                ),
            });
        }
        if manifest.chunk_hashes.len() != manifest.total_chunks as usize {
            return Err(SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!(
                    "manifest chunk_hashes count ({}) does not match total_chunks ({})",
                    manifest.chunk_hashes.len(),
                    manifest.total_chunks
                ),
            });
        }
        let transfer_id = manifest.transfer_id;
        Ok(Self {
            transfer_id,
            manifest,
            received: HashMap::new(),
        })
    }

    /// Accept a chunk. Verifies the chunk's hash against the manifest before
    /// storing it. Returns an error if the hash doesn't match or the index is
    /// out of range. Duplicate deliveries of the same index are silently accepted
    /// as long as the hash still matches.
    pub fn receive_chunk(&mut self, chunk: &Chunk) -> Result<()> {
        if chunk.transfer_id != self.transfer_id {
            return Err(SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!(
                    "chunk transfer_id {} does not match receiver {}",
                    chunk.transfer_id, self.transfer_id
                ),
            });
        }
        if chunk.chunk_index >= self.manifest.total_chunks {
            return Err(SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!(
                    "chunk_index {} out of range (total_chunks = {})",
                    chunk.chunk_index, self.manifest.total_chunks
                ),
            });
        }

        // Verify per-chunk hash.
        let expected = &self.manifest.chunk_hashes[chunk.chunk_index as usize];
        let actual = compute_sha256(&chunk.data);
        if actual != *expected {
            return Err(SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!(
                    "chunk {} hash mismatch: expected {}, got {}",
                    chunk.chunk_index, expected, actual
                ),
            });
        }

        // Also verify the embedded chunk_hash field is consistent.
        if chunk.chunk_hash != actual {
            return Err(SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!(
                    "chunk {} embedded hash mismatch: claimed {}, computed {}",
                    chunk.chunk_index, chunk.chunk_hash, actual
                ),
            });
        }

        self.received.insert(chunk.chunk_index, chunk.data.clone());
        Ok(())
    }

    /// Returns true when all chunks have been received and verified.
    pub fn is_complete(&self) -> bool {
        self.received.len() == self.manifest.total_chunks as usize
    }

    /// The set of chunk indices that have been received so far.
    pub fn received_indices(&self) -> HashSet<u32> {
        self.received.keys().copied().collect()
    }

    /// Indices of chunks that are still missing.
    pub fn missing_indices(&self) -> Vec<u32> {
        (0..self.manifest.total_chunks)
            .filter(|i| !self.received.contains_key(i))
            .collect()
    }

    /// Build a `ResumeRequest` reporting which chunks we already hold.
    pub fn resume_request(&self) -> ResumeRequest {
        ResumeRequest {
            transfer_id: self.transfer_id,
            received_indices: self.received_indices().into_iter().collect(),
        }
    }

    /// Reassemble the complete payload from received chunks. Verifies that all
    /// chunks are present and that the overall SHA-256 matches the manifest.
    pub fn finalize(self) -> Result<Vec<u8>> {
        if !self.is_complete() {
            let missing = self.missing_indices();
            return Err(SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!(
                    "cannot finalize: missing chunks {:?} (have {}/{})",
                    missing,
                    self.received.len(),
                    self.manifest.total_chunks
                ),
            });
        }

        // Reassemble in order.
        let mut assembled = Vec::with_capacity(self.manifest.total_bytes as usize);
        for i in 0..self.manifest.total_chunks {
            if let Some(data) = self.received.get(&i) {
                assembled.extend_from_slice(data);
            }
        }

        // Verify overall hash.
        let actual_hash = compute_sha256(&assembled);
        if actual_hash != self.manifest.sha256_hash {
            return Err(SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!(
                    "overall SHA-256 mismatch: expected {}, got {}",
                    self.manifest.sha256_hash, actual_hash
                ),
            });
        }

        // Verify total bytes.
        if assembled.len() as u64 != self.manifest.total_bytes {
            return Err(SomaError::Distributed {
                failure: DistributedFailure::TransportFailure,
                details: format!(
                    "reassembled size {} does not match manifest total_bytes {}",
                    assembled.len(),
                    self.manifest.total_bytes
                ),
            });
        }

        Ok(assembled)
    }
}

// ---------------------------------------------------------------------------
// Convenience: end-to-end transfer in a single call (for in-process use)
// ---------------------------------------------------------------------------

/// Perform a complete chunked transfer in memory: split, transfer all chunks,
/// reassemble, and verify. Useful for testing and for local transfers where the
/// chunking is needed for integrity but not for network transport.
pub fn transfer_in_memory(payload: &[u8], chunk_size: usize) -> Result<Vec<u8>> {
    let sender = ChunkedSender::new(chunk_size);
    let (manifest, chunks) = sender.prepare(payload);
    let mut receiver = ChunkedReceiver::new(manifest)?;
    for chunk in &chunks {
        receiver.receive_chunk(chunk)?;
    }
    receiver.finalize()
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn compute_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_reassemble_basic() {
        let data = b"hello, chunked world!";
        let result = transfer_in_memory(data, 8).unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn split_and_reassemble_exact_boundary() {
        // Payload length is an exact multiple of chunk size.
        let data = vec![0xABu8; 128];
        let result = transfer_in_memory(&data, 32).unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn split_and_reassemble_single_chunk() {
        let data = b"small";
        let result = transfer_in_memory(data, 1024).unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn empty_payload() {
        let result = transfer_in_memory(&[], 64).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn manifest_fields_correct() {
        let data = vec![1u8; 200];
        let sender = ChunkedSender::new(64);
        let (manifest, chunks) = sender.prepare(&data);

        assert_eq!(manifest.total_bytes, 200);
        // ceil(200/64) = 4 chunks: 64 + 64 + 64 + 8
        assert_eq!(manifest.total_chunks, 4);
        assert_eq!(chunks.len(), 4);
        assert_eq!(manifest.chunk_hashes.len(), 4);

        // Verify overall hash matches the payload.
        let expected_hash = compute_sha256(&data);
        assert_eq!(manifest.sha256_hash, expected_hash);

        // Verify per-chunk hashes.
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index, i as u32);
            assert_eq!(chunk.chunk_hash, manifest.chunk_hashes[i]);
            assert_eq!(chunk.chunk_hash, compute_sha256(&chunk.data));
        }
    }

    #[test]
    fn out_of_order_delivery() {
        let data = b"ABCDEFGHIJKLMNOP";
        let sender = ChunkedSender::new(4);
        let (manifest, chunks) = sender.prepare(data);

        let mut receiver = ChunkedReceiver::new(manifest).unwrap();

        // Deliver in reverse order.
        for chunk in chunks.iter().rev() {
            receiver.receive_chunk(chunk).unwrap();
        }

        let result = receiver.finalize().unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn duplicate_chunk_accepted() {
        let data = b"duplicate test data here";
        let sender = ChunkedSender::new(8);
        let (manifest, chunks) = sender.prepare(data);

        let mut receiver = ChunkedReceiver::new(manifest).unwrap();

        // Deliver chunk 0 twice.
        receiver.receive_chunk(&chunks[0]).unwrap();
        receiver.receive_chunk(&chunks[0]).unwrap();

        // Deliver the rest.
        for chunk in &chunks[1..] {
            receiver.receive_chunk(chunk).unwrap();
        }

        let result = receiver.finalize().unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn chunk_hash_mismatch_rejected() {
        let data = b"integrity check";
        let sender = ChunkedSender::new(8);
        let (manifest, mut chunks) = sender.prepare(data);

        // Corrupt chunk 0's data.
        chunks[0].data[0] ^= 0xFF;

        let mut receiver = ChunkedReceiver::new(manifest).unwrap();
        let result = receiver.receive_chunk(&chunks[0]);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("hash mismatch"), "unexpected error: {}", err);
    }

    #[test]
    fn embedded_hash_mismatch_rejected() {
        let data = b"embedded hash test";
        let sender = ChunkedSender::new(64);
        let (manifest, mut chunks) = sender.prepare(data);

        // Tamper with the embedded hash field but leave data intact.
        chunks[0].chunk_hash = "0000000000000000000000000000000000000000000000000000000000000000".to_string();

        let mut receiver = ChunkedReceiver::new(manifest).unwrap();
        let result = receiver.receive_chunk(&chunks[0]);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("embedded hash mismatch"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn overall_hash_mismatch_detected() {
        let data = b"tamper after chunks";
        let sender = ChunkedSender::new(8);
        let (mut manifest, chunks) = sender.prepare(data);

        // Corrupt the manifest's overall hash.
        manifest.sha256_hash =
            "0000000000000000000000000000000000000000000000000000000000000000".to_string();

        let mut receiver = ChunkedReceiver::new(manifest).unwrap();
        for chunk in &chunks {
            receiver.receive_chunk(chunk).unwrap();
        }

        let result = receiver.finalize();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("overall SHA-256 mismatch"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn finalize_with_missing_chunks_fails() {
        let data = vec![42u8; 100];
        let sender = ChunkedSender::new(30);
        let (manifest, chunks) = sender.prepare(&data);

        let mut receiver = ChunkedReceiver::new(manifest).unwrap();

        // Only deliver chunk 0 and 2, skip 1 and 3.
        receiver.receive_chunk(&chunks[0]).unwrap();
        receiver.receive_chunk(&chunks[2]).unwrap();

        assert!(!receiver.is_complete());
        assert_eq!(receiver.missing_indices(), vec![1, 3]);

        let result = receiver.finalize();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("missing chunks"), "unexpected error: {}", err);
    }

    #[test]
    fn resume_sends_only_missing_chunks() {
        let data = vec![7u8; 200];
        let sender = ChunkedSender::new(50);
        let (manifest, chunks) = sender.prepare(&data);

        // Simulate: receiver got chunks 0 and 2 in a prior session.
        let mut receiver = ChunkedReceiver::new(manifest).unwrap();
        receiver.receive_chunk(&chunks[0]).unwrap();
        receiver.receive_chunk(&chunks[2]).unwrap();

        // Build resume request.
        let resume = receiver.resume_request();
        assert_eq!(resume.transfer_id, chunks[0].transfer_id);
        let received_set: HashSet<u32> = resume.received_indices.into_iter().collect();
        assert!(received_set.contains(&0));
        assert!(received_set.contains(&2));

        // Sender filters to missing chunks only.
        let missing = ChunkedSender::filter_missing(&chunks, &received_set);
        assert_eq!(missing.len(), 2);
        let missing_indices: HashSet<u32> = missing.iter().map(|c| c.chunk_index).collect();
        assert!(missing_indices.contains(&1));
        assert!(missing_indices.contains(&3));

        // Deliver the missing chunks to complete the transfer.
        for chunk in &missing {
            receiver.receive_chunk(chunk).unwrap();
        }
        assert!(receiver.is_complete());

        let result = receiver.finalize().unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn chunk_index_out_of_range() {
        let data = b"small";
        let sender = ChunkedSender::new(64);
        let (manifest, _chunks) = sender.prepare(data);

        let mut receiver = ChunkedReceiver::new(manifest).unwrap();

        let bad_chunk = Chunk {
            transfer_id: receiver.resume_request().transfer_id,
            chunk_index: 99,
            data: vec![0],
            chunk_hash: compute_sha256(&[0]),
        };
        let result = receiver.receive_chunk(&bad_chunk);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("out of range"), "unexpected error: {}", err);
    }

    #[test]
    fn wrong_transfer_id_rejected() {
        let data = b"test";
        let sender = ChunkedSender::new(64);
        let (manifest, _chunks) = sender.prepare(data);

        let mut receiver = ChunkedReceiver::new(manifest).unwrap();

        let bad_chunk = Chunk {
            transfer_id: Uuid::new_v4(),
            chunk_index: 0,
            data: vec![0],
            chunk_hash: compute_sha256(&[0]),
        };
        let result = receiver.receive_chunk(&bad_chunk);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("does not match"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn manifest_with_mismatched_hash_count_rejected() {
        let manifest = TransferManifest {
            transfer_id: Uuid::new_v4(),
            total_bytes: 100,
            total_chunks: 5,
            sha256_hash: "abc".to_string(),
            chunk_hashes: vec!["a".to_string(), "b".to_string()], // only 2, not 5
        };
        let result = ChunkedReceiver::new(manifest);
        assert!(result.is_err());
    }

    #[test]
    fn large_payload_chunked_correctly() {
        // 1 MB payload, 64 KB chunks => 16 chunks.
        let data = vec![0xCDu8; 1024 * 1024];
        let sender = ChunkedSender::new(64 * 1024);
        let (manifest, chunks) = sender.prepare(&data);

        assert_eq!(manifest.total_chunks, 16);
        assert_eq!(chunks.len(), 16);

        let result = transfer_in_memory(&data, 64 * 1024).unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn default_sender_uses_64kb() {
        let sender = ChunkedSender::default();
        assert_eq!(sender.chunk_size, DEFAULT_CHUNK_SIZE);
    }

    #[test]
    fn zero_chunk_size_uses_default() {
        let sender = ChunkedSender::new(0);
        assert_eq!(sender.chunk_size, DEFAULT_CHUNK_SIZE);
    }

    #[test]
    fn is_complete_tracking() {
        let data = b"tracking test";
        let sender = ChunkedSender::new(4);
        let (manifest, chunks) = sender.prepare(data);
        let total = chunks.len();

        let mut receiver = ChunkedReceiver::new(manifest).unwrap();
        assert!(!receiver.is_complete());

        for (i, chunk) in chunks.iter().enumerate() {
            receiver.receive_chunk(chunk).unwrap();
            if i < total - 1 {
                assert!(!receiver.is_complete());
            }
        }
        assert!(receiver.is_complete());
    }
}
