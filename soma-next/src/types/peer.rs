use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::common::{
    DeterminismClass, EffectDescriptor, LatencyProfile, Precondition, RiskClass,
    RollbackSupport, SchemaRef, TerminationCondition, TrustLevel,
};
use super::skill::ObservableDecl;

/// PeerAvailability from distributed.md.
/// 6 availability states: available, degraded, busy, offline, untrusted, restricted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerAvailability {
    Available,
    Degraded,
    Busy,
    Offline,
    Untrusted,
    Restricted,
}

/// PeerSpec — required peer identity with all 12 fields from distributed.md.
/// Fields: identity, version, trust_class, transports, endpoints,
/// availability, policy_limits, packs, skills, resources, latency_class,
/// plus cost_class for the cost/latency profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerSpec {
    pub peer_id: String,
    pub version: String,
    pub trust_class: TrustLevel,
    pub supported_transports: Vec<Transport>,
    pub reachable_endpoints: Vec<String>,
    pub current_availability: PeerAvailability,
    pub policy_limits: Vec<String>,
    pub exposed_packs: Vec<String>,
    pub exposed_skills: Vec<RemoteSkillAd>,
    pub exposed_resources: Vec<RemoteResourceAd>,
    pub latency_class: String,
    pub cost_class: String,
    /// Current load on this peer (0.0 = idle, 1.0 = fully loaded).
    #[serde(default)]
    pub current_load: f64,
    /// Last time this peer was seen (heartbeat, advertisement, or response).
    #[serde(default = "chrono::Utc::now")]
    pub last_seen: DateTime<Utc>,
    pub replay_support: bool,
    pub observation_streaming: bool,
    pub advertisement_version: u64,
    /// Expiry timestamp for this advertisement. Advertisements MUST be cacheable with expiration.
    pub advertisement_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Tcp,
    WebSocket,
    UnixSocket,
    Quic,
    Http,
}

/// Remote skill advertisement — 13 fields required by distributed.md.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteSkillAd {
    pub skill_id: String,
    pub name: String,
    pub kind: String,
    pub inputs: SchemaRef,
    pub outputs: SchemaRef,
    pub preconditions: Vec<Precondition>,
    pub expected_effects: Vec<EffectDescriptor>,
    pub observables: Vec<ObservableDecl>,
    pub termination_conditions: Vec<TerminationCondition>,
    pub rollback_or_compensation: RollbackSupport,
    pub cost_prior: LatencyProfile,
    pub risk_class: RiskClass,
    pub determinism: DeterminismClass,
}

/// Remote resource advertisement — 9 fields required by distributed.md.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteResourceAd {
    pub resource_type: String,
    pub resource_id: String,
    pub version: u64,
    pub visibility: String,
    pub access_mode: AccessMode,
    pub mutation_mode: MutationMode,
    pub sync_mode: String,
    pub provenance: String,
    pub staleness_bounds_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    ReadOnly,
    ReadWrite,
    WriteOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationMode {
    Immutable,
    Append,
    Replace,
    Patch,
}

/// Remote goal submission — structured fields from distributed.md.
/// goal description, constraints, budgets, trust expectations, result/trace request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteGoalRequest {
    pub goal: serde_json::Value,
    pub constraints: Vec<String>,
    pub budgets: RemoteBudget,
    pub trust_required: TrustLevel,
    pub request_result: bool,
    pub request_trace: bool,
}

/// Budget constraints for remote goal submission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteBudget {
    pub risk_limit: f64,
    pub latency_limit_ms: u64,
    pub resource_limit: f64,
    pub step_limit: u32,
}

/// Session migration data — 8 required fields from distributed.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMigrationData {
    pub session_id: Uuid,
    pub goal: serde_json::Value,
    pub working_memory: serde_json::Value,
    pub belief_summary: serde_json::Value,
    pub pending_observations: Vec<serde_json::Value>,
    pub current_budget: RemoteBudget,
    pub trace_cursor: u64,
    pub policy_context: serde_json::Value,
}

/// Schema transfer payload — 7 fields from distributed.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaTransfer {
    pub schema_id: String,
    pub version: String,
    pub trigger_conditions: Vec<Precondition>,
    pub subgoal_structure: Vec<serde_json::Value>,
    pub candidate_skill_ordering: Vec<String>,
    pub stop_conditions: Vec<Precondition>,
    pub confidence: f64,
}

/// Routine transfer payload — 7 fields from distributed.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineTransfer {
    pub routine_id: String,
    pub match_conditions: Vec<Precondition>,
    pub compiled_skill_path: Vec<String>,
    #[serde(default)]
    pub compiled_steps: Vec<super::routine::CompiledStep>,
    pub guard_conditions: Vec<Precondition>,
    pub expected_cost: f64,
    pub expected_effect: Vec<EffectDescriptor>,
    pub confidence: f64,
    #[serde(default)]
    pub autonomous: bool,
    #[serde(default)]
    pub priority: u32,
    #[serde(default)]
    pub exclusive: bool,
    #[serde(default)]
    pub policy_scope: Option<String>,
    #[serde(default)]
    pub version: u32,
}

/// Observation in a distributed stream — 10 required fields from distributed.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamedObservation {
    pub session_id: Uuid,
    pub step_id: String,
    pub source_peer: String,
    pub skill_or_resource_ref: String,
    pub raw_result: serde_json::Value,
    pub structured_result: serde_json::Value,
    pub effect_patch: Option<serde_json::Value>,
    pub success: bool,
    pub latency_ms: u64,
    pub timestamp: DateTime<Utc>,
    /// Sequence number for ordering within the stream.
    pub sequence: u64,
}

/// Distributed trace record — 11 required fields from distributed.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistributedTrace {
    pub origin_peer: String,
    pub destination_peer: String,
    pub action_type: DistributedActionType,
    pub session_id: Option<Uuid>,
    pub goal_id: Option<String>,
    pub request_id: Uuid,
    pub routing_decision: String,
    pub policy_decision: String,
    pub result: DistributedTraceResult,
    pub failure_reason: Option<String>,
    pub timestamps: TraceTimestamps,
    /// Correlation key for cross-peer trace correlation.
    pub correlation_key: Uuid,
}

/// Timestamps for distributed trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceTimestamps {
    pub initiated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// Action types for distributed trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistributedActionType {
    GoalSubmission,
    SkillInvocation,
    ResourceQuery,
    ObservationStream,
    Delegation,
    Migration,
    SchemaTransfer,
    RoutineTransfer,
    OfflineReplay,
}

/// Result status for distributed trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistributedTraceResult {
    Success,
    Failure,
    Partial,
    Pending,
}

/// Conflict state for resource/belief sync — 5 states from distributed.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictState {
    Confirmed,
    Tentative,
    Conflicting,
    Stale,
    Unresolved,
}

/// Distributed failure classes from distributed.md — all 16 types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistributedFailure {
    PeerUnreachable,
    TransportFailure,
    AuthenticationFailure,
    AuthorizationFailure,
    TrustValidationFailure,
    UnsupportedSkill,
    UnsupportedResource,
    StaleData,
    ConflictingData,
    ReplayRejection,
    BudgetExhaustion,
    Timeout,
    PartialObservationStream,
    MigrationFailure,
    DelegationRefusal,
    PolicyViolation,
}

/// Whether a distributed failure is recoverable — 4 recoverability categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureRecoverability {
    Retryable,
    DelegatableToAnotherPeer,
    TerminalForSession,
    TerminalForActionOnly,
}

/// Structured distributed failure with recoverability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredFailure {
    pub failure: DistributedFailure,
    pub recoverability: FailureRecoverability,
    pub details: String,
}

/// Recovery strategy — 5 recovery strategies from distributed.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryStrategy {
    RetryOtherPeer,
    LocalFallback,
    ResumeFromReplay,
    ContinueFromLastObservation,
    AbortSession,
}

/// Delegation unit — 5 units from distributed.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationUnit {
    Skill,
    Subgoal,
    Session,
    ResourceOperation,
    SchemaRoutineLookup,
}
