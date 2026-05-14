use std::sync::OnceLock;
use std::time::Instant;

use soma_port_sdk::prelude::*;
use soma_port_sdk::Result;

const PORT_ID: &str = "brain";

struct BrainConfig {
    api_url: String,
    api_key: String,
    model: String,
}

pub struct BrainPort {
    spec: PortSpec,
    client: OnceLock<reqwest::blocking::Client>,
}

impl BrainPort {
    pub fn new() -> Self {
        Self {
            spec: build_spec(),
            client: OnceLock::new(),
        }
    }

    fn config(&self) -> Result<BrainConfig> {
        let provider = std::env::var("SOMA_BRAIN_PROVIDER").unwrap_or_else(|_| "openai".into());

        let (default_url, key_envs, default_model) = match provider.as_str() {
            "kimi" => (
                "https://api.moonshot.ai/v1/chat/completions",
                &["SOMA_BRAIN_API_KEY", "SOMA_KIMI_API_KEY", "KIMI_API_KEY"][..],
                "moonshot-v1-auto",
            ),
            "glm" => (
                "https://api.z.ai/api/paas/v4/chat/completions",
                &["SOMA_BRAIN_API_KEY", "SOMA_GLM_API_KEY", "GLM_API_KEY"][..],
                "glm-5.1",
            ),
            _ => (
                "https://api.openai.com/v1/chat/completions",
                &["SOMA_BRAIN_API_KEY", "OPENAI_API_KEY"][..],
                "gpt-4o-mini",
            ),
        };

        let api_url = std::env::var("SOMA_BRAIN_API_URL").unwrap_or_else(|_| default_url.into());
        let model = std::env::var("SOMA_BRAIN_MODEL").unwrap_or_else(|_| default_model.into());

        let api_key = key_envs.iter()
            .find_map(|k| std::env::var(k).ok())
            .ok_or_else(|| PortError::DependencyUnavailable(
                format!("no API key found (tried: {})", key_envs.join(", "))
            ))?;

        Ok(BrainConfig { api_url, api_key, model })
    }

    fn client(&self) -> &reqwest::blocking::Client {
        self.client.get_or_init(|| {
            reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("failed to build HTTP client")
        })
    }

    fn do_reason(&self, input: &serde_json::Value) -> Result<serde_json::Value> {
        let query = input.get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PortError::Validation("missing 'query'".into()))?;

        let cfg = self.config()?;

        let system_prompt = "You are a skill selector for an autonomous runtime. \
            You receive a goal, belief state, and candidate skills. \
            Pick the single best skill to execute next. \
            Respond with ONLY valid JSON: \
            {\"skill_recommendations\":[{\"skill_id\":\"<chosen>\",\"score\":<0.0-1.0>}],\"confidence\":<0.0-1.0>}";

        let body = serde_json::json!({
            "model": cfg.model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": query},
            ],
            "temperature": 0.1,
            "max_tokens": 256,
        });

        let resp = self.client()
            .post(&cfg.api_url)
            .bearer_auth(&cfg.api_key)
            .json(&body)
            .send()
            .map_err(|e| PortError::TransportError(format!("request failed: {e}")))?;

        let status = resp.status();
        let text = resp.text()
            .map_err(|e| PortError::TransportError(format!("read body failed: {e}")))?;

        if !status.is_success() {
            return Err(PortError::ExternalError(format!("HTTP {status}: {}", &text[..text.len().min(200)])));
        }

        let parsed: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| PortError::ExternalError(format!("invalid JSON: {e}")))?;

        let content = parsed["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim();

        // Extract JSON from content (may be wrapped in ```json blocks)
        let json_str = if let Some(start) = content.find('{') {
            if let Some(end) = content.rfind('}') {
                &content[start..=end]
            } else {
                content
            }
        } else {
            content
        };

        match serde_json::from_str::<serde_json::Value>(json_str) {
            Ok(structured) => Ok(structured),
            Err(_) => {
                // LLM didn't return valid JSON — wrap the raw text
                Ok(serde_json::json!({
                    "skill_recommendations": [],
                    "confidence": 0.0,
                    "raw_response": content,
                }))
            }
        }
    }
}

impl Port for BrainPort {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(&self, capability_id: &str, input: serde_json::Value) -> Result<PortCallRecord> {
        let start = Instant::now();
        let result = match capability_id {
            "reason" => self.do_reason(&input),
            _ => Err(PortError::Validation(format!("unknown capability: {capability_id}"))),
        };
        let latency_ms = start.elapsed().as_millis() as u64;
        match result {
            Ok(value) => Ok(PortCallRecord::success(PORT_ID, capability_id, value, latency_ms)),
            Err(e) => Ok(PortCallRecord::failure(PORT_ID, capability_id, e.failure_class(), &e.to_string(), latency_ms)),
        }
    }

    fn validate_input(&self, capability_id: &str, input: &serde_json::Value) -> Result<()> {
        match capability_id {
            "reason" => {
                if input.get("query").is_none() {
                    return Err(PortError::Validation("missing 'query'".into()));
                }
                Ok(())
            }
            _ => Err(PortError::Validation(format!("unknown capability: {capability_id}"))),
        }
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        PortLifecycleState::Active
    }
}

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    Box::into_raw(Box::new(BrainPort::new()))
}

fn build_spec() -> PortSpec {
    PortSpec {
        port_id: PORT_ID.to_string(),
        name: "Brain".to_string(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Custom,
        description: "LLM brain port for skill selection and reasoning. Wraps OpenAI/Kimi/GLM APIs.".to_string(),
        namespace: "soma.ports.brain".to_string(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![PortCapabilitySpec {
            capability_id: "reason".to_string(),
            name: "reason".to_string(),
            purpose: "Select the best skill given goal, belief, and candidates".to_string(),
            input_schema: SchemaRef::object(serde_json::json!({
                "query": {"type": "string", "description": "Goal + belief + candidates prompt"},
                "top_k_sources": {"type": "integer", "description": "Unused, reserved"},
            })),
            output_schema: SchemaRef::object(serde_json::json!({
                "skill_recommendations": {"type": "array"},
                "confidence": {"type": "number"},
            })),
            effect_class: SideEffectClass::ReadOnly,
            rollback_support: RollbackSupport::Irreversible,
            determinism_class: DeterminismClass::Stochastic,
            idempotence_class: IdempotenceClass::NonIdempotent,
            risk_class: RiskClass::Low,
            latency_profile: LatencyProfile {
                expected_latency_ms: 1000,
                p95_latency_ms: 5000,
                max_latency_ms: 30_000,
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
            max_latency_ms: 30_000,
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
            time_limit_ms: Some(30_000),
            syscall_limit: None,
        },
        observable_fields: vec!["latency_ms".to_string(), "model".to_string()],
        validation_rules: vec![],
        remote_exposure: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_loads() {
        let port = BrainPort::new();
        assert_eq!(port.spec().port_id, "brain");
        assert_eq!(port.spec().capabilities.len(), 1);
        assert_eq!(port.spec().capabilities[0].capability_id, "reason");
    }

    #[test]
    fn validate_rejects_missing_query() {
        let port = BrainPort::new();
        assert!(port.validate_input("reason", &serde_json::json!({})).is_err());
    }

    #[test]
    fn validate_accepts_query() {
        let port = BrainPort::new();
        assert!(port.validate_input("reason", &serde_json::json!({"query": "test"})).is_ok());
    }
}
