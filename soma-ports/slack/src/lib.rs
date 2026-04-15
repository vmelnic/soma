//! SOMA Slack Port -- messaging via the Slack Web API.
//!
//! Four capabilities:
//!
//! | ID | Name             | Description                              |
//! |----|------------------|------------------------------------------|
//! | 0  | `send_message`   | Post a message to a channel or thread    |
//! | 1  | `list_channels`  | List public channels in the workspace    |
//! | 2  | `upload_file`    | Upload file content to a channel         |
//! | 3  | `add_reaction`   | Add an emoji reaction to a message       |
//!
//! Uses `reqwest::blocking::Client` with Bearer auth (bot token).
//! If no bot token is configured, the port loads (lifecycle Active) but returns
//! an error on invoke explaining the missing config.

use std::time::Instant;

use soma_port_sdk::prelude::*;

const PORT_ID: &str = "soma.slack";
const SLACK_API_BASE: &str = "https://slack.com/api";

// ---------------------------------------------------------------------------
// Port struct
// ---------------------------------------------------------------------------

pub struct SlackPort {
    spec: PortSpec,
    bot_token: Option<String>,
    client: reqwest::blocking::Client,
}

impl SlackPort {
    pub fn new() -> Self {
        let bot_token = std::env::var("SOMA_SLACK_BOT_TOKEN")
            .ok()
            .or_else(|| std::env::var("SLACK_BOT_TOKEN").ok())
            .filter(|v| !v.is_empty());

        Self {
            spec: build_spec(),
            bot_token,
            client: reqwest::blocking::Client::new(),
        }
    }

    fn require_token(&self) -> soma_port_sdk::Result<&str> {
        self.bot_token.as_deref().ok_or_else(|| {
            PortError::DependencyUnavailable(
                "Slack bot token not configured. Set SOMA_SLACK_BOT_TOKEN or SLACK_BOT_TOKEN"
                    .into(),
            )
        })
    }

    fn post_json(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let token = self.require_token()?;
        let resp = self
            .client
            .post(url)
            .bearer_auth(token)
            .json(body)
            .send()
            .map_err(|e| PortError::TransportError(format!("Slack request failed: {e}")))?;

        let result: serde_json::Value = resp
            .json()
            .map_err(|e| PortError::ExternalError(format!("failed to parse Slack response: {e}")))?;

        // Slack returns 200 with ok=false for API-level errors
        if result.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let error = result["error"].as_str().unwrap_or("unknown_error");
            return Err(PortError::ExternalError(format!(
                "Slack API error: {error}"
            )));
        }

        Ok(result)
    }

    fn get_json(&self, url: &str) -> soma_port_sdk::Result<serde_json::Value> {
        let token = self.require_token()?;
        let resp = self
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .map_err(|e| PortError::TransportError(format!("Slack request failed: {e}")))?;

        let result: serde_json::Value = resp
            .json()
            .map_err(|e| PortError::ExternalError(format!("failed to parse Slack response: {e}")))?;

        if result.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let error = result["error"].as_str().unwrap_or("unknown_error");
            return Err(PortError::ExternalError(format!(
                "Slack API error: {error}"
            )));
        }

        Ok(result)
    }
}

impl Default for SlackPort {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Port trait implementation
// ---------------------------------------------------------------------------

impl Port for SlackPort {
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
            "send_message" => self.send_message(&input),
            "list_channels" => self.list_channels(&input),
            "upload_file" => self.upload_file(&input),
            "add_reaction" => self.add_reaction(&input),
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
            "send_message" => {
                require_field(input, "channel")?;
                require_field(input, "text")?;
            }
            "list_channels" => { /* all optional */ }
            "upload_file" => {
                require_field(input, "channel")?;
                require_field(input, "content")?;
                require_field(input, "filename")?;
            }
            "add_reaction" => {
                require_field(input, "channel")?;
                require_field(input, "timestamp")?;
                require_field(input, "name")?;
            }
            other => {
                return Err(PortError::Validation(format!(
                    "unknown capability: {other}"
                )));
            }
        }
        Ok(())
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        PortLifecycleState::Active
    }
}

// ---------------------------------------------------------------------------
// Capability implementations
// ---------------------------------------------------------------------------

impl SlackPort {
    fn send_message(
        &self,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let channel = get_str(input, "channel")?;
        let text = get_str(input, "text")?;

        let mut body = serde_json::json!({
            "channel": channel,
            "text": text,
        });

        if let Some(thread_ts) = input.get("thread_ts").and_then(|v| v.as_str()) {
            body["thread_ts"] = serde_json::Value::String(thread_ts.to_string());
        }

        self.post_json(&format!("{SLACK_API_BASE}/chat.postMessage"), &body)
    }

    fn list_channels(
        &self,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
            .unwrap_or(100);

        self.get_json(&format!(
            "{SLACK_API_BASE}/conversations.list?limit={limit}&types=public_channel"
        ))
    }

    fn upload_file(
        &self,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let channel = get_str(input, "channel")?;
        let content = get_str(input, "content")?;
        let filename = get_str(input, "filename")?;

        let mut body = serde_json::json!({
            "channels": channel,
            "content": content,
            "filename": filename,
        });

        if let Some(title) = input.get("title").and_then(|v| v.as_str()) {
            body["title"] = serde_json::Value::String(title.to_string());
        }

        // files.upload is deprecated in newer Slack API but widely supported.
        // For v2, would use files.uploadV2 with a multi-step flow.
        self.post_json(&format!("{SLACK_API_BASE}/files.upload"), &body)
    }

    fn add_reaction(
        &self,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let channel = get_str(input, "channel")?;
        let timestamp = get_str(input, "timestamp")?;
        let name = get_str(input, "name")?;

        let body = serde_json::json!({
            "channel": channel,
            "timestamp": timestamp,
            "name": name,
        });

        self.post_json(&format!("{SLACK_API_BASE}/reactions.add"), &body)
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
        name: "slack".into(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Messaging,
        description: "Slack messaging: send messages, list channels, upload files, add reactions"
            .into(),
        namespace: "soma.slack".into(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![
            PortCapabilitySpec {
                capability_id: "send_message".into(),
                name: "send_message".into(),
                purpose: "Post a message to a Slack channel or thread".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "channel": {"type": "string", "description": "Channel ID or name"},
                    "text": {"type": "string", "description": "Message text (supports Slack mrkdwn)"},
                    "thread_ts": {"type": "string", "description": "Thread timestamp for replies"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "ok": {"type": "boolean"},
                    "channel": {"type": "string"},
                    "ts": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
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
                capability_id: "list_channels".into(),
                name: "list_channels".into(),
                purpose: "List public channels in the Slack workspace".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "limit": {"type": "integer", "description": "Max channels to return (default 100)"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "ok": {"type": "boolean"},
                    "channels": {"type": "array"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
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
                capability_id: "upload_file".into(),
                name: "upload_file".into(),
                purpose: "Upload file content to a Slack channel".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "channel": {"type": "string", "description": "Channel ID or name"},
                    "content": {"type": "string", "description": "File content as text"},
                    "filename": {"type": "string", "description": "Filename for the upload"},
                    "title": {"type": "string", "description": "Display title for the file"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "ok": {"type": "boolean"},
                    "file": {"type": "object"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Low,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 1000,
                    p95_latency_ms: 5000,
                    max_latency_ms: 30000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Medium,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "add_reaction".into(),
                name: "add_reaction".into(),
                purpose: "Add an emoji reaction to a message".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "channel": {"type": "string", "description": "Channel containing the message"},
                    "timestamp": {"type": "string", "description": "Message timestamp"},
                    "name": {"type": "string", "description": "Emoji name without colons (e.g. thumbsup)"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "ok": {"type": "boolean"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::ConditionallyIdempotent,
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
            expected_latency_ms: 500,
            p95_latency_ms: 5000,
            max_latency_ms: 30000,
        },
        cost_profile: CostProfile {
            network_cost_class: CostClass::Medium,
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
    Box::into_raw(Box::new(SlackPort::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec() {
        let port = SlackPort::new();
        assert_eq!(port.spec().port_id, "soma.slack");
        assert_eq!(port.spec().capabilities.len(), 4);
    }

    #[test]
    fn test_lifecycle_active() {
        let port = SlackPort::new();
        assert_eq!(port.lifecycle_state(), PortLifecycleState::Active);
    }

    #[test]
    fn test_validate_send_message_missing_fields() {
        let port = SlackPort::new();
        assert!(port
            .validate_input("send_message", &serde_json::json!({}))
            .is_err());
    }

    #[test]
    fn test_validate_send_message_ok() {
        let port = SlackPort::new();
        let input = serde_json::json!({"channel": "#general", "text": "Hello"});
        assert!(port.validate_input("send_message", &input).is_ok());
    }

    #[test]
    fn test_validate_add_reaction_missing_fields() {
        let port = SlackPort::new();
        assert!(port
            .validate_input(
                "add_reaction",
                &serde_json::json!({"channel": "#general", "timestamp": "123"})
            )
            .is_err());
    }

    #[test]
    fn test_unknown_capability() {
        let port = SlackPort::new();
        assert!(port.invoke("nonexistent", serde_json::json!({})).is_err());
    }

    #[test]
    fn test_invoke_without_token_returns_failure_record() {
        let port = SlackPort::new();
        let input = serde_json::json!({"channel": "#general", "text": "test"});
        let record = port.invoke("send_message", input).unwrap();
        assert!(!record.success);
    }
}
