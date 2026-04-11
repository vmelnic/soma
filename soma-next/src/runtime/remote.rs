//! Remote executor trait and response types.
//!
//! These are the *interface* types for the distributed runtime: the trait
//! that transports implement, plus the response envelopes carried back to
//! callers. The actual transport implementations (TCP, TLS, WebSocket, Unix
//! socket), authentication, rate limiting, and so on live in the
//! `distributed` module, which is gated behind the `distributed` feature.
//!
//! Everything in this file is pure compute with no native-only dependencies
//! (no sockets, no I/O, no TLS), so it compiles on every target including
//! `wasm32-unknown-unknown`. That lets types like
//! `SessionControllerDeps.remote_executor: Option<Box<dyn RemoteExecutor>>`
//! exist on wasm even though no implementation is available there — the
//! field is simply always `None` in that build.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::Result;
use crate::types::peer::{RemoteGoalRequest, RoutineTransfer, SchemaTransfer};

// --- RemoteInvocationContext ---

/// Optional context for remote skill invocations that carries session policy
/// and budget state. When provided to `ValidatingRemoteExecutor`, these fields
/// are checked before the invocation is forwarded to the inner executor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteInvocationContext {
    /// Remaining session budget. If present and <= 0, the invocation is
    /// rejected with BudgetExhaustion before any network call.
    pub session_budget_remaining: Option<f64>,
    /// Whether the caller's policy runtime allows this invocation.
    /// Set to false to block the call with a PolicyViolation error.
    pub policy_allows: bool,
}

// --- Response types ---

/// Status of a remote goal submission — 3 possible responses from distributed.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteGoalStatus {
    /// Peer accepted and created a remote session.
    Accepted,
    /// Peer rejected with a structured reason.
    Rejected,
    /// Peer requests stricter policy or more budget.
    RequestStricterPolicy,
}

/// Response to a remote goal submission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteGoalResponse {
    pub status: RemoteGoalStatus,
    pub session_id: Option<String>,
    pub reason: Option<String>,
    /// When status is RequestStricterPolicy, the required budget/policy details.
    pub required_adjustments: Option<serde_json::Value>,
}

/// Response to a remote skill invocation, carrying the structured observation.
/// Preserves audit traceability per distributed.md.
///
/// Deserialization is lenient on several fields so this struct can also
/// accept responses from smaller peers (embedded leaf nodes like the ESP32
/// firmware) that use a flatter wire format:
///   - `peer_id` defaults to "" (the executor fills it in post-receive)
///   - `observation` also accepts the legacy/leaf field name `structured_result`
///   - `timestamp` defaults to `Utc::now()` at deserialize time
///   - `trace_id` defaults to `Uuid::nil()`
///
/// The ESP32 leaf additionally sends `failure_message` and `steps_executed`
/// which serde silently ignores (no `deny_unknown_fields`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSkillResponse {
    pub skill_id: String,
    #[serde(default)]
    pub peer_id: String,
    pub success: bool,
    #[serde(alias = "structured_result")]
    pub observation: serde_json::Value,
    pub latency_ms: u64,
    #[serde(default = "default_remote_skill_timestamp")]
    pub timestamp: DateTime<Utc>,
    /// Trace ID for audit traceability.
    #[serde(default)]
    pub trace_id: Uuid,
}

fn default_remote_skill_timestamp() -> DateTime<Utc> {
    Utc::now()
}

/// Response to a remote resource query — includes snapshot or delta,
/// version, provenance, timestamp, freshness per distributed.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteResourceResponse {
    pub resource_type: String,
    pub resource_id: String,
    /// Snapshot or delta content.
    pub data: serde_json::Value,
    /// Whether this is a snapshot or delta.
    pub data_mode: ResourceDataMode,
    pub version: u64,
    pub provenance: String,
    pub freshness_ms: u64,
    pub timestamp: DateTime<Utc>,
}

/// Whether resource data is a snapshot or delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceDataMode {
    Snapshot,
    Delta,
}

// --- RemoteExecutor trait ---

/// Remote execution interface for submitting goals, invoking skills,
/// querying resources, and transferring schemas/routines on remote peers.
pub trait RemoteExecutor: Send + Sync {
    /// Submit a goal to a remote peer using structured RemoteGoalRequest.
    /// The peer may accept, reject, or request stricter policy.
    fn submit_goal(&self, peer_id: &str, request: &RemoteGoalRequest)
        -> Result<RemoteGoalResponse>;

    /// Invoke a single remote skill. MUST validate trust, policy, skill
    /// availability, input binding, and return a structured observation
    /// with audit traceability.
    fn invoke_skill(
        &self,
        peer_id: &str,
        skill_id: &str,
        input: serde_json::Value,
    ) -> Result<RemoteSkillResponse>;

    /// Query a resource on a remote peer. Returns snapshot or delta with
    /// version, provenance, and freshness information.
    fn query_resource(
        &self,
        peer_id: &str,
        resource_type: &str,
        resource_id: &str,
    ) -> Result<RemoteResourceResponse>;

    /// Transfer a schema to a remote peer. MUST respect trust, pack policy,
    /// exposure, confidentiality, and replay protection constraints.
    fn transfer_schema(&self, peer_id: &str, schema: &SchemaTransfer) -> Result<()>;

    /// Transfer a routine to a remote peer. MUST respect trust, pack policy,
    /// exposure, confidentiality, and replay protection constraints.
    fn transfer_routine(&self, peer_id: &str, routine: &RoutineTransfer) -> Result<()>;
}
