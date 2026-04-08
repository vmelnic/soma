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
    pub latency_ms: u64,
    #[serde(default = "default_cost_profile")]
    pub resource_cost: CostProfile,
    pub confidence: f64,
    pub timestamp: DateTime<Utc>,
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
