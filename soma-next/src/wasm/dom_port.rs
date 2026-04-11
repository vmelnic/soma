//! Browser DOM port — in-tab SOMA port that manipulates the document.
//!
//! This is the first "body" port that runs in the browser: it implements
//! the same `Port` trait that the filesystem, HTTP, and ESP32 hardware
//! ports implement, but instead of touching the native filesystem or a
//! GPIO pin, it touches `document.body`. The brain (an LLM, or a learned
//! routine, or today's hardcoded test harness) composes calls to this
//! port the same way it composes calls to any other port.
//!
//! Phase 1a exposes a single capability: `append_heading`, which creates
//! an `<h1>` with the supplied text and appends it to `document.body`.
//! That's enough to prove the entire pipeline — wasm → Port trait →
//! `web_sys` → rendered HTML — actually works end to end.
//!
//! Later phase-1 steps will add more capabilities (create_element,
//! set_text, set_attr, on_event, etc.) and additional output/input ports
//! (audio, voice, keyboard).

use chrono::Utc;
use serde_json::{json, Value};
use uuid::Uuid;
use wasm_bindgen::JsCast;
use web_sys::HtmlElement;

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

/// Browser DOM port. Thin wrapper; all state lives in the browser document.
pub struct DomPort {
    spec: PortSpec,
}

impl Default for DomPort {
    fn default() -> Self {
        Self::new()
    }
}

impl DomPort {
    pub fn new() -> Self {
        Self {
            spec: build_spec(),
        }
    }
}

impl Port for DomPort {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(&self, capability_id: &str, input: Value) -> Result<PortCallRecord> {
        let start = js_sys::Date::now();
        let result = match capability_id {
            "append_heading" => append_heading(&input),
            other => {
                return Err(SomaError::Port(format!(
                    "unknown capability '{other}' on DOM port"
                )));
            }
        };

        let latency_ms = (js_sys::Date::now() - start) as u64;
        let port_id = self.spec.port_id.clone();
        let capability_id = capability_id.to_string();

        match result {
            Ok(structured) => Ok(PortCallRecord {
                observation_id: Uuid::new_v4(),
                port_id,
                capability_id,
                invocation_id: Uuid::new_v4(),
                success: true,
                failure_class: None,
                raw_result: structured.clone(),
                structured_result: structured,
                effect_patch: None,
                side_effect_summary: Some("dom_append".to_string()),
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
                capability_id,
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
            "append_heading" => {
                let text = input
                    .get("text")
                    .ok_or_else(|| SomaError::Port("missing required field 'text'".to_string()))?;
                if !text.is_string() {
                    return Err(SomaError::Port("'text' must be a string".to_string()));
                }
                Ok(())
            }
            other => Err(SomaError::Port(format!(
                "unknown capability '{other}' on DOM port"
            ))),
        }
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        PortLifecycleState::Active
    }
}

// ---------------------------------------------------------------------------
// Capability implementations — touch the DOM via web-sys
// ---------------------------------------------------------------------------

fn append_heading(input: &Value) -> Result<Value> {
    let text = input
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SomaError::Port("missing required field 'text'".to_string()))?;

    let window = web_sys::window()
        .ok_or_else(|| SomaError::Port("no `window` in this JS context".to_string()))?;
    let document = window
        .document()
        .ok_or_else(|| SomaError::Port("no `document` on window".to_string()))?;
    let body = document
        .body()
        .ok_or_else(|| SomaError::Port("no `document.body`".to_string()))?;

    let element = document
        .create_element("h1")
        .map_err(|e| SomaError::Port(format!("create_element failed: {e:?}")))?;
    let element: HtmlElement = element
        .dyn_into()
        .map_err(|_| SomaError::Port("created element is not an HTMLElement".to_string()))?;
    element.set_text_content(Some(text));

    body.append_child(&element)
        .map_err(|e| SomaError::Port(format!("append_child failed: {e:?}")))?;

    Ok(json!({
        "rendered": true,
        "tag": "h1",
        "text": text,
    }))
}

// ---------------------------------------------------------------------------
// PortSpec
// ---------------------------------------------------------------------------

fn build_spec() -> PortSpec {
    let append_heading_cap = PortCapabilitySpec {
        capability_id: "append_heading".to_string(),
        name: "append_heading".to_string(),
        purpose: "Create an <h1> with the given text and append it to document.body."
            .to_string(),
        input_schema: SchemaRef {
            schema: json!({
                "type": "object",
                "required": ["text"],
                "properties": {
                    "text": { "type": "string" }
                }
            }),
        },
        output_schema: SchemaRef {
            schema: json!({
                "type": "object",
                "properties": {
                    "rendered": { "type": "boolean" },
                    "tag": { "type": "string" },
                    "text": { "type": "string" }
                }
            }),
        },
        effect_class: SideEffectClass::LocalStateMutation,
        rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::Deterministic,
        idempotence_class: IdempotenceClass::NonIdempotent,
        risk_class: RiskClass::Low,
        latency_profile: LatencyProfile {
            expected_latency_ms: 1,
            p95_latency_ms: 5,
            max_latency_ms: 50,
        },
        cost_profile: CostProfile {
            cpu_cost_class: CostClass::Negligible,
            memory_cost_class: CostClass::Negligible,
            io_cost_class: CostClass::Negligible,
            network_cost_class: CostClass::Negligible,
            energy_cost_class: CostClass::Negligible,
        },
        remote_exposable: false,
        auth_override: None,
    };

    PortSpec {
        port_id: "dom".to_string(),
        name: "DOM".to_string(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Renderer,
        description: "In-tab browser DOM port for creating and mutating HTML elements."
            .to_string(),
        namespace: "soma.ports.dom".to_string(),
        trust_level: TrustLevel::BuiltIn,
        capabilities: vec![append_heading_cap],
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
        side_effect_class: SideEffectClass::LocalStateMutation,
        latency_profile: LatencyProfile {
            expected_latency_ms: 1,
            p95_latency_ms: 5,
            max_latency_ms: 50,
        },
        cost_profile: CostProfile {
            cpu_cost_class: CostClass::Negligible,
            memory_cost_class: CostClass::Negligible,
            io_cost_class: CostClass::Negligible,
            network_cost_class: CostClass::Negligible,
            energy_cost_class: CostClass::Negligible,
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
