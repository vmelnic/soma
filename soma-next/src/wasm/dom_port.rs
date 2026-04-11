//! Browser DOM port — in-tab SOMA port that manipulates the document.
//!
//! This is the first "body" port that runs in the browser: it implements
//! the same `Port` trait that the filesystem, HTTP, and ESP32 hardware
//! ports implement, but instead of touching the native filesystem or a
//! GPIO pin, it touches `document.body`. The brain (an LLM, a learned
//! routine, or today's JS harness) composes calls to this port the same
//! way it composes calls to any other port.
//!
//! Capabilities (phase 1b):
//!
//! | capability_id     | effect                                                  |
//! |-------------------|---------------------------------------------------------|
//! | `append_heading`  | Create `<h{level}>{text}</h{level}>`, append to body    |
//! | `append_paragraph`| Create `<p>{text}</p>`, append to body                  |
//! | `set_title`       | Set `document.title`                                    |
//! | `clear_soma`      | Remove every element SOMA rendered this session         |
//!
//! Every soma-rendered element gets `data-soma="true"` on it. `clear_soma`
//! queries `[data-soma="true"]` and removes each hit from its parent, so
//! the rest of the page (the HTML shell, dev-tool panels, etc.) stays
//! untouched. This is the minimum viable "SOMA owns this region" pattern
//! — phase 1c will let the pack manifest declare a named container that
//! SOMA exclusively manages.

use chrono::Utc;
use serde_json::{json, Value};
use uuid::Uuid;
use wasm_bindgen::JsCast;
use web_sys::{Document, HtmlElement};

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

/// Attribute marker used to tag every element SOMA has rendered. Any element
/// with this attribute set is considered owned by the runtime and can be
/// reaped by `clear_soma`; elements without it are never touched.
const DATA_SOMA: &str = "data-soma";

/// Browser DOM port. Thin wrapper — all state lives in the browser document.
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
            "append_paragraph" => append_paragraph(&input),
            "set_title" => set_title(&input),
            "clear_soma" => clear_soma(),
            other => {
                return Err(SomaError::Port(format!(
                    "unknown capability '{other}' on DOM port"
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
                side_effect_summary: Some(side_effect_for(capability_id).to_string()),
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
            "append_heading" => {
                require_string(input, "text")?;
                if let Some(level) = input.get("level") {
                    let n = level
                        .as_u64()
                        .ok_or_else(|| SomaError::Port("'level' must be an integer".to_string()))?;
                    if !(1..=6).contains(&n) {
                        return Err(SomaError::Port(
                            "'level' must be between 1 and 6".to_string(),
                        ));
                    }
                }
                Ok(())
            }
            "append_paragraph" => {
                require_string(input, "text")?;
                Ok(())
            }
            "set_title" => {
                require_string(input, "text")?;
                Ok(())
            }
            "clear_soma" => Ok(()),
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
// Helpers
// ---------------------------------------------------------------------------

fn require_string(input: &Value, field: &str) -> Result<()> {
    let value = input
        .get(field)
        .ok_or_else(|| SomaError::Port(format!("missing required field '{field}'")))?;
    if !value.is_string() {
        return Err(SomaError::Port(format!("'{field}' must be a string")));
    }
    Ok(())
}

fn side_effect_for(capability_id: &str) -> &'static str {
    match capability_id {
        "append_heading" | "append_paragraph" => "dom_append",
        "set_title" => "dom_title",
        "clear_soma" => "dom_clear",
        _ => "dom_unknown",
    }
}

fn document() -> Result<Document> {
    let window = web_sys::window()
        .ok_or_else(|| SomaError::Port("no `window` in this JS context".to_string()))?;
    window
        .document()
        .ok_or_else(|| SomaError::Port("no `document` on window".to_string()))
}

fn body(doc: &Document) -> Result<HtmlElement> {
    doc.body()
        .ok_or_else(|| SomaError::Port("no `document.body`".to_string()))
}

/// Tag an element so `clear_soma` can identify and remove it later.
fn tag_soma_owned(el: &HtmlElement) -> Result<()> {
    el.set_attribute(DATA_SOMA, "true")
        .map_err(|e| SomaError::Port(format!("set_attribute failed: {e:?}")))
}

// ---------------------------------------------------------------------------
// Capability implementations
// ---------------------------------------------------------------------------

fn append_heading(input: &Value) -> Result<Value> {
    let text = input
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SomaError::Port("missing required field 'text'".to_string()))?;

    let level = input.get("level").and_then(|v| v.as_u64()).unwrap_or(1);
    if !(1..=6).contains(&level) {
        return Err(SomaError::Port(
            "'level' must be between 1 and 6".to_string(),
        ));
    }
    let tag = format!("h{level}");

    let doc = document()?;
    let body = body(&doc)?;

    let element: HtmlElement = doc
        .create_element(&tag)
        .map_err(|e| SomaError::Port(format!("create_element failed: {e:?}")))?
        .dyn_into()
        .map_err(|_| SomaError::Port("created element is not an HTMLElement".to_string()))?;
    element.set_text_content(Some(text));
    tag_soma_owned(&element)?;

    body.append_child(&element)
        .map_err(|e| SomaError::Port(format!("append_child failed: {e:?}")))?;

    Ok(json!({
        "rendered": true,
        "tag": tag,
        "level": level,
        "text": text,
    }))
}

fn append_paragraph(input: &Value) -> Result<Value> {
    let text = input
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SomaError::Port("missing required field 'text'".to_string()))?;

    let doc = document()?;
    let body = body(&doc)?;

    let element: HtmlElement = doc
        .create_element("p")
        .map_err(|e| SomaError::Port(format!("create_element failed: {e:?}")))?
        .dyn_into()
        .map_err(|_| SomaError::Port("created element is not an HTMLElement".to_string()))?;
    element.set_text_content(Some(text));
    tag_soma_owned(&element)?;

    body.append_child(&element)
        .map_err(|e| SomaError::Port(format!("append_child failed: {e:?}")))?;

    Ok(json!({
        "rendered": true,
        "tag": "p",
        "text": text,
    }))
}

fn set_title(input: &Value) -> Result<Value> {
    let text = input
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SomaError::Port("missing required field 'text'".to_string()))?;

    let doc = document()?;
    doc.set_title(text);

    Ok(json!({
        "title_set": text,
    }))
}

fn clear_soma() -> Result<Value> {
    let doc = document()?;
    let selector = format!("[{DATA_SOMA}=\"true\"]");
    let list = doc
        .query_selector_all(&selector)
        .map_err(|e| SomaError::Port(format!("query_selector_all failed: {e:?}")))?;
    let mut removed: u32 = 0;
    for i in 0..list.length() {
        if let Some(node) = list.item(i)
            && let Some(parent) = node.parent_node()
        {
            let _ = parent.remove_child(&node);
            removed += 1;
        }
    }
    Ok(json!({
        "removed_count": removed,
    }))
}

// ---------------------------------------------------------------------------
// PortSpec
// ---------------------------------------------------------------------------

fn build_spec() -> PortSpec {
    let capabilities = vec![
        cap_append_heading(),
        cap_append_paragraph(),
        cap_set_title(),
        cap_clear_soma(),
    ];

    PortSpec {
        port_id: "dom".to_string(),
        name: "DOM".to_string(),
        version: semver::Version::new(0, 2, 0),
        kind: PortKind::Renderer,
        description: "In-tab browser DOM port for creating and mutating HTML elements."
            .to_string(),
        namespace: "soma.ports.dom".to_string(),
        trust_level: TrustLevel::BuiltIn,
        capabilities,
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
        latency_profile: dom_latency(),
        cost_profile: negligible_cost(),
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

fn dom_latency() -> LatencyProfile {
    LatencyProfile {
        expected_latency_ms: 1,
        p95_latency_ms: 5,
        max_latency_ms: 50,
    }
}

fn negligible_cost() -> CostProfile {
    CostProfile {
        cpu_cost_class: CostClass::Negligible,
        memory_cost_class: CostClass::Negligible,
        io_cost_class: CostClass::Negligible,
        network_cost_class: CostClass::Negligible,
        energy_cost_class: CostClass::Negligible,
    }
}

fn cap_append_heading() -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: "append_heading".to_string(),
        name: "append_heading".to_string(),
        purpose: "Create an <h{level}> with the given text and append it to document.body."
            .to_string(),
        input_schema: SchemaRef {
            schema: json!({
                "type": "object",
                "required": ["text"],
                "properties": {
                    "text": { "type": "string" },
                    "level": { "type": "integer", "minimum": 1, "maximum": 6 }
                }
            }),
        },
        output_schema: SchemaRef {
            schema: json!({
                "type": "object",
                "properties": {
                    "rendered": { "type": "boolean" },
                    "tag": { "type": "string" },
                    "level": { "type": "integer" },
                    "text": { "type": "string" }
                }
            }),
        },
        effect_class: SideEffectClass::LocalStateMutation,
        rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::Deterministic,
        idempotence_class: IdempotenceClass::NonIdempotent,
        risk_class: RiskClass::Low,
        latency_profile: dom_latency(),
        cost_profile: negligible_cost(),
        remote_exposable: false,
        auth_override: None,
    }
}

fn cap_append_paragraph() -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: "append_paragraph".to_string(),
        name: "append_paragraph".to_string(),
        purpose: "Create a <p> with the given text and append it to document.body.".to_string(),
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
        latency_profile: dom_latency(),
        cost_profile: negligible_cost(),
        remote_exposable: false,
        auth_override: None,
    }
}

fn cap_set_title() -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: "set_title".to_string(),
        name: "set_title".to_string(),
        purpose: "Set the browser tab title (document.title).".to_string(),
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
                "properties": { "title_set": { "type": "string" } }
            }),
        },
        effect_class: SideEffectClass::LocalStateMutation,
        rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::Deterministic,
        idempotence_class: IdempotenceClass::Idempotent,
        risk_class: RiskClass::Low,
        latency_profile: dom_latency(),
        cost_profile: negligible_cost(),
        remote_exposable: false,
        auth_override: None,
    }
}

fn cap_clear_soma() -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: "clear_soma".to_string(),
        name: "clear_soma".to_string(),
        purpose: "Remove every element previously rendered by SOMA (tagged with data-soma)."
            .to_string(),
        input_schema: SchemaRef {
            schema: json!({ "type": "object" }),
        },
        output_schema: SchemaRef {
            schema: json!({
                "type": "object",
                "properties": { "removed_count": { "type": "integer" } }
            }),
        },
        effect_class: SideEffectClass::LocalStateMutation,
        rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::Deterministic,
        idempotence_class: IdempotenceClass::Idempotent,
        risk_class: RiskClass::Low,
        latency_profile: dom_latency(),
        cost_profile: negligible_cost(),
        remote_exposable: false,
        auth_override: None,
    }
}
