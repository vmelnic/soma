//! SOMA Push Notifications Port -- FCM, WebPush, and device registration.
//!
//! Four capabilities:
//!
//! | ID | Name                | Description                                           |
//! |----|---------------------|-------------------------------------------------------|
//! | 0  | `send_fcm`          | Send push notification via Firebase Cloud Messaging   |
//! | 1  | `send_webpush`      | Send push notification via Web Push protocol (VAPID)  |
//! | 2  | `register_device`   | Register a device token for a user/platform           |
//! | 3  | `unregister_device` | Remove a device registration for a user/platform      |
//!
//! FCM uses the HTTP v1 API. WebPush sends VAPID-authenticated requests to
//! browser push endpoints. Device registrations are tracked in an in-memory
//! HashMap keyed by user_id.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use soma_port_sdk::prelude::*;

const PORT_ID: &str = "soma.push";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeviceRegistration {
    platform: String,
    token: String,
}

// ---------------------------------------------------------------------------
// Port struct
// ---------------------------------------------------------------------------

pub struct PushPort {
    spec: PortSpec,
    client: Option<reqwest::blocking::Client>,
    devices: RwLock<HashMap<String, Vec<DeviceRegistration>>>,
}

impl PushPort {
    pub fn new() -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("soma-push-port/0.1.0")
            .build()
            .ok();

        Self {
            spec: build_spec(),
            client,
            devices: RwLock::new(HashMap::new()),
        }
    }

    fn client(&self) -> soma_port_sdk::Result<&reqwest::blocking::Client> {
        self.client
            .as_ref()
            .ok_or_else(|| PortError::DependencyUnavailable("HTTP client not initialized".into()))
    }
}

impl Default for PushPort {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Port trait implementation
// ---------------------------------------------------------------------------

impl Port for PushPort {
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
            "send_fcm" => self.send_fcm(&input),
            "send_webpush" => self.send_webpush(&input),
            "register_device" => self.register_device(&input),
            "unregister_device" => self.unregister_device(&input),
            other => {
                return Err(PortError::Validation(format!(
                    "unknown capability: {other}"
                )))
            }
        };
        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(value) => Ok(PortCallRecord::success(PORT_ID, capability_id, value, latency_ms)),
            Err(e) => Ok(PortCallRecord::failure(
                PORT_ID,
                capability_id,
                e.failure_class(),
                &e.to_string(),
                latency_ms,
            )),
        }
    }

    fn validate_input(
        &self,
        capability_id: &str,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<()> {
        match capability_id {
            "send_fcm" => {
                require_field(input, "device_token")?;
                require_field(input, "title")?;
                require_field(input, "body")?;
            }
            "send_webpush" => {
                require_field(input, "subscription_json")?;
                require_field(input, "title")?;
                require_field(input, "body")?;
            }
            "register_device" => {
                require_field(input, "user_id")?;
                require_field(input, "platform")?;
                require_field(input, "token")?;
            }
            "unregister_device" => {
                require_field(input, "user_id")?;
                require_field(input, "platform")?;
            }
            other => {
                return Err(PortError::Validation(format!(
                    "unknown capability: {other}"
                )))
            }
        }
        Ok(())
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        if self.client.is_some() {
            PortLifecycleState::Active
        } else {
            PortLifecycleState::Loaded
        }
    }
}

// ---------------------------------------------------------------------------
// Capability implementations
// ---------------------------------------------------------------------------

impl PushPort {
    fn send_fcm(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let device_token = get_str(input, "device_token")?;
        let title = get_str(input, "title")?;
        let body = get_str(input, "body")?;
        let project_id = input["project_id"].as_str().unwrap_or("default-project");

        let data = input.get("data").cloned().unwrap_or(serde_json::json!({}));

        let access_token = get_str(input, "access_token")
            .unwrap_or("missing-token");

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
            .bearer_auth(access_token)
            .json(&payload)
            .send()
            .map_err(|e| PortError::TransportError(format!("FCM send failed: {e}")))?;

        if resp.status().is_success() {
            Ok(serde_json::json!({ "sent": true, "device_token": device_token }))
        } else {
            let status = resp.status().as_u16();
            let resp_body = resp.text().unwrap_or_default();
            Err(PortError::ExternalError(format!(
                "FCM send failed (HTTP {status}): {resp_body}"
            )))
        }
    }

    fn send_webpush(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let subscription_json = get_str(input, "subscription_json")?;
        let title = get_str(input, "title")?;
        let body = get_str(input, "body")?;

        let subscription: serde_json::Value = serde_json::from_str(subscription_json)
            .map_err(|e| PortError::Validation(format!("invalid subscription JSON: {e}")))?;

        let endpoint = subscription["endpoint"]
            .as_str()
            .ok_or_else(|| PortError::Validation("subscription missing 'endpoint'".into()))?;

        let payload = serde_json::json!({ "title": title, "body": body });
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| PortError::Internal(format!("payload serialization failed: {e}")))?;

        let vapid_key = input["vapid_key"].as_str().unwrap_or("");
        let authorization = format!("vapid t=placeholder, k={vapid_key}");

        let client = self.client()?;
        let resp = client
            .post(endpoint)
            .header("Authorization", &authorization)
            .header("Content-Type", "application/octet-stream")
            .header("TTL", "86400")
            .body(payload_bytes)
            .send()
            .map_err(|e| PortError::TransportError(format!("WebPush send failed: {e}")))?;

        let status = resp.status();
        if status.is_success() || status.as_u16() == 201 {
            Ok(serde_json::json!({ "sent": true }))
        } else {
            let resp_body = resp.text().unwrap_or_default();
            Err(PortError::ExternalError(format!(
                "WebPush send failed (HTTP {status}): {resp_body}"
            )))
        }
    }

    fn register_device(
        &self,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let user_id = get_str(input, "user_id")?;
        let platform = get_str(input, "platform")?;
        let token = get_str(input, "token")?;

        match platform {
            "android" | "ios" | "web" => {}
            other => {
                return Err(PortError::Validation(format!(
                    "unsupported platform '{other}': must be android, ios, or web"
                )));
            }
        }

        let mut devices = self.devices.write().map_err(|e| {
            PortError::Internal(format!("device registry lock poisoned: {e}"))
        })?;

        let registrations = devices.entry(user_id.to_string()).or_default();

        if let Some(existing) = registrations.iter_mut().find(|r| r.platform == platform) {
            existing.token = token.to_string();
        } else {
            registrations.push(DeviceRegistration {
                platform: platform.to_string(),
                token: token.to_string(),
            });
        }

        Ok(serde_json::json!({
            "registered": true,
            "user_id": user_id,
            "platform": platform,
        }))
    }

    fn unregister_device(
        &self,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let user_id = get_str(input, "user_id")?;
        let platform = get_str(input, "platform")?;

        let mut devices = self.devices.write().map_err(|e| {
            PortError::Internal(format!("device registry lock poisoned: {e}"))
        })?;

        let removed = if let Some(registrations) = devices.get_mut(user_id) {
            let before = registrations.len();
            registrations.retain(|r| r.platform != platform);
            let removed = registrations.len() < before;
            if registrations.is_empty() {
                devices.remove(user_id);
            }
            removed
        } else {
            false
        };

        Ok(serde_json::json!({
            "removed": removed,
            "user_id": user_id,
            "platform": platform,
        }))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn require_field(input: &serde_json::Value, field: &str) -> soma_port_sdk::Result<()> {
    if input.get(field).is_none() {
        return Err(PortError::Validation(format!("missing field: {field}")));
    }
    Ok(())
}

fn get_str<'a>(input: &'a serde_json::Value, field: &str) -> soma_port_sdk::Result<&'a str> {
    input[field]
        .as_str()
        .ok_or_else(|| PortError::Validation(format!("{field} must be a string")))
}

// ---------------------------------------------------------------------------
// Spec builder
// ---------------------------------------------------------------------------

fn build_spec() -> PortSpec {
    PortSpec {
        port_id: PORT_ID.into(),
        name: "push".into(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Messaging,
        description: "Push notifications: FCM, WebPush (VAPID), device registration".into(),
        namespace: "soma.push".into(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![
            PortCapabilitySpec {
                capability_id: "send_fcm".into(),
                name: "send_fcm".into(),
                purpose: "Send push notification via Firebase Cloud Messaging HTTP v1 API".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "device_token": {"type": "string"}, "title": {"type": "string"},
                    "body": {"type": "string"}, "data": {"type": "object"},
                    "project_id": {"type": "string"}, "access_token": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "sent": {"type": "boolean"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Low,
                latency_profile: LatencyProfile { expected_latency_ms: 500, p95_latency_ms: 3000, max_latency_ms: 30000 },
                cost_profile: CostProfile { network_cost_class: CostClass::Low, ..CostProfile::default() },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "send_webpush".into(),
                name: "send_webpush".into(),
                purpose: "Send push notification via Web Push protocol (VAPID)".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "subscription_json": {"type": "string"}, "title": {"type": "string"},
                    "body": {"type": "string"}, "vapid_key": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "sent": {"type": "boolean"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Low,
                latency_profile: LatencyProfile { expected_latency_ms: 500, p95_latency_ms: 3000, max_latency_ms: 30000 },
                cost_profile: CostProfile { network_cost_class: CostClass::Low, ..CostProfile::default() },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "register_device".into(),
                name: "register_device".into(),
                purpose: "Register a device token for push notifications".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "user_id": {"type": "string"}, "platform": {"type": "string"},
                    "token": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "registered": {"type": "boolean"},
                })),
                effect_class: SideEffectClass::LocalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile { expected_latency_ms: 1, p95_latency_ms: 5, max_latency_ms: 10 },
                cost_profile: CostProfile::default(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "unregister_device".into(),
                name: "unregister_device".into(),
                purpose: "Unregister a device from push notifications".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "user_id": {"type": "string"}, "platform": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "removed": {"type": "boolean"},
                })),
                effect_class: SideEffectClass::LocalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile { expected_latency_ms: 1, p95_latency_ms: 5, max_latency_ms: 10 },
                cost_profile: CostProfile::default(),
                remote_exposable: false,
                auth_override: None,
            },
        ],
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::any(),
        failure_modes: vec![
            PortFailureClass::ValidationError, PortFailureClass::ExternalError,
            PortFailureClass::TransportError, PortFailureClass::Timeout,
        ],
        side_effect_class: SideEffectClass::ExternalStateMutation,
        latency_profile: LatencyProfile { expected_latency_ms: 200, p95_latency_ms: 3000, max_latency_ms: 30000 },
        cost_profile: CostProfile { network_cost_class: CostClass::Low, ..CostProfile::default() },
        auth_requirements: AuthRequirements { methods: vec![AuthMethod::ApiKey], required: true },
        sandbox_requirements: SandboxRequirements { network_access: true, ..SandboxRequirements::default() },
        observable_fields: vec!["user_id".into(), "platform".into(), "sent".into()],
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
    Box::into_raw(Box::new(PushPort::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec() {
        let port = PushPort::new();
        assert_eq!(port.spec().port_id, "soma.push");
        assert_eq!(port.spec().capabilities.len(), 4);
    }

    #[test]
    fn test_register_device() {
        let port = PushPort::new();
        let input = serde_json::json!({
            "user_id": "user-1", "platform": "android", "token": "tok-123"
        });
        let record = port.invoke("register_device", input).unwrap();
        assert!(record.success);
    }

    #[test]
    fn test_register_invalid_platform() {
        let port = PushPort::new();
        let input = serde_json::json!({
            "user_id": "user-1", "platform": "windows", "token": "tok-123"
        });
        let record = port.invoke("register_device", input).unwrap();
        assert!(!record.success);
    }

    #[test]
    fn test_unregister_nonexistent() {
        let port = PushPort::new();
        let input = serde_json::json!({
            "user_id": "nobody", "platform": "android"
        });
        let record = port.invoke("unregister_device", input).unwrap();
        assert!(record.success);
        assert_eq!(record.raw_result["removed"], false);
    }

    #[test]
    fn test_register_then_unregister() {
        let port = PushPort::new();
        port.invoke("register_device", serde_json::json!({
            "user_id": "user-2", "platform": "ios", "token": "tok-456"
        })).unwrap();

        let record = port.invoke("unregister_device", serde_json::json!({
            "user_id": "user-2", "platform": "ios"
        })).unwrap();
        assert!(record.success);
        assert_eq!(record.raw_result["removed"], true);
    }
}
