// soma-esp32-leaf — no_std SOMA leaf node for embedded targets.
//
// Architecture (matches desktop SOMA's body/brain split):
//
//   Firmware ships with HARDWARE PRIMITIVES (the body's reflexes):
//     gpio.write, gpio.read, gpio.toggle, delay.ms, status, etc.
//
//   The brain (LLM via server SOMA) composes ROUTINES from those primitives:
//     "blink_led_kitchen" = [
//        gpio.write(7, 1),
//        delay.ms(500),
//        gpio.write(7, 0),
//        delay.ms(500),
//     ]
//
//   The brain transfers routines to the leaf via TransferRoutine wire messages.
//   The leaf STORES them and EXECUTES them locally on InvokeSkill.
//
//   Adding a new high-level skill = sending a TransferRoutine. NO firmware
//   rebuild. NO flash. The body learns at runtime.
//
//   Firmware only rebuilds when the hardware support itself changes (new
//   bus, new sensor protocol, new primitive). Skill additions are runtime.
//
// What's in this crate:
//   - TransportMessage / TransportResponse subset (the wire protocol)
//   - SkillDispatcher trait (the firmware implements this for primitives)
//   - LeafState<D> — wraps the dispatcher with routine storage and walking
//   - Routine + RoutineStep types (the leaf's compiled routine format)
//   - Frame codec (4-byte big-endian length prefix + JSON)
//
// What's NOT in this crate:
//   - tokio, async, networking — the firmware handles transport
//   - PrefixSpan, schema induction — those run on the server peer, never on leaf
//   - libloading, dynamic ports — primitives are compiled in
//   - chrono, uuid v4 — leaf doesn't generate timestamps or random IDs

#![no_std]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Wire protocol
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TransportMessage {
    /// Server invokes a skill on this leaf. The skill_id may refer to either
    /// a hardware primitive (handled by the dispatcher) or a stored routine
    /// (handled by the leaf state's routine walker).
    InvokeSkill {
        peer_id: String,
        skill_id: String,
        input: serde_json::Value,
    },
    /// Server queries this leaf's full capability inventory: primitives the
    /// firmware ships with PLUS routines previously transferred and stored.
    ListCapabilities,
    /// Server pushes a routine to this leaf for storage. After this message
    /// the routine is invokable via InvokeSkill { skill_id: routine.routine_id }.
    /// This is how the brain teaches the body new high-level skills at runtime
    /// — no firmware rebuild required.
    TransferRoutine { routine: Routine },
    /// Server removes a previously stored routine.
    RemoveRoutine { routine_id: String },
    /// Server pings to check liveness.
    Ping { nonce: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TransportResponse {
    /// Result of an InvokeSkill — single observation for primitives, or
    /// aggregated observations for routines.
    SkillResult { response: RemoteSkillResponse },
    /// Result of ListCapabilities — primitive + routine inventory.
    Capabilities {
        primitives: Vec<CapabilityDescriptor>,
        routines: Vec<RoutineDescriptor>,
    },
    /// Acknowledgement of TransferRoutine — the routine is now stored.
    RoutineStored { routine_id: String, step_count: u32 },
    /// Acknowledgement of RemoveRoutine.
    RoutineRemoved { routine_id: String },
    /// Pong response with current load average (0.0 - 1.0).
    Pong { nonce: u64, load: f64 },
    /// Error response with details.
    Error { details: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSkillResponse {
    pub skill_id: String,
    pub success: bool,
    pub structured_result: serde_json::Value,
    pub failure_message: Option<String>,
    pub latency_ms: u64,
    /// Number of primitive invocations consumed. 1 for a primitive, N for a
    /// routine that walked N steps.
    pub steps_executed: u32,
}

// ---------------------------------------------------------------------------
// Capability descriptors — what the body knows about itself
// ---------------------------------------------------------------------------

/// Describes one HARDWARE PRIMITIVE the firmware ships with. These are the
/// body's reflexes — they cannot be added or removed without rebuilding
/// firmware. Routines are composed from these.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    pub skill_id: String,
    pub description: String,
    pub input_schema: String,
    pub output_schema: String,
    pub effect: Effect,
}

/// Describes one ROUTINE currently stored on the leaf. Routines are
/// transferred at runtime via TransferRoutine; their existence does NOT
/// require a firmware rebuild.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineDescriptor {
    pub routine_id: String,
    pub description: String,
    pub step_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    ReadOnly,
    StateMutation,
    ExternalEffect,
}

// ---------------------------------------------------------------------------
// Routine — a compiled sequence of primitive invocations
// ---------------------------------------------------------------------------
//
// Routines are the unit of "learned skill" on the leaf. They're transferred
// from the brain (server peer) and stored locally. Walking a routine is just
// calling the dispatcher's invoke() for each step in sequence.
//
// This format mirrors soma-next's Routine but stripped down to what a leaf
// needs: an ordered list of (skill_id, input) pairs. No match conditions, no
// guard conditions, no expected cost — those concerns live on the server.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Routine {
    pub routine_id: String,
    pub description: String,
    pub steps: Vec<RoutineStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineStep {
    /// Primitive skill_id to invoke. MUST exist in the dispatcher's
    /// list_capabilities. The leaf does not validate this at storage time —
    /// errors surface when the routine is invoked.
    pub skill_id: String,
    /// Input passed to the primitive on this step. Hardcoded by the brain
    /// when the routine is composed.
    pub input: serde_json::Value,
}

// ---------------------------------------------------------------------------
// SomaEspPort trait — one port = one set of primitives
// ---------------------------------------------------------------------------
//
// Mirrors the soma-port-sdk Port trait from desktop SOMA, but for embedded
// targets where dynamic loading isn't possible. Each port crate (gpio,
// thermistor, dht22, bme280, ...) implements this trait. The firmware
// composes ports at BUILD time via cargo features instead of at runtime
// via dlopen.
//
// A port owns whatever hardware state it needs (claimed pins, bus handles,
// calibration data) and exposes a fixed set of primitives. Adding a new
// sensor model = adding a new port crate = adding it as a feature flag in
// the firmware Cargo.toml. NO changes to leaf or other ports.

pub trait SomaEspPort {
    /// Stable identifier for this port. Used for namespacing and diagnostics.
    /// Mirrors PortSpec.port_id in desktop SOMA.
    fn port_id(&self) -> &'static str;

    /// The primitives this port exposes. Returned upfront so the composite
    /// dispatcher can build a skill_id → port lookup table at registration
    /// time.
    fn primitives(&self) -> Vec<CapabilityDescriptor>;

    /// Execute a primitive owned by this port. The composite dispatcher
    /// only routes calls for skill_ids this port declared in primitives().
    fn invoke(
        &mut self,
        skill_id: &str,
        input: &serde_json::Value,
    ) -> Result<serde_json::Value, String>;
}

// ---------------------------------------------------------------------------
// SkillDispatcher trait — what LeafState consumes
// ---------------------------------------------------------------------------
//
// LeafState is generic over any SkillDispatcher. The simplest impl is a
// single port wrapped in a thin adapter; the realistic impl is the
// CompositeDispatcher below, which aggregates multiple ports.

pub trait SkillDispatcher {
    /// The body's full primitive inventory across all ports.
    fn list_primitives(&self) -> Vec<CapabilityDescriptor>;

    /// Route a primitive call to the port that owns it.
    fn invoke(
        &mut self,
        skill_id: &str,
        input: &serde_json::Value,
    ) -> Result<serde_json::Value, String>;

    /// Current load 0.0 - 1.0 for Pong responses.
    fn current_load(&self) -> f64 {
        0.0
    }
}

// ---------------------------------------------------------------------------
// CompositeDispatcher — aggregates multiple SomaEspPort impls
// ---------------------------------------------------------------------------
//
// The firmware constructs one of these and `register`s each port crate it
// includes via cargo features. The composite builds a skill_id → port_index
// map upfront so primitive routing is O(log n) per invocation.
//
// This is the embedded analogue of soma-next's DefaultPortRuntime — except
// instead of loading .dylib files at runtime, ports are linked at build
// time and registered at boot.

pub struct CompositeDispatcher {
    ports: Vec<alloc::boxed::Box<dyn SomaEspPort>>,
    /// skill_id -> index into self.ports
    skill_routing: BTreeMap<String, usize>,
}

impl CompositeDispatcher {
    pub fn new() -> Self {
        Self {
            ports: Vec::new(),
            skill_routing: BTreeMap::new(),
        }
    }

    /// Register a port. Builds the skill routing table from the port's
    /// declared primitives. Later registrations of the same skill_id
    /// silently overwrite earlier ones (last-write-wins).
    pub fn register(&mut self, port: alloc::boxed::Box<dyn SomaEspPort>) {
        let port_idx = self.ports.len();
        for cap in port.primitives() {
            self.skill_routing.insert(cap.skill_id, port_idx);
        }
        self.ports.push(port);
    }

    pub fn port_count(&self) -> usize {
        self.ports.len()
    }

    pub fn port_ids(&self) -> Vec<&'static str> {
        self.ports.iter().map(|p| p.port_id()).collect()
    }
}

impl Default for CompositeDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillDispatcher for CompositeDispatcher {
    fn list_primitives(&self) -> Vec<CapabilityDescriptor> {
        self.ports.iter().flat_map(|p| p.primitives()).collect()
    }

    fn invoke(
        &mut self,
        skill_id: &str,
        input: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let idx = self
            .skill_routing
            .get(skill_id)
            .copied()
            .ok_or_else(|| alloc::format!("no port handles primitive: {}", skill_id))?;
        self.ports[idx].invoke(skill_id, input)
    }

    fn current_load(&self) -> f64 {
        0.0
    }
}

// ---------------------------------------------------------------------------
// LeafState — wraps a dispatcher with runtime-mutable routine storage
// ---------------------------------------------------------------------------
//
// This is the body's brain-stem. It holds:
//   - The hardware dispatcher (primitives)
//   - A map of stored routines (transferred from the brain)
//
// On InvokeSkill it first checks routines (so a routine can shadow a
// primitive name), then falls back to the dispatcher.
//
// On TransferRoutine it stores the routine. On RemoveRoutine it removes one.
// On ListCapabilities it returns BOTH primitives and routines.

pub struct LeafState<D: SkillDispatcher> {
    dispatcher: D,
    routines: BTreeMap<String, Routine>,
}

impl<D: SkillDispatcher> LeafState<D> {
    pub fn new(dispatcher: D) -> Self {
        Self {
            dispatcher,
            routines: BTreeMap::new(),
        }
    }

    pub fn dispatcher(&self) -> &D {
        &self.dispatcher
    }

    pub fn routine_count(&self) -> usize {
        self.routines.len()
    }

    /// Store a routine. If a routine with the same ID exists, it is replaced.
    pub fn store_routine(&mut self, routine: Routine) -> u32 {
        let count = routine.steps.len() as u32;
        self.routines.insert(routine.routine_id.clone(), routine);
        count
    }

    /// Remove a routine by ID. Returns true if it existed.
    pub fn remove_routine(&mut self, routine_id: &str) -> bool {
        self.routines.remove(routine_id).is_some()
    }

    /// Return the full inventory: primitives + stored routines.
    pub fn list_all(&self) -> (Vec<CapabilityDescriptor>, Vec<RoutineDescriptor>) {
        let primitives = self.dispatcher.list_primitives();
        let routines = self
            .routines
            .values()
            .map(|r| RoutineDescriptor {
                routine_id: r.routine_id.clone(),
                description: r.description.clone(),
                step_count: r.steps.len() as u32,
            })
            .collect();
        (primitives, routines)
    }

    /// Process an incoming wire message and produce the response.
    pub fn handle(&mut self, msg: TransportMessage) -> TransportResponse {
        match msg {
            TransportMessage::Ping { nonce } => TransportResponse::Pong {
                nonce,
                load: self.dispatcher.current_load(),
            },
            TransportMessage::ListCapabilities => {
                let (primitives, routines) = self.list_all();
                TransportResponse::Capabilities {
                    primitives,
                    routines,
                }
            }
            TransportMessage::TransferRoutine { routine } => {
                let id = routine.routine_id.clone();
                let count = self.store_routine(routine);
                TransportResponse::RoutineStored {
                    routine_id: id,
                    step_count: count,
                }
            }
            TransportMessage::RemoveRoutine { routine_id } => {
                let removed = self.remove_routine(&routine_id);
                if removed {
                    TransportResponse::RoutineRemoved { routine_id }
                } else {
                    TransportResponse::Error {
                        details: alloc::format!("routine not found: {}", routine_id),
                    }
                }
            }
            TransportMessage::InvokeSkill {
                skill_id, input, ..
            } => self.invoke_skill_or_routine(&skill_id, &input),
        }
    }

    /// Resolve `skill_id` to either a stored routine or a primitive and
    /// execute it. Routines take precedence — a routine with the same name
    /// as a primitive shadows the primitive.
    fn invoke_skill_or_routine(
        &mut self,
        skill_id: &str,
        input: &serde_json::Value,
    ) -> TransportResponse {
        // Check for a stored routine first.
        // We clone the routine to release the borrow on self before invoking
        // the dispatcher. Routines are typically small (a handful of steps).
        let routine = self.routines.get(skill_id).cloned();

        if let Some(routine) = routine {
            return self.execute_routine(&routine, input);
        }

        // Fall back to the dispatcher (a primitive).
        match self.dispatcher.invoke(skill_id, input) {
            Ok(result) => TransportResponse::SkillResult {
                response: RemoteSkillResponse {
                    skill_id: skill_id.into(),
                    success: true,
                    structured_result: result,
                    failure_message: None,
                    latency_ms: 0,
                    steps_executed: 1,
                },
            },
            Err(msg) => TransportResponse::SkillResult {
                response: RemoteSkillResponse {
                    skill_id: skill_id.into(),
                    success: false,
                    structured_result: serde_json::Value::Null,
                    failure_message: Some(msg),
                    latency_ms: 0,
                    steps_executed: 0,
                },
            },
        }
    }

    /// Walk a routine, calling the dispatcher for each step. Aggregates
    /// results into a JSON array. Stops on the first failure and returns
    /// the partial trace.
    fn execute_routine(
        &mut self,
        routine: &Routine,
        _invocation_input: &serde_json::Value,
    ) -> TransportResponse {
        let mut step_results = Vec::with_capacity(routine.steps.len());
        let mut steps_executed: u32 = 0;

        for (i, step) in routine.steps.iter().enumerate() {
            match self.dispatcher.invoke(&step.skill_id, &step.input) {
                Ok(result) => {
                    steps_executed += 1;
                    step_results.push(serde_json::json!({
                        "step": i,
                        "skill_id": step.skill_id,
                        "ok": true,
                        "result": result,
                    }));
                }
                Err(msg) => {
                    step_results.push(serde_json::json!({
                        "step": i,
                        "skill_id": step.skill_id,
                        "ok": false,
                        "error": msg,
                    }));
                    return TransportResponse::SkillResult {
                        response: RemoteSkillResponse {
                            skill_id: routine.routine_id.clone(),
                            success: false,
                            structured_result: serde_json::Value::Array(step_results),
                            failure_message: Some(alloc::format!(
                                "routine '{}' failed at step {}",
                                routine.routine_id, i
                            )),
                            latency_ms: 0,
                            steps_executed,
                        },
                    };
                }
            }
        }

        TransportResponse::SkillResult {
            response: RemoteSkillResponse {
                skill_id: routine.routine_id.clone(),
                success: true,
                structured_result: serde_json::Value::Array(step_results),
                failure_message: None,
                latency_ms: 0,
                steps_executed,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Frame codec — 4-byte big-endian length prefix + JSON payload
// ---------------------------------------------------------------------------

pub const DEFAULT_MAX_FRAME: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    NeedMore,
    TooLarge,
    Decode,
}

pub fn decode_frame(
    buf: &[u8],
    max_frame: usize,
) -> Result<(TransportMessage, usize), FrameError> {
    if buf.len() < 4 {
        return Err(FrameError::NeedMore);
    }
    let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if len > max_frame {
        return Err(FrameError::TooLarge);
    }
    if buf.len() < 4 + len {
        return Err(FrameError::NeedMore);
    }
    let payload = &buf[4..4 + len];
    let msg: TransportMessage =
        serde_json::from_slice(payload).map_err(|_| FrameError::Decode)?;
    Ok((msg, 4 + len))
}

pub fn encode_response(resp: &TransportResponse) -> Result<Vec<u8>, FrameError> {
    let payload = serde_json::to_vec(resp).map_err(|_| FrameError::Decode)?;
    let len = payload.len() as u32;
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

pub fn encode_message(msg: &TransportMessage) -> Result<Vec<u8>, FrameError> {
    let payload = serde_json::to_vec(msg).map_err(|_| FrameError::Decode)?;
    let len = payload.len() as u32;
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;
    use serde_json::json;

    /// Two stub ports for testing CompositeDispatcher composition.
    /// A real firmware would use port crates from ports/core, ports/dht22, etc.

    /// CorePort: gpio.* and delay.* primitives. Real ports will own
    /// hardware-claimed pins; this stub keeps state in a BTreeMap.
    struct CorePort {
        gpio_state: alloc::rc::Rc<core::cell::RefCell<BTreeMap<u32, bool>>>,
        invocation_log: alloc::rc::Rc<core::cell::RefCell<Vec<String>>>,
    }

    impl CorePort {
        fn new(
            state: alloc::rc::Rc<core::cell::RefCell<BTreeMap<u32, bool>>>,
            log: alloc::rc::Rc<core::cell::RefCell<Vec<String>>>,
        ) -> Self {
            Self {
                gpio_state: state,
                invocation_log: log,
            }
        }
    }

    impl SomaEspPort for CorePort {
        fn port_id(&self) -> &'static str {
            "core"
        }
        fn primitives(&self) -> Vec<CapabilityDescriptor> {
            vec![
                CapabilityDescriptor {
                    skill_id: "gpio.write".to_string(),
                    description: "Set a GPIO pin high or low".to_string(),
                    input_schema: r#"{"pin":"u32","value":"bool"}"#.to_string(),
                    output_schema: r#"{"pin":"u32","value":"bool"}"#.to_string(),
                    effect: Effect::StateMutation,
                },
                CapabilityDescriptor {
                    skill_id: "gpio.read".to_string(),
                    description: "Read the current state of a GPIO pin".to_string(),
                    input_schema: r#"{"pin":"u32"}"#.to_string(),
                    output_schema: r#"{"pin":"u32","value":"bool"}"#.to_string(),
                    effect: Effect::ReadOnly,
                },
                CapabilityDescriptor {
                    skill_id: "delay.ms".to_string(),
                    description: "Sleep for N milliseconds".to_string(),
                    input_schema: r#"{"ms":"u32"}"#.to_string(),
                    output_schema: r#"{"slept_ms":"u32"}"#.to_string(),
                    effect: Effect::ReadOnly,
                },
            ]
        }
        fn invoke(
            &mut self,
            skill_id: &str,
            input: &serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            self.invocation_log.borrow_mut().push(skill_id.to_string());
            match skill_id {
                "gpio.write" => {
                    let pin = input["pin"].as_u64().ok_or("missing pin")? as u32;
                    let value = input["value"].as_bool().ok_or("missing value")?;
                    self.gpio_state.borrow_mut().insert(pin, value);
                    Ok(json!({ "pin": pin, "value": value }))
                }
                "gpio.read" => {
                    let pin = input["pin"].as_u64().ok_or("missing pin")? as u32;
                    let value = self.gpio_state.borrow().get(&pin).copied().unwrap_or(false);
                    Ok(json!({ "pin": pin, "value": value }))
                }
                "delay.ms" => {
                    let ms = input["ms"].as_u64().ok_or("missing ms")? as u32;
                    Ok(json!({ "slept_ms": ms }))
                }
                _ => Err(alloc::format!("unknown primitive: {}", skill_id)),
            }
        }
    }

    /// ThermistorPort: a sensor port with one primitive. Returns simulated
    /// data — a real port crate would use the actual sensor protocol.
    struct ThermistorPort {
        last_temp_c: f64,
    }

    impl SomaEspPort for ThermistorPort {
        fn port_id(&self) -> &'static str {
            "thermistor"
        }
        fn primitives(&self) -> Vec<CapabilityDescriptor> {
            vec![CapabilityDescriptor {
                skill_id: "thermistor.read_temp".to_string(),
                description: "Read temperature from a thermistor on a given ADC channel"
                    .to_string(),
                input_schema: r#"{"channel":"u32"}"#.to_string(),
                output_schema: r#"{"channel":"u32","temp_c":"f64"}"#.to_string(),
                effect: Effect::ReadOnly,
            }]
        }
        fn invoke(
            &mut self,
            skill_id: &str,
            input: &serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            match skill_id {
                "thermistor.read_temp" => {
                    let channel = input["channel"].as_u64().ok_or("missing channel")?;
                    self.last_temp_c += 0.5; // simulated drift
                    Ok(json!({ "channel": channel, "temp_c": self.last_temp_c }))
                }
                _ => Err(alloc::format!("unknown primitive: {}", skill_id)),
            }
        }
    }

    fn make_composite_with_both_ports() -> (
        CompositeDispatcher,
        alloc::rc::Rc<core::cell::RefCell<BTreeMap<u32, bool>>>,
        alloc::rc::Rc<core::cell::RefCell<Vec<String>>>,
    ) {
        let state = alloc::rc::Rc::new(core::cell::RefCell::new(BTreeMap::new()));
        let log = alloc::rc::Rc::new(core::cell::RefCell::new(Vec::new()));
        let mut composite = CompositeDispatcher::new();
        composite.register(alloc::boxed::Box::new(CorePort::new(state.clone(), log.clone())));
        composite.register(alloc::boxed::Box::new(ThermistorPort { last_temp_c: 20.0 }));
        (composite, state, log)
    }

    fn make_blink_routine() -> Routine {
        Routine {
            routine_id: "blink_led".to_string(),
            description: "Blink the LED on GPIO7 once (on, wait, off)".to_string(),
            steps: vec![
                RoutineStep {
                    skill_id: "gpio.write".to_string(),
                    input: json!({ "pin": 7, "value": true }),
                },
                RoutineStep {
                    skill_id: "delay.ms".to_string(),
                    input: json!({ "ms": 500 }),
                },
                RoutineStep {
                    skill_id: "gpio.write".to_string(),
                    input: json!({ "pin": 7, "value": false }),
                },
                RoutineStep {
                    skill_id: "delay.ms".to_string(),
                    input: json!({ "ms": 500 }),
                },
            ],
        }
    }

    #[test]
    fn composite_dispatcher_aggregates_primitives_from_all_ports() {
        let (composite, _state, _log) = make_composite_with_both_ports();
        let prims = composite.list_primitives();
        // CorePort: 3 (gpio.write, gpio.read, delay.ms)
        // ThermistorPort: 1 (thermistor.read_temp)
        assert_eq!(prims.len(), 4);
        assert_eq!(composite.port_count(), 2);
        assert_eq!(composite.port_ids(), vec!["core", "thermistor"]);
    }

    #[test]
    fn composite_routes_skills_to_owning_port() {
        let (mut composite, _state, _log) = make_composite_with_both_ports();
        // gpio.write -> CorePort
        let result = composite
            .invoke("gpio.write", &json!({ "pin": 5, "value": true }))
            .unwrap();
        assert_eq!(result["pin"], json!(5));
        // thermistor.read_temp -> ThermistorPort
        let result = composite
            .invoke("thermistor.read_temp", &json!({ "channel": 0 }))
            .unwrap();
        assert_eq!(result["channel"], json!(0));
        assert!(result["temp_c"].as_f64().unwrap() > 20.0);
    }

    #[test]
    fn composite_unknown_skill_errors() {
        let (mut composite, _, _) = make_composite_with_both_ports();
        let err = composite
            .invoke("not_a_real_skill", &json!({}))
            .unwrap_err();
        assert!(err.contains("no port handles"));
    }

    #[test]
    fn primitives_only_when_no_routines() {
        let (composite, _, _) = make_composite_with_both_ports();
        let mut leaf = LeafState::new(composite);
        let resp = leaf.handle(TransportMessage::ListCapabilities);
        match resp {
            TransportResponse::Capabilities {
                primitives,
                routines,
            } => {
                // 3 from core + 1 from thermistor
                assert_eq!(primitives.len(), 4);
                assert_eq!(routines.len(), 0);
            }
            _ => panic!("expected Capabilities"),
        }
    }

    #[test]
    fn routine_can_call_primitives_across_ports() {
        // A routine that reads the thermistor, then writes a GPIO based on
        // a hardcoded threshold action. This is the killer demo: a routine
        // composes primitives from MULTIPLE ports — exactly what the
        // brain/LLM would do to build cross-sensor behaviors.
        let (composite, state, log) = make_composite_with_both_ports();
        let mut leaf = LeafState::new(composite);

        let cross_port_routine = Routine {
            routine_id: "monitor_temp".to_string(),
            description: "Read temp, then assert GPIO high".to_string(),
            steps: vec![
                RoutineStep {
                    skill_id: "thermistor.read_temp".to_string(),
                    input: json!({ "channel": 0 }),
                },
                RoutineStep {
                    skill_id: "gpio.write".to_string(),
                    input: json!({ "pin": 8, "value": true }),
                },
                RoutineStep {
                    skill_id: "delay.ms".to_string(),
                    input: json!({ "ms": 100 }),
                },
            ],
        };
        leaf.handle(TransportMessage::TransferRoutine {
            routine: cross_port_routine,
        });

        let resp = leaf.handle(TransportMessage::InvokeSkill {
            peer_id: "server".to_string(),
            skill_id: "monitor_temp".to_string(),
            input: json!({}),
        });

        match resp {
            TransportResponse::SkillResult { response } => {
                assert!(response.success);
                assert_eq!(response.steps_executed, 3);
            }
            _ => panic!("expected SkillResult"),
        }

        // Verify the cross-port effects actually happened
        assert_eq!(state.borrow().get(&8), Some(&true));
        // CorePort log shows gpio.write and delay.ms (thermistor doesn't log)
        let core_log = log.borrow();
        assert!(core_log.contains(&"gpio.write".to_string()));
        assert!(core_log.contains(&"delay.ms".to_string()));
    }

    #[test]
    fn transfer_routine_then_invoke_walks_steps() {
        let (composite, state, log) = make_composite_with_both_ports();
        let mut leaf = LeafState::new(composite);

        // Brain transfers a routine
        let resp = leaf.handle(TransportMessage::TransferRoutine {
            routine: make_blink_routine(),
        });
        match resp {
            TransportResponse::RoutineStored {
                routine_id,
                step_count,
            } => {
                assert_eq!(routine_id, "blink_led");
                assert_eq!(step_count, 4);
            }
            _ => panic!("expected RoutineStored"),
        }

        // Now ListCapabilities shows it (4 primitives across both ports + the routine)
        let resp = leaf.handle(TransportMessage::ListCapabilities);
        match resp {
            TransportResponse::Capabilities {
                primitives,
                routines,
            } => {
                assert_eq!(primitives.len(), 4);
                assert_eq!(routines.len(), 1);
                assert_eq!(routines[0].routine_id, "blink_led");
                assert_eq!(routines[0].step_count, 4);
            }
            _ => panic!("expected Capabilities"),
        }

        // Brain invokes it
        let resp = leaf.handle(TransportMessage::InvokeSkill {
            peer_id: "server".to_string(),
            skill_id: "blink_led".to_string(),
            input: json!({}),
        });
        match resp {
            TransportResponse::SkillResult { response } => {
                assert!(response.success);
                assert_eq!(response.skill_id, "blink_led");
                assert_eq!(response.steps_executed, 4);
            }
            _ => panic!("expected SkillResult"),
        }

        // Verify the dispatcher actually saw all 4 primitive calls
        // CorePort log shows gpio.write and delay.ms invocations
        assert_eq!(
            *log.borrow(),
            vec!["gpio.write", "delay.ms", "gpio.write", "delay.ms"]
        );

        // And gpio7 ended low (the routine ends with value: false)
        assert_eq!(state.borrow().get(&7), Some(&false));
    }

    #[test]
    fn invoke_primitive_directly_works() {
        let (composite, state, _log) = make_composite_with_both_ports();
        let mut leaf = LeafState::new(composite);
        let resp = leaf.handle(TransportMessage::InvokeSkill {
            peer_id: "server".to_string(),
            skill_id: "gpio.write".to_string(),
            input: json!({ "pin": 5, "value": true }),
        });
        match resp {
            TransportResponse::SkillResult { response } => {
                assert!(response.success);
                assert_eq!(response.skill_id, "gpio.write");
                assert_eq!(response.steps_executed, 1);
            }
            _ => panic!("expected SkillResult"),
        }
        assert_eq!(state.borrow().get(&5), Some(&true));
    }

    #[test]
    fn routine_failure_stops_walking() {
        let (composite, state, _log) = make_composite_with_both_ports();
        let mut leaf = LeafState::new(composite);
        let bad_routine = Routine {
            routine_id: "bad".to_string(),
            description: "Routine with a step that fails".to_string(),
            steps: vec![
                RoutineStep {
                    skill_id: "gpio.write".to_string(),
                    input: json!({ "pin": 1, "value": true }),
                },
                RoutineStep {
                    skill_id: "nonexistent.primitive".to_string(),
                    input: json!({}),
                },
                // This step should NOT execute because step 1 fails
                RoutineStep {
                    skill_id: "gpio.write".to_string(),
                    input: json!({ "pin": 2, "value": true }),
                },
            ],
        };
        leaf.handle(TransportMessage::TransferRoutine {
            routine: bad_routine,
        });

        let resp = leaf.handle(TransportMessage::InvokeSkill {
            peer_id: "s".to_string(),
            skill_id: "bad".to_string(),
            input: json!({}),
        });
        match resp {
            TransportResponse::SkillResult { response } => {
                assert!(!response.success);
                assert_eq!(response.steps_executed, 1); // step 0 succeeded, step 1 failed
            }
            _ => panic!("expected SkillResult"),
        }
        // gpio2 was never written because the routine failed before reaching step 2
        assert!(!state.borrow().contains_key(&2));
    }

    #[test]
    fn remove_routine_works() {
        let (composite, _, _) = make_composite_with_both_ports();
        let mut leaf = LeafState::new(composite);
        leaf.handle(TransportMessage::TransferRoutine {
            routine: make_blink_routine(),
        });
        assert_eq!(leaf.routine_count(), 1);

        let resp = leaf.handle(TransportMessage::RemoveRoutine {
            routine_id: "blink_led".to_string(),
        });
        match resp {
            TransportResponse::RoutineRemoved { routine_id } => {
                assert_eq!(routine_id, "blink_led");
            }
            _ => panic!("expected RoutineRemoved"),
        }
        assert_eq!(leaf.routine_count(), 0);
    }

    #[test]
    fn routine_can_shadow_primitive() {
        let (composite, state, _) = make_composite_with_both_ports();
        let mut leaf = LeafState::new(composite);
        // Transfer a routine named "delay.ms" that calls gpio.write instead.
        let shadow = Routine {
            routine_id: "delay.ms".to_string(),
            description: "shadows delay.ms with a gpio.write".to_string(),
            steps: vec![RoutineStep {
                skill_id: "gpio.write".to_string(),
                input: json!({ "pin": 99, "value": true }),
            }],
        };
        leaf.handle(TransportMessage::TransferRoutine { routine: shadow });

        // Invoking "delay.ms" now hits the routine, not the primitive.
        leaf.handle(TransportMessage::InvokeSkill {
            peer_id: "s".to_string(),
            skill_id: "delay.ms".to_string(),
            input: json!({ "ms": 100 }),
        });
        assert_eq!(state.borrow().get(&99), Some(&true));
    }

    #[test]
    fn ping_returns_pong() {
        let (composite, _, _) = make_composite_with_both_ports();
        let mut leaf = LeafState::new(composite);
        let resp = leaf.handle(TransportMessage::Ping { nonce: 42 });
        match resp {
            TransportResponse::Pong { nonce, load: _ } => assert_eq!(nonce, 42),
            _ => panic!("expected Pong"),
        }
    }

    #[test]
    fn frame_round_trip_transfer_routine() {
        let msg = TransportMessage::TransferRoutine {
            routine: make_blink_routine(),
        };
        let bytes = encode_message(&msg).unwrap();
        let (decoded, consumed) = decode_frame(&bytes, 16 * 1024).unwrap();
        assert_eq!(consumed, bytes.len());
        match decoded {
            TransportMessage::TransferRoutine { routine } => {
                assert_eq!(routine.routine_id, "blink_led");
                assert_eq!(routine.steps.len(), 4);
            }
            _ => panic!("decode mismatch"),
        }
    }

    #[test]
    fn decode_frame_needs_more_when_short() {
        let buf = [0u8, 0, 0, 100, 1, 2, 3];
        match decode_frame(&buf, 1024) {
            Err(FrameError::NeedMore) => {}
            other => panic!("expected NeedMore, got {other:?}"),
        }
    }

    #[test]
    fn decode_frame_rejects_too_large() {
        let buf = [0u8, 0, 0xff, 0xff, 0, 0, 0, 0];
        match decode_frame(&buf, 1024) {
            Err(FrameError::TooLarge) => {}
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }
}
