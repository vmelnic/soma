//! SOMA Auth Port Pack -- 10 authentication capabilities.
//!
//! | Capability        | Description                                    |
//! |-------------------|------------------------------------------------|
//! | otp_generate      | Generate a 6-digit OTP code for a phone number |
//! | otp_verify        | Verify OTP code (in-memory, max 5 attempts)    |
//! | session_create    | Create an authenticated session token          |
//! | session_validate  | Validate a session token                       |
//! | session_revoke    | Revoke a session by token                      |
//! | totp_generate     | Generate a TOTP secret and provisioning URI    |
//! | totp_verify       | Verify a TOTP code against a secret            |
//! | token_generate    | Generate a random bearer token                 |
//! | token_validate    | Validate a bearer token                        |
//! | token_refresh     | Refresh a bearer token (extend expiry)         |
//!
//! This port pack is self-contained with in-memory session/OTP storage.
//! A production deployment would back this with a database port dependency.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use soma_port_sdk::prelude::*;
use totp_rs::{Algorithm, Secret, TOTP};

pub struct AuthPort {
    spec: PortSpec,
    /// In-memory OTP store: phone -> (code_hash, expires_at, attempts)
    otps: Mutex<HashMap<String, OtpEntry>>,
    /// In-memory session store: token_hash -> SessionEntry
    sessions: Mutex<HashMap<String, SessionEntry>>,
    /// In-memory token store: token_hash -> TokenEntry
    tokens: Mutex<HashMap<String, TokenEntry>>,
}

struct OtpEntry {
    code_hash: String,
    expires_at: chrono::DateTime<Utc>,
    attempts: u32,
    verified: bool,
}

struct SessionEntry {
    user_id: String,
    #[allow(dead_code)]
    device_info: String,
    #[allow(dead_code)]
    created_at: chrono::DateTime<Utc>,
    expires_at: chrono::DateTime<Utc>,
    revoked: bool,
}

struct TokenEntry {
    user_id: String,
    #[allow(dead_code)]
    created_at: chrono::DateTime<Utc>,
    expires_at: chrono::DateTime<Utc>,
    revoked: bool,
}

impl AuthPort {
    pub fn new() -> Self {
        Self {
            spec: build_spec(),
            otps: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            tokens: Mutex::new(HashMap::new()),
        }
    }

    fn sha256_hex(input: &str) -> String {
        let mut h = Sha256::new();
        h.update(input.as_bytes());
        format!("{:x}", h.finalize())
    }

    fn gen_otp() -> String {
        use rand::Rng;
        format!("{:06}", rand::thread_rng().gen_range(0u32..1_000_000))
    }

    fn gen_token() -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

impl Port for AuthPort {
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
            "otp_generate" => self.exec_otp_generate(&input),
            "otp_verify" => self.exec_otp_verify(&input),
            "session_create" => self.exec_session_create(&input),
            "session_validate" => self.exec_session_validate(&input),
            "session_revoke" => self.exec_session_revoke(&input),
            "totp_generate" => self.exec_totp_generate(&input),
            "totp_verify" => self.exec_totp_verify(&input),
            "token_generate" => self.exec_token_generate(&input),
            "token_validate" => self.exec_token_validate(&input),
            "token_refresh" => self.exec_token_refresh(&input),
            _ => {
                return Err(PortError::NotFound(format!(
                    "unknown capability: {capability_id}"
                )));
            }
        };
        let elapsed = start.elapsed().as_millis() as u64;
        match result {
            Ok(val) => Ok(PortCallRecord::success("auth", capability_id, val, elapsed)),
            Err(e) => Ok(PortCallRecord::failure(
                "auth",
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

// ---------------------------------------------------------------------------
// Capability implementations
// ---------------------------------------------------------------------------

impl AuthPort {
    fn exec_otp_generate(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let phone = get_str(input, "phone")?;
        let code = Self::gen_otp();
        let code_hash = Self::sha256_hex(&code);
        let expires_at = Utc::now() + Duration::minutes(5);

        let entry = OtpEntry {
            code_hash,
            expires_at,
            attempts: 0,
            verified: false,
        };
        self.otps.lock().unwrap().insert(phone.to_string(), entry);

        Ok(serde_json::json!({
            "phone": phone,
            "debug_code": code,
            "expires_at": expires_at.to_rfc3339()
        }))
    }

    fn exec_otp_verify(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let phone = get_str(input, "phone")?;
        let code = get_str(input, "code")?;
        let code_hash = Self::sha256_hex(code);

        let mut otps = self.otps.lock().unwrap();
        if let Some(entry) = otps.get_mut(phone) {
            entry.attempts += 1;
            if entry.verified {
                return Ok(serde_json::json!({ "valid": false, "reason": "already verified" }));
            }
            if entry.attempts > 5 {
                return Ok(serde_json::json!({ "valid": false, "reason": "max attempts exceeded" }));
            }
            if Utc::now() > entry.expires_at {
                return Ok(serde_json::json!({ "valid": false, "reason": "expired" }));
            }
            if entry.code_hash == code_hash {
                entry.verified = true;
                return Ok(serde_json::json!({ "valid": true, "user_id": format!("phone:{phone}") }));
            }
            Ok(serde_json::json!({ "valid": false, "reason": "incorrect code" }))
        } else {
            Ok(serde_json::json!({ "valid": false, "reason": "no OTP found for phone" }))
        }
    }

    fn exec_session_create(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let user_id = get_str(input, "user_id")?;
        let device_info = input
            .get("device_info")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let ttl_hours = input
            .get("ttl_hours")
            .and_then(|v| v.as_i64())
            .unwrap_or(720);

        let token = Self::gen_token();
        let token_hash = Self::sha256_hex(&token);
        let now = Utc::now();
        let expires_at = now + Duration::hours(ttl_hours);

        let entry = SessionEntry {
            user_id: user_id.to_string(),
            device_info: device_info.to_string(),
            created_at: now,
            expires_at,
            revoked: false,
        };
        self.sessions.lock().unwrap().insert(token_hash, entry);

        Ok(serde_json::json!({
            "token": token,
            "user_id": user_id,
            "expires_at": expires_at.to_rfc3339()
        }))
    }

    fn exec_session_validate(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let token = get_str(input, "token")?;
        let token_hash = Self::sha256_hex(token);

        let sessions = self.sessions.lock().unwrap();
        if let Some(entry) = sessions.get(&token_hash) {
            if entry.revoked {
                return Ok(serde_json::json!({ "valid": false, "reason": "revoked" }));
            }
            if Utc::now() > entry.expires_at {
                return Ok(serde_json::json!({ "valid": false, "reason": "expired" }));
            }
            Ok(serde_json::json!({ "valid": true, "user_id": entry.user_id }))
        } else {
            Ok(serde_json::json!({ "valid": false, "reason": "session not found" }))
        }
    }

    fn exec_session_revoke(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let token = get_str(input, "token")?;
        let token_hash = Self::sha256_hex(token);

        let mut sessions = self.sessions.lock().unwrap();
        if let Some(entry) = sessions.get_mut(&token_hash) {
            entry.revoked = true;
            Ok(serde_json::json!({ "revoked": true }))
        } else {
            Ok(serde_json::json!({ "revoked": false, "reason": "session not found" }))
        }
    }

    fn exec_totp_generate(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let user_id = get_str(input, "user_id")?;
        let secret = Secret::generate_secret();
        let secret_base32 = secret.to_encoded().to_string();
        let totp = TOTP::new(
            Algorithm::SHA1,
            6,
            1,
            30,
            secret.to_bytes().unwrap(),
            Some("SOMA".to_string()),
            user_id.to_string(),
        )
        .map_err(|e| PortError::Internal(format!("TOTP init failed: {e}")))?;
        let uri = totp.get_url();
        Ok(serde_json::json!({
            "secret": secret_base32,
            "provisioning_uri": uri,
            "user_id": user_id
        }))
    }

    fn exec_totp_verify(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let secret_base32 = get_str(input, "secret")?;
        let code = get_str(input, "code")?;
        let secret = Secret::Encoded(secret_base32.to_string());
        let secret_bytes = secret
            .to_bytes()
            .map_err(|e| PortError::Validation(format!("invalid TOTP secret: {e}")))?;
        let totp = TOTP::new(
            Algorithm::SHA1,
            6,
            1,
            30,
            secret_bytes,
            Some("SOMA".to_string()),
            "user".to_string(),
        )
        .map_err(|e| PortError::Internal(format!("TOTP init failed: {e}")))?;
        let valid = totp.check_current(code)
            .map_err(|e| PortError::Internal(format!("TOTP check failed: {e}")))?;
        Ok(serde_json::json!({ "valid": valid }))
    }

    fn exec_token_generate(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let user_id = get_str(input, "user_id")?;
        let ttl_hours = input
            .get("ttl_hours")
            .and_then(|v| v.as_i64())
            .unwrap_or(24);

        let token = Self::gen_token();
        let token_hash = Self::sha256_hex(&token);
        let now = Utc::now();
        let expires_at = now + Duration::hours(ttl_hours);

        let entry = TokenEntry {
            user_id: user_id.to_string(),
            created_at: now,
            expires_at,
            revoked: false,
        };
        self.tokens.lock().unwrap().insert(token_hash, entry);

        Ok(serde_json::json!({
            "token": token,
            "user_id": user_id,
            "expires_at": expires_at.to_rfc3339()
        }))
    }

    fn exec_token_validate(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let token = get_str(input, "token")?;
        let token_hash = Self::sha256_hex(token);

        let tokens = self.tokens.lock().unwrap();
        if let Some(entry) = tokens.get(&token_hash) {
            if entry.revoked {
                return Ok(serde_json::json!({ "valid": false, "reason": "revoked" }));
            }
            if Utc::now() > entry.expires_at {
                return Ok(serde_json::json!({ "valid": false, "reason": "expired" }));
            }
            Ok(serde_json::json!({ "valid": true, "user_id": entry.user_id }))
        } else {
            Ok(serde_json::json!({ "valid": false, "reason": "token not found" }))
        }
    }

    fn exec_token_refresh(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let token = get_str(input, "token")?;
        let ttl_hours = input
            .get("ttl_hours")
            .and_then(|v| v.as_i64())
            .unwrap_or(24);
        let token_hash = Self::sha256_hex(token);

        let mut tokens = self.tokens.lock().unwrap();
        if let Some(entry) = tokens.get_mut(&token_hash) {
            if entry.revoked {
                return Ok(serde_json::json!({ "refreshed": false, "reason": "revoked" }));
            }
            if Utc::now() > entry.expires_at {
                return Ok(serde_json::json!({ "refreshed": false, "reason": "expired" }));
            }
            let new_expires = Utc::now() + Duration::hours(ttl_hours);
            entry.expires_at = new_expires;
            Ok(serde_json::json!({
                "refreshed": true,
                "user_id": entry.user_id,
                "expires_at": new_expires.to_rfc3339()
            }))
        } else {
            Ok(serde_json::json!({ "refreshed": false, "reason": "token not found" }))
        }
    }
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
        idempotence_class: IdempotenceClass::NonIdempotent,
        risk_class: RiskClass::Low,
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
        port_id: "auth".to_string(),
        name: "Auth".to_string(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Custom,
        description: "Authentication: OTP, sessions, TOTP, bearer tokens".to_string(),
        namespace: "soma.ports.auth".to_string(),
        trust_level: TrustLevel::Trusted,
        capabilities: vec![
            cap("otp_generate", "Generate OTP", "Generate a 6-digit OTP for phone verification", SideEffectClass::LocalStateMutation, DeterminismClass::Stochastic, 5),
            cap("otp_verify", "Verify OTP", "Verify an OTP code against stored hash", SideEffectClass::LocalStateMutation, DeterminismClass::Deterministic, 5),
            cap("session_create", "Create session", "Create an authenticated session with expiry", SideEffectClass::LocalStateMutation, DeterminismClass::Stochastic, 5),
            cap("session_validate", "Validate session", "Check if a session token is valid and not expired", SideEffectClass::ReadOnly, DeterminismClass::Deterministic, 1),
            cap("session_revoke", "Revoke session", "Mark a session as revoked", SideEffectClass::LocalStateMutation, DeterminismClass::Deterministic, 1),
            cap("totp_generate", "Generate TOTP secret", "Generate a TOTP secret and provisioning URI for 2FA setup", SideEffectClass::None, DeterminismClass::Stochastic, 5),
            cap("totp_verify", "Verify TOTP", "Verify a TOTP code against a secret", SideEffectClass::None, DeterminismClass::Deterministic, 1),
            cap("token_generate", "Generate token", "Generate a random bearer token with expiry", SideEffectClass::LocalStateMutation, DeterminismClass::Stochastic, 5),
            cap("token_validate", "Validate token", "Check if a bearer token is valid and not expired", SideEffectClass::ReadOnly, DeterminismClass::Deterministic, 1),
            cap("token_refresh", "Refresh token", "Extend the expiry of a valid bearer token", SideEffectClass::LocalStateMutation, DeterminismClass::Deterministic, 1),
        ],
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::any(),
        failure_modes: vec![PortFailureClass::ValidationError, PortFailureClass::ExternalError],
        side_effect_class: SideEffectClass::LocalStateMutation,
        latency_profile: LatencyProfile {
            expected_latency_ms: 5,
            p95_latency_ms: 50,
            max_latency_ms: 500,
        },
        cost_profile: CostProfile::default(),
        auth_requirements: AuthRequirements::default(),
        sandbox_requirements: SandboxRequirements::default(),
        observable_fields: vec!["capability_id".into(), "latency_ms".into(), "user_id".into()],
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
    Box::into_raw(Box::new(AuthPort::new()))
}
