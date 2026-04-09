use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::errors::{Result, SomaError};
use crate::runtime::metrics::RuntimeMetrics;
use crate::types::belief::{BeliefPatch, BeliefState, Fact};
use crate::types::common::{Budget, CapabilityScope, CostClass, CriticDecision, FactProvenance};
use crate::types::episode::Episode;
use crate::types::goal::{GoalSourceType, GoalSpec};
use crate::types::observation::Observation;
use crate::types::routine::Routine;
use crate::types::schema::Schema;
use crate::types::common::TerminationType;
use crate::types::session::{
    BudgetDelta, CandidateScore, ControlSession, FailureRecoveryAction, OutputBinding, PolicyHook,
    PolicyTraceEntry, PreconditionResult, SessionStatus, SessionTrace, TraceStep, WorkingBinding,
    WorkingMemory,
};
use crate::types::skill::{SkillKind, SkillSpec};
use crate::distributed::remote::RemoteExecutor;

// ---------------------------------------------------------------------------
// StepResult — outcome of a single control-loop iteration
// ---------------------------------------------------------------------------

/// Result of executing one iteration of the 16-step control loop.
#[derive(Debug, Clone)]
pub enum StepResult {
    /// Loop should continue to the next iteration.
    Continue,
    /// Session needs external input before it can proceed.
    WaitingForInput(String),
    /// Session is waiting on a remote peer or delegated skill.
    WaitingForRemote(String),
    /// Goal has been achieved; session is finalized.
    Completed,
    /// Session failed irrecoverably.
    Failed(String),
    /// Session was explicitly aborted.
    Aborted,
}

// ---------------------------------------------------------------------------
// SessionRuntime — the trait
// ---------------------------------------------------------------------------

/// The SessionRuntime trait defines the canonical session lifecycle.
///
/// Implementors manage creation, stepping, pausing, resuming, aborting,
/// checkpointing, and restoring of control sessions.
pub trait SessionRuntime {
    /// Create a new session from a validated GoalSpec.
    /// Initializes budget, working memory, belief state, and trace.
    fn create_session(&mut self, goal: GoalSpec) -> Result<ControlSession>;

    /// Execute one iteration of the 16-step control loop.
    fn run_step(&mut self, session: &mut ControlSession) -> Result<StepResult>;

    /// Pause a running session. Preserves all state for later resumption.
    fn pause(&mut self, session: &mut ControlSession) -> Result<()>;

    /// Resume a paused session.
    fn resume(&mut self, session: &mut ControlSession) -> Result<()>;

    /// Abort a session. Terminal state.
    fn abort(&mut self, session: &mut ControlSession) -> Result<()>;

    /// Retrieve an immutable reference to a session by ID.
    fn get_session(&self, session_id: &Uuid) -> Option<&ControlSession>;

    /// Serialize session state for persistence or migration.
    fn checkpoint(&self, session: &ControlSession) -> Result<Vec<u8>>;

    /// Restore a session from serialized checkpoint data.
    fn restore(&mut self, data: &[u8]) -> Result<ControlSession>;
}

// ---------------------------------------------------------------------------
// Subsystem interfaces for the session controller
// ---------------------------------------------------------------------------

/// Belief runtime — builds and patches belief state.
pub trait BeliefSource: Send + Sync {
    fn build_initial_belief(&self, goal: &GoalSpec) -> Result<BeliefState>;
    fn apply_patch(&self, belief: &mut BeliefState, patch: &BeliefPatch) -> Result<()>;
}

/// Episode memory — retrieves nearest episodes for a goal/belief.
pub trait EpisodeMemory: Send + Sync {
    fn retrieve_nearest(&self, goal: &GoalSpec, belief: &BeliefState, limit: usize) -> Vec<Episode>;
}

/// Schema memory — finds schemas matching current belief/goal.
pub trait SchemaMemory: Send + Sync {
    fn retrieve_matching(&self, goal: &GoalSpec, belief: &BeliefState) -> Vec<Schema>;
}

/// Routine memory — finds routines matching current belief/goal.
pub trait RoutineMemory: Send + Sync {
    fn retrieve_matching(&self, goal: &GoalSpec, belief: &BeliefState) -> Vec<Routine>;
}

/// Skill registry — enumerates and looks up skills.
pub trait SkillRegistry: Send + Sync {
    fn enumerate_candidates(
        &self,
        goal: &GoalSpec,
        belief: &BeliefState,
        schemas: &[Schema],
        routines: &[Routine],
    ) -> Vec<SkillSpec>;

    fn get_skill(&self, skill_id: &str) -> Option<&SkillSpec>;
}

/// Skill executor — binds inputs and runs a skill.
pub trait SkillExecutor: Send + Sync {
    fn bind_inputs(
        &self,
        skill: &SkillSpec,
        belief: &BeliefState,
        working_memory: &WorkingMemory,
    ) -> Result<Vec<WorkingBinding>>;

    /// Evaluate the skill's declared preconditions against the current belief
    /// and working memory before execution is allowed to start.
    fn check_preconditions(
        &self,
        skill: &SkillSpec,
        belief: &BeliefState,
        working_memory: &WorkingMemory,
    ) -> Result<Vec<PreconditionResult>>;

    fn execute(
        &self,
        skill: &SkillSpec,
        bindings: &[WorkingBinding],
        session_id: Uuid,
    ) -> Result<Observation>;

    /// Attempt to invoke the skill's declared compensation or rollback action.
    /// Returns None if the skill is irreversible or has no compensation skill.
    fn invoke_rollback(
        &self,
        skill: &SkillSpec,
        bindings: &[WorkingBinding],
        session_id: Uuid,
    ) -> Result<Option<Observation>>;
}

/// Predictor — scores candidate skills.
pub trait CandidatePredictor: Send + Sync {
    fn score(
        &self,
        candidates: &[SkillSpec],
        goal: &GoalSpec,
        belief: &BeliefState,
        episodes: &[Episode],
    ) -> Vec<CandidateScore>;

    fn predict_top(
        &self,
        scored: &[CandidateScore],
        limit: usize,
    ) -> Vec<CandidateScore>;
}

/// Critic — evaluates observation and decides loop control flow.
pub trait Critic: Send + Sync {
    fn evaluate(
        &self,
        goal: &GoalSpec,
        belief: &BeliefState,
        observation: &Observation,
        budget: &Budget,
        step_index: u32,
    ) -> CriticDecision;
}

/// Policy engine — checks whether an action is permitted.
pub trait PolicyEngine: Send + Sync {
    fn check_skill_execution(
        &self,
        skill: &SkillSpec,
        session: &ControlSession,
    ) -> PolicyCheckResult;

    /// Check whether an action is permitted at the given lifecycle hook point.
    /// The session controller calls this at all seven hooks in the execution loop.
    fn check_hook(
        &self,
        hook: PolicyHook,
        skill: &SkillSpec,
        session: &ControlSession,
    ) -> PolicyCheckResult;
}

/// Result of a policy check.
#[derive(Debug, Clone)]
pub struct PolicyCheckResult {
    pub allowed: bool,
    pub reason: String,
    pub blocked_by_policy: bool,
    pub waiting_for_input: Option<String>,
}

// ---------------------------------------------------------------------------
// SessionController — orchestrates the 16-step loop
// ---------------------------------------------------------------------------

/// All subsystem dependencies for `SessionController`.
/// Passed as a single bundle to `SessionController::new()` to avoid the
/// too-many-arguments lint.
/// Callback that resolves a capability FQN to its declared scope.
/// Used by the session controller to enforce scope restrictions at dispatch
/// time without requiring a direct reference to the pack runtime.
pub type CapabilityScopeChecker = Box<dyn Fn(&str) -> Option<CapabilityScope> + Send + Sync>;

pub struct SessionControllerDeps {
    pub belief_source: Box<dyn BeliefSource>,
    pub episode_memory: Box<dyn EpisodeMemory>,
    pub schema_memory: Box<dyn SchemaMemory>,
    pub routine_memory: Box<dyn RoutineMemory>,
    pub skill_registry: Box<dyn SkillRegistry>,
    pub skill_executor: Box<dyn SkillExecutor>,
    pub predictor: Box<dyn CandidatePredictor>,
    pub critic: Box<dyn Critic>,
    pub policy_engine: Box<dyn PolicyEngine>,
    pub remote_executor: Option<Box<dyn RemoteExecutor>>,
    /// When set, the session controller checks each skill's declared capability
    /// scope before execution. The checker maps a skill FQN to its declared
    /// CapabilityScope. If the skill's scope is narrower than the session's
    /// invocation context, execution is denied with a PolicyDenied error.
    pub capability_scope_checker: Option<CapabilityScopeChecker>,
}

/// SessionController holds references to all subsystems and manages sessions.
///
/// It implements `SessionRuntime` by orchestrating the canonical 16-step
/// control loop, delegating each phase to the appropriate subsystem trait.
pub struct SessionController {
    sessions: HashMap<Uuid, ControlSession>,
    belief_source: Box<dyn BeliefSource>,
    episode_memory: Box<dyn EpisodeMemory>,
    schema_memory: Box<dyn SchemaMemory>,
    routine_memory: Box<dyn RoutineMemory>,
    skill_registry: Box<dyn SkillRegistry>,
    skill_executor: Box<dyn SkillExecutor>,
    predictor: Box<dyn CandidatePredictor>,
    critic: Box<dyn Critic>,
    policy_engine: Box<dyn PolicyEngine>,
    /// Optional remote executor for dispatching delegated skills to remote peers.
    /// When present, delegated skills with a `remote_endpoint` are routed through
    /// this executor instead of the local skill executor.
    remote_executor: Option<Box<dyn RemoteExecutor>>,
    /// When configured, resolves a capability FQN to its declared scope and
    /// enforces that the session's invocation context does not exceed it.
    capability_scope_checker: Option<CapabilityScopeChecker>,
    /// Default maximum steps per session if goal does not specify.
    default_max_steps: u32,
    /// Shared runtime metrics for recording session lifecycle events,
    /// step counts, port calls, policy denials, and skill invocations.
    metrics: Arc<RuntimeMetrics>,
}

impl SessionController {
    pub fn new(deps: SessionControllerDeps, metrics: Arc<RuntimeMetrics>) -> Self {
        Self {
            sessions: HashMap::new(),
            belief_source: deps.belief_source,
            episode_memory: deps.episode_memory,
            schema_memory: deps.schema_memory,
            routine_memory: deps.routine_memory,
            skill_registry: deps.skill_registry,
            skill_executor: deps.skill_executor,
            predictor: deps.predictor,
            critic: deps.critic,
            policy_engine: deps.policy_engine,
            remote_executor: deps.remote_executor,
            capability_scope_checker: deps.capability_scope_checker,
            default_max_steps: 100,
            metrics,
        }
    }

    /// List all sessions as (session_id, status_string) pairs.
    pub fn list_sessions(&self) -> Vec<(Uuid, String)> {
        self.sessions
            .iter()
            .map(|(id, s)| (*id, format!("{:?}", s.status)))
            .collect()
    }

    /// Look up a session by ID (public convenience method).
    pub fn get_session_by_id(&self, session_id: &Uuid) -> Option<&ControlSession> {
        self.sessions.get(session_id)
    }

    /// Serialize a session to JSON bytes for persistence.
    /// Delegates to the `SessionRuntime::checkpoint` implementation.
    pub fn checkpoint_session(&self, session_id: &Uuid) -> Result<Vec<u8>> {
        let session = self.sessions.get(session_id).ok_or_else(|| {
            SomaError::SessionNotFound(session_id.to_string())
        })?;
        self.checkpoint(session)
    }

    /// Deserialize a session from JSON bytes and insert it back into the
    /// controller's session map. Returns the session ID.
    pub fn restore_session(&mut self, data: &[u8]) -> Result<Uuid> {
        let session = self.restore(data)?;
        Ok(session.session_id)
    }

    /// Derive the invocation scope for a session based on its goal source.
    ///
    /// The invocation scope represents the broadest trust boundary the session
    /// is operating within. A capability whose declared scope is narrower than
    /// this value should not be invoked.
    ///   - User / Internal / Scheduler goals -> Local (default, on-device user).
    ///   - Api / Mcp goals                   -> Session (external client session).
    ///   - Peer goals                        -> Peer (remote peer invocation).
    fn invocation_scope(session: &ControlSession) -> CapabilityScope {
        match session.goal.source.source_type {
            GoalSourceType::User | GoalSourceType::Internal | GoalSourceType::Scheduler => {
                CapabilityScope::Local
            }
            GoalSourceType::Api | GoalSourceType::Mcp => CapabilityScope::Session,
            GoalSourceType::Peer => CapabilityScope::Peer,
        }
    }

    /// Validate that the goal has a positive budget and non-empty objective.
    fn validate_goal(goal: &GoalSpec) -> Result<()> {
        if goal.objective.description.is_empty() {
            return Err(SomaError::GoalValidation(
                "objective description must not be empty".into(),
            ));
        }
        if goal.risk_budget <= 0.0 {
            return Err(SomaError::GoalValidation(
                "risk budget must be positive".into(),
            ));
        }
        if goal.latency_budget_ms == 0 {
            return Err(SomaError::GoalValidation(
                "latency budget must be positive".into(),
            ));
        }
        if goal.resource_budget <= 0.0 {
            return Err(SomaError::GoalValidation(
                "resource budget must be positive".into(),
            ));
        }
        Ok(())
    }

    /// Check whether the session's budget allows another step.
    fn budget_allows_step(budget: &Budget) -> bool {
        budget.risk_remaining > 0.0
            && budget.latency_remaining_ms > 0
            && budget.resource_remaining > 0.0
            && budget.steps_remaining > 0
    }

    /// Deduct observation costs from the session budget, recording a delta.
    fn deduct_budget(
        session: &mut ControlSession,
        observation: &Observation,
        step_index: u32,
    ) {
        // Reduce the multi-dimensional cost profile to a weighted scalar.
        // Weights: CPU 0.3, memory 0.2, IO 0.2, network 0.2, energy 0.1.
        fn cost_class_value(c: CostClass) -> f64 {
            match c {
                CostClass::Negligible => 0.0,
                CostClass::Low => 0.25,
                CostClass::Medium => 0.5,
                CostClass::High => 0.75,
                CostClass::Extreme => 1.0,
            }
        }
        let cost = &observation.resource_cost;
        let scalar_cost = cost_class_value(cost.cpu_cost_class) * 0.3
            + cost_class_value(cost.memory_cost_class) * 0.2
            + cost_class_value(cost.io_cost_class) * 0.2
            + cost_class_value(cost.network_cost_class) * 0.2
            + cost_class_value(cost.energy_cost_class) * 0.1;
        let delta = BudgetDelta {
            step: step_index,
            risk_spent: 0.0, // Risk accounting is deferred to the policy engine.
            latency_spent_ms: observation.latency_ms,
            resource_spent: scalar_cost,
        };

        session.budget_remaining.latency_remaining_ms = session
            .budget_remaining
            .latency_remaining_ms
            .saturating_sub(observation.latency_ms);
        session.budget_remaining.resource_remaining -= scalar_cost;
        if session.budget_remaining.resource_remaining < 0.0 {
            session.budget_remaining.resource_remaining = 0.0;
        }
        session.budget_remaining.steps_remaining =
            session.budget_remaining.steps_remaining.saturating_sub(1);

        session.working_memory.budget_deltas.push(delta);
    }

    /// Build a belief patch from an observation (minimal: records facts from the result).
    fn build_belief_patch(observation: &Observation) -> BeliefPatch {
        let mut added_facts = Vec::new();

        // Record the observation result as a fact.
        added_facts.push(Fact {
            fact_id: format!("obs:{}", observation.observation_id),
            subject: observation
                .skill_id
                .clone()
                .unwrap_or_else(|| "unknown".into()),
            predicate: if observation.success {
                "succeeded".into()
            } else {
                "failed".into()
            },
            value: observation.structured_result.clone(),
            confidence: observation.confidence,
            provenance: FactProvenance::Observed,
            timestamp: observation.timestamp,
        });

        BeliefPatch {
            added_resources: Vec::new(),
            updated_resources: Vec::new(),
            removed_resource_ids: Vec::new(),
            added_facts,
            updated_facts: Vec::new(),
            removed_fact_ids: Vec::new(),
            binding_updates: Vec::new(),
        }
    }

    /// Record a completed trace step into the session.
    #[allow(clippy::too_many_arguments)]
    fn record_trace_step(
        session: &mut ControlSession,
        step_index: u32,
        belief_summary_before: serde_json::Value,
        episodes: &[Episode],
        schemas: &[Schema],
        routines: &[Routine],
        candidates: &[SkillSpec],
        scores: &[CandidateScore],
        selected_skill: &str,
        observation: &Observation,
        belief_patch: &BeliefPatch,
        progress_delta: f64,
        critic_decision: CriticDecision,
        policy_entries: Vec<PolicyTraceEntry>,
        bound_inputs: Vec<WorkingBinding>,
        precondition_results: Vec<PreconditionResult>,
        termination_reason: Option<TerminationType>,
        rollback_invoked: bool,
    ) {
        let step = TraceStep {
            step_index,
            belief_summary_before,
            retrieved_episodes: episodes.iter().map(|e| e.episode_id.to_string()).collect(),
            retrieved_schemas: schemas.iter().map(|s| s.schema_id.clone()).collect(),
            retrieved_routines: routines.iter().map(|r| r.routine_id.clone()).collect(),
            candidate_skills: candidates.iter().map(|c| c.skill_id.clone()).collect(),
            predicted_scores: scores.to_vec(),
            selected_skill: selected_skill.to_string(),
            port_calls: observation.port_calls.clone(),
            observation_id: observation.observation_id,
            belief_patch: serde_json::to_value(belief_patch).unwrap_or_default(),
            progress_delta,
            critic_decision: format!("{:?}", critic_decision),
            policy_decisions: policy_entries,
            bound_inputs,
            precondition_results,
            termination_reason,
            rollback_invoked,
            timestamp: Utc::now(),
        };

        session.trace.steps.push(step);
    }

    /// Translate a CriticDecision into a StepResult, handling terminal cases.
    #[allow(clippy::match_same_arms)]
    fn critic_to_step_result(decision: CriticDecision) -> StepResult {
        match decision {
            CriticDecision::Continue => StepResult::Continue,
            CriticDecision::Revise => StepResult::Continue, // Revise is a non-terminal retry.
            CriticDecision::Backtrack => StepResult::Continue, // Backtrack triggers re-planning next step.
            CriticDecision::Delegate => StepResult::WaitingForRemote("delegation requested".into()),
            CriticDecision::Stop => StepResult::Completed,
        }
    }

    /// Decide how the session should recover from a failed skill execution.
    ///
    /// The decision depends on the failure class reported in the observation
    /// and the budget still available to the session.  Terminal failures
    /// (timeout, budget, policy) yield `Stop`.  Transient infrastructure
    /// failures (port, remote) are retried when the budget permits, otherwise
    /// the controller falls back to `SwitchCandidate`.
    fn handle_failure(
        &self,
        session: &ControlSession,
        _skill: &SkillSpec,
        observation: &Observation,
        _step_index: u32,
    ) -> FailureRecoveryAction {
        use crate::types::common::SkillFailureClass;

        let failure_class = observation
            .failure_class
            .unwrap_or(SkillFailureClass::Unknown);

        match failure_class {
            SkillFailureClass::Timeout | SkillFailureClass::BudgetExhaustion => {
                FailureRecoveryAction::Stop
            }
            SkillFailureClass::PolicyDenial => FailureRecoveryAction::Stop,
            SkillFailureClass::BindingFailure | SkillFailureClass::PreconditionFailure => {
                FailureRecoveryAction::SwitchCandidate
            }
            SkillFailureClass::PortFailure | SkillFailureClass::RemoteFailure => {
                if Self::budget_allows_step(&session.budget_remaining) {
                    FailureRecoveryAction::Retry
                } else {
                    FailureRecoveryAction::SwitchCandidate
                }
            }
            SkillFailureClass::RollbackFailure => FailureRecoveryAction::Stop,
            SkillFailureClass::Unknown
            | SkillFailureClass::ValidationFailure
            | SkillFailureClass::PartialSuccess => FailureRecoveryAction::Backtrack,
        }
    }

    /// Execute a composite skill by iterating its subskill graph.
    ///
    /// For each `SubskillRef` in the composite's `subskills` list:
    ///  1. Look up the subskill by `skill_id` from the skill registry.
    ///  2. Evaluate the `branch_condition` — skip if conditions are not met.
    ///  3. Execute the subskill via the skill executor.
    ///  4. Collect the observation.
    ///  5. Evaluate the `stop_condition` — stop iteration early if met.
    ///
    /// Returns a single aggregated `Observation` that combines all substep
    /// results.  The aggregate is successful only if every executed substep
    /// succeeded (required subskills failing marks the whole composite as
    /// failed).  Latency is summed, confidence is the minimum across substeps,
    /// and port calls are concatenated.
    fn execute_composite(
        &self,
        composite_skill: &SkillSpec,
        parent_bindings: &[WorkingBinding],
        session: &mut ControlSession,
    ) -> Observation {
        let mut substep_observations: Vec<Observation> = Vec::new();
        let mut all_succeeded = true;
        let mut total_latency_ms: u64 = 0;
        let mut min_confidence: f64 = 1.0;
        let mut aggregated_port_calls = Vec::new();
        let mut aggregated_results = serde_json::Map::new();
        let mut first_failure_class = None;

        for subref in &composite_skill.subskills {
            // Evaluate branch_condition: if present and evaluates to a falsy
            // JSON value, skip this subskill.
            if let Some(ref condition) = subref.branch_condition
                && !self.evaluate_condition(condition, session) {
                    debug!(
                        session_id = %session.session_id,
                        composite = %composite_skill.skill_id,
                        subskill = %subref.skill_id,
                        "branch condition not met, skipping subskill"
                    );
                    continue;
                }

            // Look up the subskill from the registry.
            let subskill = match self.skill_registry.get_skill(&subref.skill_id) {
                Some(s) => s.clone(),
                None => {
                    warn!(
                        session_id = %session.session_id,
                        composite = %composite_skill.skill_id,
                        subskill = %subref.skill_id,
                        "subskill not found in registry"
                    );
                    if subref.required {
                        all_succeeded = false;
                        first_failure_class.get_or_insert(
                            crate::types::common::SkillFailureClass::Unknown,
                        );
                    }
                    continue;
                }
            };

            // Execute the subskill.
            let sub_obs = match self.skill_executor.execute(
                &subskill,
                parent_bindings,
                session.session_id,
            ) {
                Ok(obs) => obs,
                Err(e) => {
                    warn!(
                        session_id = %session.session_id,
                        composite = %composite_skill.skill_id,
                        subskill = %subref.skill_id,
                        error = %e,
                        "subskill execution failed"
                    );
                    Observation {
                        observation_id: Uuid::new_v4(),
                        session_id: session.session_id,
                        skill_id: Some(subref.skill_id.clone()),
                        port_calls: Vec::new(),
                        raw_result: serde_json::json!({ "error": e.to_string() }),
                        structured_result: serde_json::json!({ "error": e.to_string() }),
                        effect_patch: None,
                        success: false,
                        failure_class: Some(crate::types::common::SkillFailureClass::Unknown),
                        latency_ms: 0,
                        resource_cost: crate::types::observation::default_cost_profile(),
                        confidence: 0.0,
                        timestamp: Utc::now(),
                    }
                }
            };

            // Track aggregate metrics.
            total_latency_ms = total_latency_ms.saturating_add(sub_obs.latency_ms);
            if sub_obs.confidence < min_confidence {
                min_confidence = sub_obs.confidence;
            }
            aggregated_port_calls.extend(sub_obs.port_calls.clone());

            // Merge structured result into the aggregate keyed by subskill id.
            aggregated_results.insert(
                subref.skill_id.clone(),
                sub_obs.structured_result.clone(),
            );

            // Track per-observation into working memory.
            session
                .working_memory
                .recent_observations
                .push(sub_obs.observation_id);
            if session.working_memory.recent_observations.len() > 20 {
                session.working_memory.recent_observations.remove(0);
            }

            // If a required subskill failed, mark the composite as failed.
            if !sub_obs.success
                && subref.required {
                    all_succeeded = false;
                    first_failure_class
                        .get_or_insert(sub_obs.failure_class.unwrap_or(
                            crate::types::common::SkillFailureClass::Unknown,
                        ));
                }

            substep_observations.push(sub_obs.clone());

            // Evaluate stop_condition: if present and the condition is met,
            // terminate the subskill iteration early.
            if let Some(ref condition) = subref.stop_condition
                && self.evaluate_condition(condition, session) {
                    debug!(
                        session_id = %session.session_id,
                        composite = %composite_skill.skill_id,
                        subskill = %subref.skill_id,
                        "stop condition met, ending composite iteration"
                    );
                    break;
                }
        }

        // Build the aggregated observation for the composite skill.
        Observation {
            observation_id: Uuid::new_v4(),
            session_id: session.session_id,
            skill_id: Some(composite_skill.skill_id.clone()),
            port_calls: aggregated_port_calls,
            raw_result: serde_json::json!({
                "composite": true,
                "substep_count": substep_observations.len(),
                "substep_ids": substep_observations.iter()
                    .map(|o| o.observation_id.to_string())
                    .collect::<Vec<_>>(),
            }),
            structured_result: serde_json::Value::Object(aggregated_results),
            effect_patch: None,
            success: all_succeeded,
            failure_class: first_failure_class,
            latency_ms: total_latency_ms,
            resource_cost: crate::types::observation::default_cost_profile(),
            confidence: if substep_observations.is_empty() {
                0.0
            } else {
                min_confidence
            },
            timestamp: Utc::now(),
        }
    }

    /// Evaluate a JSON condition value against the current session state.
    ///
    /// Condition evaluation is intentionally simple: a condition is considered
    /// "met" unless it is explicitly a JSON `false`, `null`, or the integer `0`.
    /// This provides a minimal branch/stop gate that can be extended with a
    /// full expression evaluator later.  When the condition is a JSON object
    /// with a `"fact"` key, the runtime checks whether a matching fact exists
    /// in the current belief state.
    fn evaluate_condition(
        &self,
        condition: &serde_json::Value,
        session: &ControlSession,
    ) -> bool {
        match condition {
            serde_json::Value::Bool(b) => *b,
            serde_json::Value::Null => false,
            serde_json::Value::Number(n) => {
                n.as_f64().is_some_and(|v| v != 0.0)
            }
            serde_json::Value::Object(obj) => {
                // If the condition references a fact by predicate, check the
                // belief state for a matching fact.
                if let Some(serde_json::Value::String(predicate)) = obj.get("fact") {
                    session
                        .belief
                        .facts
                        .iter()
                        .any(|f| f.predicate == *predicate)
                } else {
                    // Non-empty objects are truthy by default.
                    !obj.is_empty()
                }
            }
            // Strings and arrays are truthy (non-empty check for strings).
            serde_json::Value::String(s) => !s.is_empty(),
            serde_json::Value::Array(a) => !a.is_empty(),
        }
    }

    /// Execute the core skill lifecycle for a chosen candidate.
    ///
    /// This method encapsulates the 8-step execution sequence that runs once
    /// a candidate skill has been selected:
    ///   1. Bind inputs from belief/working memory
    ///   2. Validate preconditions
    ///   3. Authorize (policy hooks: BeforeBindingFinalInputs, BeforeSideEffectingStep,
    ///      BeforeDelegation, BeforeRemoteExposure, BeforeExecutionBegins)
    ///   4. Execute the skill
    ///   5. Collect the observation
    ///   6. Apply effect patch (belief update)
    ///   7. Evaluate termination conditions
    ///   8. Return the observation and associated metadata
    ///
    /// Returns `Ok(lifecycle_output)` on success (even if the skill itself
    /// failed — a failure observation is still a valid output).  Returns
    /// `Err` when the lifecycle cannot proceed (policy denial, binding
    /// failure, precondition failure).
    #[allow(clippy::type_complexity)]
    fn execute_skill_lifecycle(
        &self,
        session: &mut ControlSession,
        chosen_skill: &SkillSpec,
        _step_index: u32,
    ) -> Result<SkillLifecycleOutput> {
        // -- 0. Capability scope enforcement ----------------------------------
        // When a scope checker is configured, verify that the skill's declared
        // capability scope is broad enough for this session's invocation context.
        // A Local-scoped capability must not be reachable from a Peer context.
        if let Some(ref checker) = self.capability_scope_checker {
            let fqn = format!("{}.{}", chosen_skill.namespace, chosen_skill.skill_id);
            let invocation_scope = Self::invocation_scope(session);
            if let Some(declared_scope) = checker(&fqn)
                && !declared_scope.is_at_least(invocation_scope) {
                    warn!(
                        session_id = %session.session_id,
                        skill = %chosen_skill.skill_id,
                        fqn = %fqn,
                        declared_scope = ?declared_scope,
                        invocation_scope = ?invocation_scope,
                        "capability scope violation: skill scope is narrower than invocation context"
                    );
                    return Err(SomaError::PolicyDenied {
                        action: format!("scope_check:{}", fqn),
                        reason: format!(
                            "capability '{}' has scope {:?} which is narrower than the session's \
                             invocation scope {:?}",
                            fqn, declared_scope, invocation_scope
                        ),
                    });
                }
        }

        // -- 1. Bind inputs ---------------------------------------------------
        let bindings = match self.skill_executor.bind_inputs(
            chosen_skill,
            &session.belief,
            &session.working_memory,
        ) {
            Ok(b) => b,
            Err(e) => {
                warn!(
                    session_id = %session.session_id,
                    skill = %chosen_skill.skill_id,
                    error = %e,
                    "binding failed"
                );
                session
                    .working_memory
                    .unresolved_slots
                    .push(format!("bind_error:{}", e));
                return Err(SomaError::Skill(format!(
                    "cannot bind inputs for skill {}: {}",
                    chosen_skill.skill_id, e
                )));
            }
        };
        session.working_memory.active_bindings = bindings.clone();

        // -- 2. Validate preconditions ----------------------------------------
        let precondition_results = match self.skill_executor.check_preconditions(
            chosen_skill,
            &session.belief,
            &session.working_memory,
        ) {
            Ok(results) => results,
            Err(e) => {
                warn!(
                    session_id = %session.session_id,
                    skill = %chosen_skill.skill_id,
                    error = %e,
                    "precondition check failed"
                );
                return Err(SomaError::Skill(format!(
                    "precondition failed for skill {}: {}",
                    chosen_skill.skill_id, e
                )));
            }
        };
        if precondition_results.iter().any(|r| !r.passed) {
            return Err(SomaError::Skill(format!(
                "precondition not met for skill {}",
                chosen_skill.skill_id
            )));
        }


        // -- 2b. Routine re-validation ----------------------------------------
        // Before executing a routine skill, re-validate its match_conditions
        // against the current belief state. Conditions that held at candidate
        // selection time may no longer hold after belief mutations from earlier
        // steps.
        if chosen_skill.kind == SkillKind::Routine && !chosen_skill.match_conditions.is_empty() {
            for condition in &chosen_skill.match_conditions {
                if !self.evaluate_condition(&condition.expression, session) {
                    warn!(
                        session_id = %session.session_id,
                        skill = %chosen_skill.skill_id,
                        condition = %condition.description,
                        "routine match condition no longer holds"
                    );
                    return Err(SomaError::SkillExecution {
                        skill_id: chosen_skill.skill_id.clone(),
                        failure_class: crate::types::common::SkillFailureClass::PreconditionFailure,
                        details: format!(
                            "routine match condition no longer satisfied: {}",
                            condition.description
                        ),
                    });
                }
            }

            // If the routine declares a confidence_threshold, verify that the
            // candidate's predicted confidence (carried forward through the
            // latest scoring) meets it. Use the average confidence of facts
            // currently in the belief state as the runtime proxy.
            if let Some(threshold) = chosen_skill.confidence_threshold {
                let current_confidence = if session.belief.facts.is_empty() {
                    0.0
                } else {
                    session.belief.facts.iter().map(|f| f.confidence).sum::<f64>()
                        / session.belief.facts.len() as f64
                };
                if current_confidence < threshold {
                    warn!(
                        session_id = %session.session_id,
                        skill = %chosen_skill.skill_id,
                        current_confidence,
                        threshold,
                        "routine confidence below threshold"
                    );
                    return Err(SomaError::SkillExecution {
                        skill_id: chosen_skill.skill_id.clone(),
                        failure_class: crate::types::common::SkillFailureClass::PreconditionFailure,
                        details: format!(
                            "routine confidence {:.3} below threshold {:.3}",
                            current_confidence, threshold
                        ),
                    });
                }
            }

            debug!(
                session_id = %session.session_id,
                skill = %chosen_skill.skill_id,
                "routine re-validation passed"
            );
        }

        // -- 3. Authorize — run all applicable policy hooks -------------------

        // BeforeBindingFinalInputs: inputs are now bound and visible.
        let pre_bind_policy = self.policy_engine.check_hook(
            PolicyHook::BeforeBindingFinalInputs,
            chosen_skill,
            session,
        );
        if !pre_bind_policy.allowed {
            return Err(SomaError::PolicyDenied {
                action: format!("bind:{}", chosen_skill.skill_id),
                reason: pre_bind_policy.reason,
            });
        }

        // BeforeSideEffectingStep: skills that mutate external state.
        let has_side_effects = chosen_skill.expected_effects.iter().any(|e| {
            !matches!(e.effect_type, crate::types::common::EffectType::Emission)
        });
        if has_side_effects {
            let side_effect_policy = self.policy_engine.check_hook(
                PolicyHook::BeforeSideEffectingStep,
                chosen_skill,
                session,
            );
            if !side_effect_policy.allowed {
                return Err(SomaError::PolicyDenied {
                    action: format!("side_effect:{}", chosen_skill.skill_id),
                    reason: side_effect_policy.reason,
                });
            }
        }

        // BeforeDelegation: skills that delegate to a remote peer.
        if chosen_skill.kind == crate::types::skill::SkillKind::Delegated {
            let delegation_policy = self.policy_engine.check_hook(
                PolicyHook::BeforeDelegation,
                chosen_skill,
                session,
            );
            if !delegation_policy.allowed {
                return Err(SomaError::PolicyDenied {
                    action: format!("delegate:{}", chosen_skill.skill_id),
                    reason: delegation_policy.reason,
                });
            }
        }

        // BeforeRemoteExposure: skills that expose data or capabilities to
        // remote peers.
        if chosen_skill.remote_exposure.enabled {
            let remote_policy = self.policy_engine.check_hook(
                PolicyHook::BeforeRemoteExposure,
                chosen_skill,
                session,
            );
            if !remote_policy.allowed {
                return Err(SomaError::PolicyDenied {
                    action: format!("remote_exposure:{}", chosen_skill.skill_id),
                    reason: remote_policy.reason,
                });
            }
        }

        // BeforeExecutionBegins: final gate before the executor runs.
        let pre_exec_policy = self.policy_engine.check_hook(
            PolicyHook::BeforeExecutionBegins,
            chosen_skill,
            session,
        );
        if !pre_exec_policy.allowed {
            return Err(SomaError::PolicyDenied {
                action: format!("execute:{}", chosen_skill.skill_id),
                reason: pre_exec_policy.reason,
            });
        }

        // -- 4. Execute the skill ---------------------------------------------
        let observation = if chosen_skill.kind == SkillKind::Composite {
            self.execute_composite(chosen_skill, &bindings, session)
        } else if chosen_skill.kind == SkillKind::Delegated
            && chosen_skill.remote_endpoint.is_some()
        {
            // Delegated dispatch: route to a remote executor when available,
            // falling back to local execution if no remote executor is wired.
            let endpoint = chosen_skill.remote_endpoint.as_deref().unwrap();
            if let Some(ref remote_exec) = self.remote_executor {
                debug!(
                    session_id = %session.session_id,
                    skill = %chosen_skill.skill_id,
                    endpoint,
                    "dispatching delegated skill to remote executor"
                );
                // Build the input payload from the current bindings.
                let input_payload = serde_json::json!({
                    "bindings": bindings.iter().map(|b| {
                        serde_json::json!({
                            "name": b.name,
                            "value": b.value,
                        })
                    }).collect::<Vec<_>>(),
                    "session_id": session.session_id.to_string(),
                });
                match remote_exec.invoke_skill(endpoint, &chosen_skill.skill_id, input_payload) {
                    Ok(resp) => {
                        // Convert RemoteSkillResponse into a local Observation.
                        Observation {
                            observation_id: resp.trace_id,
                            session_id: session.session_id,
                            skill_id: Some(resp.skill_id),
                            port_calls: Vec::new(),
                            raw_result: resp.observation.clone(),
                            structured_result: resp.observation,
                            effect_patch: None,
                            success: resp.success,
                            failure_class: if resp.success {
                                None
                            } else {
                                Some(crate::types::common::SkillFailureClass::RemoteFailure)
                            },
                            latency_ms: resp.latency_ms,
                            resource_cost: crate::types::observation::default_cost_profile(),
                            confidence: if resp.success { 1.0 } else { 0.0 },
                            timestamp: resp.timestamp,
                        }
                    }
                    Err(e) => {
                        warn!(
                            session_id = %session.session_id,
                            skill = %chosen_skill.skill_id,
                            endpoint,
                            error = %e,
                            "remote delegation failed"
                        );
                        Observation {
                            observation_id: Uuid::new_v4(),
                            session_id: session.session_id,
                            skill_id: Some(chosen_skill.skill_id.clone()),
                            port_calls: Vec::new(),
                            raw_result: serde_json::json!({ "error": e.to_string() }),
                            structured_result: serde_json::json!({ "error": e.to_string() }),
                            effect_patch: None,
                            success: false,
                            failure_class: Some(crate::types::common::SkillFailureClass::RemoteFailure),
                            latency_ms: 0,
                            resource_cost: crate::types::observation::default_cost_profile(),
                            confidence: 0.0,
                            timestamp: Utc::now(),
                        }
                    }
                }
            } else {
                // No remote executor available; fall back to local execution.
                warn!(
                    session_id = %session.session_id,
                    skill = %chosen_skill.skill_id,
                    endpoint,
                    "delegated skill has remote_endpoint but no remote executor is configured; \
                     falling back to local execution"
                );
                match self.skill_executor.execute(
                    chosen_skill,
                    &bindings,
                    session.session_id,
                ) {
                    Ok(obs) => obs,
                    Err(e) => {
                        warn!(
                            session_id = %session.session_id,
                            skill = %chosen_skill.skill_id,
                            error = %e,
                            "local fallback execution failed for delegated skill"
                        );
                        Observation {
                            observation_id: Uuid::new_v4(),
                            session_id: session.session_id,
                            skill_id: Some(chosen_skill.skill_id.clone()),
                            port_calls: Vec::new(),
                            raw_result: serde_json::json!({ "error": e.to_string() }),
                            structured_result: serde_json::json!({ "error": e.to_string() }),
                            effect_patch: None,
                            success: false,
                            failure_class: Some(crate::types::common::SkillFailureClass::Unknown),
                            latency_ms: 0,
                            resource_cost: crate::types::observation::default_cost_profile(),
                            confidence: 0.0,
                            timestamp: Utc::now(),
                        }
                    }
                }
            }
        } else {
            match self.skill_executor.execute(
                chosen_skill,
                &bindings,
                session.session_id,
            ) {
                Ok(obs) => obs,
                Err(e) => {
                    warn!(
                        session_id = %session.session_id,
                        skill = %chosen_skill.skill_id,
                        error = %e,
                        "skill execution failed"
                    );
                    Observation {
                        observation_id: Uuid::new_v4(),
                        session_id: session.session_id,
                        skill_id: Some(chosen_skill.skill_id.clone()),
                        port_calls: Vec::new(),
                        raw_result: serde_json::json!({ "error": e.to_string() }),
                        structured_result: serde_json::json!({ "error": e.to_string() }),
                        effect_patch: None,
                        success: false,
                        failure_class: Some(crate::types::common::SkillFailureClass::Unknown),
                        latency_ms: 0,
                        resource_cost: crate::types::observation::default_cost_profile(),
                        confidence: 0.0,
                        timestamp: Utc::now(),
                    }
                }
            }
        };

        // Downgrade undeclared PartialSuccess to Unknown.
        let observation = if observation.failure_class
            == Some(crate::types::common::SkillFailureClass::PartialSuccess)
            && chosen_skill.partial_success_behavior.is_none()
        {
            warn!(
                session_id = %session.session_id,
                skill = %chosen_skill.skill_id,
                "executor returned PartialSuccess but skill has no partial_success_behavior \
                 declared; downgrading to Unknown failure"
            );
            Observation {
                failure_class: Some(crate::types::common::SkillFailureClass::Unknown),
                ..observation
            }
        } else {
            observation
        };

        // -- 5. Collect the observation into working memory -------------------
        session
            .working_memory
            .recent_observations
            .push(observation.observation_id);
        if session.working_memory.recent_observations.len() > 20 {
            session.working_memory.recent_observations.remove(0);
        }

        // Build output bindings from the structured result. Each top-level
        // key in the JSON object becomes an OutputBinding that preserves
        // provenance back to the skill and observation that produced it.
        let output_bindings: Vec<OutputBinding> =
            if let Some(obj) = observation.structured_result.as_object() {
                let mut binds = Vec::with_capacity(obj.len());
                for (key, val) in obj {
                    binds.push(OutputBinding {
                        name: key.clone(),
                        value: val.clone(),
                        source_skill_id: chosen_skill.skill_id.clone(),
                        source_observation_id: observation.observation_id,
                        confidence: observation.confidence,
                    });
                }
                binds
            } else {
                Vec::new()
            };

        // Store in working memory so subsequent skills can consume them.
        session
            .working_memory
            .output_bindings
            .extend(output_bindings.clone());

        // -- 6. Apply effect patch (belief update) ----------------------------
        let belief_patch = Self::build_belief_patch(&observation);
        if let Err(e) = self
            .belief_source
            .apply_patch(&mut session.belief, &belief_patch)
        {
            warn!(
                session_id = %session.session_id,
                error = %e,
                "belief patch failed"
            );
        }

        // -- 7. Evaluate termination conditions -------------------------------
        let termination_reason = evaluate_termination_conditions(chosen_skill, &observation);

        // When a skill fails and has a declared compensation action, attempt rollback.
        let mut rollback_invoked = false;
        if !observation.success {
            let can_rollback = chosen_skill.rollback_or_compensation.support
                != crate::types::common::RollbackSupport::Irreversible
                && chosen_skill
                    .rollback_or_compensation
                    .compensation_skill
                    .is_some();
            if can_rollback {
                let rollback_policy = self.policy_engine.check_hook(
                    PolicyHook::BeforeRollback,
                    chosen_skill,
                    session,
                );
                if rollback_policy.allowed {
                    match self
                        .skill_executor
                        .invoke_rollback(chosen_skill, &bindings, session.session_id)
                    {
                        Ok(Some(_rollback_obs)) => {
                            rollback_invoked = true;
                            debug!(
                                session_id = %session.session_id,
                                skill = %chosen_skill.skill_id,
                                "rollback invoked"
                            );
                        }
                        Ok(None) => {}
                        Err(e) => {
                            warn!(
                                session_id = %session.session_id,
                                error = %e,
                                "rollback failed"
                            );
                        }
                    }
                }
            }
        }

        // -- 8. Evaluate observables ------------------------------------------
        let observable_evaluation = Self::evaluate_observables(chosen_skill, &observation);

        // -- 9. Return result -------------------------------------------------
        Ok(SkillLifecycleOutput {
            observation,
            belief_patch,
            bindings,
            precondition_results,
            termination_reason,
            rollback_invoked,
            observable_evaluation,
        })
    }

    /// Evaluate a skill's declared observables against the collected observation.
    ///
    /// Each observable has a named field and a role.  This method walks the
    /// observables and checks the corresponding field in the observation's
    /// `structured_result`, producing a summary that the caller (and trace)
    /// can use to refine the session's belief and confidence.
    pub fn evaluate_observables(
        skill: &SkillSpec,
        observation: &Observation,
    ) -> ObservableEvaluation {
        use crate::types::skill::ObservableRole;

        let mut eval = ObservableEvaluation::default();
        let result = &observation.structured_result;

        for decl in &skill.observables {
            let field_value = result.get(&decl.field);

            match decl.role {
                ObservableRole::ConfirmSuccess => {
                    // The field must exist and be non-null.
                    if let Some(v) = field_value
                        && !v.is_null() {
                            eval.success_confirmed = true;
                        }
                }

                ObservableRole::DetectPartialSuccess => {
                    // Partial completion is indicated by the field existing but
                    // containing an explicit "partial" marker string, a boolean
                    // false (meaning not fully done), or a numeric value between
                    // 0 exclusive and 1 exclusive.
                    if let Some(v) = field_value {
                        let is_partial = match v {
                            serde_json::Value::String(s) => {
                                let lower = s.to_lowercase();
                                lower.contains("partial")
                                    || lower.contains("incomplete")
                            }
                            serde_json::Value::Bool(false) => true,
                            serde_json::Value::Number(n) => {
                                n.as_f64()
                                    .map(|f| f > 0.0 && f < 1.0)
                                    .unwrap_or(false)
                            }
                            _ => false,
                        };
                        if is_partial {
                            eval.partial_detected = true;
                        }
                    }
                }

                ObservableRole::DetectAmbiguity => {
                    // Ambiguity is signalled by a null value or a string
                    // containing "ambiguous" or "uncertain".
                    if let Some(v) = field_value {
                        let is_ambiguous = match v {
                            serde_json::Value::Null => true,
                            serde_json::Value::String(s) => {
                                let lower = s.to_lowercase();
                                lower.contains("ambiguous")
                                    || lower.contains("uncertain")
                            }
                            _ => false,
                        };
                        if is_ambiguous {
                            eval.ambiguity_detected = true;
                        }
                    } else {
                        // Missing field is treated as ambiguous.
                        eval.ambiguity_detected = true;
                    }
                }

                ObservableRole::UpdateConfidence => {
                    // Extract a numeric confidence value from the field.
                    if let Some(v) = field_value
                        && let Some(c) = v.as_f64() {
                            eval.confidence_update = Some(c);
                        }
                }

                ObservableRole::General => {
                    // General observables are informational; no evaluation logic.
                }
            }
        }

        eval
    }
}

/// Intermediate output of the 8-step skill execution lifecycle.
/// Bundled into a struct to keep `execute_skill_lifecycle` return type clean.
struct SkillLifecycleOutput {
    observation: Observation,
    belief_patch: BeliefPatch,
    bindings: Vec<WorkingBinding>,
    precondition_results: Vec<PreconditionResult>,
    termination_reason: Option<TerminationType>,
    rollback_invoked: bool,
    observable_evaluation: ObservableEvaluation,
}

/// Summary of evaluating a skill's declared observables against an observation.
///
/// After skill execution, the session controller checks whether the observation
/// satisfies the skill's declared observable fields.  Each role-specific flag
/// aggregates the outcome across all observables of that role.
#[derive(Debug, Clone, Default)]
pub struct ObservableEvaluation {
    /// True when at least one ConfirmSuccess observable was found and its
    /// named field exists in the observation's structured_result.
    pub success_confirmed: bool,
    /// True when at least one DetectPartialSuccess observable was found and
    /// its field indicates partial completion.
    pub partial_detected: bool,
    /// True when at least one DetectAmbiguity observable was found and its
    /// field value is null or contains an ambiguity marker.
    pub ambiguity_detected: bool,
    /// If an UpdateConfidence observable exists and its field contains a
    /// numeric value, the extracted confidence; otherwise None.
    pub confidence_update: Option<f64>,
}

/// Check whether any of the skill's declared termination conditions fired
/// based on the outcome recorded in the observation.
fn evaluate_termination_conditions(
    skill: &SkillSpec,
    obs: &Observation,
) -> Option<TerminationType> {
    use crate::types::common::SkillFailureClass;
    for tc in &skill.termination_conditions {
        let fired = match tc.condition_type {
            TerminationType::Success => obs.success,
            TerminationType::Failure => !obs.success,
            TerminationType::Timeout => {
                obs.failure_class == Some(SkillFailureClass::Timeout)
            }
            TerminationType::BudgetExhaustion => {
                obs.failure_class == Some(SkillFailureClass::BudgetExhaustion)
            }
            TerminationType::PolicyDenial => {
                obs.failure_class == Some(SkillFailureClass::PolicyDenial)
            }
            TerminationType::ExternalError => {
                obs.failure_class == Some(SkillFailureClass::PortFailure)
                    || obs.failure_class == Some(SkillFailureClass::RemoteFailure)
            }
            // ExplicitAbort is only triggered via the abort() call, not from
            // an observation outcome.
            TerminationType::ExplicitAbort => false,
        };
        if fired {
            return Some(tc.condition_type);
        }
    }
    None
}

impl SessionRuntime for SessionController {
    fn create_session(&mut self, goal: GoalSpec) -> Result<ControlSession> {
        // Step 1 (partial): validate goal and budget.
        Self::validate_goal(&goal)?;

        let session_id = Uuid::new_v4();
        let now = Utc::now();

        // Step 2: build initial belief state.
        let belief = self.belief_source.build_initial_belief(&goal)?;

        let budget = Budget {
            risk_remaining: goal.risk_budget,
            latency_remaining_ms: goal.latency_budget_ms,
            resource_remaining: goal.resource_budget,
            steps_remaining: self.default_max_steps,
        };

        let session = ControlSession {
            session_id,
            goal,
            belief,
            working_memory: WorkingMemory {
                active_bindings: Vec::new(),
                unresolved_slots: Vec::new(),
                current_subgoal: None,
                recent_observations: Vec::new(),
                candidate_shortlist: Vec::new(),
                current_branch_state: None,
                budget_deltas: Vec::new(),
                output_bindings: Vec::new(),
                active_plan: None,
                plan_step: 0,
            },
            status: SessionStatus::Created,
            trace: SessionTrace { steps: Vec::new() },
            budget_remaining: budget,
            created_at: now,
            updated_at: now,
        };

        info!(session_id = %session.session_id, "session created");
        self.metrics.session_created();
        self.sessions.insert(session_id, session.clone());
        Ok(session)
    }

    fn run_step(&mut self, session: &mut ControlSession) -> Result<StepResult> {
        // ---------------------------------------------------------------
        // Pre-step guards
        // ---------------------------------------------------------------
        match session.status {
            SessionStatus::Completed | SessionStatus::Failed | SessionStatus::Aborted => {
                return Err(SomaError::Session(format!(
                    "session {} is in terminal state {:?}",
                    session.session_id, session.status
                )));
            }
            SessionStatus::Paused => {
                return Err(SomaError::Session(format!(
                    "session {} is paused; call resume first",
                    session.session_id
                )));
            }
            SessionStatus::BlockedByPolicy => {
                return Err(SomaError::Session(format!(
                    "session {} is blocked by policy",
                    session.session_id
                )));
            }
            _ => {}
        }

        // Transition to Running if this is the first step.
        if session.status == SessionStatus::Created {
            session.status = SessionStatus::Running;
        }

        let step_index = session.trace.steps.len() as u32;

        // ---------------------------------------------------------------
        // Step 1: validate budget
        // ---------------------------------------------------------------
        if !Self::budget_allows_step(&session.budget_remaining) {
            session.status = SessionStatus::Failed;
            session.updated_at = Utc::now();
            return Err(SomaError::BudgetExhausted(format!(
                "session {} exhausted budget at step {}",
                session.session_id, step_index
            )));
        }

        // Check deadline.
        if let Some(deadline) = session.goal.deadline
            && Utc::now() >= deadline
        {
            session.status = SessionStatus::Failed;
            session.updated_at = Utc::now();
            return Err(SomaError::BudgetExhausted(format!(
                "session {} passed deadline",
                session.session_id
            )));
        }

        debug!(session_id = %session.session_id, step = step_index, "control loop step begin");

        // ---------------------------------------------------------------
        // Step 2: capture belief summary before mutations
        // ---------------------------------------------------------------
        let belief_summary_before =
            serde_json::to_value(&session.belief).unwrap_or_default();

        // ---------------------------------------------------------------
        // Step 3: retrieve nearest episodes
        // ---------------------------------------------------------------
        let episodes =
            self.episode_memory
                .retrieve_nearest(&session.goal, &session.belief, 5);

        // ---------------------------------------------------------------
        // Step 4: retrieve matching schemas
        // ---------------------------------------------------------------
        let schemas = self
            .schema_memory
            .retrieve_matching(&session.goal, &session.belief);

        // ---------------------------------------------------------------
        // Step 5: retrieve matching routines
        // ---------------------------------------------------------------
        let routines = self
            .routine_memory
            .retrieve_matching(&session.goal, &session.belief);

        // ---------------------------------------------------------------
        // Step 6: enumerate valid skill candidates
        // ---------------------------------------------------------------
        let candidates = self.skill_registry.enumerate_candidates(
            &session.goal,
            &session.belief,
            &schemas,
            &routines,
        );

        if candidates.is_empty() {
            session.status = SessionStatus::Failed;
            session.updated_at = Utc::now();
            return Err(SomaError::NoCandidates);
        }

        // Check policy before finalising the candidate list.
        let pre_candidate_policy = self.policy_engine.check_hook(
            PolicyHook::BeforeCandidateSelection,
            &candidates[0],
            session,
        );
        if !pre_candidate_policy.allowed {
            session.status = SessionStatus::BlockedByPolicy;
            session.updated_at = Utc::now();
            self.metrics.policy_denial();
            return Err(SomaError::PolicyDenied {
                action: "candidate_selection".into(),
                reason: pre_candidate_policy.reason,
            });
        }

        // Update working memory shortlist.
        session.working_memory.candidate_shortlist =
            candidates.iter().map(|c| c.skill_id.clone()).collect();

        // ---------------------------------------------------------------
        // Step 6b: plan-following — load or advance an active plan
        // ---------------------------------------------------------------
        // If no active plan and a routine was found, load the routine's
        // compiled skill path as the plan.
        if session.working_memory.active_plan.is_none()
            && let Some(routine) = routines.iter().find(|r| !r.compiled_skill_path.is_empty())
        {
            debug!(
                session_id = %session.session_id,
                routine_id = %routine.routine_id,
                path_len = routine.compiled_skill_path.len(),
                "activating plan-following mode from routine"
            );
            session.working_memory.active_plan = Some(routine.compiled_skill_path.clone());
            session.working_memory.plan_step = 0;
        }

        // If active plan is exhausted, clear it.
        if let Some(ref plan) = session.working_memory.active_plan
            && session.working_memory.plan_step >= plan.len()
        {
            debug!(
                session_id = %session.session_id,
                "plan-following complete, clearing active plan"
            );
            session.working_memory.active_plan = None;
            session.working_memory.plan_step = 0;
        }

        // ---------------------------------------------------------------
        // Step 7: bind inputs from belief/resources
        // ---------------------------------------------------------------
        // Try to bind for each candidate; track unresolved slots.
        // We will bind for the eventually chosen candidate below, but we
        // need to know binding feasibility for scoring.
        session.working_memory.unresolved_slots.clear();

        // ---------------------------------------------------------------
        // Steps 8-10: score, rank, and choose a candidate.
        // In plan-following mode, skip scoring and use the next skill
        // from the active plan directly.
        // ---------------------------------------------------------------

        // Try plan-following first: resolve the next skill from the active plan.
        let plan_selected = session
            .working_memory
            .active_plan
            .as_ref()
            .and_then(|plan| plan.get(session.working_memory.plan_step))
            .cloned()
            .and_then(|skill_id| {
                self.skill_registry.get_skill(&skill_id).map(|s| {
                    debug!(
                        session_id = %session.session_id,
                        plan_step = session.working_memory.plan_step,
                        skill_id = %skill_id,
                        "plan-following: selecting skill from active plan"
                    );
                    (skill_id, s.clone())
                })
            });

        // If plan resolution failed (skill not found), abandon the plan.
        if session.working_memory.active_plan.is_some() && plan_selected.is_none() {
            warn!(
                session_id = %session.session_id,
                "plan skill not found in registry, abandoning plan"
            );
            session.working_memory.active_plan = None;
            session.working_memory.plan_step = 0;
        }

        let (scores, chosen_skill) = if let Some((skill_id, skill)) = plan_selected {
            let plan_score = CandidateScore {
                skill_id,
                score: 1.0,
                predicted_success: 0.95,
                predicted_cost: 0.01,
                predicted_latency_ms: 10,
                information_gain: 0.0,
            };
            (vec![plan_score], skill)
        } else {
            // Normal deliberation path: score, rank, choose.
            let scores = self.predictor.score(
                &candidates,
                &session.goal,
                &session.belief,
                &episodes,
            );
            let top_scores = self.predictor.predict_top(&scores, 3);
            if top_scores.is_empty() {
                session.status = SessionStatus::Failed;
                session.updated_at = Utc::now();
                return Err(SomaError::NoCandidates);
            }
            let chosen_score = &top_scores[0];
            let chosen_skill = match self.skill_registry.get_skill(&chosen_score.skill_id) {
                Some(skill) => skill.clone(),
                None => {
                    session.status = SessionStatus::Failed;
                    session.updated_at = Utc::now();
                    return Err(SomaError::SkillNotFound(chosen_score.skill_id.clone()));
                }
            };
            (scores, chosen_skill)
        };

        // ---------------------------------------------------------------
        // Policy check before execution
        // ---------------------------------------------------------------
        let policy_result = self.policy_engine.check_skill_execution(&chosen_skill, session);
        let policy_entries = vec![PolicyTraceEntry {
            action: format!("execute:{}", chosen_skill.skill_id),
            decision: if policy_result.allowed {
                "allowed".into()
            } else {
                "denied".into()
            },
            reason: policy_result.reason.clone(),
        }];

        if !policy_result.allowed {
            if policy_result.blocked_by_policy {
                session.status = SessionStatus::BlockedByPolicy;
                session.updated_at = Utc::now();
                self.metrics.policy_denial();

                // Still record the trace step (no observation).
                let empty_obs = Observation {
                    observation_id: Uuid::new_v4(),
                    session_id: session.session_id,
                    skill_id: Some(chosen_skill.skill_id.clone()),
                    port_calls: Vec::new(),
                    raw_result: serde_json::Value::Null,
                    structured_result: serde_json::Value::Null,
                    effect_patch: None,
                    success: false,
                    failure_class: None,
                    latency_ms: 0,
                    resource_cost: crate::types::observation::default_cost_profile(),
                    confidence: 0.0,
                    timestamp: Utc::now(),
                };
                let patch = Self::build_belief_patch(&empty_obs);
                Self::record_trace_step(
                    session,
                    step_index,
                    belief_summary_before,
                    &episodes,
                    &schemas,
                    &routines,
                    &candidates,
                    &scores,
                    &chosen_skill.skill_id,
                    &empty_obs,
                    &patch,
                    0.0,
                    CriticDecision::Stop,
                    policy_entries,
                    Vec::new(),
                    Vec::new(),
                    None,
                    false,
                );

                return Err(SomaError::PolicyDenied {
                    action: format!("execute:{}", chosen_skill.skill_id),
                    reason: policy_result.reason,
                });
            }

            if let Some(ref input_needed) = policy_result.waiting_for_input {
                session.status = SessionStatus::WaitingForInput;
                session.updated_at = Utc::now();
                return Ok(StepResult::WaitingForInput(input_needed.clone()));
            }
        }

        // ---------------------------------------------------------------
        // Execute the full skill lifecycle (bind → preconditions →
        // authorize → execute → collect → patch belief → termination)
        // ---------------------------------------------------------------
        session.working_memory.unresolved_slots.clear();

        let lifecycle_output = match self.execute_skill_lifecycle(
            session,
            &chosen_skill,
            step_index,
        ) {
            Ok(output) => output,
            Err(e) => {
                // Lifecycle errors (binding, precondition, policy denial)
                // are non-execution failures — the skill never ran.
                warn!(
                    session_id = %session.session_id,
                    skill = %chosen_skill.skill_id,
                    error = %e,
                    "skill lifecycle failed before execution"
                );
                match &e {
                    SomaError::PolicyDenied { .. } => {
                        session.status = SessionStatus::BlockedByPolicy;
                        session.updated_at = Utc::now();
                        return Err(e);
                    }
                    _ => {
                        // Binding and precondition failures are recoverable —
                        // the session waits for external input or a different
                        // candidate rather than terminating with an error.
                        session.status = SessionStatus::WaitingForInput;
                        session.updated_at = Utc::now();
                        return Ok(StepResult::WaitingForInput(e.to_string()));
                    }
                }
            }
        };

        let SkillLifecycleOutput {
            observation,
            belief_patch,
            bindings,
            precondition_results,
            termination_reason,
            rollback_invoked,
            observable_evaluation,
        } = lifecycle_output;

        // ---------------------------------------------------------------
        // Budget accounting
        // ---------------------------------------------------------------
        Self::deduct_budget(session, &observation, step_index);

        // Use observable evaluation to refine the progress assessment.
        // If ambiguity was detected, reduce confidence in the step.
        // If a confidence update was extracted, apply it to the belief.
        if let Some(conf) = observable_evaluation.confidence_update
            && let Some(fact) = session.belief.facts.last_mut() {
                fact.confidence = conf;
            }

        // ---------------------------------------------------------------
        // Critic evaluation
        // ---------------------------------------------------------------
        let progress_delta = if observation.success {
            if observable_evaluation.ambiguity_detected { 0.02 }
            else if observable_evaluation.partial_detected { 0.05 }
            else { 0.1 }
        } else {
            0.0
        };

        let raw_critic_decision = self.critic.evaluate(
            &session.goal,
            &session.belief,
            &observation,
            &session.budget_remaining,
            step_index,
        );

        // ---------------------------------------------------------------
        // Plan-following: override the critic decision based on plan state
        // ---------------------------------------------------------------
        let critic_decision = if session.working_memory.active_plan.is_some() {
            if !observation.success {
                // Failure during plan execution — abandon the plan and
                // fall back to deliberation on the next step.
                debug!(
                    session_id = %session.session_id,
                    step = step_index,
                    "plan step failed, abandoning active plan"
                );
                session.working_memory.active_plan = None;
                session.working_memory.plan_step = 0;
                CriticDecision::Revise
            } else {
                // Advance to the next step in the plan.
                session.working_memory.plan_step += 1;
                let plan_len = session
                    .working_memory
                    .active_plan
                    .as_ref()
                    .map(|p| p.len())
                    .unwrap_or(0);

                if session.working_memory.plan_step >= plan_len {
                    // Plan complete — signal completion.
                    debug!(
                        session_id = %session.session_id,
                        step = step_index,
                        "plan-following: all steps completed"
                    );
                    session.working_memory.active_plan = None;
                    session.working_memory.plan_step = 0;
                    CriticDecision::Stop
                } else {
                    // More steps remain — continue regardless of what the
                    // base critic decided (it would Stop on first success).
                    debug!(
                        session_id = %session.session_id,
                        step = step_index,
                        remaining = plan_len - session.working_memory.plan_step,
                        "plan-following: continuing to next plan step"
                    );
                    CriticDecision::Continue
                }
            }
        } else {
            raw_critic_decision
        };

        debug!(
            session_id = %session.session_id,
            step = step_index,
            critic = ?critic_decision,
            "critic decision"
        );

        // ---------------------------------------------------------------
        // Failure recovery: when the observation reports failure, consult
        // handle_failure to decide the recovery strategy.
        // ---------------------------------------------------------------
        let step_result = if !observation.success {
            let recovery = self.handle_failure(session, &chosen_skill, &observation, step_index);
            debug!(
                session_id = %session.session_id,
                step = step_index,
                recovery = ?recovery,
                "failure recovery action"
            );
            match recovery {
                FailureRecoveryAction::Retry => {
                    // Signal the caller to re-enter the loop with the same state.
                    StepResult::Continue
                }
                FailureRecoveryAction::SwitchCandidate => {
                    // Clear shortlist so the next step selects a different candidate.
                    session.working_memory.candidate_shortlist.clear();
                    StepResult::Continue
                }
                FailureRecoveryAction::Backtrack => {
                    session.working_memory.current_branch_state = Some(serde_json::json!({
                        "backtrack_from_step": step_index,
                        "failed_skill": chosen_skill.skill_id,
                    }));
                    StepResult::Continue
                }
                FailureRecoveryAction::Delegate => {
                    StepResult::WaitingForRemote(format!(
                        "delegating after failure of {}",
                        chosen_skill.skill_id
                    ))
                }
                FailureRecoveryAction::Stop => {
                    session.status = SessionStatus::Failed;
                    StepResult::Failed(format!(
                        "unrecoverable failure in skill {}",
                        chosen_skill.skill_id
                    ))
                }
            }
        } else {
            // Successful observation — use the critic's decision.
            let result = Self::critic_to_step_result(critic_decision);

            // On backtrack, store a marker for the next step to re-plan from.
            if critic_decision == CriticDecision::Backtrack {
                session.working_memory.current_branch_state = Some(serde_json::json!({
                    "backtrack_from_step": step_index,
                    "failed_skill": chosen_skill.skill_id,
                }));
            }

            // On revise, clear the candidate shortlist so the next step
            // re-enumerates with fresh scoring.
            if critic_decision == CriticDecision::Revise {
                session.working_memory.candidate_shortlist.clear();
            }

            result
        };

        // Record the step execution in metrics.
        self.metrics.step_executed();
        self.metrics.skill_invoked(&chosen_skill.skill_id);

        // Record port call latencies from the observation.
        for pc in &observation.port_calls {
            self.metrics.port_call(&pc.port_id, pc.latency_ms);
        }

        // Update session status based on step result.
        match &step_result {
            StepResult::Completed => {
                session.status = SessionStatus::Completed;
                self.metrics.session_completed();
            }
            StepResult::Failed(reason) => {
                session.status = SessionStatus::Failed;
                self.metrics.session_failed();
                warn!(session_id = %session.session_id, reason = %reason, "session failed");
            }
            StepResult::Aborted => {
                session.status = SessionStatus::Aborted;
                self.metrics.session_aborted();
            }
            StepResult::WaitingForInput(_) => {
                session.status = SessionStatus::WaitingForInput;
            }
            StepResult::WaitingForRemote(_) => {
                session.status = SessionStatus::WaitingForRemote;
            }
            StepResult::Continue => {
                // Stay Running.
            }
        }

        // ---------------------------------------------------------------
        // Persist trace and memory
        // ---------------------------------------------------------------
        Self::record_trace_step(
            session,
            step_index,
            belief_summary_before,
            &episodes,
            &schemas,
            &routines,
            &candidates,
            &scores,
            &chosen_skill.skill_id,
            &observation,
            &belief_patch,
            progress_delta,
            critic_decision,
            policy_entries,
            bindings.clone(),
            precondition_results,
            termination_reason,
            rollback_invoked,
        );

        session.updated_at = Utc::now();

        // Keep the internal map in sync.
        self.sessions
            .insert(session.session_id, session.clone());

        info!(
            session_id = %session.session_id,
            step = step_index,
            result = ?step_result,
            "control loop step complete"
        );

        Ok(step_result)
    }

    fn pause(&mut self, session: &mut ControlSession) -> Result<()> {
        match session.status {
            SessionStatus::Running
            | SessionStatus::WaitingForInput
            | SessionStatus::WaitingForRemote => {
                session.status = SessionStatus::Paused;
                session.updated_at = Utc::now();
                self.sessions.insert(session.session_id, session.clone());
                info!(session_id = %session.session_id, "session paused");
                Ok(())
            }
            _ => Err(SomaError::Session(format!(
                "cannot pause session {} in state {:?}",
                session.session_id, session.status
            ))),
        }
    }

    fn resume(&mut self, session: &mut ControlSession) -> Result<()> {
        if session.status != SessionStatus::Paused {
            return Err(SomaError::Session(format!(
                "cannot resume session {} in state {:?}; expected Paused",
                session.session_id, session.status
            )));
        }
        session.status = SessionStatus::Running;
        session.updated_at = Utc::now();
        self.sessions.insert(session.session_id, session.clone());
        info!(session_id = %session.session_id, "session resumed");
        Ok(())
    }

    fn abort(&mut self, session: &mut ControlSession) -> Result<()> {
        match session.status {
            SessionStatus::Completed | SessionStatus::Aborted => {
                return Err(SomaError::Session(format!(
                    "session {} is already in terminal state {:?}",
                    session.session_id, session.status
                )));
            }
            _ => {}
        }
        session.status = SessionStatus::Aborted;
        session.updated_at = Utc::now();
        self.metrics.session_aborted();
        self.sessions.insert(session.session_id, session.clone());
        info!(session_id = %session.session_id, "session aborted");
        Ok(())
    }

    fn get_session(&self, session_id: &Uuid) -> Option<&ControlSession> {
        self.sessions.get(session_id)
    }

    fn checkpoint(&self, session: &ControlSession) -> Result<Vec<u8>> {
        serde_json::to_vec(session).map_err(SomaError::from)
    }

    fn restore(&mut self, data: &[u8]) -> Result<ControlSession> {
        let session: ControlSession =
            serde_json::from_slice(data).map_err(SomaError::from)?;
        self.sessions.insert(session.session_id, session.clone());
        info!(session_id = %session.session_id, "session restored from checkpoint");
        Ok(session)
    }
}

// ---------------------------------------------------------------------------
// Default no-op implementations of subsystem interfaces
// ---------------------------------------------------------------------------

/// Default no-op belief source that returns an empty belief state.
pub struct NoopBeliefSource;

impl BeliefSource for NoopBeliefSource {
    fn build_initial_belief(&self, goal: &GoalSpec) -> Result<BeliefState> {
        Ok(BeliefState {
            belief_id: Uuid::new_v4(),
            session_id: goal.goal_id,
            resources: Vec::new(),
            facts: Vec::new(),
            uncertainties: Vec::new(),
            provenance: Vec::new(),
            active_bindings: Vec::new(),
            world_hash: String::new(),
            updated_at: Utc::now(),
        })
    }

    fn apply_patch(&self, belief: &mut BeliefState, patch: &BeliefPatch) -> Result<()> {
        // Minimal: append new facts and resources.
        belief.facts.extend(patch.added_facts.iter().cloned());
        belief
            .resources
            .extend(patch.added_resources.iter().cloned());
        for update in &patch.updated_facts {
            if let Some(existing) = belief.facts.iter_mut().find(|f| f.fact_id == update.fact_id) {
                *existing = update.clone();
            }
        }
        for update in &patch.updated_resources {
            if let Some(existing) = belief
                .resources
                .iter_mut()
                .find(|r| r.resource_ref == update.resource_ref)
            {
                *existing = update.clone();
            }
        }
        belief
            .facts
            .retain(|f| !patch.removed_fact_ids.contains(&f.fact_id));
        for binding in &patch.binding_updates {
            if let Some(existing) = belief
                .active_bindings
                .iter_mut()
                .find(|b| b.name == binding.name)
            {
                *existing = binding.clone();
            } else {
                belief.active_bindings.push(binding.clone());
            }
        }
        belief.updated_at = Utc::now();
        Ok(())
    }
}

/// Default no-op episode memory that returns no episodes.
pub struct NoopEpisodeMemory;

impl EpisodeMemory for NoopEpisodeMemory {
    fn retrieve_nearest(&self, _goal: &GoalSpec, _belief: &BeliefState, _limit: usize) -> Vec<Episode> {
        Vec::new()
    }
}

/// Default no-op schema memory that returns no schemas.
pub struct NoopSchemaMemory;

impl SchemaMemory for NoopSchemaMemory {
    fn retrieve_matching(&self, _goal: &GoalSpec, _belief: &BeliefState) -> Vec<Schema> {
        Vec::new()
    }
}

/// Default no-op routine memory that returns no routines.
pub struct NoopRoutineMemory;

impl RoutineMemory for NoopRoutineMemory {
    fn retrieve_matching(&self, _goal: &GoalSpec, _belief: &BeliefState) -> Vec<Routine> {
        Vec::new()
    }
}

/// Default no-op skill registry that returns no candidates.
pub struct NoopSkillRegistry;

impl SkillRegistry for NoopSkillRegistry {
    fn enumerate_candidates(
        &self,
        _goal: &GoalSpec,
        _belief: &BeliefState,
        _schemas: &[Schema],
        _routines: &[Routine],
    ) -> Vec<SkillSpec> {
        Vec::new()
    }

    fn get_skill(&self, _skill_id: &str) -> Option<&SkillSpec> {
        None
    }
}

/// Default no-op skill executor that returns a synthetic success observation.
pub struct NoopSkillExecutor;

impl SkillExecutor for NoopSkillExecutor {
    fn bind_inputs(
        &self,
        _skill: &SkillSpec,
        _belief: &BeliefState,
        _working_memory: &WorkingMemory,
    ) -> Result<Vec<WorkingBinding>> {
        Ok(Vec::new())
    }

    fn check_preconditions(
        &self,
        _skill: &SkillSpec,
        _belief: &BeliefState,
        _working_memory: &WorkingMemory,
    ) -> Result<Vec<PreconditionResult>> {
        Ok(Vec::new())
    }

    fn execute(
        &self,
        skill: &SkillSpec,
        _bindings: &[WorkingBinding],
        session_id: Uuid,
    ) -> Result<Observation> {
        Ok(Observation {
            observation_id: Uuid::new_v4(),
            session_id,
            skill_id: Some(skill.skill_id.clone()),
            port_calls: Vec::new(),
            raw_result: serde_json::json!({"status": "noop"}),
            structured_result: serde_json::json!({"status": "noop"}),
            effect_patch: None,
            success: true,
            failure_class: None,
            latency_ms: 1,
            resource_cost: crate::types::observation::default_cost_profile(),
            confidence: 1.0,
            timestamp: Utc::now(),
        })
    }

    fn invoke_rollback(
        &self,
        _skill: &SkillSpec,
        _bindings: &[WorkingBinding],
        _session_id: Uuid,
    ) -> Result<Option<Observation>> {
        Ok(None)
    }
}

/// Default no-op predictor that assigns uniform scores.
pub struct NoopPredictor;

impl CandidatePredictor for NoopPredictor {
    fn score(
        &self,
        candidates: &[SkillSpec],
        _goal: &GoalSpec,
        _belief: &BeliefState,
        _episodes: &[Episode],
    ) -> Vec<CandidateScore> {
        candidates
            .iter()
            .enumerate()
            .map(|(i, c)| CandidateScore {
                skill_id: c.skill_id.clone(),
                score: 1.0 / (1.0 + i as f64),
                predicted_success: 0.5,
                predicted_cost: 0.1,
                predicted_latency_ms: 100,
                information_gain: 0.0,
            })
            .collect()
    }

    fn predict_top(
        &self,
        scored: &[CandidateScore],
        limit: usize,
    ) -> Vec<CandidateScore> {
        let mut sorted = scored.to_vec();
        sorted.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        sorted.truncate(limit);
        sorted
    }
}

/// Default no-op critic that always returns Continue.
pub struct NoopCritic;

impl Critic for NoopCritic {
    fn evaluate(
        &self,
        _goal: &GoalSpec,
        _belief: &BeliefState,
        _observation: &Observation,
        _budget: &Budget,
        _step_index: u32,
    ) -> CriticDecision {
        CriticDecision::Continue
    }
}

/// Default no-op policy engine that allows all actions.
pub struct NoopPolicyEngine;

impl PolicyEngine for NoopPolicyEngine {
    fn check_skill_execution(
        &self,
        _skill: &SkillSpec,
        _session: &ControlSession,
    ) -> PolicyCheckResult {
        PolicyCheckResult {
            allowed: true,
            reason: "default policy: all actions allowed".into(),
            blocked_by_policy: false,
            waiting_for_input: None,
        }
    }

    fn check_hook(
        &self,
        _hook: PolicyHook,
        _skill: &SkillSpec,
        _session: &ControlSession,
    ) -> PolicyCheckResult {
        PolicyCheckResult {
            allowed: true,
            reason: "default policy: all actions allowed".into(),
            blocked_by_policy: false,
            waiting_for_input: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::common::*;
    use crate::types::goal::*;
    use crate::types::skill::*;

    /// Build a minimal valid GoalSpec for testing.
    fn test_goal() -> GoalSpec {
        GoalSpec {
            goal_id: Uuid::new_v4(),
            source: GoalSource {
                source_type: GoalSourceType::User,
                identity: Some("test".into()),
                session_id: None,
                peer_id: None,
            },
            objective: Objective {
                description: "test objective".into(),
                structured: None,
            },
            constraints: Vec::new(),
            success_conditions: Vec::new(),
            risk_budget: 1.0,
            latency_budget_ms: 10_000,
            resource_budget: 1.0,
            deadline: None,
            permissions_scope: Vec::new(),
            priority: Priority::Normal,
        }
    }

    /// Build a minimal SkillSpec for testing.
    fn test_skill(id: &str) -> SkillSpec {
        SkillSpec {
            skill_id: id.into(),
            namespace: "test-pack".into(),
            pack: "test-pack".into(),
            kind: SkillKind::Primitive,
            name: id.into(),
            description: "test skill".into(),
            version: "0.1.0".into(),
            inputs: SchemaRef {
                schema: serde_json::json!({}),
            },
            outputs: SchemaRef {
                schema: serde_json::json!({}),
            },
            required_resources: Vec::new(),
            preconditions: Vec::new(),
            expected_effects: Vec::new(),
            observables: Vec::new(),
            termination_conditions: Vec::new(),
            rollback_or_compensation: RollbackSpec {
                support: RollbackSupport::Irreversible,
                compensation_skill: None,
                description: "none".into(),
            },
            cost_prior: CostPrior {
                latency: LatencyProfile {
                    expected_latency_ms: 10,
                    p95_latency_ms: 50,
                    max_latency_ms: 100,
                },
                resource_cost: CostProfile {
                    cpu_cost_class: CostClass::Low,
                    memory_cost_class: CostClass::Low,
                    io_cost_class: CostClass::Negligible,
                    network_cost_class: CostClass::Negligible,
                    energy_cost_class: CostClass::Negligible,
                },
            },
            risk_class: RiskClass::Negligible,
            determinism: DeterminismClass::Deterministic,
            remote_exposure: RemoteExposureDecl {
                remote_scope: CapabilityScope::Local,
                peer_trust_requirements: String::new(),
                serialization_requirements: String::new(),
                rate_limits: String::new(),
                replay_protection: false,
                observation_streaming: false,
                delegation_support: false,
                enabled: false,
            },
            tags: Vec::new(),
            aliases: Vec::new(),
            capability_requirements: Vec::new(),
            subskills: Vec::new(),
            guard_conditions: Vec::new(),
            match_conditions: Vec::new(),
            telemetry_fields: Vec::new(),
            policy_overrides: Vec::new(),
            confidence_threshold: None,
            locality: None,
            remote_endpoint: None,
            remote_trust_requirement: None,
            remote_capability_contract: None,
            fallback_skill: None,
            invalidation_conditions: Vec::new(),
            nondeterminism_sources: Vec::new(),
            partial_success_behavior: None,
        }
    }

    /// A skill registry that returns a fixed set of skills.
    struct TestSkillRegistry {
        skills: Vec<SkillSpec>,
    }

    impl SkillRegistry for TestSkillRegistry {
        fn enumerate_candidates(
            &self,
            _goal: &GoalSpec,
            _belief: &BeliefState,
            _schemas: &[Schema],
            _routines: &[Routine],
        ) -> Vec<SkillSpec> {
            self.skills.clone()
        }

        fn get_skill(&self, skill_id: &str) -> Option<&SkillSpec> {
            self.skills.iter().find(|s| s.skill_id == skill_id)
        }
    }

    fn test_metrics() -> Arc<RuntimeMetrics> {
        Arc::new(RuntimeMetrics::new())
    }

    fn make_controller_with_skills(skills: Vec<SkillSpec>) -> SessionController {
        SessionController::new(SessionControllerDeps {
            belief_source: Box::new(NoopBeliefSource),
            episode_memory: Box::new(NoopEpisodeMemory),
            schema_memory: Box::new(NoopSchemaMemory),
            routine_memory: Box::new(NoopRoutineMemory),
            skill_registry: Box::new(TestSkillRegistry { skills }),
            skill_executor: Box::new(NoopSkillExecutor),
            predictor: Box::new(NoopPredictor),
            critic: Box::new(NoopCritic),
            policy_engine: Box::new(NoopPolicyEngine),
            remote_executor: None,
            capability_scope_checker: None,
        }, test_metrics())
    }

    fn make_stub_controller() -> SessionController {
        SessionController::new(SessionControllerDeps {
            belief_source: Box::new(NoopBeliefSource),
            episode_memory: Box::new(NoopEpisodeMemory),
            schema_memory: Box::new(NoopSchemaMemory),
            routine_memory: Box::new(NoopRoutineMemory),
            skill_registry: Box::new(NoopSkillRegistry),
            skill_executor: Box::new(NoopSkillExecutor),
            predictor: Box::new(NoopPredictor),
            critic: Box::new(NoopCritic),
            policy_engine: Box::new(NoopPolicyEngine),
            remote_executor: None,
            capability_scope_checker: None,
        }, test_metrics())
    }

    #[test]
    fn test_create_session() {
        let mut ctrl = make_stub_controller();
        let goal = test_goal();
        let session = ctrl.create_session(goal.clone()).unwrap();
        assert_eq!(session.status, SessionStatus::Created);
        assert_eq!(session.goal.goal_id, goal.goal_id);
        assert!(session.trace.steps.is_empty());
        assert_eq!(session.budget_remaining.risk_remaining, 1.0);
        assert_eq!(session.budget_remaining.latency_remaining_ms, 10_000);
    }

    #[test]
    fn test_create_session_invalid_goal_empty_objective() {
        let mut ctrl = make_stub_controller();
        let mut goal = test_goal();
        goal.objective.description = String::new();
        let result = ctrl.create_session(goal);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_session_invalid_goal_zero_budget() {
        let mut ctrl = make_stub_controller();
        let mut goal = test_goal();
        goal.risk_budget = 0.0;
        assert!(ctrl.create_session(goal).is_err());
    }

    #[test]
    fn test_run_step_no_candidates() {
        let mut ctrl = make_stub_controller();
        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        let result = ctrl.run_step(&mut session);
        assert!(result.is_err());
        assert_eq!(session.status, SessionStatus::Failed);
    }

    #[test]
    fn test_run_step_with_candidate() {
        let skill = test_skill("test:echo");
        let mut ctrl = make_controller_with_skills(vec![skill]);
        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        let result = ctrl.run_step(&mut session).unwrap();
        assert!(matches!(result, StepResult::Continue));
        assert_eq!(session.status, SessionStatus::Running);
        assert_eq!(session.trace.steps.len(), 1);
        assert_eq!(session.trace.steps[0].selected_skill, "test:echo");
    }

    #[test]
    fn test_run_step_deducts_budget() {
        let skill = test_skill("test:echo");
        let mut ctrl = make_controller_with_skills(vec![skill]);
        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        let steps_before = session.budget_remaining.steps_remaining;
        ctrl.run_step(&mut session).unwrap();
        assert_eq!(
            session.budget_remaining.steps_remaining,
            steps_before - 1
        );
        assert!(session.budget_remaining.latency_remaining_ms < 10_000);
    }

    #[test]
    fn test_pause_resume() {
        let skill = test_skill("test:echo");
        let mut ctrl = make_controller_with_skills(vec![skill]);
        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        ctrl.run_step(&mut session).unwrap();
        assert_eq!(session.status, SessionStatus::Running);

        ctrl.pause(&mut session).unwrap();
        assert_eq!(session.status, SessionStatus::Paused);

        // Cannot run step while paused.
        assert!(ctrl.run_step(&mut session).is_err());

        ctrl.resume(&mut session).unwrap();
        assert_eq!(session.status, SessionStatus::Running);

        // Can run step after resume.
        ctrl.run_step(&mut session).unwrap();
    }

    #[test]
    fn test_abort() {
        let skill = test_skill("test:echo");
        let mut ctrl = make_controller_with_skills(vec![skill]);
        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        ctrl.run_step(&mut session).unwrap();
        ctrl.abort(&mut session).unwrap();
        assert_eq!(session.status, SessionStatus::Aborted);

        // Cannot run step after abort.
        assert!(ctrl.run_step(&mut session).is_err());
    }

    #[test]
    fn test_abort_already_aborted() {
        let mut ctrl = make_stub_controller();
        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        ctrl.abort(&mut session).unwrap();
        assert!(ctrl.abort(&mut session).is_err());
    }

    #[test]
    fn test_get_session() {
        let mut ctrl = make_stub_controller();
        let goal = test_goal();
        let session = ctrl.create_session(goal).unwrap();
        let retrieved = ctrl.get_session(&session.session_id);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().session_id, session.session_id);

        let missing = ctrl.get_session(&Uuid::new_v4());
        assert!(missing.is_none());
    }

    #[test]
    fn test_checkpoint_restore() {
        let skill = test_skill("test:echo");
        let mut ctrl = make_controller_with_skills(vec![skill]);
        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        ctrl.run_step(&mut session).unwrap();

        let data = ctrl.checkpoint(&session).unwrap();
        assert!(!data.is_empty());

        let mut ctrl2 = make_stub_controller();
        let restored = ctrl2.restore(&data).unwrap();
        assert_eq!(restored.session_id, session.session_id);
        assert_eq!(restored.status, session.status);
        assert_eq!(restored.trace.steps.len(), session.trace.steps.len());
    }

    #[test]
    fn test_multiple_steps_trace_accumulation() {
        let skill = test_skill("test:echo");
        let mut ctrl = make_controller_with_skills(vec![skill]);
        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();

        for i in 0..5 {
            ctrl.run_step(&mut session).unwrap();
            assert_eq!(session.trace.steps.len(), i + 1);
        }

        assert_eq!(session.budget_remaining.steps_remaining, 95);
        assert_eq!(session.working_memory.recent_observations.len(), 5);
    }

    #[test]
    fn test_working_memory_observations_bounded() {
        let skill = test_skill("test:echo");
        let mut ctrl = make_controller_with_skills(vec![skill]);
        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();

        for _ in 0..25 {
            ctrl.run_step(&mut session).unwrap();
        }

        // recent_observations is bounded to 20.
        assert_eq!(session.working_memory.recent_observations.len(), 20);
    }

    #[test]
    fn test_cannot_pause_created_session() {
        let mut ctrl = make_stub_controller();
        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        assert!(ctrl.pause(&mut session).is_err());
    }

    #[test]
    fn test_cannot_resume_running_session() {
        let skill = test_skill("test:echo");
        let mut ctrl = make_controller_with_skills(vec![skill]);
        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        ctrl.run_step(&mut session).unwrap();
        assert!(ctrl.resume(&mut session).is_err());
    }

    #[test]
    fn test_step_result_from_stop_critic() {
        /// A critic that stops after the first step.
        struct StopCritic;
        impl Critic for StopCritic {
            fn evaluate(
                &self,
                _goal: &GoalSpec,
                _belief: &BeliefState,
                _observation: &Observation,
                _budget: &Budget,
                _step_index: u32,
            ) -> CriticDecision {
                CriticDecision::Stop
            }
        }

        let skill = test_skill("test:echo");
        let mut ctrl = SessionController::new(SessionControllerDeps {
            belief_source: Box::new(NoopBeliefSource),
            episode_memory: Box::new(NoopEpisodeMemory),
            schema_memory: Box::new(NoopSchemaMemory),
            routine_memory: Box::new(NoopRoutineMemory),
            skill_registry: Box::new(TestSkillRegistry {
                skills: vec![skill],
            }),
            skill_executor: Box::new(NoopSkillExecutor),
            predictor: Box::new(NoopPredictor),
            critic: Box::new(StopCritic),
            policy_engine: Box::new(NoopPolicyEngine),
            remote_executor: None,
            capability_scope_checker: None,
        }, test_metrics());

        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        let result = ctrl.run_step(&mut session).unwrap();
        assert!(matches!(result, StepResult::Completed));
        assert_eq!(session.status, SessionStatus::Completed);
    }

    #[test]
    fn test_policy_denial() {
        /// A policy engine that denies everything.
        struct DenyAllPolicy;
        impl PolicyEngine for DenyAllPolicy {
            fn check_skill_execution(
                &self,
                _skill: &SkillSpec,
                _session: &ControlSession,
            ) -> PolicyCheckResult {
                PolicyCheckResult {
                    allowed: false,
                    reason: "denied by test policy".into(),
                    blocked_by_policy: true,
                    waiting_for_input: None,
                }
            }

            fn check_hook(
                &self,
                _hook: PolicyHook,
                _skill: &SkillSpec,
                _session: &ControlSession,
            ) -> PolicyCheckResult {
                PolicyCheckResult {
                    allowed: false,
                    reason: "denied by test policy".into(),
                    blocked_by_policy: true,
                    waiting_for_input: None,
                }
            }
        }

        let skill = test_skill("test:echo");
        let mut ctrl = SessionController::new(SessionControllerDeps {
            belief_source: Box::new(NoopBeliefSource),
            episode_memory: Box::new(NoopEpisodeMemory),
            schema_memory: Box::new(NoopSchemaMemory),
            routine_memory: Box::new(NoopRoutineMemory),
            skill_registry: Box::new(TestSkillRegistry {
                skills: vec![skill],
            }),
            skill_executor: Box::new(NoopSkillExecutor),
            predictor: Box::new(NoopPredictor),
            critic: Box::new(NoopCritic),
            policy_engine: Box::new(DenyAllPolicy),
            remote_executor: None,
            capability_scope_checker: None,
        }, test_metrics());

        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        let result = ctrl.run_step(&mut session);
        assert!(result.is_err());
        assert_eq!(session.status, SessionStatus::BlockedByPolicy);
    }

    #[test]
    fn test_deadline_expiry() {
        let skill = test_skill("test:echo");
        let mut ctrl = make_controller_with_skills(vec![skill]);
        let mut goal = test_goal();
        // Set deadline in the past.
        goal.deadline = Some(Utc::now() - chrono::Duration::seconds(10));
        let mut session = ctrl.create_session(goal).unwrap();
        let result = ctrl.run_step(&mut session);
        assert!(result.is_err());
        assert_eq!(session.status, SessionStatus::Failed);
    }

    // Failing preconditions prevent execution and put the session in WaitingForInput.
    #[test]
    fn test_preconditions_failing_blocks_execution() {
        struct FailingPreconditionExecutor;
        impl SkillExecutor for FailingPreconditionExecutor {
            fn bind_inputs(
                &self,
                _skill: &SkillSpec,
                _belief: &BeliefState,
                _working_memory: &WorkingMemory,
            ) -> Result<Vec<WorkingBinding>> {
                Ok(Vec::new())
            }

            fn check_preconditions(
                &self,
                skill: &SkillSpec,
                _belief: &BeliefState,
                _working_memory: &WorkingMemory,
            ) -> Result<Vec<PreconditionResult>> {
                Ok(vec![PreconditionResult {
                    description: "resource must exist".into(),
                    passed: false,
                    reason: format!("required resource not found for {}", skill.skill_id),
                }])
            }

            fn execute(
                &self,
                skill: &SkillSpec,
                _bindings: &[WorkingBinding],
                session_id: Uuid,
            ) -> Result<Observation> {
                Ok(Observation {
                    observation_id: Uuid::new_v4(),
                    session_id,
                    skill_id: Some(skill.skill_id.clone()),
                    port_calls: Vec::new(),
                    raw_result: serde_json::json!({}),
                    structured_result: serde_json::json!({}),
                    effect_patch: None,
                    success: true,
                    failure_class: None,
                    latency_ms: 1,
                    resource_cost: crate::types::observation::default_cost_profile(),
                    confidence: 1.0,
                    timestamp: Utc::now(),
                })
            }

            fn invoke_rollback(
                &self,
                _skill: &SkillSpec,
                _bindings: &[WorkingBinding],
                _session_id: Uuid,
            ) -> Result<Option<Observation>> {
                Ok(None)
            }
        }

        let skill = test_skill("test:check");
        let mut ctrl = SessionController::new(SessionControllerDeps {
            belief_source: Box::new(NoopBeliefSource),
            episode_memory: Box::new(NoopEpisodeMemory),
            schema_memory: Box::new(NoopSchemaMemory),
            routine_memory: Box::new(NoopRoutineMemory),
            skill_registry: Box::new(TestSkillRegistry { skills: vec![skill] }),
            skill_executor: Box::new(FailingPreconditionExecutor),
            predictor: Box::new(NoopPredictor),
            critic: Box::new(NoopCritic),
            policy_engine: Box::new(NoopPolicyEngine),
            remote_executor: None,
            capability_scope_checker: None,
        }, test_metrics());

        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        let result = ctrl.run_step(&mut session).unwrap();
        assert!(matches!(result, StepResult::WaitingForInput(_)));
        assert_eq!(session.status, SessionStatus::WaitingForInput);
        // No trace step should have been recorded because we returned early.
        assert!(session.trace.steps.is_empty());
    }

    // When a skill fails and has a compensation_skill set with non-Irreversible support,
    // the rollback executor is invoked and the trace step records rollback_invoked = true.
    #[test]
    fn test_rollback_triggered_on_failure() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let rollback_called = Arc::new(AtomicBool::new(false));

        struct FailingExecutor {
            rollback_called: Arc<AtomicBool>,
        }
        impl SkillExecutor for FailingExecutor {
            fn bind_inputs(
                &self,
                _skill: &SkillSpec,
                _belief: &BeliefState,
                _working_memory: &WorkingMemory,
            ) -> Result<Vec<WorkingBinding>> {
                Ok(Vec::new())
            }

            fn check_preconditions(
                &self,
                _skill: &SkillSpec,
                _belief: &BeliefState,
                _working_memory: &WorkingMemory,
            ) -> Result<Vec<PreconditionResult>> {
                Ok(Vec::new())
            }

            fn execute(
                &self,
                skill: &SkillSpec,
                _bindings: &[WorkingBinding],
                session_id: Uuid,
            ) -> Result<Observation> {
                Ok(Observation {
                    observation_id: Uuid::new_v4(),
                    session_id,
                    skill_id: Some(skill.skill_id.clone()),
                    port_calls: Vec::new(),
                    raw_result: serde_json::json!({"error": "simulated failure"}),
                    structured_result: serde_json::json!({"error": "simulated failure"}),
                    effect_patch: None,
                    success: false,
                    failure_class: Some(crate::types::common::SkillFailureClass::Unknown),
                    latency_ms: 1,
                    resource_cost: crate::types::observation::default_cost_profile(),
                    confidence: 0.0,
                    timestamp: Utc::now(),
                })
            }

            fn invoke_rollback(
                &self,
                skill: &SkillSpec,
                _bindings: &[WorkingBinding],
                session_id: Uuid,
            ) -> Result<Option<Observation>> {
                self.rollback_called.store(true, Ordering::SeqCst);
                Ok(Some(Observation {
                    observation_id: Uuid::new_v4(),
                    session_id,
                    skill_id: Some(skill.skill_id.clone()),
                    port_calls: Vec::new(),
                    raw_result: serde_json::json!({"status": "rolled_back"}),
                    structured_result: serde_json::json!({"status": "rolled_back"}),
                    effect_patch: None,
                    success: true,
                    failure_class: None,
                    latency_ms: 1,
                    resource_cost: crate::types::observation::default_cost_profile(),
                    confidence: 1.0,
                    timestamp: Utc::now(),
                }))
            }
        }

        // Build a skill that declares it can be compensated.
        let mut skill = test_skill("test:mutable");
        skill.rollback_or_compensation = RollbackSpec {
            support: RollbackSupport::CompensatingAction,
            compensation_skill: Some("test:undo".into()),
            description: "compensating action available".into(),
        };

        let mut ctrl = SessionController::new(SessionControllerDeps {
            belief_source: Box::new(NoopBeliefSource),
            episode_memory: Box::new(NoopEpisodeMemory),
            schema_memory: Box::new(NoopSchemaMemory),
            routine_memory: Box::new(NoopRoutineMemory),
            skill_registry: Box::new(TestSkillRegistry { skills: vec![skill] }),
            skill_executor: Box::new(FailingExecutor {
                rollback_called: rollback_called.clone(),
            }),
            predictor: Box::new(NoopPredictor),
            critic: Box::new(NoopCritic),
            policy_engine: Box::new(NoopPolicyEngine),
            remote_executor: None,
            capability_scope_checker: None,
        }, test_metrics());

        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        ctrl.run_step(&mut session).unwrap();

        assert!(rollback_called.load(Ordering::SeqCst), "rollback was not called");
        assert!(session.trace.steps[0].rollback_invoked);
    }

    // The BeforeCandidateSelection hook blocks the step when the policy denies it.
    #[test]
    fn test_policy_hook_before_candidate_selection_blocks_step() {
        struct HookDenyPolicy;
        impl PolicyEngine for HookDenyPolicy {
            fn check_skill_execution(
                &self,
                _skill: &SkillSpec,
                _session: &ControlSession,
            ) -> PolicyCheckResult {
                // Allow the general execution check so only the hook fires.
                PolicyCheckResult {
                    allowed: true,
                    reason: "ok".into(),
                    blocked_by_policy: false,
                    waiting_for_input: None,
                }
            }

            fn check_hook(
                &self,
                hook: PolicyHook,
                _skill: &SkillSpec,
                _session: &ControlSession,
            ) -> PolicyCheckResult {
                // Only deny at the candidate selection hook.
                if hook == PolicyHook::BeforeCandidateSelection {
                    PolicyCheckResult {
                        allowed: false,
                        reason: "candidate selection denied".into(),
                        blocked_by_policy: true,
                        waiting_for_input: None,
                    }
                } else {
                    PolicyCheckResult {
                        allowed: true,
                        reason: "ok".into(),
                        blocked_by_policy: false,
                        waiting_for_input: None,
                    }
                }
            }
        }

        let skill = test_skill("test:echo");
        let mut ctrl = SessionController::new(SessionControllerDeps {
            belief_source: Box::new(NoopBeliefSource),
            episode_memory: Box::new(NoopEpisodeMemory),
            schema_memory: Box::new(NoopSchemaMemory),
            routine_memory: Box::new(NoopRoutineMemory),
            skill_registry: Box::new(TestSkillRegistry { skills: vec![skill] }),
            skill_executor: Box::new(NoopSkillExecutor),
            predictor: Box::new(NoopPredictor),
            critic: Box::new(NoopCritic),
            policy_engine: Box::new(HookDenyPolicy),
            remote_executor: None,
            capability_scope_checker: None,
        }, test_metrics());

        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        let result = ctrl.run_step(&mut session);
        assert!(result.is_err());
        assert_eq!(session.status, SessionStatus::BlockedByPolicy);
    }

    // After a successful run_step, the trace step's precondition_results field
    // reflects the results returned by check_preconditions.
    #[test]
    fn test_trace_step_precondition_results_populated() {
        struct PassingPreconditionExecutor;
        impl SkillExecutor for PassingPreconditionExecutor {
            fn bind_inputs(
                &self,
                _skill: &SkillSpec,
                _belief: &BeliefState,
                _working_memory: &WorkingMemory,
            ) -> Result<Vec<WorkingBinding>> {
                Ok(Vec::new())
            }

            fn check_preconditions(
                &self,
                _skill: &SkillSpec,
                _belief: &BeliefState,
                _working_memory: &WorkingMemory,
            ) -> Result<Vec<PreconditionResult>> {
                Ok(vec![
                    PreconditionResult {
                        description: "network reachable".into(),
                        passed: true,
                        reason: "ping succeeded".into(),
                    },
                    PreconditionResult {
                        description: "auth token present".into(),
                        passed: true,
                        reason: "token found in belief".into(),
                    },
                ])
            }

            fn execute(
                &self,
                skill: &SkillSpec,
                _bindings: &[WorkingBinding],
                session_id: Uuid,
            ) -> Result<Observation> {
                Ok(Observation {
                    observation_id: Uuid::new_v4(),
                    session_id,
                    skill_id: Some(skill.skill_id.clone()),
                    port_calls: Vec::new(),
                    raw_result: serde_json::json!({}),
                    structured_result: serde_json::json!({}),
                    effect_patch: None,
                    success: true,
                    failure_class: None,
                    latency_ms: 1,
                    resource_cost: crate::types::observation::default_cost_profile(),
                    confidence: 1.0,
                    timestamp: Utc::now(),
                })
            }

            fn invoke_rollback(
                &self,
                _skill: &SkillSpec,
                _bindings: &[WorkingBinding],
                _session_id: Uuid,
            ) -> Result<Option<Observation>> {
                Ok(None)
            }
        }

        let skill = test_skill("test:echo");
        let mut ctrl = SessionController::new(SessionControllerDeps {
            belief_source: Box::new(NoopBeliefSource),
            episode_memory: Box::new(NoopEpisodeMemory),
            schema_memory: Box::new(NoopSchemaMemory),
            routine_memory: Box::new(NoopRoutineMemory),
            skill_registry: Box::new(TestSkillRegistry { skills: vec![skill] }),
            skill_executor: Box::new(PassingPreconditionExecutor),
            predictor: Box::new(NoopPredictor),
            critic: Box::new(NoopCritic),
            policy_engine: Box::new(NoopPolicyEngine),
            remote_executor: None,
            capability_scope_checker: None,
        }, test_metrics());

        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        ctrl.run_step(&mut session).unwrap();

        assert_eq!(session.trace.steps.len(), 1);
        let step = &session.trace.steps[0];
        assert_eq!(step.precondition_results.len(), 2);
        assert!(step.precondition_results[0].passed);
        assert_eq!(step.precondition_results[0].description, "network reachable");
        assert!(step.precondition_results[1].passed);
        assert_eq!(step.precondition_results[1].description, "auth token present");
    }

    // -----------------------------------------------------------------------
    // Composite skill execution tests
    // -----------------------------------------------------------------------

    fn test_composite_skill(id: &str, subskills: Vec<SubskillRef>) -> SkillSpec {
        let mut skill = test_skill(id);
        skill.kind = SkillKind::Composite;
        skill.subskills = subskills;
        skill
    }

    fn test_subskill_ref(skill_id: &str) -> SubskillRef {
        SubskillRef {
            skill_id: skill_id.into(),
            ordering: SubskillOrdering::Sequential,
            required: true,
            branch_condition: None,
            stop_condition: None,
        }
    }

    #[test]
    fn test_composite_executes_all_subskills() {
        let sub_a = test_skill("test:sub_a");
        let sub_b = test_skill("test:sub_b");
        let composite = test_composite_skill(
            "test:composite",
            vec![
                test_subskill_ref("test:sub_a"),
                test_subskill_ref("test:sub_b"),
            ],
        );

        let mut ctrl = make_controller_with_skills(vec![composite, sub_a, sub_b]);
        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        let result = ctrl.run_step(&mut session).unwrap();

        assert!(matches!(result, StepResult::Continue));
        assert_eq!(session.trace.steps.len(), 1);
        let step = &session.trace.steps[0];
        assert_eq!(step.selected_skill, "test:composite");
        assert!(session.working_memory.recent_observations.len() >= 2);
    }

    #[test]
    fn test_composite_skips_subskill_when_branch_condition_false() {
        let sub_a = test_skill("test:sub_a");
        let sub_b = test_skill("test:sub_b");

        let mut ref_a = test_subskill_ref("test:sub_a");
        ref_a.branch_condition = Some(serde_json::json!(false));

        let composite = test_composite_skill(
            "test:composite",
            vec![ref_a, test_subskill_ref("test:sub_b")],
        );

        let mut ctrl = make_controller_with_skills(vec![composite, sub_a, sub_b]);
        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        ctrl.run_step(&mut session).unwrap();

        let step = &session.trace.steps[0];
        assert_eq!(step.selected_skill, "test:composite");
    }

    #[test]
    fn test_composite_stops_on_stop_condition() {
        let sub_a = test_skill("test:sub_a");
        let sub_b = test_skill("test:sub_b");

        let mut ref_a = test_subskill_ref("test:sub_a");
        ref_a.stop_condition = Some(serde_json::json!(true));

        let composite = test_composite_skill(
            "test:composite",
            vec![ref_a, test_subskill_ref("test:sub_b")],
        );

        let mut ctrl = make_controller_with_skills(vec![composite, sub_a, sub_b]);
        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        ctrl.run_step(&mut session).unwrap();

        let step = &session.trace.steps[0];
        assert_eq!(step.selected_skill, "test:composite");
    }

    #[test]
    fn test_composite_missing_required_subskill_fails() {
        let composite = test_composite_skill(
            "test:composite",
            vec![test_subskill_ref("test:nonexistent")],
        );

        let mut ctrl = make_controller_with_skills(vec![composite]);
        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        let result = ctrl.run_step(&mut session);
        assert!(result.is_ok());
    }

    #[test]
    fn test_composite_optional_missing_subskill_succeeds() {
        let sub_a = test_skill("test:sub_a");

        let mut ref_missing = test_subskill_ref("test:nonexistent");
        ref_missing.required = false;

        let composite = test_composite_skill(
            "test:composite",
            vec![ref_missing, test_subskill_ref("test:sub_a")],
        );

        let mut ctrl = make_controller_with_skills(vec![composite, sub_a]);
        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        let result = ctrl.run_step(&mut session).unwrap();

        assert!(matches!(result, StepResult::Continue));
        let step = &session.trace.steps[0];
        assert_eq!(step.selected_skill, "test:composite");
    }

    #[test]
    fn test_composite_empty_subskills_produces_empty_observation() {
        let composite = test_composite_skill("test:composite", vec![]);

        let mut ctrl = make_controller_with_skills(vec![composite]);
        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        let result = ctrl.run_step(&mut session).unwrap();

        assert!(matches!(result, StepResult::Continue));
        let step = &session.trace.steps[0];
        assert_eq!(step.selected_skill, "test:composite");
    }

    #[test]
    fn test_evaluate_condition_variants() {
        let ctrl = make_stub_controller();
        let goal = test_goal();
        let mut dummy_ctrl = make_stub_controller();
        let session = dummy_ctrl.create_session(goal).unwrap();

        assert!(ctrl.evaluate_condition(&serde_json::json!(true), &session));
        assert!(!ctrl.evaluate_condition(&serde_json::json!(false), &session));
        assert!(!ctrl.evaluate_condition(&serde_json::json!(null), &session));
        assert!(!ctrl.evaluate_condition(&serde_json::json!(0), &session));
        assert!(ctrl.evaluate_condition(&serde_json::json!(1), &session));
        assert!(ctrl.evaluate_condition(&serde_json::json!(-1), &session));
        assert!(!ctrl.evaluate_condition(&serde_json::json!(""), &session));
        assert!(ctrl.evaluate_condition(&serde_json::json!("ok"), &session));
        assert!(!ctrl.evaluate_condition(&serde_json::json!([]), &session));
        assert!(ctrl.evaluate_condition(&serde_json::json!([1]), &session));
        assert!(!ctrl.evaluate_condition(&serde_json::json!({}), &session));
        assert!(ctrl.evaluate_condition(&serde_json::json!({"key": "val"}), &session));
    }

    // --- Capability scope enforcement tests ---

    #[test]
    fn test_invocation_scope_derived_from_goal_source() {
        use crate::types::goal::*;

        let mut goal = test_goal();
        let mut ctrl = make_stub_controller();

        // User source -> Local
        goal.source.source_type = GoalSourceType::User;
        let session = ctrl.create_session(goal.clone()).unwrap();
        assert_eq!(
            SessionController::invocation_scope(&session),
            CapabilityScope::Local
        );

        // Internal source -> Local
        goal.goal_id = Uuid::new_v4();
        goal.source.source_type = GoalSourceType::Internal;
        let session = ctrl.create_session(goal.clone()).unwrap();
        assert_eq!(
            SessionController::invocation_scope(&session),
            CapabilityScope::Local
        );

        // Scheduler source -> Local
        goal.goal_id = Uuid::new_v4();
        goal.source.source_type = GoalSourceType::Scheduler;
        let session = ctrl.create_session(goal.clone()).unwrap();
        assert_eq!(
            SessionController::invocation_scope(&session),
            CapabilityScope::Local
        );

        // Api source -> Session
        goal.goal_id = Uuid::new_v4();
        goal.source.source_type = GoalSourceType::Api;
        let session = ctrl.create_session(goal.clone()).unwrap();
        assert_eq!(
            SessionController::invocation_scope(&session),
            CapabilityScope::Session
        );

        // Mcp source -> Session
        goal.goal_id = Uuid::new_v4();
        goal.source.source_type = GoalSourceType::Mcp;
        let session = ctrl.create_session(goal.clone()).unwrap();
        assert_eq!(
            SessionController::invocation_scope(&session),
            CapabilityScope::Session
        );

        // Peer source -> Peer
        goal.goal_id = Uuid::new_v4();
        goal.source.source_type = GoalSourceType::Peer;
        let session = ctrl.create_session(goal.clone()).unwrap();
        assert_eq!(
            SessionController::invocation_scope(&session),
            CapabilityScope::Peer
        );
    }

    #[test]
    fn test_scope_enforcement_allows_broad_capability_from_narrow_context() {
        // A Public-scoped capability should be callable from a Local (User) context.
        let skill = test_skill("test:echo");
        let scope_checker: CapabilityScopeChecker = Box::new(|fqn: &str| {
            if fqn == "test-pack.test:echo" {
                Some(CapabilityScope::Public)
            } else {
                None
            }
        });
        let mut ctrl = SessionController::new(SessionControllerDeps {
            belief_source: Box::new(NoopBeliefSource),
            episode_memory: Box::new(NoopEpisodeMemory),
            schema_memory: Box::new(NoopSchemaMemory),
            routine_memory: Box::new(NoopRoutineMemory),
            skill_registry: Box::new(TestSkillRegistry {
                skills: vec![skill],
            }),
            skill_executor: Box::new(NoopSkillExecutor),
            predictor: Box::new(NoopPredictor),
            critic: Box::new(NoopCritic),
            policy_engine: Box::new(NoopPolicyEngine),
            remote_executor: None,
            capability_scope_checker: Some(scope_checker),
        }, test_metrics());

        let goal = test_goal(); // User source -> Local scope
        let mut session = ctrl.create_session(goal).unwrap();
        let result = ctrl.run_step(&mut session).unwrap();
        assert!(matches!(result, StepResult::Continue));
    }

    #[test]
    fn test_scope_enforcement_denies_narrow_capability_from_broad_context() {
        // A Local-scoped capability should be denied from a Peer context.
        let skill = test_skill("test:echo");
        let scope_checker: CapabilityScopeChecker = Box::new(|fqn: &str| {
            if fqn == "test-pack.test:echo" {
                Some(CapabilityScope::Local)
            } else {
                None
            }
        });
        let mut ctrl = SessionController::new(SessionControllerDeps {
            belief_source: Box::new(NoopBeliefSource),
            episode_memory: Box::new(NoopEpisodeMemory),
            schema_memory: Box::new(NoopSchemaMemory),
            routine_memory: Box::new(NoopRoutineMemory),
            skill_registry: Box::new(TestSkillRegistry {
                skills: vec![skill],
            }),
            skill_executor: Box::new(NoopSkillExecutor),
            predictor: Box::new(NoopPredictor),
            critic: Box::new(NoopCritic),
            policy_engine: Box::new(NoopPolicyEngine),
            remote_executor: None,
            capability_scope_checker: Some(scope_checker),
        }, test_metrics());

        // Peer source -> Peer scope, which is broader than the skill's Local scope.
        let mut goal = test_goal();
        goal.source.source_type = GoalSourceType::Peer;
        let mut session = ctrl.create_session(goal).unwrap();
        let result = ctrl.run_step(&mut session);

        // Should fail with PolicyDenied because the scope check fires.
        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(err_msg.contains("scope"), "error should mention scope: {}", err_msg);
        assert_eq!(session.status, SessionStatus::BlockedByPolicy);
    }

    #[test]
    fn test_scope_enforcement_allows_same_scope() {
        // A Session-scoped capability should be callable from a Session (Api) context.
        let skill = test_skill("test:echo");
        let scope_checker: CapabilityScopeChecker = Box::new(|fqn: &str| {
            if fqn == "test-pack.test:echo" {
                Some(CapabilityScope::Session)
            } else {
                None
            }
        });
        let mut ctrl = SessionController::new(SessionControllerDeps {
            belief_source: Box::new(NoopBeliefSource),
            episode_memory: Box::new(NoopEpisodeMemory),
            schema_memory: Box::new(NoopSchemaMemory),
            routine_memory: Box::new(NoopRoutineMemory),
            skill_registry: Box::new(TestSkillRegistry {
                skills: vec![skill],
            }),
            skill_executor: Box::new(NoopSkillExecutor),
            predictor: Box::new(NoopPredictor),
            critic: Box::new(NoopCritic),
            policy_engine: Box::new(NoopPolicyEngine),
            remote_executor: None,
            capability_scope_checker: Some(scope_checker),
        }, test_metrics());

        let mut goal = test_goal();
        goal.source.source_type = GoalSourceType::Api; // Api -> Session scope
        let mut session = ctrl.create_session(goal).unwrap();
        let result = ctrl.run_step(&mut session).unwrap();
        assert!(matches!(result, StepResult::Continue));
    }

    #[test]
    fn test_scope_enforcement_skipped_when_no_checker() {
        // Without a scope checker, no scope enforcement happens.
        let skill = test_skill("test:echo");
        let mut ctrl = SessionController::new(SessionControllerDeps {
            belief_source: Box::new(NoopBeliefSource),
            episode_memory: Box::new(NoopEpisodeMemory),
            schema_memory: Box::new(NoopSchemaMemory),
            routine_memory: Box::new(NoopRoutineMemory),
            skill_registry: Box::new(TestSkillRegistry {
                skills: vec![skill],
            }),
            skill_executor: Box::new(NoopSkillExecutor),
            predictor: Box::new(NoopPredictor),
            critic: Box::new(NoopCritic),
            policy_engine: Box::new(NoopPolicyEngine),
            remote_executor: None,
            capability_scope_checker: None,
        }, test_metrics());

        // Even a Peer-sourced goal executes fine without a scope checker.
        let mut goal = test_goal();
        goal.source.source_type = GoalSourceType::Peer;
        let mut session = ctrl.create_session(goal).unwrap();
        let result = ctrl.run_step(&mut session).unwrap();
        assert!(matches!(result, StepResult::Continue));
    }

    #[test]
    fn test_scope_enforcement_unknown_fqn_allowed() {
        // When the checker returns None for an unregistered FQN, allow execution.
        let skill = test_skill("test:echo");
        let scope_checker: CapabilityScopeChecker = Box::new(|_fqn: &str| {
            None // Unknown capability
        });
        let mut ctrl = SessionController::new(SessionControllerDeps {
            belief_source: Box::new(NoopBeliefSource),
            episode_memory: Box::new(NoopEpisodeMemory),
            schema_memory: Box::new(NoopSchemaMemory),
            routine_memory: Box::new(NoopRoutineMemory),
            skill_registry: Box::new(TestSkillRegistry {
                skills: vec![skill],
            }),
            skill_executor: Box::new(NoopSkillExecutor),
            predictor: Box::new(NoopPredictor),
            critic: Box::new(NoopCritic),
            policy_engine: Box::new(NoopPolicyEngine),
            remote_executor: None,
            capability_scope_checker: Some(scope_checker),
        }, test_metrics());

        let mut goal = test_goal();
        goal.source.source_type = GoalSourceType::Peer;
        let mut session = ctrl.create_session(goal).unwrap();
        let result = ctrl.run_step(&mut session).unwrap();
        assert!(matches!(result, StepResult::Continue));
    }

    // -----------------------------------------------------------------------
    // evaluate_observables tests
    // -----------------------------------------------------------------------

    fn test_observation(structured_result: serde_json::Value) -> Observation {
        Observation {
            observation_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            skill_id: Some("test:skill".into()),
            port_calls: Vec::new(),
            raw_result: serde_json::json!({}),
            structured_result,
            effect_patch: None,
            success: true,
            failure_class: None,
            latency_ms: 1,
            resource_cost: crate::types::observation::default_cost_profile(),
            confidence: 0.9,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn test_evaluate_observables_empty_observables() {
        let skill = test_skill("test:empty");
        let obs = test_observation(serde_json::json!({"result": "ok"}));
        let eval = SessionController::evaluate_observables(&skill, &obs);
        assert!(!eval.success_confirmed);
        assert!(!eval.partial_detected);
        assert!(!eval.ambiguity_detected);
        assert!(eval.confidence_update.is_none());
    }

    #[test]
    fn test_evaluate_observables_confirm_success_present() {
        let mut skill = test_skill("test:obs");
        skill.observables = vec![ObservableDecl {
            field: "status".into(),
            role: ObservableRole::ConfirmSuccess,
        }];
        let obs = test_observation(serde_json::json!({"status": "done"}));
        let eval = SessionController::evaluate_observables(&skill, &obs);
        assert!(eval.success_confirmed);
    }

    #[test]
    fn test_evaluate_observables_confirm_success_null_field() {
        let mut skill = test_skill("test:obs");
        skill.observables = vec![ObservableDecl {
            field: "status".into(),
            role: ObservableRole::ConfirmSuccess,
        }];
        let obs = test_observation(serde_json::json!({"status": null}));
        let eval = SessionController::evaluate_observables(&skill, &obs);
        assert!(!eval.success_confirmed);
    }

    #[test]
    fn test_evaluate_observables_confirm_success_missing_field() {
        let mut skill = test_skill("test:obs");
        skill.observables = vec![ObservableDecl {
            field: "status".into(),
            role: ObservableRole::ConfirmSuccess,
        }];
        let obs = test_observation(serde_json::json!({"other": "value"}));
        let eval = SessionController::evaluate_observables(&skill, &obs);
        assert!(!eval.success_confirmed);
    }

    #[test]
    fn test_evaluate_observables_detect_partial_string() {
        let mut skill = test_skill("test:obs");
        skill.observables = vec![ObservableDecl {
            field: "completion".into(),
            role: ObservableRole::DetectPartialSuccess,
        }];
        let obs = test_observation(serde_json::json!({"completion": "partial result"}));
        let eval = SessionController::evaluate_observables(&skill, &obs);
        assert!(eval.partial_detected);
    }

    #[test]
    fn test_evaluate_observables_detect_partial_incomplete() {
        let mut skill = test_skill("test:obs");
        skill.observables = vec![ObservableDecl {
            field: "completion".into(),
            role: ObservableRole::DetectPartialSuccess,
        }];
        let obs = test_observation(serde_json::json!({"completion": "incomplete transfer"}));
        let eval = SessionController::evaluate_observables(&skill, &obs);
        assert!(eval.partial_detected);
    }

    #[test]
    fn test_evaluate_observables_detect_partial_bool_false() {
        let mut skill = test_skill("test:obs");
        skill.observables = vec![ObservableDecl {
            field: "complete".into(),
            role: ObservableRole::DetectPartialSuccess,
        }];
        let obs = test_observation(serde_json::json!({"complete": false}));
        let eval = SessionController::evaluate_observables(&skill, &obs);
        assert!(eval.partial_detected);
    }

    #[test]
    fn test_evaluate_observables_detect_partial_fraction() {
        let mut skill = test_skill("test:obs");
        skill.observables = vec![ObservableDecl {
            field: "progress".into(),
            role: ObservableRole::DetectPartialSuccess,
        }];
        let obs = test_observation(serde_json::json!({"progress": 0.5}));
        let eval = SessionController::evaluate_observables(&skill, &obs);
        assert!(eval.partial_detected);
    }

    #[test]
    fn test_evaluate_observables_detect_partial_full_is_not_partial() {
        let mut skill = test_skill("test:obs");
        skill.observables = vec![ObservableDecl {
            field: "progress".into(),
            role: ObservableRole::DetectPartialSuccess,
        }];
        let obs = test_observation(serde_json::json!({"progress": 1.0}));
        let eval = SessionController::evaluate_observables(&skill, &obs);
        assert!(!eval.partial_detected);
    }

    #[test]
    fn test_evaluate_observables_detect_ambiguity_null() {
        let mut skill = test_skill("test:obs");
        skill.observables = vec![ObservableDecl {
            field: "intent".into(),
            role: ObservableRole::DetectAmbiguity,
        }];
        let obs = test_observation(serde_json::json!({"intent": null}));
        let eval = SessionController::evaluate_observables(&skill, &obs);
        assert!(eval.ambiguity_detected);
    }

    #[test]
    fn test_evaluate_observables_detect_ambiguity_string() {
        let mut skill = test_skill("test:obs");
        skill.observables = vec![ObservableDecl {
            field: "intent".into(),
            role: ObservableRole::DetectAmbiguity,
        }];
        let obs = test_observation(serde_json::json!({"intent": "result is ambiguous"}));
        let eval = SessionController::evaluate_observables(&skill, &obs);
        assert!(eval.ambiguity_detected);
    }

    #[test]
    fn test_evaluate_observables_detect_ambiguity_uncertain() {
        let mut skill = test_skill("test:obs");
        skill.observables = vec![ObservableDecl {
            field: "intent".into(),
            role: ObservableRole::DetectAmbiguity,
        }];
        let obs = test_observation(serde_json::json!({"intent": "uncertain outcome"}));
        let eval = SessionController::evaluate_observables(&skill, &obs);
        assert!(eval.ambiguity_detected);
    }

    #[test]
    fn test_evaluate_observables_detect_ambiguity_missing_field() {
        let mut skill = test_skill("test:obs");
        skill.observables = vec![ObservableDecl {
            field: "intent".into(),
            role: ObservableRole::DetectAmbiguity,
        }];
        let obs = test_observation(serde_json::json!({"other": "value"}));
        let eval = SessionController::evaluate_observables(&skill, &obs);
        assert!(eval.ambiguity_detected);
    }

    #[test]
    fn test_evaluate_observables_detect_ambiguity_clear_value() {
        let mut skill = test_skill("test:obs");
        skill.observables = vec![ObservableDecl {
            field: "intent".into(),
            role: ObservableRole::DetectAmbiguity,
        }];
        let obs = test_observation(serde_json::json!({"intent": "clearly resolved"}));
        let eval = SessionController::evaluate_observables(&skill, &obs);
        assert!(!eval.ambiguity_detected);
    }

    #[test]
    fn test_evaluate_observables_update_confidence() {
        let mut skill = test_skill("test:obs");
        skill.observables = vec![ObservableDecl {
            field: "conf".into(),
            role: ObservableRole::UpdateConfidence,
        }];
        let obs = test_observation(serde_json::json!({"conf": 0.85}));
        let eval = SessionController::evaluate_observables(&skill, &obs);
        assert_eq!(eval.confidence_update, Some(0.85));
    }

    #[test]
    fn test_evaluate_observables_update_confidence_non_numeric() {
        let mut skill = test_skill("test:obs");
        skill.observables = vec![ObservableDecl {
            field: "conf".into(),
            role: ObservableRole::UpdateConfidence,
        }];
        let obs = test_observation(serde_json::json!({"conf": "high"}));
        let eval = SessionController::evaluate_observables(&skill, &obs);
        assert!(eval.confidence_update.is_none());
    }

    #[test]
    fn test_evaluate_observables_general_is_noop() {
        let mut skill = test_skill("test:obs");
        skill.observables = vec![ObservableDecl {
            field: "log".into(),
            role: ObservableRole::General,
        }];
        let obs = test_observation(serde_json::json!({"log": "something happened"}));
        let eval = SessionController::evaluate_observables(&skill, &obs);
        assert!(!eval.success_confirmed);
        assert!(!eval.partial_detected);
        assert!(!eval.ambiguity_detected);
        assert!(eval.confidence_update.is_none());
    }

    #[test]
    fn test_evaluate_observables_multiple_roles() {
        let mut skill = test_skill("test:obs");
        skill.observables = vec![
            ObservableDecl {
                field: "status".into(),
                role: ObservableRole::ConfirmSuccess,
            },
            ObservableDecl {
                field: "confidence".into(),
                role: ObservableRole::UpdateConfidence,
            },
            ObservableDecl {
                field: "intent".into(),
                role: ObservableRole::DetectAmbiguity,
            },
        ];
        let obs = test_observation(serde_json::json!({
            "status": "ok",
            "confidence": 0.92,
            "intent": "clearly file transfer"
        }));
        let eval = SessionController::evaluate_observables(&skill, &obs);
        assert!(eval.success_confirmed);
        assert!(!eval.ambiguity_detected);
        assert_eq!(eval.confidence_update, Some(0.92));
    }

    #[test]
    fn test_checkpoint_session_by_id() {
        let skill = test_skill("test:echo");
        let mut ctrl = make_controller_with_skills(vec![skill]);
        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        ctrl.run_step(&mut session).unwrap();

        let data = ctrl.checkpoint_session(&session.session_id).unwrap();
        assert!(!data.is_empty());

        // Deserialized data should have the same session ID.
        let deserialized: ControlSession = serde_json::from_slice(&data).unwrap();
        assert_eq!(deserialized.session_id, session.session_id);
        assert_eq!(deserialized.trace.steps.len(), 1);
    }

    #[test]
    fn test_checkpoint_session_not_found() {
        let ctrl = make_stub_controller();
        let result = ctrl.checkpoint_session(&Uuid::new_v4());
        assert!(result.is_err());
    }

    #[test]
    fn test_restore_session_returns_id() {
        let skill = test_skill("test:echo");
        let mut ctrl = make_controller_with_skills(vec![skill]);
        let goal = test_goal();
        let mut session = ctrl.create_session(goal).unwrap();
        ctrl.run_step(&mut session).unwrap();

        let data = ctrl.checkpoint_session(&session.session_id).unwrap();

        let mut ctrl2 = make_stub_controller();
        let restored_id = ctrl2.restore_session(&data).unwrap();
        assert_eq!(restored_id, session.session_id);

        // The session should be accessible via the controller.
        let restored = ctrl2.get_session_by_id(&restored_id).unwrap();
        assert_eq!(restored.status, SessionStatus::Running);
        assert_eq!(restored.trace.steps.len(), 1);
    }

    #[test]
    fn test_restore_session_invalid_data() {
        let mut ctrl = make_stub_controller();
        let result = ctrl.restore_session(b"this is not json");
        assert!(result.is_err());
    }

}
