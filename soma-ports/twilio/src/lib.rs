//! SOMA Twilio Port -- communications via the Twilio REST API.
//!
//! Four capabilities:
//!
//! | ID | Name             | Description                              |
//! |----|------------------|------------------------------------------|
//! | 0  | `send_sms`       | Send an SMS message                      |
//! | 1  | `send_whatsapp`  | Send a WhatsApp message                  |
//! | 2  | `make_call`      | Initiate a phone call with TwiML         |
//! | 3  | `list_messages`  | List recent messages                     |
//!
//! Uses `reqwest::blocking::Client` with HTTP Basic Auth (account SID + auth token).
//! If credentials are not configured, the port loads (lifecycle Active) but returns
//! an error on invoke explaining the missing config.

use std::time::Instant;

use soma_port_sdk::prelude::*;

const PORT_ID: &str = "soma.twilio";
const TWILIO_API_BASE: &str = "https://api.twilio.com/2010-04-01";

// ---------------------------------------------------------------------------
// Port struct
// ---------------------------------------------------------------------------

pub struct TwilioPort {
    spec: PortSpec,
    account_sid: Option<String>,
    auth_token: Option<String>,
    from_number: Option<String>,
    client: reqwest::blocking::Client,
}

impl TwilioPort {
    pub fn new() -> Self {
        let account_sid = std::env::var("SOMA_TWILIO_ACCOUNT_SID")
            .ok()
            .or_else(|| std::env::var("TWILIO_ACCOUNT_SID").ok())
            .filter(|v| !v.is_empty());

        let auth_token = std::env::var("SOMA_TWILIO_AUTH_TOKEN")
            .ok()
            .or_else(|| std::env::var("TWILIO_AUTH_TOKEN").ok())
            .filter(|v| !v.is_empty());

        let from_number = std::env::var("SOMA_TWILIO_FROM_NUMBER")
            .ok()
            .or_else(|| std::env::var("TWILIO_FROM_NUMBER").ok())
            .filter(|v| !v.is_empty());

        Self {
            spec: build_spec(),
            account_sid,
            auth_token,
            from_number,
            client: reqwest::blocking::Client::new(),
        }
    }

    fn require_config(&self) -> soma_port_sdk::Result<(&str, &str, &str)> {
        let sid = self.account_sid.as_deref().ok_or_else(|| {
            PortError::DependencyUnavailable(
                "Twilio account SID not configured. Set SOMA_TWILIO_ACCOUNT_SID".into(),
            )
        })?;
        let token = self.auth_token.as_deref().ok_or_else(|| {
            PortError::DependencyUnavailable(
                "Twilio auth token not configured. Set SOMA_TWILIO_AUTH_TOKEN".into(),
            )
        })?;
        let from = self.from_number.as_deref().ok_or_else(|| {
            PortError::DependencyUnavailable(
                "Twilio from number not configured. Set SOMA_TWILIO_FROM_NUMBER".into(),
            )
        })?;
        Ok((sid, token, from))
    }

    fn post_form(
        &self,
        url: &str,
        params: &[(&str, &str)],
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let (sid, token, _) = self.require_config()?;
        let resp = self
            .client
            .post(url)
            .basic_auth(sid, Some(token))
            .form(params)
            .send()
            .map_err(|e| PortError::TransportError(format!("Twilio request failed: {e}")))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| PortError::ExternalError(format!("failed to parse Twilio response: {e}")))?;

        if !status.is_success() {
            let msg = body["message"]
                .as_str()
                .unwrap_or("unknown Twilio error");
            return Err(PortError::ExternalError(format!(
                "Twilio API error ({status}): {msg}"
            )));
        }

        Ok(body)
    }

    fn get_json(&self, url: &str) -> soma_port_sdk::Result<serde_json::Value> {
        let (sid, token, _) = self.require_config()?;
        let resp = self
            .client
            .get(url)
            .basic_auth(sid, Some(token))
            .send()
            .map_err(|e| PortError::TransportError(format!("Twilio request failed: {e}")))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| PortError::ExternalError(format!("failed to parse Twilio response: {e}")))?;

        if !status.is_success() {
            let msg = body["message"]
                .as_str()
                .unwrap_or("unknown Twilio error");
            return Err(PortError::ExternalError(format!(
                "Twilio API error ({status}): {msg}"
            )));
        }

        Ok(body)
    }
}

impl Default for TwilioPort {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Port trait implementation
// ---------------------------------------------------------------------------

impl Port for TwilioPort {
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
            "send_sms" => self.send_sms(&input),
            "send_whatsapp" => self.send_whatsapp(&input),
            "make_call" => self.make_call(&input),
            "list_messages" => self.list_messages(&input),
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
            "send_sms" | "send_whatsapp" => {
                require_field(input, "to")?;
                require_field(input, "body")?;
            }
            "make_call" => {
                require_field(input, "to")?;
                require_field(input, "twiml_url")?;
            }
            "list_messages" => { /* all optional */ }
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

impl TwilioPort {
    fn send_sms(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let (sid, _, from) = self.require_config()?;
        let to = get_str(input, "to")?;
        let body = get_str(input, "body")?;

        let url = format!("{TWILIO_API_BASE}/Accounts/{sid}/Messages.json");
        self.post_form(&url, &[("To", to), ("From", from), ("Body", body)])
    }

    fn send_whatsapp(
        &self,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let (sid, _, from) = self.require_config()?;
        let to = get_str(input, "to")?;
        let body = get_str(input, "body")?;

        // Prefix "whatsapp:" to numbers for the WhatsApp channel
        let wa_to = if to.starts_with("whatsapp:") {
            to.to_string()
        } else {
            format!("whatsapp:{to}")
        };
        let wa_from = if from.starts_with("whatsapp:") {
            from.to_string()
        } else {
            format!("whatsapp:{from}")
        };

        let url = format!("{TWILIO_API_BASE}/Accounts/{sid}/Messages.json");
        self.post_form(&url, &[("To", &wa_to), ("From", &wa_from), ("Body", body)])
    }

    fn make_call(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let (sid, _, from) = self.require_config()?;
        let to = get_str(input, "to")?;
        let twiml_url = get_str(input, "twiml_url")?;

        let url = format!("{TWILIO_API_BASE}/Accounts/{sid}/Calls.json");
        self.post_form(&url, &[("To", to), ("From", from), ("Url", twiml_url)])
    }

    fn list_messages(
        &self,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let (sid, _, _) = self.require_config()?;
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
            .unwrap_or(20);

        let url = format!(
            "{TWILIO_API_BASE}/Accounts/{sid}/Messages.json?PageSize={limit}"
        );
        self.get_json(&url)
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
        name: "twilio".into(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Messaging,
        description: "Twilio communications: SMS, WhatsApp, and voice calls".into(),
        namespace: "soma.twilio".into(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![
            PortCapabilitySpec {
                capability_id: "send_sms".into(),
                name: "send_sms".into(),
                purpose: "Send an SMS message to a phone number".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "to": {"type": "string", "description": "Destination phone number (E.164)"},
                    "body": {"type": "string", "description": "Message text"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "sid": {"type": "string"},
                    "status": {"type": "string"},
                    "to": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Medium,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 1000,
                    p95_latency_ms: 5000,
                    max_latency_ms: 15000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Medium,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "send_whatsapp".into(),
                name: "send_whatsapp".into(),
                purpose: "Send a WhatsApp message to a phone number".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "to": {"type": "string", "description": "Destination phone number (E.164)"},
                    "body": {"type": "string", "description": "Message text"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "sid": {"type": "string"},
                    "status": {"type": "string"},
                    "to": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Medium,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 1000,
                    p95_latency_ms: 5000,
                    max_latency_ms: 15000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Medium,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "make_call".into(),
                name: "make_call".into(),
                purpose: "Initiate a phone call with a TwiML application URL".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "to": {"type": "string", "description": "Destination phone number (E.164)"},
                    "twiml_url": {"type": "string", "description": "URL returning TwiML instructions"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "sid": {"type": "string"},
                    "status": {"type": "string"},
                    "to": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::High,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 2000,
                    p95_latency_ms: 10000,
                    max_latency_ms: 30000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::High,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "list_messages".into(),
                name: "list_messages".into(),
                purpose: "List recent messages from the account".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "limit": {"type": "integer", "description": "Max number of messages to return"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "messages": {"type": "array"},
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
            expected_latency_ms: 1000,
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
    Box::into_raw(Box::new(TwilioPort::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec() {
        let port = TwilioPort::new();
        assert_eq!(port.spec().port_id, "soma.twilio");
        assert_eq!(port.spec().capabilities.len(), 4);
    }

    #[test]
    fn test_lifecycle_active() {
        let port = TwilioPort::new();
        assert_eq!(port.lifecycle_state(), PortLifecycleState::Active);
    }

    #[test]
    fn test_validate_send_sms_missing_fields() {
        let port = TwilioPort::new();
        assert!(port
            .validate_input("send_sms", &serde_json::json!({}))
            .is_err());
    }

    #[test]
    fn test_validate_send_sms_ok() {
        let port = TwilioPort::new();
        let input = serde_json::json!({"to": "+15551234567", "body": "Hello"});
        assert!(port.validate_input("send_sms", &input).is_ok());
    }

    #[test]
    fn test_validate_make_call_missing_fields() {
        let port = TwilioPort::new();
        assert!(port
            .validate_input("make_call", &serde_json::json!({"to": "+15551234567"}))
            .is_err());
    }

    #[test]
    fn test_unknown_capability() {
        let port = TwilioPort::new();
        assert!(port.invoke("nonexistent", serde_json::json!({})).is_err());
    }

    #[test]
    fn test_invoke_without_config_returns_failure_record() {
        let port = TwilioPort::new();
        let input = serde_json::json!({"to": "+15551234567", "body": "test"});
        let record = port.invoke("send_sms", input).unwrap();
        assert!(!record.success);
    }
}
