//! SOMA SMTP Port -- email delivery via the `lettre` crate.
//!
//! Three capabilities:
//!
//! | ID | Name               | Description                              |
//! |----|--------------------|------------------------------------------|
//! | 0  | `send_plain`       | Send a plain-text email                  |
//! | 1  | `send_html`        | Send an HTML email                       |
//! | 2  | `send_attachment`  | Send an email with a binary attachment   |
//!
//! The Port trait is synchronous but lettre's SMTP transport is async. A
//! dedicated tokio runtime is created at construction time and `block_on()`
//! bridges async operations into the sync interface.

use std::sync::OnceLock;
use std::time::Instant;

use lettre::message::{Attachment, MultiPart, SinglePart, header::ContentType};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use soma_port_sdk::prelude::*;

const PORT_ID: &str = "soma.smtp";

// ---------------------------------------------------------------------------
// Port struct
// ---------------------------------------------------------------------------

pub struct SmtpPort {
    spec: PortSpec,
    host: OnceLock<String>,
    port: OnceLock<u16>,
    credentials: OnceLock<Credentials>,
    from: OnceLock<String>,
    runtime: OnceLock<tokio::runtime::Runtime>,
}

impl SmtpPort {
    pub fn new() -> Self {
        Self {
            spec: build_spec(),
            host: OnceLock::new(),
            port: OnceLock::new(),
            credentials: OnceLock::new(),
            from: OnceLock::new(),
            runtime: OnceLock::new(),
        }
    }

    fn build_transport(&self) -> soma_port_sdk::Result<AsyncSmtpTransport<Tokio1Executor>> {
        let host = self
            .host
            .get()
            .ok_or_else(|| PortError::DependencyUnavailable("SMTP host not configured".into()))?;
        let port = self
            .port
            .get()
            .ok_or_else(|| PortError::DependencyUnavailable("SMTP port not configured".into()))?;
        let creds = self.credentials.get().ok_or_else(|| {
            PortError::DependencyUnavailable("SMTP credentials not configured".into())
        })?;

        let transport = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
            .map_err(|e| PortError::TransportError(format!("SMTP relay error: {e}")))?
            .port(*port)
            .credentials(creds.clone())
            .build();

        Ok(transport)
    }

    fn rt(&self) -> soma_port_sdk::Result<&tokio::runtime::Runtime> {
        self.runtime
            .get()
            .ok_or_else(|| PortError::DependencyUnavailable("SMTP runtime not initialized".into()))
    }

    fn sender_address(&self) -> soma_port_sdk::Result<&str> {
        self.from.get().map(|s| s.as_str()).ok_or_else(|| {
            PortError::DependencyUnavailable("SMTP from address not configured".into())
        })
    }
}

impl Default for SmtpPort {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Port trait implementation
// ---------------------------------------------------------------------------

impl Port for SmtpPort {
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
            "send_plain" => self.send_plain(&input),
            "send_html" => self.send_html(&input),
            "send_attachment" => self.send_attachment(&input),
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
            "send_plain" | "send_html" => {
                require_field(input, "to")?;
                require_field(input, "subject")?;
                require_field(input, "body")?;
            }
            "send_attachment" => {
                require_field(input, "to")?;
                require_field(input, "subject")?;
                require_field(input, "body")?;
                require_field(input, "attachment_name")?;
                require_field(input, "attachment_data")?;
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
        if self.host.get().is_some() {
            PortLifecycleState::Active
        } else {
            PortLifecycleState::Loaded
        }
    }
}

// ---------------------------------------------------------------------------
// Capability implementations
// ---------------------------------------------------------------------------

impl SmtpPort {
    fn send_plain(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let to = get_str(input, "to")?;
        let subject = get_str(input, "subject")?;
        let body = get_str(input, "body")?;
        let from = self.sender_address()?;

        let message = Message::builder()
            .from(
                from.parse()
                    .map_err(|e| PortError::Validation(format!("invalid from: {e}")))?,
            )
            .to(to
                .parse()
                .map_err(|e| PortError::Validation(format!("invalid to: {e}")))?)
            .subject(subject)
            .body(body.to_string())
            .map_err(|e| PortError::Internal(format!("failed to build email: {e}")))?;

        let transport = self.build_transport()?;
        let rt = self.rt()?;

        rt.block_on(async {
            transport
                .send(message)
                .await
                .map_err(|e| PortError::ExternalError(format!("SMTP send failed: {e}")))
        })?;

        Ok(serde_json::json!({ "sent": true, "to": to }))
    }

    fn send_html(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let to = get_str(input, "to")?;
        let subject = get_str(input, "subject")?;
        let body = get_str(input, "body")?;
        let from = self.sender_address()?;

        let message = Message::builder()
            .from(
                from.parse()
                    .map_err(|e| PortError::Validation(format!("invalid from: {e}")))?,
            )
            .to(to
                .parse()
                .map_err(|e| PortError::Validation(format!("invalid to: {e}")))?)
            .subject(subject)
            .singlepart(
                SinglePart::builder()
                    .header(ContentType::TEXT_HTML)
                    .body(body.to_string()),
            )
            .map_err(|e| PortError::Internal(format!("failed to build email: {e}")))?;

        let transport = self.build_transport()?;
        let rt = self.rt()?;

        rt.block_on(async {
            transport
                .send(message)
                .await
                .map_err(|e| PortError::ExternalError(format!("SMTP send failed: {e}")))
        })?;

        Ok(serde_json::json!({ "sent": true, "to": to }))
    }

    fn send_attachment(
        &self,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let to = get_str(input, "to")?;
        let subject = get_str(input, "subject")?;
        let body = get_str(input, "body")?;
        let attachment_name = get_str(input, "attachment_name")?;
        let attachment_b64 = get_str(input, "attachment_data")?;
        let from = self.sender_address()?;

        use base64::Engine;
        let attachment_data = base64::engine::general_purpose::STANDARD
            .decode(attachment_b64)
            .map_err(|e| {
                PortError::Validation(format!("invalid base64 in attachment_data: {e}"))
            })?;

        let attachment = Attachment::new(attachment_name.to_string()).body(
            attachment_data,
            ContentType::parse("application/octet-stream").unwrap(),
        );

        let message = Message::builder()
            .from(
                from.parse()
                    .map_err(|e| PortError::Validation(format!("invalid from: {e}")))?,
            )
            .to(to
                .parse()
                .map_err(|e| PortError::Validation(format!("invalid to: {e}")))?)
            .subject(subject)
            .multipart(
                MultiPart::mixed()
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_PLAIN)
                            .body(body.to_string()),
                    )
                    .singlepart(attachment),
            )
            .map_err(|e| PortError::Internal(format!("failed to build email: {e}")))?;

        let transport = self.build_transport()?;
        let rt = self.rt()?;

        rt.block_on(async {
            transport
                .send(message)
                .await
                .map_err(|e| PortError::ExternalError(format!("SMTP send failed: {e}")))
        })?;

        Ok(serde_json::json!({ "sent": true, "to": to, "attachment": attachment_name }))
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
        name: "smtp".into(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Messaging,
        description: "SMTP email delivery: plain text, HTML, and attachments via lettre".into(),
        namespace: "soma.smtp".into(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![
            PortCapabilitySpec {
                capability_id: "send_plain".into(),
                name: "send_plain".into(),
                purpose: "Send a plain-text email to a recipient".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "to": {"type": "string"}, "subject": {"type": "string"},
                    "body": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "sent": {"type": "boolean"}, "to": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Low,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 500,
                    p95_latency_ms: 5000,
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
                capability_id: "send_html".into(),
                name: "send_html".into(),
                purpose: "Send an HTML email to a recipient".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "to": {"type": "string"}, "subject": {"type": "string"},
                    "body": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "sent": {"type": "boolean"}, "to": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Low,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 500,
                    p95_latency_ms: 5000,
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
                capability_id: "send_attachment".into(),
                name: "send_attachment".into(),
                purpose: "Send an email with a binary attachment".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "to": {"type": "string"}, "subject": {"type": "string"},
                    "body": {"type": "string"}, "attachment_name": {"type": "string"},
                    "attachment_data": {"type": "string", "description": "Base64-encoded binary"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "sent": {"type": "boolean"}, "to": {"type": "string"},
                    "attachment": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Low,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 1000,
                    p95_latency_ms: 10000,
                    max_latency_ms: 30000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Medium,
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
            methods: vec![AuthMethod::ApiKey],
            required: true,
        },
        sandbox_requirements: SandboxRequirements {
            network_access: true,
            ..SandboxRequirements::default()
        },
        observable_fields: vec!["to".into(), "subject".into(), "sent".into()],
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
    Box::into_raw(Box::new(SmtpPort::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec() {
        let port = SmtpPort::new();
        assert_eq!(port.spec().port_id, "soma.smtp");
        assert_eq!(port.spec().capabilities.len(), 3);
    }

    #[test]
    fn test_lifecycle_before_config() {
        let port = SmtpPort::new();
        assert_eq!(port.lifecycle_state(), PortLifecycleState::Loaded);
    }

    #[test]
    fn test_validate_send_plain_missing_fields() {
        let port = SmtpPort::new();
        assert!(
            port.validate_input("send_plain", &serde_json::json!({}))
                .is_err()
        );
    }

    #[test]
    fn test_validate_send_plain_all_fields() {
        let port = SmtpPort::new();
        let input = serde_json::json!({"to": "a@b.com", "subject": "hi", "body": "hello"});
        assert!(port.validate_input("send_plain", &input).is_ok());
    }

    #[test]
    fn test_invoke_without_config() {
        let port = SmtpPort::new();
        let record = port
            .invoke(
                "send_plain",
                serde_json::json!({
                    "to": "a@b.com", "subject": "test", "body": "hello"
                }),
            )
            .unwrap();
        assert!(!record.success);
    }

    #[test]
    fn test_unknown_capability() {
        let port = SmtpPort::new();
        assert!(port.invoke("nonexistent", serde_json::json!({})).is_err());
    }
}
