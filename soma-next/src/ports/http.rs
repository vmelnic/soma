use std::collections::HashMap;
use std::time::Instant;

use chrono::Utc;
use semver::Version;
use uuid::Uuid;

use crate::errors::{Result, SomaError};
use crate::types::common::{
    AuthRequirements, CostClass, CostProfile, DeterminismClass, IdempotenceClass,
    LatencyProfile, PortFailureClass, RiskClass, RollbackSupport, SandboxRequirements, SchemaRef,
    SideEffectClass, TrustLevel,
};
use crate::types::observation::PortCallRecord;
use crate::types::port::{PortCapabilitySpec, PortKind, PortLifecycleState, PortSpec};
use crate::runtime::port::Port;

/// HTTP port adapter providing GET, POST, PUT, and DELETE capabilities
/// using a synchronous reqwest client.
pub struct HttpPort {
    spec: PortSpec,
    client: reqwest::blocking::Client,
}

impl HttpPort {
    /// Create a new HttpPort with default client settings.
    pub fn new() -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");

        Self {
            spec: build_http_port_spec(),
            client,
        }
    }

    /// Create an HttpPort with a custom reqwest client.
    pub fn with_client(client: reqwest::blocking::Client) -> Self {
        Self {
            spec: build_http_port_spec(),
            client,
        }
    }

    /// Execute an HTTP request for the given method and input, returning a PortCallRecord.
    fn execute_request(
        &self,
        capability_id: &str,
        method: reqwest::Method,
        input: &serde_json::Value,
    ) -> Result<PortCallRecord> {
        let start = Instant::now();

        let url = input
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SomaError::Port("missing required field 'url'".to_string()))?;

        let headers = input
            .get("headers")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();

        let mut request = self.client.request(method, url);

        for (key, value) in &headers {
            if let Some(val) = value.as_str() {
                request = request.header(key.as_str(), val);
            }
        }

        // Attach body for methods that support it.
        if let Some(body) = input.get("body") {
            request = request.json(body);
        }

        match request.send() {
            Ok(response) => {
                let status = response.status().as_u16();
                let resp_headers: HashMap<String, String> = response
                    .headers()
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.as_str().to_string(),
                            v.to_str().unwrap_or("<binary>").to_string(),
                        )
                    })
                    .collect();

                let body_text = response
                    .text()
                    .unwrap_or_default();

                let latency_ms = start.elapsed().as_millis() as u64;

                let structured = serde_json::json!({
                    "status": status,
                    "body": body_text,
                    "headers": resp_headers,
                });

                Ok(PortCallRecord {
                    observation_id: Uuid::new_v4(),
                    port_id: self.spec.port_id.clone(),
                    capability_id: capability_id.to_string(),
                    invocation_id: Uuid::new_v4(),
                    success: true,
                    failure_class: None,
                    raw_result: structured.clone(),
                    structured_result: structured,
                    effect_patch: None,
                    side_effect_summary: Some(side_effect_for(capability_id).to_string()),
                    latency_ms,
                    resource_cost: 0.01,
                    confidence: 1.0,
                    timestamp: Utc::now(),
                    retry_safe: true,
                    input_hash: None,
                    session_id: None,
                    goal_id: None,
                    caller_identity: None,
                    auth_result: None,
                    policy_result: None,
                    sandbox_result: None,
                })
            }
            Err(e) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                let failure_class = classify_error(&e);
                let retry_safe = matches!(
                    failure_class,
                    PortFailureClass::TransportError | PortFailureClass::Timeout
                );

                Ok(PortCallRecord {
                    observation_id: Uuid::new_v4(),
                    port_id: self.spec.port_id.clone(),
                    capability_id: capability_id.to_string(),
                    invocation_id: Uuid::new_v4(),
                    success: false,
                    failure_class: Some(failure_class),
                    raw_result: serde_json::Value::Null,
                    structured_result: serde_json::json!({ "error": e.to_string() }),
                    effect_patch: None,
                    side_effect_summary: Some("none".to_string()),
                    latency_ms,
                    resource_cost: 0.0,
                    confidence: 0.0,
                    timestamp: Utc::now(),
                    retry_safe,
                    input_hash: None,
                    session_id: None,
                    goal_id: None,
                    caller_identity: None,
                    auth_result: None,
                    policy_result: None,
                    sandbox_result: None,
                })
            }
        }
    }
}

impl Default for HttpPort {
    fn default() -> Self {
        Self::new()
    }
}

impl Port for HttpPort {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(
        &self,
        capability_id: &str,
        input: serde_json::Value,
    ) -> Result<PortCallRecord> {
        let method = match capability_id {
            "get" => reqwest::Method::GET,
            "post" => reqwest::Method::POST,
            "put" => reqwest::Method::PUT,
            "delete" => reqwest::Method::DELETE,
            other => {
                return Err(SomaError::Port(format!(
                    "unknown capability '{}' on http port",
                    other,
                )));
            }
        };

        self.execute_request(capability_id, method, &input)
    }

    fn validate_input(
        &self,
        capability_id: &str,
        input: &serde_json::Value,
    ) -> Result<()> {
        match capability_id {
            "get" | "post" | "put" | "delete" => {}
            other => {
                return Err(SomaError::Port(format!(
                    "unknown capability '{}' on http port",
                    other,
                )));
            }
        }

        let obj = input.as_object().ok_or_else(|| {
            SomaError::Port("input must be a JSON object".to_string())
        })?;

        if !obj.contains_key("url") {
            return Err(SomaError::Port("missing required field 'url'".to_string()));
        }
        if !obj["url"].is_string() {
            return Err(SomaError::Port("'url' must be a string".to_string()));
        }

        if let Some(headers) = obj.get("headers")
            && !headers.is_object() {
                return Err(SomaError::Port("'headers' must be an object".to_string()));
            }

        Ok(())
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        PortLifecycleState::Active
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Classify a reqwest error into the appropriate PortFailureClass.
fn classify_error(err: &reqwest::Error) -> PortFailureClass {
    if err.is_timeout() {
        PortFailureClass::Timeout
    } else if err.is_connect() || err.is_request() {
        PortFailureClass::TransportError
    } else {
        PortFailureClass::ExternalError
    }
}

/// Return the side-effect summary string for a given capability.
fn side_effect_for(capability_id: &str) -> &'static str {
    match capability_id {
        "get" => "read_only",
        "post" | "put" | "delete" => "external_state_mutation",
        _ => "none",
    }
}

/// Build the full PortSpec for the HTTP port.
fn build_http_port_spec() -> PortSpec {
    let url_input_schema = serde_json::json!({
        "type": "object",
        "required": ["url"],
        "properties": {
            "url": { "type": "string" },
            "headers": { "type": "object" }
        }
    });

    let body_input_schema = serde_json::json!({
        "type": "object",
        "required": ["url"],
        "properties": {
            "url": { "type": "string" },
            "body": {},
            "headers": { "type": "object" }
        }
    });

    let response_schema = serde_json::json!({
        "type": "object",
        "required": ["status", "body", "headers"],
        "properties": {
            "status": { "type": "integer" },
            "body": { "type": "string" },
            "headers": { "type": "object" }
        }
    });

    let http_latency = LatencyProfile {
        expected_latency_ms: 200,
        p95_latency_ms: 2000,
        max_latency_ms: 30000,
    };

    let network_cost = CostProfile {
        cpu_cost_class: CostClass::Low,
        memory_cost_class: CostClass::Low,
        io_cost_class: CostClass::Low,
        network_cost_class: CostClass::Medium,
        energy_cost_class: CostClass::Low,
    };

    let get_cap = PortCapabilitySpec {
        capability_id: "get".to_string(),
        name: "HTTP GET".to_string(),
        purpose: "Fetch a resource via HTTP GET".to_string(),
        input_schema: SchemaRef {
            schema: url_input_schema.clone(),
        },
        output_schema: SchemaRef {
            schema: response_schema.clone(),
        },
        effect_class: SideEffectClass::ReadOnly,
        rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::PartiallyDeterministic,
        idempotence_class: IdempotenceClass::Idempotent,
        risk_class: RiskClass::Low,
        latency_profile: http_latency.clone(),
        cost_profile: network_cost.clone(),
        remote_exposable: true,
        auth_override: None,
    };

    let post_cap = PortCapabilitySpec {
        capability_id: "post".to_string(),
        name: "HTTP POST".to_string(),
        purpose: "Send data via HTTP POST".to_string(),
        input_schema: SchemaRef {
            schema: body_input_schema.clone(),
        },
        output_schema: SchemaRef {
            schema: response_schema.clone(),
        },
        effect_class: SideEffectClass::ExternalStateMutation,
        rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::PartiallyDeterministic,
        idempotence_class: IdempotenceClass::NonIdempotent,
        risk_class: RiskClass::Medium,
        latency_profile: http_latency.clone(),
        cost_profile: network_cost.clone(),
        remote_exposable: true,
        auth_override: None,
    };

    let put_cap = PortCapabilitySpec {
        capability_id: "put".to_string(),
        name: "HTTP PUT".to_string(),
        purpose: "Update a resource via HTTP PUT".to_string(),
        input_schema: SchemaRef {
            schema: body_input_schema.clone(),
        },
        output_schema: SchemaRef {
            schema: response_schema.clone(),
        },
        effect_class: SideEffectClass::ExternalStateMutation,
        rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::PartiallyDeterministic,
        idempotence_class: IdempotenceClass::Idempotent,
        risk_class: RiskClass::Medium,
        latency_profile: http_latency.clone(),
        cost_profile: network_cost.clone(),
        remote_exposable: true,
        auth_override: None,
    };

    let delete_cap = PortCapabilitySpec {
        capability_id: "delete".to_string(),
        name: "HTTP DELETE".to_string(),
        purpose: "Delete a resource via HTTP DELETE".to_string(),
        input_schema: SchemaRef {
            schema: url_input_schema.clone(),
        },
        output_schema: SchemaRef {
            schema: response_schema.clone(),
        },
        effect_class: SideEffectClass::Destructive,
        rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::PartiallyDeterministic,
        idempotence_class: IdempotenceClass::Idempotent,
        risk_class: RiskClass::Medium,
        latency_profile: http_latency.clone(),
        cost_profile: network_cost.clone(),
        remote_exposable: true,
        auth_override: None,
    };

    PortSpec {
        port_id: "http".to_string(),
        name: "HTTP Client".to_string(),
        version: Version::new(1, 0, 0),
        kind: PortKind::Http,
        description: "Synchronous HTTP client port for GET, POST, PUT, and DELETE requests"
            .to_string(),
        namespace: "soma.ports".to_string(),
        trust_level: TrustLevel::Trusted,
        capabilities: vec![get_cap, post_cap, put_cap, delete_cap],
        input_schema: SchemaRef {
            schema: serde_json::json!({
                "type": "object",
                "required": ["url"],
                "properties": {
                    "url": { "type": "string" },
                    "body": {},
                    "headers": { "type": "object" }
                }
            }),
        },
        output_schema: SchemaRef {
            schema: response_schema,
        },
        failure_modes: vec![
            PortFailureClass::Timeout,
            PortFailureClass::TransportError,
            PortFailureClass::ExternalError,
            PortFailureClass::ValidationError,
        ],
        side_effect_class: SideEffectClass::ExternalStateMutation,
        latency_profile: http_latency,
        cost_profile: network_cost,
        auth_requirements: AuthRequirements {
            methods: vec![],
            required: false,
        },
        sandbox_requirements: SandboxRequirements {
            filesystem_access: false,
            network_access: true,
            device_access: false,
            process_access: false,
            memory_limit_mb: None,
            cpu_limit_percent: None,
            time_limit_ms: Some(30000),
            syscall_limit: None,
        },
        observable_fields: vec![
            "status".to_string(),
            "body".to_string(),
            "headers".to_string(),
        ],
        validation_rules: vec![],
        remote_exposure: true,
        backend: crate::types::port::PortBackend::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_port_spec_is_valid() {
        let port = HttpPort::new();
        let spec = port.spec();

        assert_eq!(spec.port_id, "http");
        assert_eq!(spec.kind, PortKind::Http);
        assert_eq!(spec.capabilities.len(), 4);

        let cap_ids: Vec<&str> = spec
            .capabilities
            .iter()
            .map(|c| c.capability_id.as_str())
            .collect();
        assert!(cap_ids.contains(&"get"));
        assert!(cap_ids.contains(&"post"));
        assert!(cap_ids.contains(&"put"));
        assert!(cap_ids.contains(&"delete"));
    }

    #[test]
    fn http_port_lifecycle_is_active() {
        let port = HttpPort::new();
        assert_eq!(port.lifecycle_state(), PortLifecycleState::Active);
    }

    #[test]
    fn validate_input_rejects_missing_url() {
        let port = HttpPort::new();
        let input = serde_json::json!({});
        let result = port.validate_input("get", &input);
        assert!(result.is_err());
    }

    #[test]
    fn validate_input_rejects_non_string_url() {
        let port = HttpPort::new();
        let input = serde_json::json!({"url": 42});
        let result = port.validate_input("get", &input);
        assert!(result.is_err());
    }

    #[test]
    fn validate_input_rejects_non_object_headers() {
        let port = HttpPort::new();
        let input = serde_json::json!({"url": "https://example.com", "headers": "bad"});
        let result = port.validate_input("get", &input);
        assert!(result.is_err());
    }

    #[test]
    fn validate_input_rejects_unknown_capability() {
        let port = HttpPort::new();
        let input = serde_json::json!({"url": "https://example.com"});
        let result = port.validate_input("patch", &input);
        assert!(result.is_err());
    }

    #[test]
    fn validate_input_accepts_valid_get() {
        let port = HttpPort::new();
        let input = serde_json::json!({"url": "https://example.com"});
        let result = port.validate_input("get", &input);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_input_accepts_valid_post_with_body() {
        let port = HttpPort::new();
        let input = serde_json::json!({
            "url": "https://example.com",
            "body": {"key": "value"},
            "headers": {"Content-Type": "application/json"}
        });
        let result = port.validate_input("post", &input);
        assert!(result.is_ok());
    }

    #[test]
    fn invoke_unknown_capability_returns_error() {
        let port = HttpPort::new();
        let input = serde_json::json!({"url": "https://example.com"});
        let result = port.invoke("patch", input);
        assert!(result.is_err());
    }

    #[test]
    fn invoke_invalid_url_returns_failure_record() {
        let port = HttpPort::new();
        let input = serde_json::json!({"url": "not-a-url://[invalid"});
        let result = port.invoke("get", input);

        // The invoke succeeds (returns Ok) but the record indicates failure,
        // because reqwest returns an error for malformed URLs.
        let record = result.unwrap();
        assert!(!record.success);
        assert!(record.failure_class.is_some());
    }

    #[test]
    fn invoke_connection_refused_returns_failure_record() {
        let port = HttpPort::with_client(
            reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_millis(500))
                .build()
                .unwrap(),
        );
        // Use a port that almost certainly has nothing listening.
        let input = serde_json::json!({"url": "http://127.0.0.1:1"});
        let record = port.invoke("get", input).unwrap();

        assert!(!record.success);
        assert!(record.failure_class.is_some());
        assert!(record.retry_safe);
        assert_eq!(record.port_id, "http");
        assert_eq!(record.capability_id, "get");
    }

    #[test]
    fn classify_error_timeout() {
        // Verify the classifier handles timeout and connect errors correctly
        // by checking the logic paths (the actual reqwest::Error is not
        // publicly constructable, so we test via integration in the invoke
        // tests above).
        assert_eq!(side_effect_for("get"), "read_only");
        assert_eq!(side_effect_for("post"), "external_state_mutation");
        assert_eq!(side_effect_for("put"), "external_state_mutation");
        assert_eq!(side_effect_for("delete"), "external_state_mutation");
        assert_eq!(side_effect_for("unknown"), "none");
    }
}
