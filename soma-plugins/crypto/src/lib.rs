//! SOMA Crypto Plugin — 13 cryptographic conventions for the SOMA runtime.
//!
//! This plugin provides production-grade cryptographic primitives that the Mind
//! can invoke through structured conventions. Every operation uses well-audited
//! Rust crate implementations — no hand-rolled crypto.
//!
//! # Conventions
//!
//! | ID | Name | Description |
//! |----|------|-------------|
//! | 0 | `hash_sha256` | SHA-256 digest, returned as hex |
//! | 1 | `hash_argon2` | Password hash with random salt (Argon2id, PHC string) |
//! | 2 | `verify_argon2` | Verify password against Argon2 PHC hash |
//! | 3 | `random_bytes` | Cryptographically secure random bytes (OS entropy) |
//! | 4 | `random_hex` | Random bytes as hex string |
//! | 5 | `random_uuid` | UUID v4 |
//! | 6 | `sign_ed25519` | Ed25519 signature (deterministic, 64-byte output) |
//! | 7 | `verify_ed25519` | Ed25519 signature verification |
//! | 8 | `encrypt_aead` | ChaCha20-Poly1305 authenticated encryption |
//! | 9 | `decrypt_aead` | ChaCha20-Poly1305 authenticated decryption |
//! | 10 | `jwt_sign` | JWT signing with HS256 |
//! | 11 | `jwt_verify` | JWT verification and claims extraction |
//! | 12 | `hmac_sha256` | HMAC-SHA256 message authentication code |
//!
//! # Design choices
//!
//! - **Argon2id** (not bcrypt/scrypt): Winner of the Password Hashing Competition,
//!   resistant to both GPU and side-channel attacks. The `id` variant combines
//!   Argon2i (side-channel resistant) and Argon2d (GPU resistant).
//!
//! - **ChaCha20-Poly1305** (not AES-GCM): Constant-time without hardware AES
//!   instructions, making it safe on any CPU. Avoids AES-GCM's catastrophic
//!   nonce-reuse failure mode (`ChaCha20` degrades more gracefully). Also the
//!   cipher used in the Synaptic Protocol wire format.
//!
//! - **Ed25519** (not ECDSA): Deterministic signatures (no nonce to leak),
//!   faster verification, smaller keys. Uses the `ed25519-dalek` crate
//!   (same as `soma-core`'s protocol encryption).
//!
//! - **Nonce handling for AEAD**: `encrypt_aead` generates a random 12-byte
//!   nonce and prepends it to the ciphertext. `decrypt_aead` splits the first
//!   12 bytes as the nonce. This self-contained format avoids nonce-management
//!   bugs at the caller level.

#![allow(clippy::unnecessary_wraps)] // Convention methods must return Result per trait contract

use std::fmt::Write as _;

use argon2::password_hash::rand_core::OsRng as Argon2OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use hmac::Mac;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use rand::RngCore;
use sha2::{Digest, Sha256};
use soma_plugin_sdk::prelude::*;
use uuid::Uuid;

/// The SOMA crypto plugin.
///
/// Stateless — all keying material is supplied per-call via convention
/// arguments. No secrets are held in memory between invocations.
pub struct CryptoPlugin;

impl SomaPlugin for CryptoPlugin {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "crypto"
    }

    #[allow(clippy::unnecessary_literal_bound)]
    fn version(&self) -> &str {
        "0.1.0"
    }

    #[allow(clippy::unnecessary_literal_bound)]
    fn description(&self) -> &str {
        "Cryptographic operations: hashing, signing, encryption, JWT, random generation"
    }

    fn trust_level(&self) -> TrustLevel {
        TrustLevel::BuiltIn
    }

    #[allow(clippy::too_many_lines)]
    fn conventions(&self) -> Vec<Convention> {
        vec![
            // 0: hash_sha256
            Convention {
                id: 0,
                name: "hash_sha256".into(),
                description: "SHA-256 hash of data, returned as hex string".into(),
                call_pattern: "hash_sha256(data)".into(),
                args: vec![ArgSpec {
                    name: "data".into(),
                    arg_type: ArgType::String,
                    required: true,
                    description: "Data to hash".into(),
                }],
                returns: ReturnSpec::Value("String".into()),
                is_deterministic: true,
                estimated_latency_ms: 1,
                max_latency_ms: 10,
                side_effects: vec![],
                cleanup: None,
            },
            // 1: hash_argon2
            Convention {
                id: 1,
                name: "hash_argon2".into(),
                description: "Argon2 password hash with random salt, returned as PHC string".into(),
                call_pattern: "hash_argon2(password)".into(),
                args: vec![ArgSpec {
                    name: "password".into(),
                    arg_type: ArgType::String,
                    required: true,
                    description: "Password to hash".into(),
                }],
                returns: ReturnSpec::Value("String".into()),
                is_deterministic: false,
                estimated_latency_ms: 100,
                max_latency_ms: 2000,
                side_effects: vec![],
                cleanup: None,
            },
            // 2: verify_argon2
            Convention {
                id: 2,
                name: "verify_argon2".into(),
                description: "Verify a password against an Argon2 PHC hash string".into(),
                call_pattern: "verify_argon2(password, hash)".into(),
                args: vec![
                    ArgSpec {
                        name: "password".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Password to verify".into(),
                    },
                    ArgSpec {
                        name: "hash".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Argon2 PHC hash string to verify against".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bool".into()),
                is_deterministic: true,
                estimated_latency_ms: 100,
                max_latency_ms: 2000,
                side_effects: vec![],
                cleanup: None,
            },
            // 3: random_bytes
            Convention {
                id: 3,
                name: "random_bytes".into(),
                description: "Generate cryptographically secure random bytes".into(),
                call_pattern: "random_bytes(count)".into(),
                args: vec![ArgSpec {
                    name: "count".into(),
                    arg_type: ArgType::Int,
                    required: true,
                    description: "Number of random bytes to generate".into(),
                }],
                returns: ReturnSpec::Value("Bytes".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 10,
                side_effects: vec![],
                cleanup: None,
            },
            // 4: random_hex
            Convention {
                id: 4,
                name: "random_hex".into(),
                description: "Generate random bytes and return as hex string".into(),
                call_pattern: "random_hex(count)".into(),
                args: vec![ArgSpec {
                    name: "count".into(),
                    arg_type: ArgType::Int,
                    required: true,
                    description: "Number of random bytes (hex string will be 2x this length)".into(),
                }],
                returns: ReturnSpec::Value("String".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 10,
                side_effects: vec![],
                cleanup: None,
            },
            // 5: random_uuid
            Convention {
                id: 5,
                name: "random_uuid".into(),
                description: "Generate a random UUID v4".into(),
                call_pattern: "random_uuid()".into(),
                args: vec![],
                returns: ReturnSpec::Value("String".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 10,
                side_effects: vec![],
                cleanup: None,
            },
            // 6: sign_ed25519
            Convention {
                id: 6,
                name: "sign_ed25519".into(),
                description: "Sign data with Ed25519 private key".into(),
                call_pattern: "sign_ed25519(data, key)".into(),
                args: vec![
                    ArgSpec {
                        name: "data".into(),
                        arg_type: ArgType::Bytes,
                        required: true,
                        description: "Data to sign".into(),
                    },
                    ArgSpec {
                        name: "key".into(),
                        arg_type: ArgType::Bytes,
                        required: true,
                        description: "Ed25519 private key (32 bytes)".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bytes".into()),
                is_deterministic: true,
                estimated_latency_ms: 1,
                max_latency_ms: 50,
                side_effects: vec![],
                cleanup: None,
            },
            // 7: verify_ed25519
            Convention {
                id: 7,
                name: "verify_ed25519".into(),
                description: "Verify an Ed25519 signature".into(),
                call_pattern: "verify_ed25519(data, signature, pubkey)".into(),
                args: vec![
                    ArgSpec {
                        name: "data".into(),
                        arg_type: ArgType::Bytes,
                        required: true,
                        description: "Original data that was signed".into(),
                    },
                    ArgSpec {
                        name: "signature".into(),
                        arg_type: ArgType::Bytes,
                        required: true,
                        description: "Ed25519 signature (64 bytes)".into(),
                    },
                    ArgSpec {
                        name: "pubkey".into(),
                        arg_type: ArgType::Bytes,
                        required: true,
                        description: "Ed25519 public key (32 bytes)".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bool".into()),
                is_deterministic: true,
                estimated_latency_ms: 1,
                max_latency_ms: 50,
                side_effects: vec![],
                cleanup: None,
            },
            // 8: encrypt_aead
            Convention {
                id: 8,
                name: "encrypt_aead".into(),
                description: "Encrypt with ChaCha20-Poly1305; returns nonce (12 bytes) prepended to ciphertext".into(),
                call_pattern: "encrypt_aead(plaintext, key)".into(),
                args: vec![
                    ArgSpec {
                        name: "plaintext".into(),
                        arg_type: ArgType::Bytes,
                        required: true,
                        description: "Data to encrypt".into(),
                    },
                    ArgSpec {
                        name: "key".into(),
                        arg_type: ArgType::Bytes,
                        required: true,
                        description: "Encryption key (32 bytes)".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bytes".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 50,
                side_effects: vec![],
                cleanup: None,
            },
            // 9: decrypt_aead
            Convention {
                id: 9,
                name: "decrypt_aead".into(),
                description: "Decrypt ChaCha20-Poly1305 ciphertext (expects nonce prepended)".into(),
                call_pattern: "decrypt_aead(ciphertext, key)".into(),
                args: vec![
                    ArgSpec {
                        name: "ciphertext".into(),
                        arg_type: ArgType::Bytes,
                        required: true,
                        description: "Nonce (12 bytes) + ciphertext to decrypt".into(),
                    },
                    ArgSpec {
                        name: "key".into(),
                        arg_type: ArgType::Bytes,
                        required: true,
                        description: "Decryption key (32 bytes)".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bytes".into()),
                is_deterministic: true,
                estimated_latency_ms: 1,
                max_latency_ms: 50,
                side_effects: vec![],
                cleanup: None,
            },
            // 10: jwt_sign
            Convention {
                id: 10,
                name: "jwt_sign".into(),
                description: "Sign a JWT with HS256; claims is a JSON string".into(),
                call_pattern: "jwt_sign(claims, secret)".into(),
                args: vec![
                    ArgSpec {
                        name: "claims".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "JSON string of JWT claims".into(),
                    },
                    ArgSpec {
                        name: "secret".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "HMAC secret for signing".into(),
                    },
                ],
                returns: ReturnSpec::Value("String".into()),
                is_deterministic: true,
                estimated_latency_ms: 1,
                max_latency_ms: 50,
                side_effects: vec![],
                cleanup: None,
            },
            // 11: jwt_verify
            Convention {
                id: 11,
                name: "jwt_verify".into(),
                description: "Verify a JWT and return decoded claims as JSON string".into(),
                call_pattern: "jwt_verify(token, secret)".into(),
                args: vec![
                    ArgSpec {
                        name: "token".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "JWT token string".into(),
                    },
                    ArgSpec {
                        name: "secret".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "HMAC secret for verification".into(),
                    },
                ],
                returns: ReturnSpec::Value("String".into()),
                is_deterministic: true,
                estimated_latency_ms: 1,
                max_latency_ms: 50,
                side_effects: vec![],
                cleanup: None,
            },
            // 12: hmac_sha256
            Convention {
                id: 12,
                name: "hmac_sha256".into(),
                description: "Compute HMAC-SHA256 message authentication code".into(),
                call_pattern: "hmac_sha256(data, key)".into(),
                args: vec![
                    ArgSpec {
                        name: "data".into(),
                        arg_type: ArgType::Bytes,
                        required: true,
                        description: "Data to authenticate".into(),
                    },
                    ArgSpec {
                        name: "key".into(),
                        arg_type: ArgType::Bytes,
                        required: true,
                        description: "HMAC key".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bytes".into()),
                is_deterministic: true,
                estimated_latency_ms: 1,
                max_latency_ms: 10,
                side_effects: vec![],
                cleanup: None,
            },
        ]
    }

    fn execute(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        match convention_id {
            0 => Self::hash_sha256(&args),
            1 => Self::hash_argon2(&args),
            2 => Self::verify_argon2(&args),
            3 => Self::random_bytes(&args),
            4 => Self::random_hex(&args),
            5 => Self::random_uuid(),
            6 => Self::sign_ed25519(&args),
            7 => Self::verify_ed25519(&args),
            8 => Self::encrypt_aead(&args),
            9 => Self::decrypt_aead(&args),
            10 => Self::jwt_sign(&args),
            11 => Self::jwt_verify(&args),
            12 => Self::hmac_sha256(&args),
            _ => Err(PluginError::NotFound(format!(
                "unknown convention_id: {convention_id}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Convention implementations
// ---------------------------------------------------------------------------

impl CryptoPlugin {
    /// Convention 0 -- SHA-256 hash of a UTF-8 string, returned as lowercase hex.
    fn hash_sha256(args: &[Value]) -> Result<Value, PluginError> {
        let data = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: data".into()))?
            .as_str()?;

        let mut hasher = Sha256::new();
        hasher.update(data.as_bytes());
        let hash = hasher.finalize();
        let hex = hex_encode(&hash);
        Ok(Value::String(hex))
    }

    /// Convention 1 -- Argon2id password hash with random salt.
    ///
    /// Returns a PHC-format string (e.g. `$argon2id$v=19$m=19456,t=2,p=1$...`).
    /// Uses OS-sourced entropy for the salt via `OsRng`.
    fn hash_argon2(args: &[Value]) -> Result<Value, PluginError> {
        let password = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: password".into()))?
            .as_str()?;

        let salt = SaltString::generate(&mut Argon2OsRng);
        let argon2 = Argon2::default();
        let hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| PluginError::Failed(format!("argon2 hash failed: {e}")))?;

        Ok(Value::String(hash.to_string()))
    }

    /// Convention 2 -- Verify a password against an Argon2 PHC hash string.
    ///
    /// Returns `Value::Bool(true)` on match, `Value::Bool(false)` on mismatch.
    /// Invalid PHC format is an `InvalidArg` error, not a false result.
    fn verify_argon2(args: &[Value]) -> Result<Value, PluginError> {
        let password = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: password".into()))?
            .as_str()?;
        let hash_str = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: hash".into()))?
            .as_str()?;

        let parsed = PasswordHash::new(hash_str)
            .map_err(|e| PluginError::InvalidArg(format!("invalid PHC hash string: {e}")))?;
        let argon2 = Argon2::default();
        let valid = argon2
            .verify_password(password.as_bytes(), &parsed)
            .is_ok();

        Ok(Value::Bool(valid))
    }

    /// Convention 3 -- Generate cryptographically secure random bytes.
    ///
    /// Uses the OS CSPRNG via `rand::thread_rng()`. Count is clamped to
    /// `[1, 65536]` to prevent accidental multi-megabyte allocations.
    fn random_bytes(args: &[Value]) -> Result<Value, PluginError> {
        let count = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: count".into()))?
            .as_int()?;

        if count <= 0 || count > 65536 {
            return Err(PluginError::InvalidArg(
                "count must be between 1 and 65536".into(),
            ));
        }

        // Safety: count is validated to be in [1, 65536], so the cast is lossless.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let mut buf = vec![0u8; count as usize];
        rand::thread_rng().fill_bytes(&mut buf);
        Ok(Value::Bytes(buf))
    }

    /// Convention 4 -- Generate random bytes and return as lowercase hex string.
    ///
    /// The output string is `2 * count` characters long.
    fn random_hex(args: &[Value]) -> Result<Value, PluginError> {
        let count = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: count".into()))?
            .as_int()?;

        if count <= 0 || count > 65536 {
            return Err(PluginError::InvalidArg(
                "count must be between 1 and 65536".into(),
            ));
        }

        // Safety: count is validated to be in [1, 65536], so the cast is lossless.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let mut buf = vec![0u8; count as usize];
        rand::thread_rng().fill_bytes(&mut buf);
        Ok(Value::String(hex_encode(&buf)))
    }

    /// Convention 5 -- Generate a random UUID v4 string.
    fn random_uuid() -> Result<Value, PluginError> {
        Ok(Value::String(Uuid::new_v4().to_string()))
    }

    /// Convention 6 -- Sign data with an Ed25519 private key (32 bytes seed).
    ///
    /// Returns the 64-byte signature as `Value::Bytes`. The signature is
    /// deterministic for the same key+data pair (RFC 8032).
    fn sign_ed25519(args: &[Value]) -> Result<Value, PluginError> {
        let data = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: data".into()))?
            .as_bytes()?;
        let key_bytes = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: key".into()))?
            .as_bytes()?;

        let key_array: [u8; 32] = key_bytes
            .try_into()
            .map_err(|_| PluginError::InvalidArg("Ed25519 private key must be 32 bytes".into()))?;

        let signing_key = SigningKey::from_bytes(&key_array);
        let signature = signing_key.sign(data);

        Ok(Value::Bytes(signature.to_bytes().to_vec()))
    }

    /// Convention 7 -- Verify an Ed25519 signature against data and public key.
    ///
    /// Returns `Value::Bool(true)` if valid, `Value::Bool(false)` if the
    /// signature does not match. Malformed keys/signatures are `InvalidArg`.
    fn verify_ed25519(args: &[Value]) -> Result<Value, PluginError> {
        let data = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: data".into()))?
            .as_bytes()?;
        let sig_bytes = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: signature".into()))?
            .as_bytes()?;
        let pubkey_bytes = args
            .get(2)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: pubkey".into()))?
            .as_bytes()?;

        let sig_array: [u8; 64] = sig_bytes
            .try_into()
            .map_err(|_| PluginError::InvalidArg("Ed25519 signature must be 64 bytes".into()))?;
        let pub_array: [u8; 32] = pubkey_bytes
            .try_into()
            .map_err(|_| PluginError::InvalidArg("Ed25519 public key must be 32 bytes".into()))?;

        let signature = ed25519_dalek::Signature::from_bytes(&sig_array);
        let verifying_key = VerifyingKey::from_bytes(&pub_array)
            .map_err(|e| PluginError::InvalidArg(format!("invalid Ed25519 public key: {e}")))?;

        let valid = verifying_key.verify(data, &signature).is_ok();
        Ok(Value::Bool(valid))
    }

    /// Convention 8 -- Encrypt with ChaCha20-Poly1305 AEAD.
    ///
    /// Generates a random 12-byte nonce and prepends it to the ciphertext.
    /// Output format: `[nonce: 12 bytes][ciphertext + 16-byte Poly1305 tag]`.
    /// The key must be exactly 32 bytes.
    fn encrypt_aead(args: &[Value]) -> Result<Value, PluginError> {
        let plaintext = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: plaintext".into()))?
            .as_bytes()?;
        let key_bytes = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: key".into()))?
            .as_bytes()?;

        if key_bytes.len() != 32 {
            return Err(PluginError::InvalidArg(
                "ChaCha20-Poly1305 key must be 32 bytes".into(),
            ));
        }

        let cipher = ChaCha20Poly1305::new_from_slice(key_bytes)
            .map_err(|e| PluginError::Failed(format!("cipher init failed: {e}")))?;

        // Generate random 12-byte nonce via OS CSPRNG
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| PluginError::Failed(format!("encryption failed: {e}")))?;

        // Prepend nonce to ciphertext so decryption is self-contained
        let mut result = Vec::with_capacity(12 + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);

        Ok(Value::Bytes(result))
    }

    /// Convention 9 -- Decrypt ChaCha20-Poly1305 ciphertext.
    ///
    /// Expects the nonce (12 bytes) prepended to the ciphertext, matching
    /// the format produced by `encrypt_aead`. Authenticates before decrypting
    /// (AEAD guarantee) -- returns `Failed` on tampered data.
    fn decrypt_aead(args: &[Value]) -> Result<Value, PluginError> {
        let combined = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: ciphertext".into()))?
            .as_bytes()?;
        let key_bytes = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: key".into()))?
            .as_bytes()?;

        if key_bytes.len() != 32 {
            return Err(PluginError::InvalidArg(
                "ChaCha20-Poly1305 key must be 32 bytes".into(),
            ));
        }
        if combined.len() < 12 {
            return Err(PluginError::InvalidArg(
                "ciphertext too short: must include 12-byte nonce prefix".into(),
            ));
        }

        let (nonce_bytes, ciphertext) = combined.split_at(12);
        let cipher = ChaCha20Poly1305::new_from_slice(key_bytes)
            .map_err(|e| PluginError::Failed(format!("cipher init failed: {e}")))?;
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| PluginError::Failed(format!("decryption failed: {e}")))?;

        Ok(Value::Bytes(plaintext))
    }

    /// Convention 10 -- Sign a JWT with HS256.
    ///
    /// `claims` must be a valid JSON string. The `secret` is used as the HMAC
    /// key. Returns the compact JWT string (`header.payload.signature`).
    fn jwt_sign(args: &[Value]) -> Result<Value, PluginError> {
        let claims_json = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: claims".into()))?
            .as_str()?;
        let secret = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: secret".into()))?
            .as_str()?;

        let claims: serde_json::Value = serde_json::from_str(claims_json)
            .map_err(|e| PluginError::InvalidArg(format!("invalid JSON claims: {e}")))?;

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .map_err(|e| PluginError::Failed(format!("JWT signing failed: {e}")))?;

        Ok(Value::String(token))
    }

    /// Convention 11 -- Verify and decode a JWT signed with HS256.
    ///
    /// Validates the signature but does *not* enforce `exp` claims, allowing
    /// the caller to handle expiration logic. Returns the claims as a JSON string.
    fn jwt_verify(args: &[Value]) -> Result<Value, PluginError> {
        let token = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: token".into()))?
            .as_str()?;
        let secret = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: secret".into()))?
            .as_str()?;

        let mut validation = Validation::default();
        validation.required_spec_claims.clear();
        validation.validate_exp = false;

        let token_data = decode::<serde_json::Value>(
            token,
            &DecodingKey::from_secret(secret.as_bytes()),
            &validation,
        )
        .map_err(|e| PluginError::Failed(format!("JWT verification failed: {e}")))?;

        let claims_json = serde_json::to_string(&token_data.claims)
            .map_err(|e| PluginError::Failed(format!("JSON serialization failed: {e}")))?;

        Ok(Value::String(claims_json))
    }

    /// Convention 12 -- HMAC-SHA256 message authentication code.
    ///
    /// Accepts arbitrary-length key and data. Returns the 32-byte MAC
    /// as `Value::Bytes`.
    fn hmac_sha256(args: &[Value]) -> Result<Value, PluginError> {
        let data = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: data".into()))?
            .as_bytes()?;
        let key = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: key".into()))?
            .as_bytes()?;

        let mut mac = <hmac::Hmac<Sha256> as Mac>::new_from_slice(key)
            .map_err(|e| PluginError::Failed(format!("HMAC init failed: {e}")))?;
        mac.update(data);
        let result = mac.finalize();

        Ok(Value::Bytes(result.into_bytes().to_vec()))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Encode a byte slice as a lowercase hex string.
///
/// Uses `write!` into a pre-allocated `String` to avoid intermediate
/// allocations per byte.
fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // write! to a String is infallible, but we must handle the Result.
        let _ = write!(s, "{b:02x}");
    }
    s
}

// ---------------------------------------------------------------------------
// C ABI entry point
// ---------------------------------------------------------------------------

/// FFI entry point called by the SOMA plugin loader (`plugin/dynamic.rs`).
///
/// Returns a heap-allocated `CryptoPlugin` as a trait object pointer.
/// The caller takes ownership and is responsible for eventually dropping it.
#[allow(improper_ctypes_definitions)] // Trait objects have no C equivalent; SOMA uses a known ABI.
#[unsafe(no_mangle)]
pub extern "C" fn soma_plugin_init() -> *mut dyn SomaPlugin {
    Box::into_raw(Box::new(CryptoPlugin))
}
