//! Browser voice port — in-tab SOMA port that transcribes microphone audio.
//!
//! Uses the Web Speech API `SpeechRecognition` (available in Chromium-based
//! browsers and Safari; Firefox has partial support). The port is the
//! "microphone" side of the brain/body split: the body reports what it
//! heard, the brain decides what to do with the transcript.
//!
//! The async nature of speech recognition is the interesting part. A
//! `Port::invoke` call must return synchronously, but recognized utterances
//! arrive asynchronously from the browser whenever the user speaks. The
//! solution: `start_listening` registers event-handler closures that push
//! transcripts into a thread-local buffer; subsequent `get_last_transcript`
//! and `get_all_transcripts` calls read from that buffer. No
//! synchronous blocking, no JS Promises leaking into the Port trait.
//!
//! Phase 1c capabilities:
//!
//! | capability_id          | effect                                     |
//! |------------------------|--------------------------------------------|
//! | `start_listening`      | Create SpeechRecognition, attach handlers  |
//! | `stop_listening`       | Stop recognition, release handlers         |
//! | `get_last_transcript`  | Return the most recent recognized text     |
//! | `get_all_transcripts`  | Return the entire session transcript Vec  |
//! | `clear_transcripts`    | Empty the transcript buffer                |
//!
//! The browser will prompt the user for microphone permission the first
//! time `start_listening` runs. SOMA's philosophy says the body never
//! asks, so once permission is granted the runtime treats the microphone
//! as a plain sensor (like the ESP32 thermistor) that the brain can poll.

use std::cell::RefCell;

use chrono::Utc;
use serde_json::{json, Value};
use uuid::Uuid;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use web_sys::{SpeechRecognition, SpeechRecognitionError, SpeechRecognitionEvent};

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

/// Per-tab voice state. `thread_local!` + `RefCell` is the standard
/// single-threaded wasm pattern — wasm32-unknown-unknown has no real
/// threads and all events arrive through the JS event loop, so the
/// Rust borrow checker's guarantees about RefCell still hold as long
/// as no borrow leaks across an await boundary (and there are no
/// awaits here — everything is synchronous).
#[derive(Default)]
struct VoiceInner {
    recognition: Option<SpeechRecognition>,
    /// Accumulated recognized transcripts, oldest → newest.
    transcripts: Vec<String>,
    /// Whether the recognition is actively listening. Flipped to true on
    /// start_listening and to false on stop_listening or an `end` event.
    listening: bool,
    /// Event-handler closures we need to keep alive so the browser can
    /// call back into Rust. Dropping any of these detaches the handler.
    _onresult: Option<Closure<dyn FnMut(SpeechRecognitionEvent)>>,
    _onerror: Option<Closure<dyn FnMut(SpeechRecognitionError)>>,
    _onend: Option<Closure<dyn FnMut()>>,
}

thread_local! {
    static VOICE: RefCell<VoiceInner> = RefCell::new(VoiceInner::default());
}

pub struct VoicePort {
    spec: PortSpec,
}

impl Default for VoicePort {
    fn default() -> Self {
        Self::new()
    }
}

impl VoicePort {
    pub fn new() -> Self {
        Self {
            spec: build_spec(),
        }
    }
}

impl Port for VoicePort {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(&self, capability_id: &str, _input: Value) -> Result<PortCallRecord> {
        let start = js_sys::Date::now();
        let result = match capability_id {
            "start_listening" => start_listening(),
            "stop_listening" => stop_listening(),
            "get_last_transcript" => get_last_transcript(),
            "get_all_transcripts" => get_all_transcripts(),
            "clear_transcripts" => clear_transcripts(),
            other => {
                return Err(SomaError::Port(format!(
                    "unknown capability '{other}' on voice port"
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

    fn validate_input(&self, capability_id: &str, _input: &Value) -> Result<()> {
        match capability_id {
            "start_listening"
            | "stop_listening"
            | "get_last_transcript"
            | "get_all_transcripts"
            | "clear_transcripts" => Ok(()),
            other => Err(SomaError::Port(format!(
                "unknown capability '{other}' on voice port"
            ))),
        }
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        PortLifecycleState::Active
    }
}

fn side_effect_for(capability_id: &str) -> &'static str {
    match capability_id {
        "start_listening" => "voice_start",
        "stop_listening" => "voice_stop",
        "get_last_transcript" | "get_all_transcripts" => "voice_read",
        "clear_transcripts" => "voice_clear",
        _ => "voice_unknown",
    }
}

// ---------------------------------------------------------------------------
// Capability implementations
// ---------------------------------------------------------------------------

fn start_listening() -> Result<Value> {
    // Snapshot the current state without holding a borrow during the
    // rest of the function. `listening` distinguishes the no-op case;
    // `stale` means a previous session ended naturally (onend fired)
    // but the old SpeechRecognition + closures are still in VOICE.
    // That stale state MUST be torn down before we create new closures,
    // because the old recognition still holds references to the old
    // Closure handles and firing events against dropped closures is
    // what produced the "closure invoked after being dropped" crash.
    let (already_listening, has_stale) = VOICE.with(|v| {
        let inner = v.borrow();
        (inner.listening, inner.recognition.is_some() && !inner.listening)
    });
    if already_listening {
        return Ok(json!({ "listening": true, "restart": false }));
    }
    if has_stale {
        let _ = stop_listening();
    }

    // Construct the recognizer. web-sys binds to `window.SpeechRecognition`,
    // which modern Chrome/Safari alias to `webkitSpeechRecognition`. If the
    // browser has neither, we fail closed with a clear error.
    let recognition = SpeechRecognition::new().map_err(|e| {
        SomaError::Port(format!(
            "SpeechRecognition constructor unavailable (no Web Speech API in this browser): {e:?}"
        ))
    })?;

    // `set_continuous` is fallible on some platforms; on others it's a
    // plain setter. Ignore the Result in either case — "best effort"
    // configuration semantics match the Web Speech API's permissive model.
    let _ = recognition.set_continuous(true);
    recognition.set_interim_results(false);
    recognition.set_lang("en-US");

    // onresult: push the recognized transcript into the thread-local buffer.
    // The closure is stored in VOICE so its lifetime matches the recognition.
    let onresult = Closure::<dyn FnMut(SpeechRecognitionEvent)>::new(
        |event: SpeechRecognitionEvent| {
            let Some(results) = event.results() else {
                return;
            };
            let idx = event.result_index();
            let mut recognized: Vec<String> = Vec::new();
            // Each onresult carries every result from result_index forward.
            // In continuous mode that can be a run of segments from a single
            // utterance; we treat each as a distinct transcript entry so the
            // brain can see exactly what the body heard.
            for i in idx..results.length() {
                let result = results.item(i);
                let alt = result.item(0);
                let text = alt.transcript().trim().to_string();
                if !text.is_empty() {
                    recognized.push(text);
                }
            }
            if !recognized.is_empty() {
                VOICE.with(|v| {
                    let mut inner = v.borrow_mut();
                    for text in &recognized {
                        web_sys::console::log_2(
                            &wasm_bindgen::JsValue::from_str("[voice] heard:"),
                            &wasm_bindgen::JsValue::from_str(text),
                        );
                        inner.transcripts.push(text.clone());
                    }
                });
            }
        },
    );

    let onerror = Closure::<dyn FnMut(SpeechRecognitionError)>::new(
        |event: SpeechRecognitionError| {
            let msg = event.message().unwrap_or_default();
            web_sys::console::warn_2(
                &wasm_bindgen::JsValue::from_str("[voice] error:"),
                &wasm_bindgen::JsValue::from_str(&msg),
            );
        },
    );

    let onend = Closure::<dyn FnMut()>::new(|| {
        // onend can fire DURING `stop_listening` on some browsers (Chrome
        // delivers it synchronously inside `recognition.stop()`). If that
        // happens while stop_listening is already holding a mut borrow of
        // VOICE, a plain `borrow_mut()` would panic with "already
        // borrowed". `try_borrow_mut` turns the race into a harmless
        // no-op — stop_listening is already resetting listening anyway.
        VOICE.with(|v| {
            if let Ok(mut inner) = v.try_borrow_mut() {
                inner.listening = false;
            }
        });
        web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(
            "[voice] recognition ended",
        ));
    });

    recognition.set_onresult(Some(onresult.as_ref().unchecked_ref()));
    recognition.set_onerror(Some(onerror.as_ref().unchecked_ref()));
    recognition.set_onend(Some(onend.as_ref().unchecked_ref()));

    recognition.start().map_err(|e| {
        SomaError::Port(format!(
            "SpeechRecognition.start() failed (microphone permission denied?): {e:?}"
        ))
    })?;

    // Move everything into the thread-local state so the closures stay
    // alive for the lifetime of the listening session.
    VOICE.with(|v| {
        let mut inner = v.borrow_mut();
        inner.recognition = Some(recognition);
        inner._onresult = Some(onresult);
        inner._onerror = Some(onerror);
        inner._onend = Some(onend);
        inner.listening = true;
    });

    Ok(json!({ "listening": true, "restart": false }))
}

fn stop_listening() -> Result<Value> {
    // Take the recognition + every event-handler closure out of the
    // thread-local FIRST, release the borrow, then talk to JS. This
    // ordering is the whole point — calling `recognition.stop()` or any
    // setter while still holding a mut borrow of VOICE lets a synchronous
    // event delivery (Chrome does this for `end`) re-enter the closures,
    // which then panic on a double mut-borrow.
    let (recognition, onresult, onerror, onend) = VOICE.with(|v| {
        let mut inner = v.borrow_mut();
        let r = inner.recognition.take();
        let a = inner._onresult.take();
        let b = inner._onerror.take();
        let c = inner._onend.take();
        inner.listening = false;
        (r, a, b, c)
    });

    // Unregister every handler from the SpeechRecognition instance before
    // we drop the Closure handles. If the browser still had events queued
    // after this point it can no longer reach the (now-invalid) JS
    // function wrappers. This is what fixes the
    // "closure invoked after being dropped" error.
    if let Some(ref rec) = recognition {
        rec.set_onresult(None);
        rec.set_onerror(None);
        rec.set_onend(None);
        rec.stop();
    }

    // NOW it's safe to drop the closures. Explicit drop documents the
    // ordering constraint; without this the same effect would happen at
    // end-of-scope but the intent would be invisible.
    drop((onresult, onerror, onend, recognition));

    Ok(json!({ "listening": false }))
}

fn get_last_transcript() -> Result<Value> {
    VOICE.with(|v| {
        let inner = v.borrow();
        let last = inner.transcripts.last().cloned();
        Ok(json!({
            "text": last,
            "listening": inner.listening,
            "total_transcripts": inner.transcripts.len(),
        }))
    })
}

fn get_all_transcripts() -> Result<Value> {
    VOICE.with(|v| {
        let inner = v.borrow();
        Ok(json!({
            "transcripts": inner.transcripts.clone(),
            "listening": inner.listening,
            "total_transcripts": inner.transcripts.len(),
        }))
    })
}

fn clear_transcripts() -> Result<Value> {
    VOICE.with(|v| {
        let mut inner = v.borrow_mut();
        let cleared = inner.transcripts.len();
        inner.transcripts.clear();
        Ok(json!({ "cleared_count": cleared }))
    })
}

// ---------------------------------------------------------------------------
// PortSpec
// ---------------------------------------------------------------------------

fn build_spec() -> PortSpec {
    let capabilities = vec![
        cap_start_listening(),
        cap_stop_listening(),
        cap_get_last_transcript(),
        cap_get_all_transcripts(),
        cap_clear_transcripts(),
    ];

    PortSpec {
        port_id: "voice".to_string(),
        name: "Voice".to_string(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Sensor,
        description: "In-tab browser microphone port using the Web Speech API.".to_string(),
        namespace: "soma.ports.voice".to_string(),
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
        side_effect_class: SideEffectClass::ReadOnly,
        latency_profile: voice_latency(),
        cost_profile: negligible_cost(),
        auth_requirements: AuthRequirements {
            methods: vec![],
            required: false,
        },
        sandbox_requirements: SandboxRequirements {
            filesystem_access: false,
            network_access: false,
            // Microphone is a device, but the browser already enforces its
            // own permission prompt. SOMA's sandbox layer can stay hands-off.
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

fn voice_latency() -> LatencyProfile {
    LatencyProfile {
        expected_latency_ms: 1,
        p95_latency_ms: 10,
        max_latency_ms: 100,
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

fn cap_start_listening() -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: "start_listening".to_string(),
        name: "start_listening".to_string(),
        purpose: "Start listening on the microphone via the Web Speech API.".to_string(),
        input_schema: SchemaRef {
            schema: json!({ "type": "object" }),
        },
        output_schema: SchemaRef {
            schema: json!({
                "type": "object",
                "properties": {
                    "listening": { "type": "boolean" },
                    "restart": { "type": "boolean" }
                }
            }),
        },
        effect_class: SideEffectClass::LocalStateMutation,
        rollback_support: RollbackSupport::CompensatingAction,
        determinism_class: DeterminismClass::Deterministic,
        idempotence_class: IdempotenceClass::Idempotent,
        risk_class: RiskClass::Low,
        latency_profile: voice_latency(),
        cost_profile: negligible_cost(),
        remote_exposable: false,
        auth_override: None,
    }
}

fn cap_stop_listening() -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: "stop_listening".to_string(),
        name: "stop_listening".to_string(),
        purpose: "Stop the active SpeechRecognition session.".to_string(),
        input_schema: SchemaRef {
            schema: json!({ "type": "object" }),
        },
        output_schema: SchemaRef {
            schema: json!({
                "type": "object",
                "properties": { "listening": { "type": "boolean" } }
            }),
        },
        effect_class: SideEffectClass::LocalStateMutation,
        rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::Deterministic,
        idempotence_class: IdempotenceClass::Idempotent,
        risk_class: RiskClass::Low,
        latency_profile: voice_latency(),
        cost_profile: negligible_cost(),
        remote_exposable: false,
        auth_override: None,
    }
}

fn cap_get_last_transcript() -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: "get_last_transcript".to_string(),
        name: "get_last_transcript".to_string(),
        purpose: "Return the most recent recognized transcript (null if none yet)."
            .to_string(),
        input_schema: SchemaRef {
            schema: json!({ "type": "object" }),
        },
        output_schema: SchemaRef {
            schema: json!({
                "type": "object",
                "properties": {
                    "text": { "type": ["string", "null"] },
                    "listening": { "type": "boolean" },
                    "total_transcripts": { "type": "integer" }
                }
            }),
        },
        effect_class: SideEffectClass::ReadOnly,
        rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::Stochastic,
        idempotence_class: IdempotenceClass::Idempotent,
        risk_class: RiskClass::Negligible,
        latency_profile: voice_latency(),
        cost_profile: negligible_cost(),
        remote_exposable: false,
        auth_override: None,
    }
}

fn cap_get_all_transcripts() -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: "get_all_transcripts".to_string(),
        name: "get_all_transcripts".to_string(),
        purpose: "Return every transcript recognized in this session.".to_string(),
        input_schema: SchemaRef {
            schema: json!({ "type": "object" }),
        },
        output_schema: SchemaRef {
            schema: json!({
                "type": "object",
                "properties": {
                    "transcripts": { "type": "array", "items": { "type": "string" } },
                    "listening": { "type": "boolean" },
                    "total_transcripts": { "type": "integer" }
                }
            }),
        },
        effect_class: SideEffectClass::ReadOnly,
        rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::Stochastic,
        idempotence_class: IdempotenceClass::Idempotent,
        risk_class: RiskClass::Negligible,
        latency_profile: voice_latency(),
        cost_profile: negligible_cost(),
        remote_exposable: false,
        auth_override: None,
    }
}

fn cap_clear_transcripts() -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: "clear_transcripts".to_string(),
        name: "clear_transcripts".to_string(),
        purpose: "Empty the transcript buffer.".to_string(),
        input_schema: SchemaRef {
            schema: json!({ "type": "object" }),
        },
        output_schema: SchemaRef {
            schema: json!({
                "type": "object",
                "properties": { "cleared_count": { "type": "integer" } }
            }),
        },
        effect_class: SideEffectClass::LocalStateMutation,
        rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::Deterministic,
        idempotence_class: IdempotenceClass::Idempotent,
        risk_class: RiskClass::Negligible,
        latency_profile: voice_latency(),
        cost_profile: negligible_cost(),
        remote_exposable: false,
        auth_override: None,
    }
}
