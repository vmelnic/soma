use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::common::{AuthOutcome, CostClass, CostProfile, PolicyOutcome, PortFailureClass, SandboxOutcome, SkillFailureClass};

pub fn default_cost_profile() -> CostProfile {
    CostProfile {
        cpu_cost_class: CostClass::Negligible,
        memory_cost_class: CostClass::Negligible,
        io_cost_class: CostClass::Negligible,
        network_cost_class: CostClass::Negligible,
        energy_cost_class: CostClass::Negligible,
    }
}

fn latency_bucket(ms: u64) -> CostClass {
    match ms {
        0..=10 => CostClass::Negligible,
        11..=100 => CostClass::Low,
        101..=1_000 => CostClass::Medium,
        1_001..=10_000 => CostClass::High,
        _ => CostClass::Extreme,
    }
}

fn payload_bucket(bytes: usize) -> CostClass {
    match bytes {
        0..=1_024 => CostClass::Negligible,
        1_025..=65_536 => CostClass::Low,
        65_537..=1_048_576 => CostClass::Medium,
        1_048_577..=10_485_760 => CostClass::High,
        _ => CostClass::Extreme,
    }
}

fn estimate_payload_bytes(record: &PortCallRecord) -> usize {
    serde_json::to_vec(&record.raw_result).map(|v| v.len()).unwrap_or(0)
        + serde_json::to_vec(&record.structured_result)
            .map(|v| v.len())
            .unwrap_or(0)
}

/// Derive a `CostProfile` from an actual `PortCallRecord` so the body
/// reports observed cost rather than the all-Negligible default. Latency
/// drives cpu/memory/energy buckets; payload size drives io/network.
/// Ports that already publish per-class data should keep their override
/// — this is the fallback used by the adapter boundary.
pub fn cost_from_port_record(record: &PortCallRecord) -> CostProfile {
    let lat = latency_bucket(record.latency_ms);
    let bytes = payload_bucket(estimate_payload_bytes(record));
    CostProfile {
        cpu_cost_class: lat,
        memory_cost_class: bytes,
        io_cost_class: bytes,
        network_cost_class: bytes,
        energy_cost_class: lat,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(latency_ms: u64, payload: serde_json::Value) -> PortCallRecord {
        PortCallRecord {
            observation_id: Uuid::new_v4(),
            port_id: "test".into(),
            capability_id: "x".into(),
            invocation_id: Uuid::new_v4(),
            success: true,
            failure_class: None,
            raw_result: payload,
            structured_result: serde_json::Value::Null,
            effect_patch: None,
            side_effect_summary: None,
            latency_ms,
            resource_cost: 0.0,
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
        }
    }

    #[test]
    fn quick_small_call_is_negligible() {
        let p = cost_from_port_record(&rec(5, serde_json::json!({"k": 1})));
        assert_eq!(p.cpu_cost_class, CostClass::Negligible);
        assert_eq!(p.io_cost_class, CostClass::Negligible);
    }

    #[test]
    fn slow_call_buckets_to_high_or_extreme() {
        let p = cost_from_port_record(&rec(5_000, serde_json::Value::Null));
        assert_eq!(p.cpu_cost_class, CostClass::High);
        let p2 = cost_from_port_record(&rec(60_000, serde_json::Value::Null));
        assert_eq!(p2.cpu_cost_class, CostClass::Extreme);
    }

    #[test]
    fn large_payload_drives_io_class() {
        let big = serde_json::Value::String("x".repeat(200_000));
        let p = cost_from_port_record(&rec(5, big));
        assert_eq!(p.io_cost_class, CostClass::Medium);
        assert_eq!(p.network_cost_class, CostClass::Medium);
        assert_eq!(p.cpu_cost_class, CostClass::Negligible);
    }
}

/// Observation — structured result from skill or port execution.
/// Every invocation MUST emit an Observation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub observation_id: Uuid,
    pub session_id: Uuid,
    pub skill_id: Option<String>,
    pub port_calls: Vec<PortCallRecord>,
    pub raw_result: serde_json::Value,
    pub structured_result: serde_json::Value,
    pub effect_patch: Option<serde_json::Value>,
    pub success: bool,
    pub failure_class: Option<SkillFailureClass>,
    /// Structured failure detail with cause-specific context. The brain
    /// consumes this to decide retry vs. abandon vs. switch-skill without
    /// parsing free-form error strings. Always None when success=true.
    #[serde(default)]
    pub failure_detail: Option<FailureDetail>,
    pub latency_ms: u64,
    #[serde(default = "default_cost_profile")]
    pub resource_cost: CostProfile,
    pub confidence: f64,
    pub timestamp: DateTime<Utc>,
}

/// Typed cause + context for a failed observation. Brain reads this to
/// route the next decision (retry, switch skill, ask user, abandon).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum FailureDetail {
    /// A required input binding could not be resolved.
    BindingMissing {
        binding_name: String,
        message: String,
    },
    /// A skill precondition evaluated to false before execution.
    PreconditionFailed {
        precondition: String,
        message: String,
    },
    /// Policy engine denied the action.
    PolicyDenied {
        rule: String,
        message: String,
    },
    /// The hard latency cap fired before the call returned.
    Timeout {
        budget_ms: u64,
        elapsed_ms: u64,
    },
    /// Port-level failure surfaced upward as a skill failure.
    PortFailure {
        port_id: String,
        capability_id: String,
        port_class: PortFailureClass,
        message: String,
    },
    /// Remote/delegated execution failed.
    RemoteFailure {
        peer_id: Option<String>,
        message: String,
    },
    /// Session budget hit zero mid-run (post-deduction).
    BudgetExhausted {
        dimension: String,
    },
    /// Anything else; carries the original message verbatim.
    Other {
        message: String,
    },
}

impl FailureDetail {
    pub fn message(&self) -> &str {
        match self {
            FailureDetail::BindingMissing { message, .. }
            | FailureDetail::PreconditionFailed { message, .. }
            | FailureDetail::PolicyDenied { message, .. }
            | FailureDetail::PortFailure { message, .. }
            | FailureDetail::RemoteFailure { message, .. }
            | FailureDetail::Other { message } => message,
            FailureDetail::Timeout { budget_ms, elapsed_ms } => {
                // No message field; return empty so the caller can
                // format its own string from the numeric fields.
                let _ = (budget_ms, elapsed_ms);
                ""
            }
            FailureDetail::BudgetExhausted { dimension } => dimension,
        }
    }

    pub fn class(&self) -> SkillFailureClass {
        match self {
            FailureDetail::BindingMissing { .. } => SkillFailureClass::BindingFailure,
            FailureDetail::PreconditionFailed { .. } => SkillFailureClass::PreconditionFailure,
            FailureDetail::PolicyDenied { .. } => SkillFailureClass::PolicyDenial,
            FailureDetail::Timeout { .. } => SkillFailureClass::Timeout,
            FailureDetail::PortFailure { .. } => SkillFailureClass::PortFailure,
            FailureDetail::RemoteFailure { .. } => SkillFailureClass::RemoteFailure,
            FailureDetail::BudgetExhausted { .. } => SkillFailureClass::BudgetExhaustion,
            FailureDetail::Other { .. } => SkillFailureClass::Unknown,
        }
    }
}

/// Record of a single port call within an observation.
///
/// Per port-spec.md, every invocation MUST emit an observation containing
/// all of these fields. The `observation_id` is the unique identity of this
/// observation record (distinct from `invocation_id` which identifies the
/// request). `retry_safe` reports whether recovery or retry is safe per the
/// failure model. `input_hash` captures a SHA-256 digest of the input for
/// tracing obligations without storing the full payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortCallRecord {
    pub observation_id: Uuid,
    pub port_id: String,
    pub capability_id: String,
    pub invocation_id: Uuid,
    pub success: bool,
    pub failure_class: Option<PortFailureClass>,
    pub raw_result: serde_json::Value,
    pub structured_result: serde_json::Value,
    pub effect_patch: Option<serde_json::Value>,
    pub side_effect_summary: Option<String>,
    pub latency_ms: u64,
    pub resource_cost: f64,
    pub confidence: f64,
    pub timestamp: DateTime<Utc>,
    /// Whether recovery or retry is safe (failure model requirement).
    pub retry_safe: bool,
    /// SHA-256 hex digest of the invocation input (tracing obligation).
    pub input_hash: Option<String>,
    // --- Tracing obligation fields (port-spec.md Tracing Obligations) ---
    /// Session that triggered this invocation.
    pub session_id: Option<Uuid>,
    /// Goal associated with the session at invocation time.
    pub goal_id: Option<String>,
    /// Identity of the caller (local session or peer identity for remote calls).
    pub caller_identity: Option<String>,
    /// Outcome of the auth check performed before invocation.
    pub auth_result: Option<AuthOutcome>,
    /// Outcome of the policy check performed before invocation.
    pub policy_result: Option<PolicyOutcome>,
    /// Outcome of the sandbox constraint check.
    pub sandbox_result: Option<SandboxOutcome>,
}
