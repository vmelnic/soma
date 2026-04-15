//! SOMA Google Mail (Gmail) Port -- send and read email via the Gmail API.
//!
//! Four capabilities:
//!
//! | ID | Name             | Description                           |
//! |----|------------------|---------------------------------------|
//! | 0  | `send_email`     | Send an email (RFC 2822, base64url)   |
//! | 1  | `list_messages`  | List messages with optional query      |
//! | 2  | `get_message`    | Get a single message by ID             |
//! | 3  | `list_labels`    | List all labels in the mailbox         |
//!
//! Auth: OAuth2 Bearer token via `SOMA_GOOGLE_ACCESS_TOKEN` or `GOOGLE_ACCESS_TOKEN`.

use std::time::Instant;

use base64::Engine;
use soma_port_sdk::prelude::*;

const PORT_ID: &str = "soma.google.mail";
const BASE_URL: &str = "https://gmail.googleapis.com/gmail/v1/users/me";

// ---------------------------------------------------------------------------
// Port struct
// ---------------------------------------------------------------------------

pub struct GoogleMailPort {
    spec: PortSpec,
}

impl GoogleMailPort {
    pub fn new() -> Self {
        Self {
            spec: build_spec(),
        }
    }

    fn access_token() -> soma_port_sdk::Result<String> {
        std::env::var("SOMA_GOOGLE_ACCESS_TOKEN")
            .or_else(|_| std::env::var("GOOGLE_ACCESS_TOKEN"))
            .map_err(|_| {
                PortError::DependencyUnavailable(
                    "Google access token not set. Set SOMA_GOOGLE_ACCESS_TOKEN or GOOGLE_ACCESS_TOKEN"
                        .into(),
                )
            })
    }

    fn client() -> reqwest::blocking::Client {
        reqwest::blocking::Client::new()
    }
}

impl Default for GoogleMailPort {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Port trait implementation
// ---------------------------------------------------------------------------

impl Port for GoogleMailPort {
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
            "send_email" => self.send_email(&input),
            "list_messages" => self.list_messages(&input),
            "get_message" => self.get_message(&input),
            "list_labels" => self.list_labels(),
            other => {
                return Err(PortError::Validation(format!(
                    "unknown capability: {other}"
                )));
            }
        };
        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(value) => Ok(PortCallRecord::success(
                PORT_ID,
                capability_id,
                value,
                latency_ms,
            )),
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
            "send_email" => {
                require_field(input, "to")?;
                require_field(input, "subject")?;
                require_field(input, "body")?;
            }
            "list_messages" => {}
            "get_message" => {
                require_field(input, "message_id")?;
            }
            "list_labels" => {}
            other => {
                return Err(PortError::Validation(format!(
                    "unknown capability: {other}"
                )));
            }
        }
        Ok(())
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        if Self::access_token().is_ok() {
            PortLifecycleState::Active
        } else {
            PortLifecycleState::Loaded
        }
    }
}

// ---------------------------------------------------------------------------
// Capability implementations
// ---------------------------------------------------------------------------

impl GoogleMailPort {
    fn send_email(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let token = Self::access_token()?;
        let to = get_str(input, "to")?;
        let subject = get_str(input, "subject")?;
        let body = get_str(input, "body")?;
        let cc = input.get("cc").and_then(|v| v.as_str());
        let bcc = input.get("bcc").and_then(|v| v.as_str());

        // Build RFC 2822 message
        let mut rfc2822 = String::new();
        rfc2822.push_str(&format!("To: {to}\r\n"));
        if let Some(cc_val) = cc {
            rfc2822.push_str(&format!("Cc: {cc_val}\r\n"));
        }
        if let Some(bcc_val) = bcc {
            rfc2822.push_str(&format!("Bcc: {bcc_val}\r\n"));
        }
        rfc2822.push_str(&format!("Subject: {subject}\r\n"));
        rfc2822.push_str("Content-Type: text/plain; charset=UTF-8\r\n");
        rfc2822.push_str("\r\n");
        rfc2822.push_str(body);

        // Gmail API expects base64url-encoded RFC 2822
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rfc2822.as_bytes());

        let payload = serde_json::json!({ "raw": encoded });

        let resp = Self::client()
            .post(format!("{BASE_URL}/messages/send"))
            .bearer_auth(&token)
            .json(&payload)
            .send()
            .map_err(|e| PortError::TransportError(format!("HTTP request failed: {e}")))?;

        let status = resp.status();
        let resp_body: serde_json::Value = resp
            .json()
            .map_err(|e| PortError::ExternalError(format!("failed to parse response: {e}")))?;

        if !status.is_success() {
            return Err(PortError::ExternalError(format!(
                "Gmail API error {status}: {resp_body}"
            )));
        }

        Ok(serde_json::json!({
            "sent": true,
            "to": to,
            "message_id": resp_body.get("id").and_then(|v| v.as_str()).unwrap_or(""),
            "thread_id": resp_body.get("threadId").and_then(|v| v.as_str()).unwrap_or(""),
        }))
    }

    fn list_messages(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let token = Self::access_token()?;

        let mut request = Self::client()
            .get(format!("{BASE_URL}/messages"))
            .bearer_auth(&token);

        if let Some(query) = input.get("query").and_then(|v| v.as_str()) {
            request = request.query(&[("q", query)]);
        }
        if let Some(max_results) = input.get("max_results").and_then(|v| v.as_u64()) {
            request = request.query(&[("maxResults", &max_results.to_string())]);
        }
        if let Some(page_token) = input.get("page_token").and_then(|v| v.as_str()) {
            request = request.query(&[("pageToken", page_token)]);
        }

        let resp = request
            .send()
            .map_err(|e| PortError::TransportError(format!("HTTP request failed: {e}")))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| PortError::ExternalError(format!("failed to parse response: {e}")))?;

        if !status.is_success() {
            return Err(PortError::ExternalError(format!(
                "Gmail API error {status}: {body}"
            )));
        }

        Ok(body)
    }

    fn get_message(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let token = Self::access_token()?;
        let message_id = get_str(input, "message_id")?;

        let resp = Self::client()
            .get(format!("{BASE_URL}/messages/{message_id}"))
            .bearer_auth(&token)
            .send()
            .map_err(|e| PortError::TransportError(format!("HTTP request failed: {e}")))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| PortError::ExternalError(format!("failed to parse response: {e}")))?;

        if !status.is_success() {
            return Err(PortError::ExternalError(format!(
                "Gmail API error {status}: {body}"
            )));
        }

        Ok(body)
    }

    fn list_labels(&self) -> soma_port_sdk::Result<serde_json::Value> {
        let token = Self::access_token()?;

        let resp = Self::client()
            .get(format!("{BASE_URL}/labels"))
            .bearer_auth(&token)
            .send()
            .map_err(|e| PortError::TransportError(format!("HTTP request failed: {e}")))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| PortError::ExternalError(format!("failed to parse response: {e}")))?;

        if !status.is_success() {
            return Err(PortError::ExternalError(format!(
                "Gmail API error {status}: {body}"
            )));
        }

        Ok(body)
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
        name: "google-mail".into(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Messaging,
        description: "Gmail API: send email, list/get messages, list labels".into(),
        namespace: "soma.google.mail".into(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![
            PortCapabilitySpec {
                capability_id: "send_email".into(),
                name: "send_email".into(),
                purpose: "Send an email via Gmail".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "to": {"type": "string"},
                    "subject": {"type": "string"},
                    "body": {"type": "string"},
                    "cc": {"type": "string"},
                    "bcc": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "sent": {"type": "boolean"},
                    "to": {"type": "string"},
                    "message_id": {"type": "string"},
                    "thread_id": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Low,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 500,
                    p95_latency_ms: 3000,
                    max_latency_ms: 10000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "list_messages".into(),
                name: "list_messages".into(),
                purpose: "List messages in the Gmail inbox with optional search query".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "query": {"type": "string", "description": "Gmail search query (e.g. \"from:alice subject:report\")"},
                    "max_results": {"type": "integer", "description": "Maximum number of messages to return"},
                    "page_token": {"type": "string", "description": "Token for fetching next page"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "description": "any",
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 300,
                    p95_latency_ms: 2000,
                    max_latency_ms: 10000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "get_message".into(),
                name: "get_message".into(),
                purpose: "Get a single Gmail message by ID".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "message_id": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "description": "any",
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 200,
                    p95_latency_ms: 1500,
                    max_latency_ms: 10000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "list_labels".into(),
                name: "list_labels".into(),
                purpose: "List all labels in the Gmail mailbox".into(),
                input_schema: SchemaRef::object(serde_json::json!({})),
                output_schema: SchemaRef::object(serde_json::json!({
                    "description": "any",
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 200,
                    p95_latency_ms: 1500,
                    max_latency_ms: 10000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
        ],
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::any(),
        failure_modes: vec![
            PortFailureClass::ValidationError,
            PortFailureClass::ExternalError,
            PortFailureClass::TransportError,
            PortFailureClass::Timeout,
            PortFailureClass::DependencyUnavailable,
            PortFailureClass::AuthorizationDenied,
        ],
        side_effect_class: SideEffectClass::ExternalStateMutation,
        latency_profile: LatencyProfile {
            expected_latency_ms: 300,
            p95_latency_ms: 3000,
            max_latency_ms: 10000,
        },
        cost_profile: CostProfile {
            network_cost_class: CostClass::Low,
            ..CostProfile::default()
        },
        auth_requirements: AuthRequirements {
            methods: vec![AuthMethod::BearerToken],
            required: true,
        },
        sandbox_requirements: SandboxRequirements {
            network_access: true,
            ..SandboxRequirements::default()
        },
        observable_fields: vec![],
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
    Box::into_raw(Box::new(GoogleMailPort::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec() {
        let port = GoogleMailPort::new();
        assert_eq!(port.spec().port_id, "soma.google.mail");
        assert_eq!(port.spec().capabilities.len(), 4);
    }

    #[test]
    fn test_lifecycle_without_token() {
        unsafe { std::env::remove_var("SOMA_GOOGLE_ACCESS_TOKEN") };
        unsafe { std::env::remove_var("GOOGLE_ACCESS_TOKEN") };
        let port = GoogleMailPort::new();
        assert_eq!(port.lifecycle_state(), PortLifecycleState::Loaded);
    }

    #[test]
    fn test_validate_send_email_missing_fields() {
        let port = GoogleMailPort::new();
        assert!(port
            .validate_input("send_email", &serde_json::json!({}))
            .is_err());
    }

    #[test]
    fn test_validate_send_email_all_fields() {
        let port = GoogleMailPort::new();
        let input = serde_json::json!({
            "to": "test@example.com",
            "subject": "Test",
            "body": "Hello",
        });
        assert!(port.validate_input("send_email", &input).is_ok());
    }

    #[test]
    fn test_validate_list_messages_no_required_fields() {
        let port = GoogleMailPort::new();
        assert!(port
            .validate_input("list_messages", &serde_json::json!({}))
            .is_ok());
    }

    #[test]
    fn test_validate_get_message_missing_id() {
        let port = GoogleMailPort::new();
        assert!(port
            .validate_input("get_message", &serde_json::json!({}))
            .is_err());
    }

    #[test]
    fn test_validate_list_labels_no_required_fields() {
        let port = GoogleMailPort::new();
        assert!(port
            .validate_input("list_labels", &serde_json::json!({}))
            .is_ok());
    }

    #[test]
    fn test_unknown_capability() {
        let port = GoogleMailPort::new();
        assert!(port.invoke("nonexistent", serde_json::json!({})).is_err());
    }
}
