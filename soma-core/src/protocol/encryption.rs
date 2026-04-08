//! Synaptic Protocol encryption layer (Spec Sections 9.4, 12.3).
//!
//! Provides the cryptographic primitives for peer-to-peer SOMA communication:
//!
//! - **X25519** Diffie-Hellman key exchange for forward-secret session keys
//! - **ChaCha20-Poly1305** AEAD for per-signal authenticated encryption
//! - **Ed25519** identity keys for SOMA authentication and plugin signing
//! - **SHA-256 KDF** with domain separator to derive session keys from ECDH shared secrets
//!
//! Encryption is negotiated during the handshake phase. When both peers
//! support it, all subsequent signals are encrypted. The `ENCRYPTED` flag
//! in `SignalFlags` indicates per-signal encryption status.
//!
//! Nonces use separate monotonic counters for send and receive directions,
//! preventing nonce reuse across a session's lifetime.

use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce, aead::{Aead, KeyInit}};
use sha2::{Sha256, Digest};

/// Ed25519 identity keypair for a SOMA instance (Spec Section 12.3).
///
/// Each SOMA node has a single long-lived identity. The Ed25519 seed
/// deterministically derives both the signing keypair and the X25519
/// static key used for session establishment.
#[allow(dead_code)] // Spec feature for SOMA identity
pub struct SomaIdentity {
    /// Ed25519 signing key (32-byte seed).
    pub signing_key: [u8; 32],
    /// Ed25519 verification key (public, 32 bytes).
    pub verify_key: [u8; 32],
    /// X25519 static secret derived from the Ed25519 seed.
    pub x25519_secret: [u8; 32],
    /// X25519 public key advertised during handshake.
    pub x25519_public: [u8; 32],
}

#[allow(dead_code)] // Spec feature for SOMA identity
impl SomaIdentity {
    /// Generate a new random identity using OS entropy.
    pub fn generate() -> Self {
        use ed25519_dalek::SigningKey;
        use rand::rngs::OsRng;

        let signing = SigningKey::generate(&mut OsRng);
        let verify = signing.verifying_key();

        // X25519 key is derived from the same Ed25519 seed for a single identity root
        let x_secret = x25519_dalek::StaticSecret::from(signing.to_bytes());
        let x_public = x25519_dalek::PublicKey::from(&x_secret);

        Self {
            signing_key: signing.to_bytes(),
            verify_key: verify.to_bytes(),
            x25519_secret: x_secret.to_bytes(),
            x25519_public: x_public.to_bytes(),
        }
    }

    /// Reconstruct an identity deterministically from a 32-byte seed.
    pub fn from_seed(seed: [u8; 32]) -> Self {
        use ed25519_dalek::SigningKey;

        let signing = SigningKey::from_bytes(&seed);
        let verify = signing.verifying_key();

        let x_secret = x25519_dalek::StaticSecret::from(seed);
        let x_public = x25519_dalek::PublicKey::from(&x_secret);

        Self {
            signing_key: signing.to_bytes(),
            verify_key: verify.to_bytes(),
            x25519_secret: x_secret.to_bytes(),
            x25519_public: x_public.to_bytes(),
        }
    }
}

/// Session-scoped symmetric keys derived from X25519 key exchange.
///
/// Nonces use separate atomic counters for send and receive to avoid
/// coordination between reader and writer threads while guaranteeing
/// uniqueness within each direction.
pub struct SessionKeys {
    /// 256-bit encryption key derived via SHA-256 KDF (not the raw ECDH output).
    pub encrypt_key: [u8; 32],
    /// Monotonic nonce counter for outbound encryption.
    pub send_nonce: AtomicU64,
    /// Monotonic nonce counter for inbound decryption.
    pub recv_nonce: AtomicU64,
}

#[allow(dead_code)] // Spec feature for session encryption
impl SessionKeys {
    /// Derive session keys from X25519 ECDH between our static secret and the
    /// peer's public key.
    ///
    /// The raw shared secret is never used directly. Instead it is fed through
    /// SHA-256 with the domain separator `"soma-session-key-v1"` to produce the
    /// 256-bit encryption key, preventing cross-protocol key reuse.
    pub fn derive(our_secret: &[u8; 32], peer_public: &[u8; 32]) -> Self {
        let secret = x25519_dalek::StaticSecret::from(*our_secret);
        let public = x25519_dalek::PublicKey::from(*peer_public);
        let shared = secret.diffie_hellman(&public);

        // KDF: domain-separated SHA-256 prevents cross-protocol key reuse
        let mut hasher = Sha256::new();
        hasher.update(shared.as_bytes());
        hasher.update(b"soma-session-key-v1");
        let derived = hasher.finalize();
        let mut encrypt_key = [0u8; 32];
        encrypt_key.copy_from_slice(&derived);

        Self {
            encrypt_key,
            send_nonce: AtomicU64::new(0),
            recv_nonce: AtomicU64::new(0),
        }
    }

    /// Atomically increment and return the next 12-byte send nonce.
    ///
    /// The counter occupies the low 8 bytes (little-endian); the high 4 bytes
    /// remain zero. This gives 2^64 unique nonces per direction per session.
    pub fn next_send_nonce(&self) -> [u8; 12] {
        let counter = self.send_nonce.fetch_add(1, Ordering::Relaxed) + 1;
        let mut nonce = [0u8; 12];
        let bytes = counter.to_le_bytes();
        nonce[..8].copy_from_slice(&bytes);
        nonce
    }

    /// Atomically increment and return the next 12-byte receive nonce.
    pub fn next_recv_nonce(&self) -> [u8; 12] {
        let counter = self.recv_nonce.fetch_add(1, Ordering::Relaxed) + 1;
        let mut nonce = [0u8; 12];
        let bytes = counter.to_le_bytes();
        nonce[..8].copy_from_slice(&bytes);
        nonce
    }

    /// Legacy alias for [`next_send_nonce`](Self::next_send_nonce).
    pub fn next_nonce(&self) -> [u8; 12] {
        self.next_send_nonce()
    }
}

/// Encrypt a signal payload with ChaCha20-Poly1305 AEAD.
///
/// `aad` (additional authenticated data) should be the signal header bytes
/// so that header tampering is detected on decryption. Returns ciphertext
/// with appended 16-byte Poly1305 authentication tag.
pub fn encrypt_payload(
    key: &[u8; 32],
    nonce: &[u8; 12],
    plaintext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce = Nonce::from_slice(nonce);
    cipher.encrypt(nonce, chacha20poly1305::aead::Payload { msg: plaintext, aad })
        .map_err(|e| anyhow::anyhow!("encryption failed: {e}"))
}

/// Decrypt and authenticate a signal payload with ChaCha20-Poly1305 AEAD.
///
/// Returns the plaintext only if the Poly1305 tag verifies against both
/// the ciphertext and the provided `aad`.
pub fn decrypt_payload(
    key: &[u8; 32],
    nonce: &[u8; 12],
    ciphertext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce = Nonce::from_slice(nonce);
    cipher.decrypt(nonce, chacha20poly1305::aead::Payload { msg: ciphertext, aad })
        .map_err(|e| anyhow::anyhow!("decryption failed: {e}"))
}

/// Produce a 64-byte Ed25519 signature over `data`.
#[allow(dead_code)] // Spec feature for Ed25519 signing
pub fn sign(signing_key: &[u8; 32], data: &[u8]) -> [u8; 64] {
    use ed25519_dalek::{SigningKey, Signer};
    let key = SigningKey::from_bytes(signing_key);
    let sig = key.sign(data);
    sig.to_bytes()
}

/// Verify an Ed25519 signature. Returns `false` on invalid key or signature.
pub fn verify(verify_key: &[u8; 32], data: &[u8], signature: &[u8; 64]) -> bool {
    use ed25519_dalek::{VerifyingKey, Verifier, Signature};
    let Ok(key) = VerifyingKey::from_bytes(verify_key) else { return false };
    let sig = Signature::from_bytes(signature);
    key.verify(data, &sig).is_ok()
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
        // Same inputs must produce identical keys (deterministic KDF).
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
        let keys = SessionKeys::derive(&[1; 32], &[2; 32]);
        let n1 = keys.next_nonce();
        let n2 = keys.next_nonce();
        assert_ne!(n1, n2);
    }

    #[test]
    fn test_directional_nonces() {
        let keys = SessionKeys::derive(&[1; 32], &[2; 32]);
        let send1 = keys.next_send_nonce();
        let send2 = keys.next_send_nonce();
        let recv1 = keys.next_recv_nonce();

        // Send nonces increment independently
        assert_ne!(send1, send2);
        // Recv nonce starts at 1 (same counter value as send1, since they're independent)
        assert_eq!(send1, recv1);
    }

    #[test]
    fn test_kdf_not_raw_secret() {
        // Verify that the derived key is NOT the raw shared secret
        // (i.e., the KDF is actually applied)
        let secret = [1u8; 32];
        let peer_pub = [2u8; 32];

        // Compute raw shared secret for comparison
        let raw_secret = x25519_dalek::StaticSecret::from(secret);
        let raw_public = x25519_dalek::PublicKey::from(peer_pub);
        let raw_shared = raw_secret.diffie_hellman(&raw_public);

        let keys = SessionKeys::derive(&secret, &peer_pub);
        assert_ne!(keys.encrypt_key, raw_shared.to_bytes(),
            "Derived key must not be the raw shared secret");
    }
}
