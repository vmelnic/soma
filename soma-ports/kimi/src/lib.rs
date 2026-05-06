use std::sync::OnceLock;
use std::time::Instant;

use soma_port_sdk::prelude::*;
use soma_port_sdk::Result;

const PORT_ID: &str = "kimi";
const API_BASE: &str = "https://api.moonshot.ai/v1";
const DEFAULT_MODEL: &str = "moonshot-v1-auto";

pub struct KimiPort {
    spec: PortSpec,
    client: OnceLock<reqwest::blocking::Client>,
}

impl KimiPort {
    pub fn new() -> Self {
        Self {
            spec: build_spec(),
            client: OnceLock::new(),
        }
    }

    fn api_key(&self) -> Result<String> {
        std::env::var("SOMA_KIMI_API_KEY")
            .or_else(|_| std::env::var("KIMI_API_KEY"))
            .map_err(|_| {
                PortError::DependencyUnavailable(
                    "SOMA_KIMI_API_KEY or KIMI_API_KEY not set".into(),
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
    ) -> Result<serde_json::Value> {
        let key = self.api_key()?;

        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
        });

        let resp = self
            .client()
            .post(format!("{API_BASE}/chat/completions"))
            .bearer_auth(&key)
            .json(&body)
            .send()
            .map_err(|e| PortError::TransportError(format!("request failed: {e}")))?;

        let status = resp.status();
        let text = resp
            .text()
            .map_err(|e| PortError::TransportError(format!("read body failed: {e}")))?;

        if !status.is_success() {
            return Err(PortError::ExternalError(format!("HTTP {status}: {text}")));
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
            .unwrap_or(DEFAULT_MODEL);

        let temperature = input
            .get("temperature")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.7);

        let max_tokens = input
            .get("max_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(8192);

        self.chat_completion(messages, model, temperature, max_tokens)
    }
}

impl Port for KimiPort {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(&self, capability_id: &str, input: serde_json::Value) -> Result<PortCallRecord> {
        let start = Instant::now();
        let result = match capability_id {
            "generate" => self.do_generate(&input),
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
            "generate" => {
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
    Box::into_raw(Box::new(KimiPort::new()))
}

fn build_spec() -> PortSpec {
    PortSpec {
        port_id: PORT_ID.to_string(),
        name: "Kimi".to_string(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Custom,
        description: "Kimi autoregressive LLM brain via Moonshot AI API".to_string(),
        namespace: "soma.ports.kimi".to_string(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![PortCapabilitySpec {
            capability_id: "generate".to_string(),
            name: "generate".to_string(),
            purpose: "Chat completion via Moonshot Kimi".to_string(),
            input_schema: SchemaRef::object(serde_json::json!({
                "messages": {"type": "array"},
                "model": {"type": "string"},
                "temperature": {"type": "number"},
                "max_tokens": {"type": "integer"},
            })),
            output_schema: SchemaRef::object(serde_json::json!({
                "content": {"type": "string"},
                "model": {"type": "string"},
                "usage": {"type": "object"},
                "finish_reason": {"type": "string"},
            })),
            effect_class: SideEffectClass::ReadOnly,
            rollback_support: RollbackSupport::Irreversible,
            determinism_class: DeterminismClass::Stochastic,
            idempotence_class: IdempotenceClass::NonIdempotent,
            risk_class: RiskClass::Low,
            latency_profile: LatencyProfile {
                expected_latency_ms: 1000,
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
            remote_exposable: false,
            auth_override: None,
        }],
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
            expected_latency_ms: 1000,
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
        let port = KimiPort::new();
        assert_eq!(port.spec().port_id, PORT_ID);
        assert_eq!(port.spec().capabilities.len(), 1);
    }

    #[test]
    fn validate_rejects_missing_messages() {
        let port = KimiPort::new();
        assert!(port.validate_input("generate", &serde_json::json!({})).is_err());
    }

    #[test]
    fn validate_accepts_messages() {
        let port = KimiPort::new();
        assert!(port
            .validate_input("generate", &serde_json::json!({"messages": []}))
            .is_ok());
    }
}
