//! SOMA Auth Plugin — OTP verification, session management, token hashing.

use soma_plugin_sdk::prelude::*;
use sha2::{Sha256, Digest};
use std::collections::HashMap;

struct DbState {
    client: tokio_postgres::Client,
    _handle: tokio::task::JoinHandle<()>,
}

// Safety: tokio_postgres::Client is Send+Sync
unsafe impl Send for DbState {}
unsafe impl Sync for DbState {}

pub struct AuthPlugin {
    db: tokio::sync::Mutex<Option<DbState>>,
    otp_ttl_minutes: i64,
    session_ttl_hours: i64,
}

impl AuthPlugin {
    pub fn new() -> Self {
        Self { db: tokio::sync::Mutex::new(None), otp_ttl_minutes: 5, session_ttl_hours: 720 }
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

    async fn init_db(conn_str: &str) -> Result<DbState, PluginError> {
        let (client, conn) = tokio_postgres::connect(conn_str, tokio_postgres::NoTls).await
            .map_err(|e| PluginError::ConnectionRefused(format!("Auth DB: {}", e)))?;
        let handle = tokio::spawn(async move { if let Err(e) = conn.await { eprintln!("[auth] db error: {}", e); } });
        client.batch_execute(
            "CREATE TABLE IF NOT EXISTS _soma_otps (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                phone VARCHAR(20) NOT NULL, code_hash TEXT NOT NULL,
                expires_at TIMESTAMP NOT NULL, attempts INT DEFAULT 0,
                verified BOOLEAN DEFAULT FALSE, created_at TIMESTAMP DEFAULT NOW()
            );
            CREATE TABLE IF NOT EXISTS _soma_sessions (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                token_hash TEXT NOT NULL, user_id TEXT NOT NULL, device_info TEXT,
                created_at TIMESTAMP DEFAULT NOW(), expires_at TIMESTAMP NOT NULL,
                revoked BOOLEAN DEFAULT FALSE
            );"
        ).await.map_err(|e| PluginError::Failed(format!("create tables: {}", e)))?;
        Ok(DbState { client, _handle: handle })
    }

    async fn exec_db(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        let guard = self.db.lock().await;
        let client = &guard.as_ref()
            .ok_or(PluginError::Failed("Auth DB not connected. Check [plugins.auth] config.".into()))?
            .client;

        match convention_id {
            0 => { // generate_otp
                let phone = args.first().ok_or(PluginError::InvalidArg("missing phone".into()))?.as_str()?;
                let code = Self::gen_otp();
                let hash = Self::sha256_hex(&code);
                let ttl = self.otp_ttl_minutes;
                let row = client.query_one(
                    &format!("INSERT INTO _soma_otps (phone, code_hash, expires_at) VALUES ($1, $2, NOW() + INTERVAL '{} minutes') RETURNING id", ttl),
                    &[&phone, &hash],
                ).await.map_err(|e| PluginError::Failed(format!("generate_otp: {}", e)))?;
                let id: uuid::Uuid = row.get(0);
                let mut m = HashMap::new();
                m.insert("otp_id".into(), Value::String(id.to_string()));
                m.insert("debug_code".into(), Value::String(code));
                m.insert("phone".into(), Value::String(phone.to_string()));
                Ok(Value::Map(m))
            }
            1 => { // verify_otp
                let phone = args.first().ok_or(PluginError::InvalidArg("missing phone".into()))?.as_str()?;
                let code = args.get(1).ok_or(PluginError::InvalidArg("missing code".into()))?.as_str()?;
                let hash = Self::sha256_hex(code);
                let result = client.query_opt(
                    "UPDATE _soma_otps SET verified = TRUE WHERE phone = $1 AND code_hash = $2 AND verified = FALSE AND expires_at > NOW() AND attempts < 5 RETURNING id",
                    &[&phone, &hash],
                ).await.map_err(|e| PluginError::Failed(format!("verify_otp: {}", e)))?;
                let _ = client.execute(
                    "UPDATE _soma_otps SET attempts = attempts + 1 WHERE phone = $1 AND verified = FALSE AND expires_at > NOW()",
                    &[&phone],
                ).await;
                let mut m = HashMap::new();
                if result.is_some() {
                    m.insert("valid".into(), Value::Bool(true));
                    m.insert("user_id".into(), Value::String(format!("phone:{}", phone)));
                } else {
                    m.insert("valid".into(), Value::Bool(false));
                    m.insert("user_id".into(), Value::Null);
                }
                Ok(Value::Map(m))
            }
            2 => { // create_session
                let user_id = args.first().ok_or(PluginError::InvalidArg("missing user_id".into()))?.as_str()?;
                let device = args.get(1).and_then(|v| v.as_str().ok()).unwrap_or("unknown");
                let token = uuid::Uuid::new_v4().to_string();
                let token_hash = Self::sha256_hex(&token);
                let ttl = self.session_ttl_hours;
                client.execute(
                    &format!("INSERT INTO _soma_sessions (token_hash, user_id, device_info, expires_at) VALUES ($1, $2, $3, NOW() + INTERVAL '{} hours')", ttl),
                    &[&token_hash, &user_id, &device],
                ).await.map_err(|e| PluginError::Failed(format!("create_session: {}", e)))?;
                let mut m = HashMap::new();
                m.insert("token".into(), Value::String(token));
                m.insert("user_id".into(), Value::String(user_id.to_string()));
                Ok(Value::Map(m))
            }
            3 => { // validate_session
                let token = args.first().ok_or(PluginError::InvalidArg("missing token".into()))?.as_str()?;
                let hash = Self::sha256_hex(token);
                let row = client.query_opt(
                    "SELECT user_id FROM _soma_sessions WHERE token_hash = $1 AND revoked = FALSE AND expires_at > NOW()",
                    &[&hash],
                ).await.map_err(|e| PluginError::Failed(format!("validate_session: {}", e)))?;
                let mut m = HashMap::new();
                if let Some(r) = row {
                    let uid: String = r.get(0);
                    m.insert("valid".into(), Value::Bool(true));
                    m.insert("user_id".into(), Value::String(uid));
                } else {
                    m.insert("valid".into(), Value::Bool(false));
                    m.insert("user_id".into(), Value::Null);
                }
                Ok(Value::Map(m))
            }
            4 => { // revoke_session
                let token = args.first().ok_or(PluginError::InvalidArg("missing token".into()))?.as_str()?;
                let hash = Self::sha256_hex(token);
                client.execute("UPDATE _soma_sessions SET revoked = TRUE WHERE token_hash = $1", &[&hash])
                    .await.map_err(|e| PluginError::Failed(format!("revoke_session: {}", e)))?;
                Ok(Value::Null)
            }
            5 => { // revoke_all_sessions
                let user_id = args.first().ok_or(PluginError::InvalidArg("missing user_id".into()))?.as_str()?;
                let count = client.execute(
                    "UPDATE _soma_sessions SET revoked = TRUE WHERE user_id = $1 AND revoked = FALSE",
                    &[&user_id],
                ).await.map_err(|e| PluginError::Failed(format!("revoke_all: {}", e)))?;
                Ok(Value::Int(count as i64))
            }
            6 => { // list_sessions
                let user_id = args.first().ok_or(PluginError::InvalidArg("missing user_id".into()))?.as_str()?;
                let rows = client.query(
                    "SELECT id, device_info, created_at::TEXT FROM _soma_sessions WHERE user_id = $1 AND revoked = FALSE AND expires_at > NOW()",
                    &[&user_id],
                ).await.map_err(|e| PluginError::Failed(format!("list_sessions: {}", e)))?;
                let sessions: Vec<Value> = rows.iter().map(|r| {
                    let id: uuid::Uuid = r.get(0);
                    let dev: Option<String> = r.get(1);
                    let created: Option<String> = r.get(2);
                    let mut m = HashMap::new();
                    m.insert("id".into(), Value::String(id.to_string()));
                    m.insert("device_info".into(), dev.map(Value::String).unwrap_or(Value::Null));
                    m.insert("created_at".into(), created.map(Value::String).unwrap_or(Value::Null));
                    Value::Map(m)
                }).collect();
                Ok(Value::List(sessions))
            }
            _ => Err(PluginError::NotFound(format!("Unknown auth convention: {}", convention_id))),
        }
    }
}

impl SomaPlugin for AuthPlugin {
    fn name(&self) -> &str { "auth" }
    fn version(&self) -> &str { "0.1.0" }
    fn description(&self) -> &str { "Authentication: OTP, sessions, token hashing" }
    fn trust_level(&self) -> TrustLevel { TrustLevel::BuiltIn }

    fn conventions(&self) -> Vec<Convention> {
        vec![
            Convention { id: 0, name: "generate_otp".into(), description: "Generate OTP for phone verification".into(), call_pattern: "direct".into(),
                args: vec![ArgSpec { name: "phone".into(), arg_type: ArgType::String, required: true, description: "Phone number".into() }],
                returns: ReturnSpec::Value("map".into()), is_deterministic: false, estimated_latency_ms: 50, max_latency_ms: 5000,
                side_effects: vec![SideEffect("database".into())], cleanup: None },
            Convention { id: 1, name: "verify_otp".into(), description: "Verify OTP code".into(), call_pattern: "direct".into(),
                args: vec![
                    ArgSpec { name: "phone".into(), arg_type: ArgType::String, required: true, description: "Phone number".into() },
                    ArgSpec { name: "code".into(), arg_type: ArgType::String, required: true, description: "OTP code".into() },
                ],
                returns: ReturnSpec::Value("map".into()), is_deterministic: false, estimated_latency_ms: 50, max_latency_ms: 5000,
                side_effects: vec![SideEffect("database".into())], cleanup: None },
            Convention { id: 2, name: "create_session".into(), description: "Create authenticated session".into(), call_pattern: "direct".into(),
                args: vec![
                    ArgSpec { name: "user_id".into(), arg_type: ArgType::String, required: true, description: "User ID".into() },
                    ArgSpec { name: "device_info".into(), arg_type: ArgType::String, required: false, description: "Device info".into() },
                ],
                returns: ReturnSpec::Value("map".into()), is_deterministic: false, estimated_latency_ms: 50, max_latency_ms: 5000,
                side_effects: vec![SideEffect("database".into())], cleanup: None },
            Convention { id: 3, name: "validate_session".into(), description: "Validate session token".into(), call_pattern: "direct".into(),
                args: vec![ArgSpec { name: "token".into(), arg_type: ArgType::String, required: true, description: "Session token".into() }],
                returns: ReturnSpec::Value("map".into()), is_deterministic: false, estimated_latency_ms: 20, max_latency_ms: 5000,
                side_effects: vec![], cleanup: None },
            Convention { id: 4, name: "revoke_session".into(), description: "Revoke session".into(), call_pattern: "direct".into(),
                args: vec![ArgSpec { name: "token".into(), arg_type: ArgType::String, required: true, description: "Session token".into() }],
                returns: ReturnSpec::Void, is_deterministic: false, estimated_latency_ms: 20, max_latency_ms: 5000,
                side_effects: vec![SideEffect("database".into())], cleanup: None },
            Convention { id: 5, name: "revoke_all_sessions".into(), description: "Revoke all user sessions".into(), call_pattern: "direct".into(),
                args: vec![ArgSpec { name: "user_id".into(), arg_type: ArgType::String, required: true, description: "User ID".into() }],
                returns: ReturnSpec::Value("int".into()), is_deterministic: false, estimated_latency_ms: 50, max_latency_ms: 5000,
                side_effects: vec![SideEffect("database".into())], cleanup: None },
            Convention { id: 6, name: "list_sessions".into(), description: "List active sessions".into(), call_pattern: "direct".into(),
                args: vec![ArgSpec { name: "user_id".into(), arg_type: ArgType::String, required: true, description: "User ID".into() }],
                returns: ReturnSpec::Value("list".into()), is_deterministic: false, estimated_latency_ms: 20, max_latency_ms: 5000,
                side_effects: vec![], cleanup: None },
            Convention { id: 7, name: "hash_token".into(), description: "SHA-256 hash for storage".into(), call_pattern: "direct".into(),
                args: vec![ArgSpec { name: "token".into(), arg_type: ArgType::String, required: true, description: "Token".into() }],
                returns: ReturnSpec::Value("string".into()), is_deterministic: true, estimated_latency_ms: 1, max_latency_ms: 100,
                side_effects: vec![], cleanup: None },
            Convention { id: 8, name: "generate_totp_secret".into(), description: "Generate TOTP 2FA secret".into(), call_pattern: "direct".into(),
                args: vec![ArgSpec { name: "user_id".into(), arg_type: ArgType::String, required: true, description: "User ID".into() }],
                returns: ReturnSpec::Value("map".into()), is_deterministic: false, estimated_latency_ms: 5, max_latency_ms: 100,
                side_effects: vec![], cleanup: None },
            Convention { id: 9, name: "verify_totp".into(), description: "Verify TOTP code (stub)".into(), call_pattern: "direct".into(),
                args: vec![
                    ArgSpec { name: "secret".into(), arg_type: ArgType::String, required: true, description: "TOTP secret".into() },
                    ArgSpec { name: "code".into(), arg_type: ArgType::String, required: true, description: "TOTP code".into() },
                ],
                returns: ReturnSpec::Value("bool".into()), is_deterministic: true, estimated_latency_ms: 1, max_latency_ms: 100,
                side_effects: vec![], cleanup: None },
        ]
    }

    fn execute(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        match convention_id {
            7 => { // hash_token — pure, no DB
                let token = args.first().ok_or(PluginError::InvalidArg("missing token".into()))?.as_str()?;
                Ok(Value::String(Self::sha256_hex(token)))
            }
            8 => { // generate_totp_secret — pure, base32 secret
                use rand::Rng;
                let uid = args.first().ok_or(PluginError::InvalidArg("missing user_id".into()))?.as_str()?;
                const BASE32: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
                let mut rng = rand::thread_rng();
                let secret: String = (0..32).map(|_| BASE32[rng.gen_range(0..32)] as char).collect();
                let mut m = HashMap::new();
                m.insert("secret".into(), Value::String(secret.clone()));
                m.insert("provisioning_uri".into(), Value::String(
                    format!("otpauth://totp/SOMA:{}?secret={}&issuer=SOMA&digits=6&period=30", uid, secret)
                ));
                Ok(Value::Map(m))
            }
            9 => { // verify_totp — MVP stub
                let _secret = args.first().ok_or(PluginError::InvalidArg("missing secret".into()))?.as_str()?;
                let _code = args.get(1).ok_or(PluginError::InvalidArg("missing code".into()))?.as_str()?;
                // Full TOTP verification requires the totp-rs crate
                Ok(Value::Bool(false))
            }
            _ => { // DB operations — bridge to async
                let handle = tokio::runtime::Handle::try_current()
                    .map_err(|_| PluginError::Failed("No tokio runtime".into()))?;
                tokio::task::block_in_place(|| handle.block_on(self.exec_db(convention_id, args)))
            }
        }
    }

    fn execute_async(&self, convention_id: u32, args: Vec<Value>)
        -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, PluginError>> + Send + '_>>
    {
        Box::pin(async move {
            match convention_id {
                7 | 8 | 9 => self.execute(convention_id, args),
                _ => self.exec_db(convention_id, args).await,
            }
        })
    }

    fn on_load(&mut self, config: &PluginConfig) -> Result<(), PluginError> {
        let host = config.get_str("db_host").unwrap_or("localhost");
        let port = config.get_int("db_port").unwrap_or(5432);
        let dbname = config.get_str("db_name").unwrap_or("soma");
        let user = config.get_str("db_user").unwrap_or("soma");
        let password = config.get_str("db_password_env")
            .and_then(|env_key| std::env::var(env_key).ok())
            .unwrap_or_default();
        self.otp_ttl_minutes = config.get_int("otp_ttl_minutes").unwrap_or(5);
        self.session_ttl_hours = config.get_int("session_ttl_hours").unwrap_or(720);

        let conn_str = format!("host={} port={} dbname={} user={} password={}", host, port, dbname, user, password);

        let db = match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                std::thread::scope(|s| {
                    s.spawn(|| handle.block_on(Self::init_db(&conn_str)))
                        .join().map_err(|_| PluginError::Failed("init thread panicked".into()))?
                })?
            }
            Err(_) => {
                // No tokio runtime yet — create a temporary one for initialization
                let rt = tokio::runtime::Runtime::new()
                    .map_err(|e| PluginError::Failed(format!("Failed to create runtime: {}", e)))?;
                rt.block_on(Self::init_db(&conn_str))?
            }
        };
        *self.db.try_lock().unwrap() = Some(db);
        Ok(())
    }

    fn on_unload(&mut self) -> Result<(), PluginError> {
        *self.db.try_lock().unwrap() = None;
        Ok(())
    }

    fn permissions(&self) -> PluginPermissions {
        PluginPermissions { network: vec!["tcp:*:5432".into()], env_vars: vec!["SOMA_AUTH_DB_PASSWORD".into()], ..Default::default() }
    }

    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "properties": {
                "db_host": {"type": "string", "default": "localhost"},
                "db_port": {"type": "integer", "default": 5432},
                "db_name": {"type": "string", "default": "soma"},
                "db_user": {"type": "string", "default": "soma"},
                "db_password_env": {"type": "string", "default": "SOMA_AUTH_DB_PASSWORD"},
                "otp_ttl_minutes": {"type": "integer", "default": 5},
                "session_ttl_hours": {"type": "integer", "default": 720}
            }
        }))
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn soma_plugin_init() -> *mut dyn SomaPlugin {
    Box::into_raw(Box::new(AuthPlugin::new()))
}
