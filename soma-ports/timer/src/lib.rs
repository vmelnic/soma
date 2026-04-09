//! SOMA Timer Port -- timer/scheduler capabilities.
//!
//! Four capabilities:
//!
//! | ID | Name            | Description                                          |
//! |----|-----------------|------------------------------------------------------|
//! | 0  | `set_timeout`   | Set a one-shot timer that fires after a delay        |
//! | 1  | `set_interval`  | Set a recurring timer that fires at regular intervals |
//! | 2  | `cancel_timer`  | Cancel an active timer by ID                         |
//! | 3  | `list_active`   | List all active timers with remaining time           |
//!
//! Pure state machine -- stores timer entries in a HashMap keyed by UUID.
//! No external services or async runtime needed.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use soma_port_sdk::prelude::*;
use uuid::Uuid;

const PORT_ID: &str = "soma.timer";

// ---------------------------------------------------------------------------
// Timer types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimerKind {
    Timeout,
    Interval,
}

#[derive(Debug, Clone)]
struct TimerEntry {
    id: String,
    label: String,
    kind: TimerKind,
    interval_ms: u64,
    /// Retained for runtime diagnostics (e.g., timer age reporting).
    #[allow(dead_code)]
    created_at: Instant,
    next_fire: Instant,
}

// ---------------------------------------------------------------------------
// Port struct
// ---------------------------------------------------------------------------

pub struct TimerPort {
    spec: PortSpec,
    state: Mutex<TimerState>,
}

struct TimerState {
    timers: HashMap<String, TimerEntry>,
}

impl TimerPort {
    pub fn new() -> Self {
        Self {
            spec: build_spec(),
            state: Mutex::new(TimerState {
                timers: HashMap::new(),
            }),
        }
    }
}

impl Default for TimerPort {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Port trait implementation
// ---------------------------------------------------------------------------

impl Port for TimerPort {
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
            "set_timeout" => self.set_timeout(&input),
            "set_interval" => self.set_interval(&input),
            "cancel_timer" => self.cancel_timer(&input),
            "list_active" => self.list_active(),
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
            "set_timeout" | "set_interval" => {
                require_field(input, "label")?;
                require_field(input, "delay_ms")?;
            }
            "cancel_timer" => {
                require_field(input, "timer_id")?;
            }
            "list_active" => {}
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

impl TimerPort {
    fn set_timeout(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let label = get_str(input, "label")?;
        let delay_ms = get_u64(input, "delay_ms")?;

        if delay_ms == 0 {
            return Err(PortError::Validation("delay_ms must be positive".into()));
        }

        let now = Instant::now();
        let id = Uuid::new_v4().to_string();

        let mut state = self.state.lock().unwrap();
        state.timers.insert(
            id.clone(),
            TimerEntry {
                id: id.clone(),
                label: label.to_string(),
                kind: TimerKind::Timeout,
                interval_ms: delay_ms,
                created_at: now,
                next_fire: now + std::time::Duration::from_millis(delay_ms),
            },
        );

        Ok(serde_json::json!({
            "timer_id": id,
            "label": label,
            "type": "timeout",
            "delay_ms": delay_ms,
        }))
    }

    fn set_interval(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let label = get_str(input, "label")?;
        let delay_ms = get_u64(input, "delay_ms")?;

        if delay_ms == 0 {
            return Err(PortError::Validation("delay_ms must be positive".into()));
        }

        let now = Instant::now();
        let id = Uuid::new_v4().to_string();

        let mut state = self.state.lock().unwrap();
        state.timers.insert(
            id.clone(),
            TimerEntry {
                id: id.clone(),
                label: label.to_string(),
                kind: TimerKind::Interval,
                interval_ms: delay_ms,
                created_at: now,
                next_fire: now + std::time::Duration::from_millis(delay_ms),
            },
        );

        Ok(serde_json::json!({
            "timer_id": id,
            "label": label,
            "type": "interval",
            "interval_ms": delay_ms,
        }))
    }

    fn cancel_timer(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let timer_id = get_str(input, "timer_id")?;

        let mut state = self.state.lock().unwrap();
        let removed = state.timers.remove(timer_id).is_some();

        Ok(serde_json::json!({
            "cancelled": removed,
            "timer_id": timer_id,
        }))
    }

    fn list_active(&self) -> soma_port_sdk::Result<serde_json::Value> {
        let state = self.state.lock().unwrap();
        let now = Instant::now();

        let mut entries: Vec<serde_json::Value> = state
            .timers
            .values()
            .map(|entry| {
                let remaining_ms = if entry.next_fire > now {
                    entry.next_fire.duration_since(now).as_millis() as u64
                } else {
                    0
                };

                serde_json::json!({
                    "timer_id": entry.id,
                    "label": entry.label,
                    "type": match entry.kind {
                        TimerKind::Timeout => "timeout",
                        TimerKind::Interval => "interval",
                    },
                    "interval_ms": entry.interval_ms,
                    "remaining_ms": remaining_ms,
                })
            })
            .collect();

        entries.sort_by_key(|e| e["timer_id"].as_str().unwrap_or("").to_string());

        Ok(serde_json::json!({
            "timers": entries,
            "count": entries.len(),
        }))
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

fn get_u64(input: &serde_json::Value, field: &str) -> soma_port_sdk::Result<u64> {
    input[field]
        .as_u64()
        .ok_or_else(|| PortError::Validation(format!("{field} must be a positive integer")))
}

// ---------------------------------------------------------------------------
// Spec builder
// ---------------------------------------------------------------------------

fn build_spec() -> PortSpec {
    PortSpec {
        port_id: PORT_ID.into(),
        name: "timer".into(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Custom,
        description: "Timer/scheduler: timeouts, intervals, cancellation, and listing".into(),
        namespace: "soma.timer".into(),
        trust_level: TrustLevel::BuiltIn,
        capabilities: vec![
            PortCapabilitySpec {
                capability_id: "set_timeout".into(),
                name: "set_timeout".into(),
                purpose: "Set a one-shot timer that fires after a delay".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "label": {"type": "string"}, "delay_ms": {"type": "integer"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "timer_id": {"type": "string"}, "label": {"type": "string"},
                    "type": {"type": "string"}, "delay_ms": {"type": "integer"},
                })),
                effect_class: SideEffectClass::LocalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 1,
                    p95_latency_ms: 5,
                    max_latency_ms: 10,
                },
                cost_profile: CostProfile::default(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "set_interval".into(),
                name: "set_interval".into(),
                purpose: "Set a recurring timer that fires at regular intervals".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "label": {"type": "string"}, "delay_ms": {"type": "integer"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "timer_id": {"type": "string"}, "label": {"type": "string"},
                    "type": {"type": "string"}, "interval_ms": {"type": "integer"},
                })),
                effect_class: SideEffectClass::LocalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 1,
                    p95_latency_ms: 5,
                    max_latency_ms: 10,
                },
                cost_profile: CostProfile::default(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "cancel_timer".into(),
                name: "cancel_timer".into(),
                purpose: "Cancel an active timer by ID".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "timer_id": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "cancelled": {"type": "boolean"},
                })),
                effect_class: SideEffectClass::LocalStateMutation,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 1,
                    p95_latency_ms: 5,
                    max_latency_ms: 10,
                },
                cost_profile: CostProfile::default(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "list_active".into(),
                name: "list_active".into(),
                purpose: "List all active timers with remaining time".into(),
                input_schema: SchemaRef::any(),
                output_schema: SchemaRef::object(serde_json::json!({
                    "timers": {"type": "array"}, "count": {"type": "integer"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 1,
                    p95_latency_ms: 5,
                    max_latency_ms: 10,
                },
                cost_profile: CostProfile::default(),
                remote_exposable: false,
                auth_override: None,
            },
        ],
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::any(),
        failure_modes: vec![PortFailureClass::ValidationError],
        side_effect_class: SideEffectClass::LocalStateMutation,
        latency_profile: LatencyProfile {
            expected_latency_ms: 1,
            p95_latency_ms: 5,
            max_latency_ms: 10,
        },
        cost_profile: CostProfile::default(),
        auth_requirements: AuthRequirements::default(),
        sandbox_requirements: SandboxRequirements::default(),
        observable_fields: vec!["timer_id".into(), "label".into(), "type".into()],
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
    Box::into_raw(Box::new(TimerPort::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec() {
        let port = TimerPort::new();
        assert_eq!(port.spec().port_id, "soma.timer");
        assert_eq!(port.spec().capabilities.len(), 4);
    }

    #[test]
    fn test_set_timeout() {
        let port = TimerPort::new();
        let record = port
            .invoke(
                "set_timeout",
                serde_json::json!({
                    "label": "test-timeout", "delay_ms": 1000
                }),
            )
            .unwrap();
        assert!(record.success);
        assert_eq!(record.raw_result["type"], "timeout");
        assert!(record.raw_result["timer_id"].is_string());
    }

    #[test]
    fn test_set_interval() {
        let port = TimerPort::new();
        let record = port
            .invoke(
                "set_interval",
                serde_json::json!({
                    "label": "heartbeat", "delay_ms": 500
                }),
            )
            .unwrap();
        assert!(record.success);
        assert_eq!(record.raw_result["type"], "interval");
    }

    #[test]
    fn test_cancel_timer() {
        let port = TimerPort::new();

        let record = port
            .invoke(
                "set_timeout",
                serde_json::json!({
                    "label": "cancel-me", "delay_ms": 5000
                }),
            )
            .unwrap();
        let timer_id = record.raw_result["timer_id"].as_str().unwrap().to_string();

        let cancel = port
            .invoke(
                "cancel_timer",
                serde_json::json!({
                    "timer_id": timer_id
                }),
            )
            .unwrap();
        assert!(cancel.success);
        assert_eq!(cancel.raw_result["cancelled"], true);
    }

    #[test]
    fn test_cancel_nonexistent() {
        let port = TimerPort::new();
        let cancel = port
            .invoke(
                "cancel_timer",
                serde_json::json!({
                    "timer_id": "does-not-exist"
                }),
            )
            .unwrap();
        assert!(cancel.success);
        assert_eq!(cancel.raw_result["cancelled"], false);
    }

    #[test]
    fn test_list_active() {
        let port = TimerPort::new();
        port.invoke(
            "set_timeout",
            serde_json::json!({
                "label": "t1", "delay_ms": 10000
            }),
        )
        .unwrap();
        port.invoke(
            "set_interval",
            serde_json::json!({
                "label": "t2", "delay_ms": 5000
            }),
        )
        .unwrap();

        let list = port.invoke("list_active", serde_json::json!({})).unwrap();
        assert!(list.success);
        assert_eq!(list.raw_result["count"], 2);
    }

    #[test]
    fn test_zero_delay_rejected() {
        let port = TimerPort::new();
        let record = port
            .invoke(
                "set_timeout",
                serde_json::json!({
                    "label": "zero", "delay_ms": 0
                }),
            )
            .unwrap();
        assert!(!record.success);
    }
}
