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
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce, aead::{Aead, KeyInit}};

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
        use ed25519_dalek::SigningKey;
        use rand::rngs::OsRng;

        let signing = SigningKey::generate(&mut OsRng);
        let verify = signing.verifying_key();

        // Derive X25519 from Ed25519 seed
        let x_secret = x25519_dalek::StaticSecret::from(signing.to_bytes());
        let x_public = x25519_dalek::PublicKey::from(&x_secret);

        Self {
            signing_key: signing.to_bytes(),
            verify_key: verify.to_bytes(),
            x25519_secret: x_secret.to_bytes(),
            x25519_public: x_public.to_bytes(),
        }
    }

    /// Load identity from a 32-byte seed (deterministic key derivation).
    pub fn from_seed(seed: [u8; 32]) -> Self {
        use ed25519_dalek::SigningKey;

        let signing = SigningKey::from_bytes(&seed);
        let verify = signing.verifying_key();

        // Derive X25519 from Ed25519 seed
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

/// Shared session key derived from X25519 key exchange.
pub struct SessionKeys {
    /// Encryption key (32 bytes, derived from ECDH shared secret)
    pub encrypt_key: [u8; 32],
    /// Nonce counter for ChaCha20-Poly1305
    pub nonce_counter: u64,
}

impl SessionKeys {
    /// Derive session keys from our secret and peer's public key via X25519 ECDH.
    pub fn derive(our_secret: &[u8; 32], peer_public: &[u8; 32]) -> Self {
        let secret = x25519_dalek::StaticSecret::from(*our_secret);
        let public = x25519_dalek::PublicKey::from(*peer_public);
        let shared = secret.diffie_hellman(&public);
        Self {
            encrypt_key: shared.to_bytes(),
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

/// Encrypt a signal payload using ChaCha20-Poly1305 AEAD.
/// Returns ciphertext with appended 16-byte authentication tag.
pub fn encrypt_payload(
    key: &[u8; 32],
    nonce: &[u8; 12],
    plaintext: &[u8],
    aad: &[u8], // additional authenticated data (signal header)
) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce = Nonce::from_slice(nonce);
    cipher.encrypt(nonce, chacha20poly1305::aead::Payload { msg: plaintext, aad })
        .map_err(|e| anyhow::anyhow!("encryption failed: {}", e))
}

/// Decrypt a signal payload using ChaCha20-Poly1305 AEAD.
/// Input is ciphertext with appended authentication tag.
/// Returns plaintext if authentication succeeds.
pub fn decrypt_payload(
    key: &[u8; 32],
    nonce: &[u8; 12],
    ciphertext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce = Nonce::from_slice(nonce);
    cipher.decrypt(nonce, chacha20poly1305::aead::Payload { msg: ciphertext, aad })
        .map_err(|e| anyhow::anyhow!("decryption failed: {}", e))
}

/// Sign data with Ed25519.
pub fn sign(signing_key: &[u8; 32], data: &[u8]) -> [u8; 64] {
    use ed25519_dalek::{SigningKey, Signer};
    let key = SigningKey::from_bytes(signing_key);
    let sig = key.sign(data);
    sig.to_bytes()
}

/// Verify an Ed25519 signature.
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
