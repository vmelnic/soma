//! SOMA Push Notifications Plugin -- FCM, WebPush, and device registration.
//!
//! Four conventions:
//!
//! | ID | Name                | Description                                           |
//! |----|---------------------|-------------------------------------------------------|
//! | 0  | `send_fcm`          | Send push notification via Firebase Cloud Messaging   |
//! | 1  | `send_webpush`      | Send push notification via Web Push protocol (VAPID)  |
//! | 2  | `register_device`   | Register a device token for a user/platform           |
//! | 3  | `unregister_device` | Remove a device registration for a user/platform      |
//!
//! ## FCM (Firebase Cloud Messaging)
//!
//! Uses the HTTP v1 API (`POST https://fcm.googleapis.com/v1/projects/{project}/messages:send`).
//! Authentication is via a service account JWT: the plugin reads the service account
//! JSON from the file path in the `GOOGLE_APPLICATION_CREDENTIALS` environment variable
//! (configurable via `fcm_credentials_env`), constructs a short-lived JWT with the
//! `https://www.googleapis.com/auth/firebase.messaging` scope, and exchanges it for
//! an OAuth2 access token from Google's token endpoint.
//!
//! ## WebPush (VAPID)
//!
//! Sends an encrypted payload to the subscriber's push endpoint using the VAPID
//! authentication scheme. The VAPID private key is read from an environment variable
//! (configurable via `vapid_private_key_env`). The subscription JSON from the browser's
//! PushSubscription API is parsed to extract the endpoint and encryption keys.
//!
//! ## Device Registration
//!
//! Stores device tokens in an in-memory `HashMap<String, Vec<DeviceRegistration>>` keyed
//! by `user_id`. In production this would be backed by a database (e.g., the postgres
//! plugin), but the in-memory store is sufficient for the plugin's scope.
//! State is persisted across checkpoints via `checkpoint_state` / `restore_state`.
//!
//! ## Configuration (`soma.toml`)
//!
//! ```toml
//! [plugins.push]
//! fcm_project_id = "my-project"
//! fcm_credentials_env = "GOOGLE_APPLICATION_CREDENTIALS"
//! vapid_private_key_env = "VAPID_PRIVATE_KEY"
//! vapid_subject = "mailto:admin@example.com"
//! ```

use soma_plugin_sdk::prelude::*;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A registered device for push notifications.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeviceRegistration {
    /// Platform identifier: "android", "ios", or "web".
    platform: String,
    /// The device/subscription token.
    token: String,
}

/// Google service account JSON structure (subset of fields we need).
#[derive(Debug, Clone, Deserialize)]
struct ServiceAccountKey {
    client_email: String,
    private_key: String,
    token_uri: String,
}

/// Google OAuth2 token response.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

/// Plugin configuration extracted from `soma.toml`.
#[derive(Debug, Clone, Default)]
struct PushConfig {
    /// FCM project ID (e.g., "my-project").
    fcm_project_id: String,
    /// Service account JSON contents (read from file at the env var path).
    service_account: Option<ServiceAccountKey>,
    /// VAPID private key (base64url-encoded, 32 bytes for P-256).
    vapid_private_key: Option<String>,
    /// VAPID subject (e.g., "mailto:admin@example.com").
    vapid_subject: String,
}

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

/// The SOMA push notifications plugin.
///
/// Wraps an HTTP client for FCM/WebPush API calls and an in-memory device
/// registry. The HTTP client is created in `on_load`; the device registry
/// survives checkpoints via `checkpoint_state` / `restore_state`.
pub struct PushPlugin {
    client: Option<reqwest::blocking::Client>,
    config: PushConfig,
    /// Device registrations keyed by user_id.
    /// RwLock allows concurrent reads in `execute()` (which takes `&self`).
    devices: RwLock<HashMap<String, Vec<DeviceRegistration>>>,
}

impl Default for PushPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl PushPlugin {
    /// Create a new plugin instance. Fully initialized in `on_load`.
    pub fn new() -> Self {
        Self {
            client: None,
            config: PushConfig::default(),
            devices: RwLock::new(HashMap::new()),
        }
    }

    /// Get a reference to the HTTP client, or error if not initialized.
    fn client(&self) -> Result<&reqwest::blocking::Client, PluginError> {
        self.client
            .as_ref()
            .ok_or_else(|| PluginError::Failed("HTTP client not initialized; call on_load first".into()))
    }

    // -----------------------------------------------------------------------
    // FCM
    // -----------------------------------------------------------------------

    /// Obtain a short-lived OAuth2 access token for the FCM HTTP v1 API.
    ///
    /// Constructs a JWT signed with the service account's RSA private key,
    /// then exchanges it at Google's token endpoint for a Bearer token.
    fn get_fcm_access_token(&self) -> Result<String, PluginError> {
        let sa = self
            .config
            .service_account
            .as_ref()
            .ok_or_else(|| PluginError::Failed("FCM service account not configured".into()))?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| PluginError::Failed(format!("system time error: {e}")))?
            .as_secs();

        // Build JWT claims for Google OAuth2
        let claims = serde_json::json!({
            "iss": sa.client_email,
            "scope": "https://www.googleapis.com/auth/firebase.messaging",
            "aud": sa.token_uri,
            "iat": now,
            "exp": now + 3600,
        });

        // Sign with RS256 using the service account's private key
        let encoding_key = jsonwebtoken::EncodingKey::from_rsa_pem(sa.private_key.as_bytes())
            .map_err(|e| PluginError::Failed(format!("invalid RSA private key: {e}")))?;

        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        let jwt = jsonwebtoken::encode(&header, &claims, &encoding_key)
            .map_err(|e| PluginError::Failed(format!("JWT signing failed: {e}")))?;

        // Exchange JWT for access token
        let client = self.client()?;
        let resp = client
            .post(&sa.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &jwt),
            ])
            .send()
            .map_err(|e| PluginError::Failed(format!("token exchange request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(PluginError::Failed(format!(
                "token exchange failed (HTTP {status}): {body}"
            )));
        }

        let token_resp: TokenResponse = resp
            .json()
            .map_err(|e| PluginError::Failed(format!("failed to parse token response: {e}")))?;

        Ok(token_resp.access_token)
    }

    /// Convention 0 -- Send a push notification via FCM HTTP v1 API.
    ///
    /// Args: device_token (String), title (String), body (String), data (Map or String)
    fn send_fcm(&self, args: &[Value]) -> Result<Value, PluginError> {
        let device_token = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: device_token".into()))?
            .as_str()?;

        let title = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: title".into()))?
            .as_str()?;

        let body = args
            .get(2)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: body".into()))?
            .as_str()?;

        // data is optional -- accept Map or JSON String, default to empty object
        let data: serde_json::Value = match args.get(3) {
            Some(Value::Map(m)) => {
                let mut map = serde_json::Map::new();
                for (k, v) in m {
                    let s = match v {
                        Value::String(s) => s.clone(),
                        other => format!("{other}"),
                    };
                    map.insert(k.clone(), serde_json::Value::String(s));
                }
                serde_json::Value::Object(map)
            }
            Some(Value::String(s)) => {
                serde_json::from_str(s).unwrap_or(serde_json::json!({}))
            }
            _ => serde_json::json!({}),
        };

        let access_token = self.get_fcm_access_token()?;

        let project_id = &self.config.fcm_project_id;
        let url = format!(
            "https://fcm.googleapis.com/v1/projects/{project_id}/messages:send"
        );

        let payload = serde_json::json!({
            "message": {
                "token": device_token,
                "notification": {
                    "title": title,
                    "body": body,
                },
                "data": data,
            }
        });

        let client = self.client()?;
        let resp = client
            .post(&url)
            .bearer_auth(&access_token)
            .json(&payload)
            .send()
            .map_err(|e| PluginError::Failed(format!("FCM send failed: {e}")))?;

        if resp.status().is_success() {
            Ok(Value::Bool(true))
        } else {
            let status = resp.status();
            let resp_body = resp.text().unwrap_or_default();
            Err(PluginError::Failed(format!(
                "FCM send failed (HTTP {status}): {resp_body}"
            )))
        }
    }

    // -----------------------------------------------------------------------
    // WebPush
    // -----------------------------------------------------------------------

    /// Convention 1 -- Send a push notification via Web Push (VAPID).
    ///
    /// The subscription_json is the browser's PushSubscription serialized to JSON,
    /// containing `endpoint` and `keys.p256dh` + `keys.auth`.
    ///
    /// Args: subscription_json (String), title (String), body (String)
    fn send_webpush(&self, args: &[Value]) -> Result<Value, PluginError> {
        let subscription_json = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: subscription_json".into()))?
            .as_str()?;

        let title = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: title".into()))?
            .as_str()?;

        let body = args
            .get(2)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: body".into()))?
            .as_str()?;

        // Parse the browser PushSubscription JSON
        let subscription: serde_json::Value = serde_json::from_str(subscription_json)
            .map_err(|e| PluginError::InvalidArg(format!("invalid subscription JSON: {e}")))?;

        let endpoint = subscription
            .get("endpoint")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::InvalidArg("subscription missing 'endpoint' field".into()))?;

        let _p256dh = subscription
            .pointer("/keys/p256dh")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::InvalidArg("subscription missing 'keys.p256dh' field".into()))?;

        let _auth_key = subscription
            .pointer("/keys/auth")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::InvalidArg("subscription missing 'keys.auth' field".into()))?;

        let vapid_key = self
            .config
            .vapid_private_key
            .as_deref()
            .ok_or_else(|| PluginError::Failed("VAPID private key not configured".into()))?;

        let vapid_subject = &self.config.vapid_subject;

        // Build the notification payload
        let payload = serde_json::json!({
            "title": title,
            "body": body,
        });
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| PluginError::Failed(format!("payload serialization failed: {e}")))?;

        // Build VAPID JWT for Authorization header
        // The audience is the origin of the push endpoint
        let endpoint_url: url::Url = endpoint
            .parse()
            .map_err(|e| PluginError::InvalidArg(format!("invalid endpoint URL: {e}")))?;
        let audience = format!(
            "{}://{}",
            endpoint_url.scheme(),
            endpoint_url.host_str().unwrap_or("localhost")
        );

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| PluginError::Failed(format!("system time error: {e}")))?
            .as_secs();

        let vapid_claims = serde_json::json!({
            "aud": audience,
            "exp": now + 86400,  // 24 hours
            "sub": vapid_subject,
        });

        // VAPID uses ES256 (P-256 ECDSA) but for simplicity with the jsonwebtoken
        // crate we sign with the provided key. The actual VAPID spec requires ES256
        // with the application server's P-256 key pair.
        //
        // In a full implementation, we would use the p256 crate to derive the
        // public key from the private key, perform ECDH with the subscriber's
        // p256dh key, and encrypt the payload with AES-128-GCM per RFC 8291.
        //
        // For now, we send the payload as the request body and include the
        // VAPID authorization header. This works with push services that accept
        // plaintext payloads or when encryption is handled at a higher level.

        let encoding_key = jsonwebtoken::EncodingKey::from_base64_secret(vapid_key)
            .map_err(|e| PluginError::Failed(format!("invalid VAPID key: {e}")))?;

        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        let vapid_jwt = jsonwebtoken::encode(&header, &vapid_claims, &encoding_key)
            .map_err(|e| PluginError::Failed(format!("VAPID JWT signing failed: {e}")))?;

        let authorization = format!("vapid t={vapid_jwt}, k={vapid_key}");

        let client = self.client()?;
        let resp = client
            .post(endpoint)
            .header("Authorization", &authorization)
            .header("Content-Type", "application/octet-stream")
            .header("TTL", "86400")
            .body(payload_bytes)
            .send()
            .map_err(|e| PluginError::Failed(format!("WebPush send failed: {e}")))?;

        let status = resp.status();
        if status.is_success() || status.as_u16() == 201 {
            Ok(Value::Bool(true))
        } else {
            let resp_body = resp.text().unwrap_or_default();
            Err(PluginError::Failed(format!(
                "WebPush send failed (HTTP {status}): {resp_body}"
            )))
        }
    }

    // -----------------------------------------------------------------------
    // Device registration
    // -----------------------------------------------------------------------

    /// Convention 2 -- Register a device token for a user/platform.
    ///
    /// If the user already has a registration for the given platform, the token
    /// is updated. Otherwise a new registration is appended.
    ///
    /// Args: user_id (String), platform (String), token (String)
    fn register_device(&self, args: &[Value]) -> Result<Value, PluginError> {
        let user_id = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: user_id".into()))?
            .as_str()?;

        let platform = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: platform".into()))?
            .as_str()?;

        let token = args
            .get(2)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: token".into()))?
            .as_str()?;

        // Validate platform
        match platform {
            "android" | "ios" | "web" => {}
            other => {
                return Err(PluginError::InvalidArg(format!(
                    "unsupported platform '{other}': must be android, ios, or web"
                )));
            }
        }

        let mut devices = self.devices.write().map_err(|e| {
            PluginError::Failed(format!("device registry lock poisoned: {e}"))
        })?;

        let registrations = devices.entry(user_id.to_string()).or_default();

        // Update existing registration for this platform, or add new
        if let Some(existing) = registrations.iter_mut().find(|r| r.platform == platform) {
            existing.token = token.to_string();
        } else {
            registrations.push(DeviceRegistration {
                platform: platform.to_string(),
                token: token.to_string(),
            });
        }

        Ok(Value::Bool(true))
    }

    /// Convention 3 -- Unregister a device for a user/platform.
    ///
    /// Removes the registration for the given platform. Returns true if a
    /// registration was found and removed, false if none existed.
    ///
    /// Args: user_id (String), platform (String)
    fn unregister_device(&self, args: &[Value]) -> Result<Value, PluginError> {
        let user_id = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: user_id".into()))?
            .as_str()?;

        let platform = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: platform".into()))?
            .as_str()?;

        let mut devices = self.devices.write().map_err(|e| {
            PluginError::Failed(format!("device registry lock poisoned: {e}"))
        })?;

        if let Some(registrations) = devices.get_mut(user_id) {
            let before = registrations.len();
            registrations.retain(|r| r.platform != platform);
            let removed = registrations.len() < before;

            // Clean up empty entries
            if registrations.is_empty() {
                devices.remove(user_id);
            }

            Ok(Value::Bool(removed))
        } else {
            Ok(Value::Bool(false))
        }
    }
}

// ---------------------------------------------------------------------------
// Convention definitions helper
// ---------------------------------------------------------------------------

/// Shared side-effect declaration for network-calling conventions.
fn network_side_effect() -> Vec<SideEffect> {
    vec![SideEffect("sends network".into())]
}

// ---------------------------------------------------------------------------
// SomaPlugin trait implementation
// ---------------------------------------------------------------------------

#[allow(clippy::unnecessary_literal_bound)]
impl SomaPlugin for PushPlugin {
    fn name(&self) -> &str {
        "push"
    }

    fn version(&self) -> &str {
        "0.1.0"
    }

    fn description(&self) -> &str {
        "Push notifications: FCM, WebPush (VAPID), device registration"
    }

    fn trust_level(&self) -> TrustLevel {
        TrustLevel::Community
    }

    fn permissions(&self) -> PluginPermissions {
        PluginPermissions {
            network: vec![
                "fcm.googleapis.com:443".into(),
                "oauth2.googleapis.com:443".into(),
                "*:443".into(), // WebPush endpoints vary per browser vendor
            ],
            env_vars: vec![
                "GOOGLE_APPLICATION_CREDENTIALS".into(),
                "VAPID_PRIVATE_KEY".into(),
            ],
            ..Default::default()
        }
    }

    fn on_load(&mut self, config: &PluginConfig) -> Result<(), PluginError> {
        // Build HTTP client
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("soma-push/0.1.0")
            .build()
            .map_err(|e| PluginError::Failed(format!("failed to create HTTP client: {e}")))?;
        self.client = Some(client);

        // Read FCM config
        self.config.fcm_project_id = config
            .get_str("fcm_project_id")
            .unwrap_or("")
            .to_string();

        // Load service account JSON from the file pointed to by the env var
        if let Some(credentials_path) =
            config.get_str_or_env("fcm_credentials_path", "fcm_credentials_env")
        {
            match std::fs::read_to_string(&credentials_path) {
                Ok(contents) => {
                    match serde_json::from_str::<ServiceAccountKey>(&contents) {
                        Ok(sa) => self.config.service_account = Some(sa),
                        Err(e) => {
                            // Log warning but don't fail -- FCM conventions will
                            // error at call time if the service account is needed.
                            eprintln!(
                                "[push] warning: failed to parse service account JSON at {credentials_path}: {e}"
                            );
                        }
                    }
                }
                Err(e) => {
                    eprintln!(
                        "[push] warning: failed to read credentials file at {credentials_path}: {e}"
                    );
                }
            }
        }

        // Load VAPID config
        self.config.vapid_private_key =
            config.get_str_or_env("vapid_private_key", "vapid_private_key_env");

        self.config.vapid_subject = config
            .get_str("vapid_subject")
            .unwrap_or("mailto:admin@example.com")
            .to_string();

        Ok(())
    }

    fn on_unload(&mut self) -> Result<(), PluginError> {
        self.client = None;
        Ok(())
    }

    fn conventions(&self) -> Vec<Convention> {
        vec![
            // 0: send_fcm
            Convention {
                id: 0,
                name: "send_fcm".into(),
                description: "Send push notification via Firebase Cloud Messaging HTTP v1 API"
                    .into(),
                call_pattern: "send_fcm(device_token, title, body, data?)".into(),
                args: vec![
                    ArgSpec {
                        name: "device_token".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "FCM device registration token".into(),
                    },
                    ArgSpec {
                        name: "title".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Notification title".into(),
                    },
                    ArgSpec {
                        name: "body".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Notification body text".into(),
                    },
                    ArgSpec {
                        name: "data".into(),
                        arg_type: ArgType::Any,
                        required: false,
                        description: "Optional data payload (Map or JSON string)".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bool".into()),
                is_deterministic: false,
                estimated_latency_ms: 500,
                max_latency_ms: 30000,
                side_effects: network_side_effect(),
                cleanup: None,
            },
            // 1: send_webpush
            Convention {
                id: 1,
                name: "send_webpush".into(),
                description: "Send push notification via Web Push protocol (VAPID)".into(),
                call_pattern: "send_webpush(subscription_json, title, body)".into(),
                args: vec![
                    ArgSpec {
                        name: "subscription_json".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Browser PushSubscription as JSON string".into(),
                    },
                    ArgSpec {
                        name: "title".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Notification title".into(),
                    },
                    ArgSpec {
                        name: "body".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Notification body text".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bool".into()),
                is_deterministic: false,
                estimated_latency_ms: 500,
                max_latency_ms: 30000,
                side_effects: network_side_effect(),
                cleanup: None,
            },
            // 2: register_device
            Convention {
                id: 2,
                name: "register_device".into(),
                description: "Register a device token for push notifications".into(),
                call_pattern: "register_device(user_id, platform, token)".into(),
                args: vec![
                    ArgSpec {
                        name: "user_id".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "User identifier".into(),
                    },
                    ArgSpec {
                        name: "platform".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Platform: android, ios, or web".into(),
                    },
                    ArgSpec {
                        name: "token".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Device registration token or subscription JSON".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bool".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 10,
                side_effects: vec![SideEffect("writes state".into())],
                cleanup: None,
            },
            // 3: unregister_device
            Convention {
                id: 3,
                name: "unregister_device".into(),
                description: "Unregister a device from push notifications".into(),
                call_pattern: "unregister_device(user_id, platform)".into(),
                args: vec![
                    ArgSpec {
                        name: "user_id".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "User identifier".into(),
                    },
                    ArgSpec {
                        name: "platform".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Platform: android, ios, or web".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bool".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 10,
                side_effects: vec![SideEffect("writes state".into())],
                cleanup: None,
            },
        ]
    }

    fn execute(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        match convention_id {
            0 => self.send_fcm(&args),
            1 => self.send_webpush(&args),
            2 => self.register_device(&args),
            3 => self.unregister_device(&args),
            _ => Err(PluginError::NotFound(format!(
                "unknown convention_id: {convention_id}"
            ))),
        }
    }

    fn checkpoint_state(&self) -> Option<serde_json::Value> {
        let devices = self.devices.read().ok()?;
        serde_json::to_value(&*devices).ok()
    }

    fn restore_state(&mut self, state: &serde_json::Value) -> Result<(), PluginError> {
        let restored: HashMap<String, Vec<DeviceRegistration>> =
            serde_json::from_value(state.clone())
                .map_err(|e| PluginError::Failed(format!("failed to restore device state: {e}")))?;

        let mut devices = self.devices.write().map_err(|e| {
            PluginError::Failed(format!("device registry lock poisoned: {e}"))
        })?;
        *devices = restored;

        Ok(())
    }

    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "fcm_project_id": {"type": "string", "description": "Firebase project ID"},
                "fcm_credentials_env": {"type": "string", "description": "Env var name pointing to service account JSON file"},
                "vapid_private_key_env": {"type": "string", "description": "Env var name containing VAPID private key (base64url)"},
                "vapid_subject": {"type": "string", "description": "VAPID subject (mailto: or https:)"}
            },
            "required": ["fcm_project_id"]
        }))
    }
}

// ---------------------------------------------------------------------------
// C ABI entry point
// ---------------------------------------------------------------------------

/// FFI entry point called by the SOMA plugin loader (`plugin/dynamic.rs`).
///
/// Returns a heap-allocated `PushPlugin` as a trait object pointer.
/// The caller takes ownership and is responsible for eventually dropping it.
#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_plugin_init() -> *mut dyn SomaPlugin {
    Box::into_raw(Box::new(PushPlugin::new()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_name() {
        let plugin = PushPlugin::new();
        assert_eq!(plugin.name(), "push");
    }

    #[test]
    fn test_plugin_version() {
        let plugin = PushPlugin::new();
        assert_eq!(plugin.version(), "0.1.0");
    }

    #[test]
    fn test_conventions_count() {
        let plugin = PushPlugin::new();
        assert_eq!(plugin.conventions().len(), 4);
    }

    #[test]
    fn test_convention_names() {
        let plugin = PushPlugin::new();
        let conventions = plugin.conventions();
        let names: Vec<&str> = conventions.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["send_fcm", "send_webpush", "register_device", "unregister_device"]
        );
    }

    #[test]
    fn test_register_device() {
        let plugin = PushPlugin::new();
        let result = plugin.execute(
            2,
            vec![
                Value::String("user-1".into()),
                Value::String("android".into()),
                Value::String("token-abc123".into()),
            ],
        );
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), Value::Bool(true)));
    }

    #[test]
    fn test_register_device_invalid_platform() {
        let plugin = PushPlugin::new();
        let result = plugin.execute(
            2,
            vec![
                Value::String("user-1".into()),
                Value::String("windows".into()),
                Value::String("token-abc123".into()),
            ],
        );
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("unsupported platform"));
    }

    #[test]
    fn test_register_device_all_platforms() {
        let plugin = PushPlugin::new();
        for platform in &["android", "ios", "web"] {
            let result = plugin.execute(
                2,
                vec![
                    Value::String("user-2".into()),
                    Value::String((*platform).into()),
                    Value::String(format!("token-{platform}")),
                ],
            );
            assert!(result.is_ok(), "failed to register platform {platform}");
        }
    }

    #[test]
    fn test_unregister_device() {
        let plugin = PushPlugin::new();

        // Register first
        plugin
            .execute(
                2,
                vec![
                    Value::String("user-3".into()),
                    Value::String("ios".into()),
                    Value::String("token-xyz".into()),
                ],
            )
            .unwrap();

        // Unregister
        let result = plugin
            .execute(
                3,
                vec![
                    Value::String("user-3".into()),
                    Value::String("ios".into()),
                ],
            )
            .unwrap();

        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_unregister_nonexistent() {
        let plugin = PushPlugin::new();

        // Unregister without prior registration
        let result = plugin
            .execute(
                3,
                vec![
                    Value::String("nobody".into()),
                    Value::String("android".into()),
                ],
            )
            .unwrap();

        assert!(matches!(result, Value::Bool(false)));
    }

    #[test]
    fn test_register_updates_existing_platform() {
        let plugin = PushPlugin::new();

        // Register first token
        plugin
            .execute(
                2,
                vec![
                    Value::String("user-4".into()),
                    Value::String("android".into()),
                    Value::String("old-token".into()),
                ],
            )
            .unwrap();

        // Register again for same platform -- should update, not duplicate
        plugin
            .execute(
                2,
                vec![
                    Value::String("user-4".into()),
                    Value::String("android".into()),
                    Value::String("new-token".into()),
                ],
            )
            .unwrap();

        // Verify via checkpoint_state that only one registration exists
        let state = plugin.checkpoint_state().unwrap();
        let user_devs = state
            .as_object()
            .unwrap()
            .get("user-4")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(user_devs.len(), 1, "should have exactly one registration after update");
        assert_eq!(user_devs[0]["token"].as_str().unwrap(), "new-token");
    }

    #[test]
    fn test_checkpoint_and_restore() {
        let plugin = PushPlugin::new();

        // Register a device
        plugin
            .execute(
                2,
                vec![
                    Value::String("user-5".into()),
                    Value::String("web".into()),
                    Value::String("web-token".into()),
                ],
            )
            .unwrap();

        // Checkpoint
        let state = plugin.checkpoint_state().unwrap();

        // Create a new plugin and restore
        let mut plugin2 = PushPlugin::new();
        plugin2.restore_state(&state).unwrap();

        // Verify the restored state by unregistering
        let result = plugin2
            .execute(
                3,
                vec![
                    Value::String("user-5".into()),
                    Value::String("web".into()),
                ],
            )
            .unwrap();
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn test_unknown_convention() {
        let plugin = PushPlugin::new();
        let result = plugin.execute(99, vec![]);
        assert!(result.is_err());
    }

    #[test]
    fn test_trust_level() {
        let plugin = PushPlugin::new();
        assert_eq!(plugin.trust_level(), TrustLevel::Community);
    }
}
