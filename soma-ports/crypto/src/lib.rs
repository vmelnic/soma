//! SOMA Crypto Port Pack -- 13 cryptographic capabilities.
//!
//! | Capability      | Description                                      |
//! |-----------------|--------------------------------------------------|
//! | sha256          | SHA-256 digest, returned as hex                  |
//! | sha512          | SHA-512 digest, returned as hex                  |
//! | hmac            | HMAC-SHA256 message authentication code          |
//! | bcrypt_hash     | bcrypt password hash with random salt             |
//! | bcrypt_verify   | Verify password against bcrypt hash               |
//! | aes_encrypt     | AES-256-GCM authenticated encryption              |
//! | aes_decrypt     | AES-256-GCM authenticated decryption              |
//! | rsa_sign        | RSA-PKCS1v15 SHA-256 signature                    |
//! | rsa_verify      | RSA-PKCS1v15 SHA-256 signature verification       |
//! | jwt_sign        | Sign JWT with HS256                               |
//! | jwt_verify      | Verify JWT and return decoded claims              |
//! | random_bytes    | Cryptographically secure random bytes             |
//! | random_string   | Random alphanumeric string                        |
//!
//! All keying material is supplied per-call. No secrets are held between
//! invocations.

use std::fmt::Write as _;
use std::time::Instant;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce as AesNonce};
use hmac::Mac;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use rand::RngCore;
use rsa::pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey};
use rsa::pkcs1v15::{SigningKey, VerifyingKey};
use rsa::signature::{SignatureEncoding, Signer, Verifier};
use sha2::{Digest, Sha256, Sha512};
use soma_port_sdk::prelude::*;

pub struct CryptoPort {
    spec: PortSpec,
}

impl CryptoPort {
    pub fn new() -> Self {
        Self { spec: build_spec() }
    }
}

impl Port for CryptoPort {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(
        &self,
        capability_id: &str,
        input: serde_json::Value,
    ) -> soma_port_sdk::Result<PortCallRecord> {
        let start = Instant::now();
        let result = match capability_id {
            "sha256" => exec_sha256(&input),
            "sha512" => exec_sha512(&input),
            "hmac" => exec_hmac(&input),
            "bcrypt_hash" => exec_bcrypt_hash(&input),
            "bcrypt_verify" => exec_bcrypt_verify(&input),
            "aes_encrypt" => exec_aes_encrypt(&input),
            "aes_decrypt" => exec_aes_decrypt(&input),
            "rsa_sign" => exec_rsa_sign(&input),
            "rsa_verify" => exec_rsa_verify(&input),
            "jwt_sign" => exec_jwt_sign(&input),
            "jwt_verify" => exec_jwt_verify(&input),
            "random_bytes" => exec_random_bytes(&input),
            "random_string" => exec_random_string(&input),
            _ => {
                return Err(PortError::NotFound(format!(
                    "unknown capability: {capability_id}"
                )));
            }
        };
        let elapsed = start.elapsed().as_millis() as u64;
        match result {
            Ok(val) => Ok(PortCallRecord::success("crypto", capability_id, val, elapsed)),
            Err(e) => Ok(PortCallRecord::failure(
                "crypto",
                capability_id,
                e.failure_class(),
                &e.to_string(),
                elapsed,
            )),
        }
    }

    fn validate_input(
        &self,
        capability_id: &str,
        _input: &serde_json::Value,
    ) -> soma_port_sdk::Result<()> {
        if self.spec.capabilities.iter().any(|c| c.capability_id == capability_id) {
            Ok(())
        } else {
            Err(PortError::NotFound(format!(
                "unknown capability: {capability_id}"
            )))
        }
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        PortLifecycleState::Active
    }
}

// ---------------------------------------------------------------------------
// Input helpers
// ---------------------------------------------------------------------------

fn get_str<'a>(input: &'a serde_json::Value, field: &str) -> soma_port_sdk::Result<&'a str> {
    input
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| PortError::Validation(format!("missing string field: {field}")))
}

fn get_bytes(input: &serde_json::Value, field: &str) -> soma_port_sdk::Result<Vec<u8>> {
    if let Some(s) = input.get(field).and_then(|v| v.as_str()) {
        Ok(s.as_bytes().to_vec())
    } else if let Some(arr) = input.get(field).and_then(|v| v.as_array()) {
        arr.iter()
            .map(|v| {
                v.as_u64()
                    .map(|n| n as u8)
                    .ok_or_else(|| PortError::Validation(format!("invalid byte in {field}")))
            })
            .collect()
    } else {
        Err(PortError::Validation(format!("missing field: {field}")))
    }
}

fn get_int(input: &serde_json::Value, field: &str) -> soma_port_sdk::Result<i64> {
    input
        .get(field)
        .and_then(|v| v.as_i64())
        .ok_or_else(|| PortError::Validation(format!("missing integer field: {field}")))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn hex_decode(hex: &str) -> soma_port_sdk::Result<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return Err(PortError::Validation("hex string must have even length".into()));
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|_| PortError::Validation("invalid hex character".into()))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Capability implementations
// ---------------------------------------------------------------------------

fn exec_sha256(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let data = get_str(input, "data")?;
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    Ok(serde_json::json!({ "hash": hex_encode(&hasher.finalize()) }))
}

fn exec_sha512(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let data = get_str(input, "data")?;
    let mut hasher = Sha512::new();
    hasher.update(data.as_bytes());
    Ok(serde_json::json!({ "hash": hex_encode(&hasher.finalize()) }))
}

fn exec_hmac(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let data = get_bytes(input, "data")?;
    let key = get_bytes(input, "key")?;
    let mut mac = <hmac::Hmac<Sha256> as Mac>::new_from_slice(&key)
        .map_err(|e| PortError::Internal(format!("HMAC init failed: {e}")))?;
    mac.update(&data);
    let result = mac.finalize();
    Ok(serde_json::json!({ "mac": hex_encode(&result.into_bytes()) }))
}

fn exec_bcrypt_hash(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let password = get_str(input, "password")?;
    let cost = input
        .get("cost")
        .and_then(|v| v.as_u64())
        .unwrap_or(12) as u32;
    let hash = bcrypt::hash(password, cost)
        .map_err(|e| PortError::ExternalError(format!("bcrypt hash failed: {e}")))?;
    Ok(serde_json::json!({ "hash": hash }))
}

fn exec_bcrypt_verify(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let password = get_str(input, "password")?;
    let hash = get_str(input, "hash")?;
    let valid = bcrypt::verify(password, hash)
        .map_err(|e| PortError::ExternalError(format!("bcrypt verify failed: {e}")))?;
    Ok(serde_json::json!({ "valid": valid }))
}

fn exec_aes_encrypt(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let plaintext = get_bytes(input, "plaintext")?;
    let key = get_bytes(input, "key")?;
    if key.len() != 32 {
        return Err(PortError::Validation(
            "AES-256-GCM key must be 32 bytes".into(),
        ));
    }
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| PortError::Internal(format!("cipher init: {e}")))?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = AesNonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_ref())
        .map_err(|e| PortError::ExternalError(format!("encryption failed: {e}")))?;
    // Prepend nonce to ciphertext so decryption is self-contained
    let mut combined = Vec::with_capacity(12 + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);
    Ok(serde_json::json!({ "ciphertext": combined }))
}

fn exec_aes_decrypt(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let combined = get_bytes(input, "ciphertext")?;
    let key = get_bytes(input, "key")?;
    if key.len() != 32 {
        return Err(PortError::Validation(
            "AES-256-GCM key must be 32 bytes".into(),
        ));
    }
    if combined.len() < 12 {
        return Err(PortError::Validation(
            "ciphertext too short: must include 12-byte nonce prefix".into(),
        ));
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| PortError::Internal(format!("cipher init: {e}")))?;
    let nonce = AesNonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| PortError::ExternalError(format!("decryption failed: {e}")))?;
    Ok(serde_json::json!({ "plaintext": plaintext }))
}

fn exec_rsa_sign(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let data = get_bytes(input, "data")?;
    let private_key_pem = get_str(input, "private_key_pem")?;
    let rsa_key = rsa::RsaPrivateKey::from_pkcs1_pem(private_key_pem)
        .map_err(|e| PortError::Validation(format!("invalid RSA private key PEM: {e}")))?;
    let signing_key = SigningKey::<Sha256>::new(rsa_key);
    let signature = signing_key.sign(&data);
    Ok(serde_json::json!({ "signature": hex_encode(&signature.to_bytes()) }))
}

fn exec_rsa_verify(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let data = get_bytes(input, "data")?;
    let signature_hex = get_str(input, "signature")?;
    let public_key_pem = get_str(input, "public_key_pem")?;
    let sig_bytes = hex_decode(signature_hex)?;
    let rsa_key = rsa::RsaPublicKey::from_pkcs1_pem(public_key_pem)
        .map_err(|e| PortError::Validation(format!("invalid RSA public key PEM: {e}")))?;
    let verifying_key = VerifyingKey::<Sha256>::new(rsa_key);
    let signature = rsa::pkcs1v15::Signature::try_from(sig_bytes.as_slice())
        .map_err(|e| PortError::Validation(format!("invalid RSA signature: {e}")))?;
    let valid = verifying_key.verify(&data, &signature).is_ok();
    Ok(serde_json::json!({ "valid": valid }))
}

fn exec_jwt_sign(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let claims_json = get_str(input, "claims")?;
    let secret = get_str(input, "secret")?;
    let claims: serde_json::Value = serde_json::from_str(claims_json)
        .map_err(|e| PortError::Validation(format!("invalid JSON claims: {e}")))?;
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| PortError::ExternalError(format!("JWT signing failed: {e}")))?;
    Ok(serde_json::json!({ "token": token }))
}

fn exec_jwt_verify(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let token = get_str(input, "token")?;
    let secret = get_str(input, "secret")?;
    let mut validation = Validation::default();
    validation.required_spec_claims.clear();
    validation.validate_exp = false;
    let token_data = decode::<serde_json::Value>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|e| PortError::ExternalError(format!("JWT verification failed: {e}")))?;
    Ok(serde_json::json!({ "claims": token_data.claims }))
}

fn exec_random_bytes(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let count = get_int(input, "count")?;
    if count <= 0 || count > 65536 {
        return Err(PortError::Validation(
            "count must be between 1 and 65536".into(),
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let mut buf = vec![0u8; count as usize];
    rand::thread_rng().fill_bytes(&mut buf);
    Ok(serde_json::json!({ "bytes": buf }))
}

fn exec_random_string(input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
    let length = get_int(input, "length")?;
    if length <= 0 || length > 65536 {
        return Err(PortError::Validation(
            "length must be between 1 and 65536".into(),
        ));
    }
    use rand::Rng;
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let s: String = (0..length as usize)
        .map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char)
        .collect();
    Ok(serde_json::json!({ "value": s }))
}

// ---------------------------------------------------------------------------
// PortSpec builder
// ---------------------------------------------------------------------------

fn cap(
    id: &str,
    name: &str,
    purpose: &str,
    effect: SideEffectClass,
    determinism: DeterminismClass,
    latency_ms: u64,
) -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: id.to_string(),
        name: name.to_string(),
        purpose: purpose.to_string(),
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::any(),
        effect_class: effect,
        rollback_support: RollbackSupport::Irreversible,
        determinism_class: determinism,
        idempotence_class: IdempotenceClass::Idempotent,
        risk_class: RiskClass::Negligible,
        latency_profile: LatencyProfile {
            expected_latency_ms: latency_ms,
            p95_latency_ms: latency_ms * 5,
            max_latency_ms: latency_ms * 20,
        },
        cost_profile: CostProfile::default(),
        remote_exposable: false,
        auth_override: None,
    }
}

fn build_spec() -> PortSpec {
    PortSpec {
        port_id: "crypto".to_string(),
        name: "Crypto".to_string(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Custom,
        description: "Cryptographic operations: hashing, signing, encryption, JWT, random generation".to_string(),
        namespace: "soma.ports.crypto".to_string(),
        trust_level: TrustLevel::Trusted,
        capabilities: vec![
            cap("sha256", "SHA-256", "Compute SHA-256 hex digest", SideEffectClass::None, DeterminismClass::Deterministic, 1),
            cap("sha512", "SHA-512", "Compute SHA-512 hex digest", SideEffectClass::None, DeterminismClass::Deterministic, 1),
            cap("hmac", "HMAC-SHA256", "Compute HMAC-SHA256 authentication code", SideEffectClass::None, DeterminismClass::Deterministic, 1),
            cap("bcrypt_hash", "bcrypt hash", "Hash a password with bcrypt", SideEffectClass::None, DeterminismClass::Stochastic, 100),
            cap("bcrypt_verify", "bcrypt verify", "Verify password against bcrypt hash", SideEffectClass::None, DeterminismClass::Deterministic, 100),
            cap("aes_encrypt", "AES-256-GCM encrypt", "Encrypt data with AES-256-GCM", SideEffectClass::None, DeterminismClass::Stochastic, 1),
            cap("aes_decrypt", "AES-256-GCM decrypt", "Decrypt AES-256-GCM ciphertext", SideEffectClass::None, DeterminismClass::Deterministic, 1),
            cap("rsa_sign", "RSA sign", "Sign data with RSA-PKCS1v15-SHA256", SideEffectClass::None, DeterminismClass::Deterministic, 10),
            cap("rsa_verify", "RSA verify", "Verify RSA-PKCS1v15-SHA256 signature", SideEffectClass::None, DeterminismClass::Deterministic, 5),
            cap("jwt_sign", "JWT sign", "Sign JWT with HS256", SideEffectClass::None, DeterminismClass::Deterministic, 1),
            cap("jwt_verify", "JWT verify", "Verify JWT and decode claims", SideEffectClass::None, DeterminismClass::Deterministic, 1),
            cap("random_bytes", "Random bytes", "Generate cryptographically secure random bytes", SideEffectClass::None, DeterminismClass::Stochastic, 1),
            cap("random_string", "Random string", "Generate random alphanumeric string", SideEffectClass::None, DeterminismClass::Stochastic, 1),
        ],
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::any(),
        failure_modes: vec![PortFailureClass::ValidationError, PortFailureClass::ExternalError],
        side_effect_class: SideEffectClass::None,
        latency_profile: LatencyProfile {
            expected_latency_ms: 1,
            p95_latency_ms: 50,
            max_latency_ms: 2000,
        },
        cost_profile: CostProfile::default(),
        auth_requirements: AuthRequirements::default(),
        sandbox_requirements: SandboxRequirements::default(),
        observable_fields: vec!["capability_id".into(), "latency_ms".into()],
        validation_rules: vec![],
        remote_exposure: false,
    }
}

// ---------------------------------------------------------------------------
// C ABI entry point
// ---------------------------------------------------------------------------

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    Box::into_raw(Box::new(CryptoPort::new()))
}
