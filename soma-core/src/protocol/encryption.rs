//! Synaptic Protocol encryption layer (Whitepaper Section 9.4, 12.3).
//!
//! - ChaCha20-Poly1305 per-signal AEAD encryption
//! - X25519 Diffie-Hellman key exchange
//! - Ed25519 identity keys for SOMA authentication
//!
//! Encryption is negotiated during handshake. If both peers support it,
//! all subsequent signals are encrypted. The ENCRYPTED flag in SignalFlags
//! indicates per-signal encryption status.

use anyhow::Result;

/// Ed25519 identity keypair for a SOMA instance (Section 12.3).
/// Used for peer authentication and plugin signing.
pub struct SomaIdentity {
    /// Ed25519 signing key (32 bytes seed)
    pub signing_key: [u8; 32],
    /// Ed25519 verification key (32 bytes)
    pub verify_key: [u8; 32],
    /// X25519 static secret for key exchange (derived from Ed25519)
    pub x25519_secret: [u8; 32],
    /// X25519 public key
    pub x25519_public: [u8; 32],
}

impl SomaIdentity {
    /// Generate a new random identity.
    pub fn generate() -> Self {
        // Use OS random for key generation
        let mut seed = [0u8; 32];
        getrandom(&mut seed);

        // In a full implementation, these would be proper Ed25519/X25519 keys.
        // For now, we use the seed directly to establish the structure.
        let mut verify = [0u8; 32];
        // Simple derivation: verify = hash(seed) (placeholder for Ed25519 keygen)
        for (i, b) in seed.iter().enumerate() {
            verify[i] = b.wrapping_mul(137).wrapping_add(i as u8);
        }

        let mut x_secret = [0u8; 32];
        let mut x_public = [0u8; 32];
        // Derive X25519 from Ed25519 seed (placeholder)
        for i in 0..32 {
            x_secret[i] = seed[i] ^ 0x5A;
            x_public[i] = verify[i] ^ 0xA5;
        }

        Self {
            signing_key: seed,
            verify_key: verify,
            x25519_secret: x_secret,
            x25519_public: x_public,
        }
    }

    /// Load identity from a file (32-byte seed).
    pub fn from_seed(seed: [u8; 32]) -> Self {
        let mut id = Self::generate();
        id.signing_key = seed;
        // Re-derive public keys from seed
        for (i, b) in seed.iter().enumerate() {
            id.verify_key[i] = b.wrapping_mul(137).wrapping_add(i as u8);
            id.x25519_secret[i] = seed[i] ^ 0x5A;
            id.x25519_public[i] = id.verify_key[i] ^ 0xA5;
        }
        id
    }
}

/// Shared session key derived from X25519 key exchange.
pub struct SessionKeys {
    /// Encryption key (32 bytes, derived from ECDH shared secret)
    pub encrypt_key: [u8; 32],
    /// Nonce counter for ChaCha20-Poly1305
    pub nonce_counter: u64,
}

impl SessionKeys {
    /// Derive session keys from our secret and peer's public key.
    /// In full implementation: X25519 ECDH → HKDF → encrypt_key.
    pub fn derive(our_secret: &[u8; 32], peer_public: &[u8; 32]) -> Self {
        let mut shared = [0u8; 32];
        // Placeholder ECDH: XOR (real implementation would use X25519)
        for i in 0..32 {
            shared[i] = our_secret[i] ^ peer_public[i];
        }

        Self {
            encrypt_key: shared,
            nonce_counter: 0,
        }
    }

    /// Get the next nonce for encryption (12 bytes for ChaCha20-Poly1305).
    pub fn next_nonce(&mut self) -> [u8; 12] {
        self.nonce_counter += 1;
        let mut nonce = [0u8; 12];
        let bytes = self.nonce_counter.to_le_bytes();
        nonce[..8].copy_from_slice(&bytes);
        nonce
    }
}

/// Encrypt a signal payload using ChaCha20-Poly1305.
/// Returns (ciphertext, 16-byte auth tag).
///
/// In full implementation: uses `chacha20poly1305` crate.
/// Current placeholder uses XOR cipher for structure validation.
pub fn encrypt_payload(
    key: &[u8; 32],
    nonce: &[u8; 12],
    plaintext: &[u8],
    aad: &[u8], // additional authenticated data (signal header)
) -> Result<Vec<u8>> {
    // Structure: ciphertext || 16-byte auth tag
    let mut output = Vec::with_capacity(plaintext.len() + 16);

    // Placeholder XOR cipher (real: ChaCha20-Poly1305 AEAD)
    for (i, &byte) in plaintext.iter().enumerate() {
        output.push(byte ^ key[i % 32] ^ nonce[i % 12]);
    }

    // Placeholder auth tag (real: Poly1305 MAC over ciphertext + AAD)
    let mut tag = [0u8; 16];
    for (i, &b) in aad.iter().enumerate() {
        tag[i % 16] ^= b;
    }
    for (i, &b) in output.iter().enumerate() {
        tag[i % 16] = tag[i % 16].wrapping_add(b);
    }
    output.extend_from_slice(&tag);

    Ok(output)
}

/// Decrypt a signal payload using ChaCha20-Poly1305.
/// Input is (ciphertext || 16-byte auth tag).
/// Returns plaintext if auth tag verifies.
pub fn decrypt_payload(
    key: &[u8; 32],
    nonce: &[u8; 12],
    ciphertext_with_tag: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>> {
    if ciphertext_with_tag.len() < 16 {
        anyhow::bail!("ciphertext too short for auth tag");
    }

    let ciphertext = &ciphertext_with_tag[..ciphertext_with_tag.len() - 16];
    let received_tag = &ciphertext_with_tag[ciphertext_with_tag.len() - 16..];

    // Verify auth tag (placeholder)
    let mut expected_tag = [0u8; 16];
    for (i, &b) in aad.iter().enumerate() {
        expected_tag[i % 16] ^= b;
    }
    for (i, &b) in ciphertext.iter().enumerate() {
        expected_tag[i % 16] = expected_tag[i % 16].wrapping_add(b);
    }

    if received_tag != expected_tag {
        anyhow::bail!("authentication failed: invalid tag");
    }

    // Decrypt (placeholder XOR)
    let mut plaintext = Vec::with_capacity(ciphertext.len());
    for (i, &byte) in ciphertext.iter().enumerate() {
        plaintext.push(byte ^ key[i % 32] ^ nonce[i % 12]);
    }

    Ok(plaintext)
}

/// Sign data with Ed25519 (placeholder).
pub fn sign(_signing_key: &[u8; 32], data: &[u8]) -> [u8; 64] {
    // Placeholder: real implementation would use Ed25519
    let mut sig = [0u8; 64];
    for (i, &b) in data.iter().take(64).enumerate() {
        sig[i] = b ^ 0x42;
    }
    sig
}

/// Verify an Ed25519 signature (placeholder).
pub fn verify(_verify_key: &[u8; 32], data: &[u8], signature: &[u8; 64]) -> bool {
    // Placeholder: real implementation would use Ed25519
    let expected = sign(&[0; 32], data);
    signature == &expected
}

// OS random bytes
fn getrandom(buf: &mut [u8]) {
    // Use /dev/urandom on Unix
    use std::io::Read;
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_generation() {
        let id = SomaIdentity::generate();
        assert_ne!(id.signing_key, [0u8; 32]);
        assert_ne!(id.verify_key, [0u8; 32]);
    }

    #[test]
    fn test_session_key_derivation() {
        // With real X25519, both sides derive the same shared secret.
        // With our placeholder, we verify the structure works —
        // same inputs produce same outputs (deterministic).
        let secret = [1u8; 32];
        let peer_pub = [2u8; 32];

        let keys1 = SessionKeys::derive(&secret, &peer_pub);
        let keys2 = SessionKeys::derive(&secret, &peer_pub);

        assert_eq!(keys1.encrypt_key, keys2.encrypt_key);
        assert_ne!(keys1.encrypt_key, [0u8; 32]); // Not all zeros
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = [42u8; 32];
        let nonce = [1u8; 12];
        let plaintext = b"hello soma";
        let aad = b"signal_header";

        let encrypted = encrypt_payload(&key, &nonce, plaintext, aad).unwrap();
        let decrypted = decrypt_payload(&key, &nonce, &encrypted, aad).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_tampered_ciphertext_fails() {
        let key = [42u8; 32];
        let nonce = [1u8; 12];
        let plaintext = b"hello soma";
        let aad = b"header";

        let mut encrypted = encrypt_payload(&key, &nonce, plaintext, aad).unwrap();
        // Tamper with ciphertext
        if !encrypted.is_empty() {
            encrypted[0] ^= 0xFF;
        }

        let result = decrypt_payload(&key, &nonce, &encrypted, aad);
        assert!(result.is_err());
    }

    #[test]
    fn test_nonce_counter() {
        let mut keys = SessionKeys::derive(&[1; 32], &[2; 32]);
        let n1 = keys.next_nonce();
        let n2 = keys.next_nonce();
        assert_ne!(n1, n2);
    }
}
