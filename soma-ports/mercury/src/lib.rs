use std::sync::OnceLock;
use std::time::Instant;

use soma_port_sdk::prelude::*;
use soma_port_sdk::Result;

const PORT_ID: &str = "mercury";
const API_BASE: &str = "https://api.inceptionlabs.ai/v1";

pub struct MercuryPort {
    spec: PortSpec,
    client: OnceLock<reqwest::blocking::Client>,
}

impl MercuryPort {
    pub fn new() -> Self {
        Self {
            spec: build_spec(),
            client: OnceLock::new(),
        }
    }

    fn api_key(&self) -> Result<String> {
        std::env::var("SOMA_MERCURY_API_KEY")
            .or_else(|_| std::env::var("INCEPTION_API_KEY"))
            .map_err(|_| {
                PortError::DependencyUnavailable(
                    "SOMA_MERCURY_API_KEY or INCEPTION_API_KEY not set".into(),
                )
            })
    }

    fn client(&self) -> &reqwest::blocking::Client {
        self.client.get_or_init(|| {
            reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("failed to build HTTP client")
        })
    }

    fn chat_completion(
        &self,
        messages: &serde_json::Value,
        model: &str,
        temperature: f64,
        max_tokens: u64,
        reasoning_effort: &str,
    ) -> Result<serde_json::Value> {
        let key = self.api_key()?;
        let key = key.as_str();

        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
            "reasoning_effort": reasoning_effort,
        });

        let resp = self
            .client()
            .post(format!("{API_BASE}/chat/completions"))
            .bearer_auth(key)
            .json(&body)
            .send()
            .map_err(|e| PortError::TransportError(format!("request failed: {e}")))?;

        let status = resp.status();
        let text = resp
            .text()
            .map_err(|e| PortError::TransportError(format!("read body failed: {e}")))?;

        if !status.is_success() {
            return Err(PortError::ExternalError(format!(
                "HTTP {status}: {text}"
            )));
        }

        let parsed: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| PortError::ExternalError(format!("invalid JSON: {e}")))?;

        let content = parsed["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let usage = &parsed["usage"];

        Ok(serde_json::json!({
            "content": content,
            "model": parsed["model"],
            "usage": {
                "prompt_tokens": usage["prompt_tokens"],
                "completion_tokens": usage["completion_tokens"],
                "total_tokens": usage["total_tokens"],
            },
            "reasoning_effort": reasoning_effort,
            "finish_reason": parsed["choices"][0]["finish_reason"],
        }))
    }

    fn do_generate(&self, input: &serde_json::Value) -> Result<serde_json::Value> {
        let messages = input
            .get("messages")
            .ok_or_else(|| PortError::Validation("missing 'messages'".into()))?;

        let model = input
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("mercury-2");

        let temperature = input
            .get("temperature")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.75);

        let max_tokens = input
            .get("max_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(8192);

        let reasoning_effort = input
            .get("reasoning_effort")
            .and_then(|v| v.as_str())
            .unwrap_or("medium");

        self.chat_completion(messages, model, temperature, max_tokens, reasoning_effort)
    }

    fn do_reason(&self, input: &serde_json::Value) -> Result<serde_json::Value> {
        let messages = input
            .get("messages")
            .ok_or_else(|| PortError::Validation("missing 'messages'".into()))?;

        let model = input
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("mercury-2");

        let max_tokens = input
            .get("max_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(8192);

        self.chat_completion(messages, model, 0.75, max_tokens, "high")
    }
}

impl Port for MercuryPort {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(&self, capability_id: &str, input: serde_json::Value) -> Result<PortCallRecord> {
        let start = Instant::now();
        let result = match capability_id {
            "generate" => self.do_generate(&input),
            "reason" => self.do_reason(&input),
            _ => Err(PortError::Validation(format!(
                "unknown capability: {capability_id}"
            ))),
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

    fn validate_input(&self, capability_id: &str, input: &serde_json::Value) -> Result<()> {
        match capability_id {
            "generate" | "reason" => {
                if input.get("messages").is_none() {
                    return Err(PortError::Validation("missing 'messages'".into()));
                }
                Ok(())
            }
            _ => Err(PortError::Validation(format!(
                "unknown capability: {capability_id}"
            ))),
        }
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        PortLifecycleState::Active
    }
}

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    Box::into_raw(Box::new(MercuryPort::new()))
}

fn build_spec() -> PortSpec {
    fn cap(
        id: &str,
        purpose: &str,
        latency_expected: u64,
        latency_p95: u64,
    ) -> PortCapabilitySpec {
        PortCapabilitySpec {
            capability_id: id.to_string(),
            name: id.to_string(),
            purpose: purpose.to_string(),
            input_schema: SchemaRef::object(serde_json::json!({
                "messages": {"type": "array", "description": "Chat messages [{role, content}]"},
                "model": {"type": "string", "description": "Model ID (default: mercury-2)"},
                "temperature": {"type": "number", "description": "Sampling temperature (default: 0.75)"},
                "max_tokens": {"type": "integer", "description": "Max output tokens (default: 8192)"},
            })),
            output_schema: SchemaRef::object(serde_json::json!({
                "content": {"type": "string"},
                "model": {"type": "string"},
                "usage": {"type": "object"},
                "reasoning_effort": {"type": "string"},
                "finish_reason": {"type": "string"},
            })),
            effect_class: SideEffectClass::ReadOnly,
            rollback_support: RollbackSupport::Irreversible,
            determinism_class: DeterminismClass::Stochastic,
            idempotence_class: IdempotenceClass::NonIdempotent,
            risk_class: RiskClass::Low,
            latency_profile: LatencyProfile {
                expected_latency_ms: latency_expected,
                p95_latency_ms: latency_p95,
                max_latency_ms: 120_000,
            },
            cost_profile: CostProfile {
                cpu_cost_class: CostClass::Negligible,
                memory_cost_class: CostClass::Negligible,
                io_cost_class: CostClass::Low,
                network_cost_class: CostClass::Medium,
                energy_cost_class: CostClass::Low,
            },
            remote_exposable: false,
            auth_override: None,
        }
    }

    PortSpec {
        port_id: PORT_ID.to_string(),
        name: "Mercury".to_string(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Custom,
        description: "Diffusion LLM brain via Inception Labs Mercury API".to_string(),
        namespace: "soma.ports.mercury".to_string(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![
            cap("generate", "Chat completion with configurable reasoning effort", 500, 3000),
            cap("reason", "Deep reasoning with reasoning_effort=high", 2000, 10000),
        ],
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::any(),
        failure_modes: vec![
            PortFailureClass::ValidationError,
            PortFailureClass::DependencyUnavailable,
            PortFailureClass::TransportError,
            PortFailureClass::ExternalError,
            PortFailureClass::Timeout,
        ],
        side_effect_class: SideEffectClass::ReadOnly,
        latency_profile: LatencyProfile {
            expected_latency_ms: 500,
            p95_latency_ms: 5000,
            max_latency_ms: 120_000,
        },
        cost_profile: CostProfile {
            cpu_cost_class: CostClass::Negligible,
            memory_cost_class: CostClass::Negligible,
            io_cost_class: CostClass::Low,
            network_cost_class: CostClass::Medium,
            energy_cost_class: CostClass::Low,
        },
        auth_requirements: AuthRequirements {
            methods: vec![AuthMethod::ApiKey],
            required: true,
        },
        sandbox_requirements: SandboxRequirements {
            filesystem_access: false,
            network_access: true,
            device_access: false,
            process_access: false,
            memory_limit_mb: None,
            cpu_limit_percent: None,
            time_limit_ms: Some(120_000),
            syscall_limit: None,
        },
        observable_fields: vec![
            "latency_ms".to_string(),
            "model".to_string(),
            "reasoning_effort".to_string(),
            "token_count".to_string(),
        ],
        validation_rules: vec![],
        remote_exposure: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_loads() {
        let port = MercuryPort::new();
        assert_eq!(port.spec().port_id, PORT_ID);
        assert_eq!(port.spec().capabilities.len(), 2);
    }

    #[test]
    fn validate_rejects_missing_messages() {
        let port = MercuryPort::new();
        let err = port.validate_input("generate", &serde_json::json!({}));
        assert!(err.is_err());
    }

    #[test]
    fn validate_accepts_messages() {
        let port = MercuryPort::new();
        let ok = port.validate_input(
            "generate",
            &serde_json::json!({"messages": [{"role": "user", "content": "hi"}]}),
        );
        assert!(ok.is_ok());
    }

    #[test]
    fn unknown_capability_rejected() {
        let port = MercuryPort::new();
        let err = port.validate_input("nonexistent", &serde_json::json!({}));
        assert!(err.is_err());
    }
}
