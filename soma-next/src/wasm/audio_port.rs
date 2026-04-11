//! Browser audio port — in-tab SOMA port that speaks text aloud.
//!
//! Uses the Web Speech API (`window.speechSynthesis` +
//! `SpeechSynthesisUtterance`) to produce audible output. The port is the
//! "speaker" in the brain/body split: the brain composes what to say, the
//! body renders it as sound without any idea what the content means.
//!
//! Phase 1b capability: `say_text { text }`. Optional future capabilities
//! (set_voice, set_rate, set_pitch, stop) stay out of scope until there's
//! a real need for them. Web Speech API returns immediately — the audio
//! plays asynchronously — so `latency_ms` reports only the time to
//! dispatch the utterance to the speech queue, not the time to finish
//! speaking.

use chrono::Utc;
use serde_json::{json, Value};
use uuid::Uuid;
use web_sys::SpeechSynthesisUtterance;

use crate::errors::{Result, SomaError};
use crate::runtime::port::Port;
use crate::types::common::{
    AuthRequirements, CostClass, CostProfile, DeterminismClass, IdempotenceClass, LatencyProfile,
    PortFailureClass, RiskClass, RollbackSupport, SandboxRequirements, SchemaRef, SideEffectClass,
    TrustLevel,
};
use crate::types::observation::PortCallRecord;
use crate::types::port::{
    PortBackend, PortCapabilitySpec, PortKind, PortLifecycleState, PortSpec,
};

pub struct AudioPort {
    spec: PortSpec,
}

impl Default for AudioPort {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioPort {
    pub fn new() -> Self {
        Self {
            spec: build_spec(),
        }
    }
}

impl Port for AudioPort {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(&self, capability_id: &str, input: Value) -> Result<PortCallRecord> {
        let start = js_sys::Date::now();
        let result = match capability_id {
            "say_text" => say_text(&input),
            other => {
                return Err(SomaError::Port(format!(
                    "unknown capability '{other}' on audio port"
                )));
            }
        };

        let latency_ms = (js_sys::Date::now() - start) as u64;
        let port_id = self.spec.port_id.clone();
        let cap_str = capability_id.to_string();

        match result {
            Ok(structured) => Ok(PortCallRecord {
                observation_id: Uuid::new_v4(),
                port_id,
                capability_id: cap_str,
                invocation_id: Uuid::new_v4(),
                success: true,
                failure_class: None,
                raw_result: structured.clone(),
                structured_result: structured,
                effect_patch: None,
                side_effect_summary: Some("audio_speak".to_string()),
                latency_ms,
                resource_cost: 0.0001,
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
            }),
            Err(e) => Ok(PortCallRecord {
                observation_id: Uuid::new_v4(),
                port_id,
                capability_id: cap_str,
                invocation_id: Uuid::new_v4(),
                success: false,
                failure_class: Some(PortFailureClass::ExternalError),
                raw_result: Value::Null,
                structured_result: json!({ "error": e.to_string() }),
                effect_patch: None,
                side_effect_summary: Some("none".to_string()),
                latency_ms,
                resource_cost: 0.0,
                confidence: 0.0,
                timestamp: Utc::now(),
                retry_safe: false,
                input_hash: None,
                session_id: None,
                goal_id: None,
                caller_identity: None,
                auth_result: None,
                policy_result: None,
                sandbox_result: None,
            }),
        }
    }

    fn validate_input(&self, capability_id: &str, input: &Value) -> Result<()> {
        match capability_id {
            "say_text" => {
                let text = input.get("text").ok_or_else(|| {
                    SomaError::Port("missing required field 'text'".to_string())
                })?;
                if !text.is_string() {
                    return Err(SomaError::Port("'text' must be a string".to_string()));
                }
                Ok(())
            }
            other => Err(SomaError::Port(format!(
                "unknown capability '{other}' on audio port"
            ))),
        }
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        PortLifecycleState::Active
    }
}

// ---------------------------------------------------------------------------
// Capability implementation
// ---------------------------------------------------------------------------

fn say_text(input: &Value) -> Result<Value> {
    let text = input
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SomaError::Port("missing required field 'text'".to_string()))?;

    let window = web_sys::window()
        .ok_or_else(|| SomaError::Port("no `window` in this JS context".to_string()))?;

    // `speech_synthesis()` returns Result<SpeechSynthesis, JsValue>. Browsers
    // that don't support the Web Speech API (which is rare in 2026 but
    // possible in locked-down WebViews) fail closed with a clear message.
    let synth = window
        .speech_synthesis()
        .map_err(|e| SomaError::Port(format!("speechSynthesis unavailable: {e:?}")))?;

    let utterance = SpeechSynthesisUtterance::new_with_text(text)
        .map_err(|e| SomaError::Port(format!("SpeechSynthesisUtterance::new failed: {e:?}")))?;

    synth.speak(&utterance);

    Ok(json!({
        "spoken": text,
        "queue_pending": synth.pending(),
    }))
}

// ---------------------------------------------------------------------------
// PortSpec
// ---------------------------------------------------------------------------

fn build_spec() -> PortSpec {
    let say_text_cap = PortCapabilitySpec {
        capability_id: "say_text".to_string(),
        name: "say_text".to_string(),
        purpose: "Speak the given text aloud via the Web Speech API.".to_string(),
        input_schema: SchemaRef {
            schema: json!({
                "type": "object",
                "required": ["text"],
                "properties": { "text": { "type": "string" } }
            }),
        },
        output_schema: SchemaRef {
            schema: json!({
                "type": "object",
                "properties": {
                    "spoken": { "type": "string" },
                    "queue_pending": { "type": "boolean" }
                }
            }),
        },
        effect_class: SideEffectClass::ExternalStateMutation,
        rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::Deterministic,
        idempotence_class: IdempotenceClass::NonIdempotent,
        risk_class: RiskClass::Low,
        latency_profile: LatencyProfile {
            expected_latency_ms: 1,
            p95_latency_ms: 10,
            max_latency_ms: 200,
        },
        cost_profile: CostProfile {
            cpu_cost_class: CostClass::Negligible,
            memory_cost_class: CostClass::Negligible,
            io_cost_class: CostClass::Negligible,
            network_cost_class: CostClass::Negligible,
            energy_cost_class: CostClass::Low,
        },
        remote_exposable: false,
        auth_override: None,
    };

    PortSpec {
        port_id: "audio".to_string(),
        name: "Audio".to_string(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Actuator,
        description: "In-tab browser audio port using the Web Speech API.".to_string(),
        namespace: "soma.ports.audio".to_string(),
        trust_level: TrustLevel::BuiltIn,
        capabilities: vec![say_text_cap],
        input_schema: SchemaRef {
            schema: json!({ "type": "object" }),
        },
        output_schema: SchemaRef {
            schema: json!({ "description": "any" }),
        },
        failure_modes: vec![
            PortFailureClass::ExternalError,
            PortFailureClass::ValidationError,
        ],
        side_effect_class: SideEffectClass::ExternalStateMutation,
        latency_profile: LatencyProfile {
            expected_latency_ms: 1,
            p95_latency_ms: 10,
            max_latency_ms: 200,
        },
        cost_profile: CostProfile {
            cpu_cost_class: CostClass::Negligible,
            memory_cost_class: CostClass::Negligible,
            io_cost_class: CostClass::Negligible,
            network_cost_class: CostClass::Negligible,
            energy_cost_class: CostClass::Low,
        },
        auth_requirements: AuthRequirements {
            methods: vec![],
            required: false,
        },
        sandbox_requirements: SandboxRequirements {
            filesystem_access: false,
            network_access: false,
            device_access: false,
            process_access: false,
            memory_limit_mb: None,
            cpu_limit_percent: None,
            time_limit_ms: None,
            syscall_limit: None,
        },
        observable_fields: vec![],
        validation_rules: vec![],
        remote_exposure: false,
        backend: PortBackend::default(),
    }
}
