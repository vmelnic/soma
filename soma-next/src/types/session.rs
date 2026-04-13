use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::belief::BeliefState;
use super::common::Budget;
use super::goal::GoalSpec;
use super::observation::PortCallRecord;

/// What the session controller should do after a skill execution fails.
/// The `handle_failure` method on `SessionController` returns one of these
/// based on the failure class, remaining budget, and rollback outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureRecoveryAction {
    /// Re-execute the same skill (transient failure, budget still allows it).
    Retry,
    /// Pick the next-best candidate from the scored shortlist.
    SwitchCandidate,
    /// Roll back to a prior branch state and re-plan from there.
    Backtrack,
    /// Delegate the failed step to a remote peer.
    Delegate,
    /// Abort the session — no viable recovery path.
    Stop,
}

/// Identifies where in the skill execution lifecycle a policy check occurs.
/// The session controller calls the policy engine at each of these points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyHook {
    BeforeCandidateSelection,
    BeforeBindingFinalInputs,
    BeforeExecutionBegins,
    BeforeSideEffectingStep,
    BeforeDelegation,
    BeforeRollback,
    BeforeRemoteExposure,
}

/// Records whether one precondition passed or failed during execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreconditionResult {
    pub description: String,
    pub passed: bool,
    pub reason: String,
}

/// SessionStatus — required states from spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Created,
    Running,
    Paused,
    WaitingForInput,
    WaitingForRemote,
    BlockedByPolicy,
    Completed,
    Failed,
    Aborted,
}

/// ControlSession — the primary execution unit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlSession {
    pub session_id: Uuid,
    pub goal: GoalSpec,
    pub belief: BeliefState,
    pub working_memory: WorkingMemory,
    pub status: SessionStatus,
    pub trace: SessionTrace,
    pub budget_remaining: Budget,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Session-local working memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingMemory {
    pub active_bindings: Vec<WorkingBinding>,
    pub unresolved_slots: Vec<String>,
    pub current_subgoal: Option<String>,
    pub recent_observations: Vec<Uuid>,
    pub candidate_shortlist: Vec<String>,
    pub current_branch_state: Option<serde_json::Value>,
    pub budget_deltas: Vec<BudgetDelta>,
    pub output_bindings: Vec<OutputBinding>,
    /// When a routine matches, the compiled skill path is loaded here for
    /// plan-following mode. The control loop executes each skill in order
    /// instead of scoring/selecting from scratch each step.
    #[serde(default)]
    pub active_plan: Option<Vec<String>>,
    /// Current position within `active_plan`. Incremented after each
    /// successful plan step; reset to 0 when a new plan is loaded.
    #[serde(default)]
    pub plan_step: usize,
    /// Set to true when the session activates a compiled routine's plan.
    /// Used by episode storage to skip noise — successful plan-following
    /// sessions don't need new episodes (the routine already captures the behavior).
    #[serde(default)]
    pub used_plan_following: bool,
}

/// Provenance of a bound input value — which source the runtime drew the value from.
/// Every binding carries this so callers can reason about confidence and replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingSource {
    /// Bound from an explicit goal field.
    GoalField,
    /// Bound from a belief-state resource.
    BeliefResource,
    /// Bound from a prior observation in the session.
    PriorObservation,
    /// Bound from current working memory.
    WorkingMemory,
    /// Bound from a remote observation (delegated step).
    RemoteObservation,
    /// Bound from a pack-defined default.
    PackDefault,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingBinding {
    pub name: String,
    pub value: serde_json::Value,
    /// Which source this value was drawn from.
    pub source: BindingSource,
}

/// Output binding that preserves provenance through the execution pipeline.
/// Each output carries its originating skill, the observation that produced it,
/// and the confidence level of the result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputBinding {
    pub name: String,
    pub value: serde_json::Value,
    pub source_skill_id: String,
    pub source_observation_id: Uuid,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetDelta {
    pub step: u32,
    pub risk_spent: f64,
    pub latency_spent_ms: u64,
    pub resource_spent: f64,
}

/// SessionTrace — per-session trace storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTrace {
    pub steps: Vec<TraceStep>,
}

/// TraceStep — each step in the session trace.
/// Includes all fields required by the spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceStep {
    pub step_index: u32,
    pub belief_summary_before: serde_json::Value,
    pub retrieved_episodes: Vec<String>,
    pub retrieved_schemas: Vec<String>,
    pub retrieved_routines: Vec<String>,
    pub candidate_skills: Vec<String>,
    pub predicted_scores: Vec<CandidateScore>,
    pub selected_skill: String,
    pub port_calls: Vec<PortCallRecord>,
    pub observation_id: Uuid,
    pub belief_patch: serde_json::Value,
    pub progress_delta: f64,
    pub critic_decision: String,
    pub policy_decisions: Vec<PolicyTraceEntry>,
    /// Which inputs were bound and from where.
    pub bound_inputs: Vec<WorkingBinding>,
    /// Pass/fail result for each precondition that was evaluated.
    pub precondition_results: Vec<PreconditionResult>,
    /// Which termination condition fired, if any.
    pub termination_reason: Option<crate::types::common::TerminationType>,
    /// Whether rollback or compensation was invoked this step.
    pub rollback_invoked: bool,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateScore {
    pub skill_id: String,
    pub score: f64,
    pub predicted_success: f64,
    pub predicted_cost: f64,
    pub predicted_latency_ms: u64,
    pub information_gain: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyTraceEntry {
    pub action: String,
    pub decision: String,
    pub reason: String,
}
