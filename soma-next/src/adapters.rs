//! Adapter structs implementing SessionControllerDeps sub-traits.
//!
//! These adapters bridge the standalone runtime components (PortRuntime,
//! SkillRuntime, BeliefRuntime, GoalRuntime) into the session controller's
//! dependency trait system.

use std::sync::{Arc, Mutex};

use chrono::Utc;
use uuid::Uuid;

use crate::errors::{Result, SomaError};
use crate::memory::episodes::EpisodeStore;
use crate::memory::routines::RoutineStore;
use crate::memory::schemas::SchemaStore;
use crate::runtime::belief::{BeliefRuntime, DefaultBeliefRuntime};
use crate::runtime::policy::{DefaultPolicyRuntime, PolicyContext, PolicyRuntime};
use crate::runtime::port::{DefaultPortRuntime, PortRuntime};
use crate::runtime::session::{
    BeliefSource, CandidatePredictor, Critic, EpisodeMemory, PolicyEngine, RoutineMemory,
    SchemaMemory, SkillExecutor, SkillRegistry,
};
use crate::runtime::skill::{DefaultSkillRuntime, SkillRuntime};
use crate::types::belief::{BeliefPatch, BeliefState};
use crate::types::common::{
    Budget, CriticDecision, EffectType, RollbackSupport, SideEffectClass, TrustLevel,
};
use crate::types::episode::Episode;
use crate::types::goal::GoalSpec;
use crate::types::observation::{Observation, PortCallRecord, default_cost_profile};
use crate::types::policy::PolicyTargetType;
use crate::types::port::InvocationContext;
use crate::types::routine::Routine;
use crate::types::schema::Schema;
use crate::types::session::{
    BindingSource, CandidateScore, ControlSession, PolicyHook,
    PreconditionResult, WorkingBinding, WorkingMemory,
};
use crate::types::skill::SkillSpec;

use crate::runtime::session::PolicyCheckResult;

// ---------------------------------------------------------------------------
// SimpleBeliefSource
// ---------------------------------------------------------------------------

/// Creates empty belief states and delegates patching to DefaultBeliefRuntime.
pub struct SimpleBeliefSource {
    belief_runtime: DefaultBeliefRuntime,
}

impl Default for SimpleBeliefSource {
    fn default() -> Self {
        Self::new()
    }
}

impl SimpleBeliefSource {
    pub fn new() -> Self {
        Self {
            belief_runtime: DefaultBeliefRuntime::new(),
        }
    }
}

impl BeliefSource for SimpleBeliefSource {
    fn build_initial_belief(&self, _goal: &GoalSpec) -> Result<BeliefState> {
        let session_id = Uuid::new_v4();
        self.belief_runtime.create_belief(session_id)
    }

    fn apply_patch(&self, belief: &mut BeliefState, patch: &BeliefPatch) -> Result<()> {
        self.belief_runtime.apply_patch(belief, patch.clone())
    }
}

// ---------------------------------------------------------------------------
// EmptyEpisodeMemory
// ---------------------------------------------------------------------------

pub struct EmptyEpisodeMemory;

impl EpisodeMemory for EmptyEpisodeMemory {
    fn retrieve_nearest(
        &self,
        _goal: &GoalSpec,
        _belief: &BeliefState,
        _limit: usize,
    ) -> Vec<Episode> {
        vec![]
    }
}

// ---------------------------------------------------------------------------
// EpisodeMemoryAdapter
// ---------------------------------------------------------------------------

/// Wraps a shared DefaultEpisodeStore so the session controller can retrieve
/// nearest episodes during the control loop, and the CLI can store episodes
/// after session completion.
pub struct EpisodeMemoryAdapter {
    store: Arc<Mutex<dyn EpisodeStore + Send>>,
    embedder: Arc<dyn crate::memory::embedder::GoalEmbedder + Send + Sync>,
}

impl EpisodeMemoryAdapter {
    pub fn new(
        store: Arc<Mutex<dyn EpisodeStore + Send>>,
        embedder: Arc<dyn crate::memory::embedder::GoalEmbedder + Send + Sync>,
    ) -> Self {
        Self { store, embedder }
    }

    /// Store a completed episode. Called by the CLI after a session finishes.
    /// If the store is at capacity, the evicted episode is logged at debug level.
    /// Computes the embedding before storing if the episode lacks one.
    pub fn store(&self, mut episode: Episode) -> Result<()> {
        if episode.embedding.is_none() {
            episode.embedding = Some(self.embedder.embed(&episode.goal_fingerprint));
        }
        let mut s = self.store.lock().map_err(|e| {
            SomaError::Memory(format!("episode store lock poisoned: {}", e))
        })?;
        if let Some(evicted) = s.store(episode)? {
            tracing::debug!(
                episode_id = %evicted.episode_id,
                goal = %evicted.goal_fingerprint,
                "evicted episode from ring buffer"
            );
        }
        Ok(())
    }
}

impl EpisodeMemory for EpisodeMemoryAdapter {
    fn retrieve_nearest(
        &self,
        goal: &GoalSpec,
        _belief: &BeliefState,
        limit: usize,
    ) -> Vec<Episode> {
        let s = match self.store.lock() {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        // Try embedding-based retrieval first.
        let embedding = self.embedder.embed(&goal.objective.description);
        let by_embedding = s.retrieve_by_embedding(&embedding, 0.7, limit);
        if !by_embedding.is_empty() {
            return by_embedding.into_iter().cloned().collect();
        }

        // Fallback to prefix matching.
        s.retrieve_nearest(&goal.objective.description, limit)
            .into_iter()
            .cloned()
            .collect()
    }
}

// ---------------------------------------------------------------------------
// EmptySchemaMemory
// ---------------------------------------------------------------------------

pub struct EmptySchemaMemory;

impl SchemaMemory for EmptySchemaMemory {
    fn retrieve_matching(&self, _goal: &GoalSpec, _belief: &BeliefState) -> Vec<Schema> {
        vec![]
    }
}

// ---------------------------------------------------------------------------
// SchemaMemoryAdapter
// ---------------------------------------------------------------------------

/// Wraps a shared DefaultSchemaStore so the session controller can retrieve
/// matching schemas during the control loop. The trigger context is built
/// from the goal's fingerprint (objective description).
pub struct SchemaMemoryAdapter {
    store: Arc<Mutex<dyn SchemaStore + Send>>,
}

impl SchemaMemoryAdapter {
    pub fn new(store: Arc<Mutex<dyn SchemaStore + Send>>) -> Self {
        Self { store }
    }
}

impl SchemaMemory for SchemaMemoryAdapter {
    fn retrieve_matching(&self, goal: &GoalSpec, _belief: &BeliefState) -> Vec<Schema> {
        let s = match self.store.lock() {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let trigger_context = serde_json::json!({
            "goal_fingerprint": goal.objective.description,
        });
        s.find_matching(&trigger_context)
            .into_iter()
            .cloned()
            .collect()
    }
}

// ---------------------------------------------------------------------------
// EmptyRoutineMemory
// ---------------------------------------------------------------------------

pub struct EmptyRoutineMemory;

impl RoutineMemory for EmptyRoutineMemory {
    fn retrieve_matching(&self, _goal: &GoalSpec, _belief: &BeliefState) -> Vec<Routine> {
        vec![]
    }
}

// ---------------------------------------------------------------------------
// RoutineMemoryAdapter
// ---------------------------------------------------------------------------

/// Wraps a shared DefaultRoutineStore so the session controller can retrieve
/// matching routines during the control loop. The match context is built
/// from the goal's fingerprint (objective description).
pub struct RoutineMemoryAdapter {
    store: Arc<Mutex<dyn RoutineStore + Send>>,
}

impl RoutineMemoryAdapter {
    pub fn new(store: Arc<Mutex<dyn RoutineStore + Send>>) -> Self {
        Self { store }
    }
}

impl RoutineMemory for RoutineMemoryAdapter {
    fn retrieve_matching(&self, goal: &GoalSpec, _belief: &BeliefState) -> Vec<Routine> {
        let s = match self.store.lock() {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let match_context = serde_json::json!({
            "goal_fingerprint": goal.objective.description,
        });
        s.find_matching(&match_context)
            .into_iter()
            .cloned()
            .collect()
    }
}

// ---------------------------------------------------------------------------
// SkillRegistryAdapter
// ---------------------------------------------------------------------------

/// Wraps DefaultSkillRuntime behind Arc<Mutex<>> to satisfy the SkillRegistry
/// trait that returns references. Since SkillRegistry::get_skill returns
/// `Option<&SkillSpec>`, and the session controller clones the result, we
/// store a snapshot of the skills at construction time.
pub struct SkillRegistryAdapter {
    skills: Vec<SkillSpec>,
}

impl SkillRegistryAdapter {
    pub fn new(skill_runtime: &DefaultSkillRuntime) -> Self {
        let skills: Vec<SkillSpec> = skill_runtime
            .list_skills(None)
            .into_iter()
            .cloned()
            .collect();
        Self { skills }
    }
}

impl SkillRegistry for SkillRegistryAdapter {
    fn enumerate_candidates(
        &self,
        _goal: &GoalSpec,
        _belief: &BeliefState,
        schemas: &[Schema],
        routines: &[Routine],
    ) -> Vec<SkillSpec> {
        // If a routine matches and has a compiled skill path, narrow
        // candidates to just those skills (plan-following mode).
        if let Some(routine) = routines.iter().find(|r| !r.compiled_skill_path.is_empty()) {
            let narrowed: Vec<SkillSpec> = routine
                .compiled_skill_path
                .iter()
                .filter_map(|sid| self.skills.iter().find(|s| s.skill_id == *sid))
                .cloned()
                .collect();
            if !narrowed.is_empty() {
                return narrowed;
            }
            // If none of the routine's skill IDs could be resolved, fall through.
        }

        // If a schema matches but no routine, narrow to the schema's
        // candidate_skill_ordering.
        if let Some(schema) = schemas.iter().find(|s| !s.candidate_skill_ordering.is_empty()) {
            let narrowed: Vec<SkillSpec> = schema
                .candidate_skill_ordering
                .iter()
                .filter_map(|sid| self.skills.iter().find(|s| s.skill_id == *sid))
                .cloned()
                .collect();
            if !narrowed.is_empty() {
                return narrowed;
            }
        }

        // Fallback: return all skills for full deliberation.
        self.skills.clone()
    }

    fn get_skill(&self, skill_id: &str) -> Option<&SkillSpec> {
        self.skills.iter().find(|s| s.skill_id == skill_id)
    }
}

// ---------------------------------------------------------------------------
// PortBackedSkillExecutor
// ---------------------------------------------------------------------------

/// The key adapter: maps skill execution to port invocations.
///
/// For each skill, parses capability_requirements to find the port_id and
/// capability_id, then calls port_runtime.invoke(). Converts the resulting
/// PortCallRecord into an Observation.
pub struct PortBackedSkillExecutor {
    port_runtime: Arc<Mutex<DefaultPortRuntime>>,
}

impl PortBackedSkillExecutor {
    pub fn new(port_runtime: Arc<Mutex<DefaultPortRuntime>>) -> Self {
        Self { port_runtime }
    }

    /// Parse a skill's capability_requirements to extract (port_id, capability_id).
    /// Expected format: "port:<port_id>/<capability_id>" or just "<port_id>/<capability_id>".
    fn resolve_port_capability(skill: &SkillSpec) -> Result<(String, String)> {
        for req in &skill.capability_requirements {
            let cleaned = req.strip_prefix("port:").unwrap_or(req);
            if let Some((port_id, cap_id)) = cleaned.split_once('/') {
                return Ok((port_id.to_string(), cap_id.to_string()));
            }
        }
        Err(SomaError::Skill(format!(
            "skill '{}' has no port capability requirement in format 'port_id/capability_id'",
            skill.skill_id
        )))
    }

    /// Extract input parameters from goal context and bindings.
    fn build_port_input(
        _skill: &SkillSpec,
        bindings: &[WorkingBinding],
    ) -> serde_json::Value {
        let mut input = serde_json::Map::new();
        for binding in bindings {
            input.insert(binding.name.clone(), binding.value.clone());
        }
        serde_json::Value::Object(input)
    }
}

impl SkillExecutor for PortBackedSkillExecutor {
    fn bind_inputs(
        &self,
        skill: &SkillSpec,
        belief: &BeliefState,
        working_memory: &WorkingMemory,
    ) -> Result<Vec<WorkingBinding>> {
        // Extract required inputs from the skill's input schema and try to
        // find matching values in the belief state's active_bindings, the
        // working memory active_bindings, or the goal objective.
        let schema_obj = match skill.inputs.schema.as_object() {
            Some(obj) => obj,
            None => return Ok(Vec::new()),
        };

        let properties = match schema_obj.get("properties").and_then(|p| p.as_object()) {
            Some(props) => props,
            None => return Ok(Vec::new()),
        };

        let required_fields: Vec<&str> = schema_obj
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        let mut bindings = Vec::new();

        for (key, _property_schema) in properties {
            // Priority: working_memory bindings -> belief bindings -> default
            let from_wm = working_memory
                .active_bindings
                .iter()
                .find(|b| b.name == *key);

            if let Some(wb) = from_wm {
                bindings.push(wb.clone());
                continue;
            }

            let from_belief = belief
                .active_bindings
                .iter()
                .find(|b| b.name == *key);

            if let Some(bb) = from_belief {
                bindings.push(WorkingBinding {
                    name: bb.name.clone(),
                    value: bb.value.clone(),
                    source: BindingSource::BeliefResource,
                });
                continue;
            }

            // Not found — if required, fail
            if required_fields.contains(&key.as_str()) {
                return Err(SomaError::Skill(format!(
                    "required input '{}' for skill '{}' not found in belief or working memory",
                    key, skill.skill_id
                )));
            }
        }

        Ok(bindings)
    }

    fn check_preconditions(
        &self,
        _skill: &SkillSpec,
        _belief: &BeliefState,
        _working_memory: &WorkingMemory,
    ) -> Result<Vec<PreconditionResult>> {
        // All preconditions pass for the initial wiring.
        Ok(Vec::new())
    }

    fn execute(
        &self,
        skill: &SkillSpec,
        bindings: &[WorkingBinding],
        session_id: Uuid,
    ) -> Result<Observation> {
        let (port_id, capability_id) = Self::resolve_port_capability(skill)?;
        let port_input = Self::build_port_input(skill, bindings);

        let ctx = InvocationContext::for_session(session_id, None, None);

        let record = {
            let rt = self.port_runtime.lock().map_err(|e| {
                SomaError::Port(format!("port runtime lock poisoned: {}", e))
            })?;
            rt.invoke(&port_id, &capability_id, port_input, &ctx)?
        };

        let observation = port_call_to_observation(record, session_id, &skill.skill_id);
        Ok(observation)
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

/// Convert a PortCallRecord into an Observation.
fn port_call_to_observation(
    record: PortCallRecord,
    session_id: Uuid,
    skill_id: &str,
) -> Observation {
    Observation {
        observation_id: Uuid::new_v4(),
        session_id,
        skill_id: Some(skill_id.to_string()),
        port_calls: vec![record.clone()],
        raw_result: record.raw_result.clone(),
        structured_result: record.structured_result.clone(),
        effect_patch: record.effect_patch.clone(),
        success: record.success,
        failure_class: if record.success {
            None
        } else {
            Some(crate::types::common::SkillFailureClass::PortFailure)
        },
        latency_ms: record.latency_ms,
        resource_cost: default_cost_profile(),
        confidence: record.confidence,
        timestamp: Utc::now(),
    }
}

// ---------------------------------------------------------------------------
// SimpleCandidatePredictor
// ---------------------------------------------------------------------------

/// Scores candidates using keyword matching against the goal description.
/// Skills whose name or description shares words with the goal score higher.
/// Penalizes skills that have failed recently within the session.
pub struct SimpleCandidatePredictor {
    failure_counts: std::sync::Mutex<std::collections::HashMap<String, u32>>,
}

impl SimpleCandidatePredictor {
    pub fn new() -> Self {
        Self {
            failure_counts: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Record a skill invocation result for score adjustment.
    pub fn record_outcome(&self, skill_id: &str, success: bool) {
        if let Ok(mut counts) = self.failure_counts.lock() {
            if success {
                counts.remove(skill_id);
            } else {
                *counts.entry(skill_id.to_string()).or_insert(0) += 1;
            }
        }
    }

    /// Compute a relevance score for a skill against a goal description.
    fn relevance_score(skill: &SkillSpec, goal_text: &str) -> f64 {
        let goal_lower = goal_text.to_lowercase();
        let goal_words: Vec<&str> = goal_lower.split_whitespace().collect();

        let mut score = 0.0;

        let skill_text = format!(
            "{} {} {} {}",
            skill.name.to_lowercase(),
            skill.description.to_lowercase(),
            skill.skill_id.to_lowercase(),
            skill.tags.join(" ").to_lowercase(),
        );

        for word in &goal_words {
            if word.len() < 3 {
                continue;
            }
            if skill_text.contains(word) {
                score += 1.0;
            }
        }

        // Keyword-to-capability mapping for filesystem and database operations.
        let keyword_map: &[(&[&str], &str)] = &[
            // Filesystem
            (&["list", "ls", "dir", "files", "directory", "readdir", "entries"], "readdir"),
            (&["read", "cat", "show", "view", "content", "contents", "readfile"], "readfile"),
            (&["write", "save", "writefile"], "writefile"),
            (&["stat", "info", "metadata", "size", "permissions"], "stat"),
            (&["mkdir", "make directory"], "mkdir"),
            (&["rmdir", "remove directory"], "rmdir"),
            (&["rm", "remove file", "unlink"], "rm"),
            // Database
            (&["count", "how many", "total number"], "count"),
            (&["query", "select", "sql", "fetch rows"], "query"),
            (&["find", "look up", "search", "find_many"], "find"),
            (&["insert", "add", "create record"], "insert"),
            (&["update", "modify", "change", "set"], "update"),
            (&["delete", "remove", "drop row"], "delete"),
            (&["aggregate", "sum", "average", "avg", "min", "max", "group"], "aggregate"),
        ];

        for (keywords, cap_id) in keyword_map {
            for kw in *keywords {
                if goal_lower.contains(kw) {
                    for req in &skill.capability_requirements {
                        if req.contains(cap_id) {
                            score += 10.0;
                        }
                    }
                }
            }
        }

        score
    }
}

impl Default for SimpleCandidatePredictor {
    fn default() -> Self {
        Self::new()
    }
}

impl CandidatePredictor for SimpleCandidatePredictor {
    fn score(
        &self,
        candidates: &[SkillSpec],
        goal: &GoalSpec,
        _belief: &BeliefState,
        _episodes: &[Episode],
    ) -> Vec<CandidateScore> {
        let goal_text = &goal.objective.description;
        let failure_counts = self.failure_counts.lock().ok();

        candidates
            .iter()
            .map(|skill| {
                let relevance = Self::relevance_score(skill, goal_text);

                // Penalize skills that have failed — score decays with each failure.
                let failures = failure_counts
                    .as_ref()
                    .and_then(|m| m.get(&skill.skill_id))
                    .copied()
                    .unwrap_or(0);
                let penalty = 1.0 / (1.0 + failures as f64);

                CandidateScore {
                    skill_id: skill.skill_id.clone(),
                    score: relevance * penalty,
                    predicted_success: 0.9 * penalty,
                    predicted_cost: 0.01,
                    predicted_latency_ms: 10,
                    information_gain: 0.5,
                }
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

// ---------------------------------------------------------------------------
// SimpleSessionCritic
// ---------------------------------------------------------------------------

/// Returns Stop on success, Abort after repeated identical failures, Continue otherwise.
pub struct SimpleSessionCritic {
    recent_errors: std::sync::Mutex<Vec<String>>,
}

impl SimpleSessionCritic {
    pub fn new() -> Self {
        Self {
            recent_errors: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Max consecutive identical errors before aborting.
    const DEAD_END_THRESHOLD: usize = 3;
}

impl Default for SimpleSessionCritic {
    fn default() -> Self {
        Self::new()
    }
}

impl Critic for SimpleSessionCritic {
    fn evaluate(
        &self,
        _goal: &GoalSpec,
        _belief: &BeliefState,
        observation: &Observation,
        _budget: &Budget,
        _step_index: u32,
    ) -> CriticDecision {
        if observation.success {
            // Clear error history on success.
            if let Ok(mut errors) = self.recent_errors.lock() {
                errors.clear();
            }
            return CriticDecision::Stop;
        }

        // Track error messages to detect dead ends.
        let error_msg = observation
            .raw_result
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("")
            .to_string();

        if let Ok(mut errors) = self.recent_errors.lock() {
            errors.push(error_msg);

            // Check if the last N errors are all identical.
            if errors.len() >= Self::DEAD_END_THRESHOLD {
                let recent = &errors[errors.len() - Self::DEAD_END_THRESHOLD..];
                if recent.windows(2).all(|w| w[0] == w[1]) && !recent[0].is_empty() {
                    return CriticDecision::Stop;
                }
            }
        }

        CriticDecision::Continue
    }
}

// ---------------------------------------------------------------------------
// PolicyEngineAdapter
// ---------------------------------------------------------------------------

/// Bridges DefaultPolicyRuntime (the full policy engine with rules, host/pack
/// partitioning, rate limiting, and default-deny on destructive ops) into the
/// session controller's PolicyEngine trait.
///
/// For each check, it builds a PolicyContext from the skill and session state,
/// then delegates to DefaultPolicyRuntime::evaluate() or ::check_skill().
pub struct PolicyEngineAdapter {
    inner: DefaultPolicyRuntime,
    max_steps: u32,
}

impl PolicyEngineAdapter {
    pub fn new(runtime: DefaultPolicyRuntime, max_steps: u32) -> Self {
        Self {
            inner: runtime,
            max_steps,
        }
    }

    /// Derive the SideEffectClass from a skill's declared effects and rollback support.
    fn derive_side_effect_class(skill: &SkillSpec) -> SideEffectClass {
        // If the skill has no expected effects, it's read-only.
        if skill.expected_effects.is_empty() {
            return SideEffectClass::ReadOnly;
        }

        let has_deletion = skill
            .expected_effects
            .iter()
            .any(|e| e.effect_type == EffectType::Deletion);

        let irreversible = skill.rollback_or_compensation.support == RollbackSupport::Irreversible;

        if has_deletion && irreversible {
            return SideEffectClass::Irreversible;
        }

        if has_deletion {
            return SideEffectClass::Destructive;
        }

        // Creation/Update that touches external resources is an external state mutation.
        let mutates = skill.expected_effects.iter().any(|e| {
            matches!(
                e.effect_type,
                EffectType::Creation
                    | EffectType::Update
                    | EffectType::Scheduling
                    | EffectType::Notification
                    | EffectType::Delegation
                    | EffectType::Synchronization
            )
        });

        if mutates {
            SideEffectClass::LocalStateMutation
        } else {
            SideEffectClass::ReadOnly
        }
    }

    /// Derive trust level from the session's goal source.
    fn derive_trust_level(session: &ControlSession) -> TrustLevel {
        use crate::types::goal::GoalSourceType;
        match session.goal.source.source_type {
            GoalSourceType::User | GoalSourceType::Internal | GoalSourceType::Scheduler => {
                TrustLevel::Trusted
            }
            GoalSourceType::Api | GoalSourceType::Mcp => TrustLevel::Verified,
            GoalSourceType::Peer => TrustLevel::Restricted,
        }
    }

    /// Build a PolicyContext from the skill and session.
    fn build_context(skill: &SkillSpec, session: &ControlSession) -> PolicyContext {
        PolicyContext {
            session_id: Some(session.session_id),
            trust_level: Self::derive_trust_level(session),
            namespace: skill.namespace.clone(),
            budget_remaining: Some(session.budget_remaining.resource_remaining),
            side_effect_class: Some(Self::derive_side_effect_class(skill)),
        }
    }

    /// Check budget exhaustion: deny if steps or resource budget are exhausted.
    fn check_budget(session: &ControlSession, max_steps: u32) -> Option<PolicyCheckResult> {
        let step_count = session.trace.steps.len() as u32;
        if step_count >= max_steps || session.budget_remaining.steps_remaining == 0 {
            return Some(PolicyCheckResult {
                allowed: false,
                reason: format!(
                    "budget exhausted: step {} of {} maximum",
                    step_count, max_steps,
                ),
                blocked_by_policy: true,
                waiting_for_input: None,
            });
        }

        if session.budget_remaining.resource_remaining <= 0.0 {
            return Some(PolicyCheckResult {
                allowed: false,
                reason: "budget exhausted: resource budget depleted".to_string(),
                blocked_by_policy: true,
                waiting_for_input: None,
            });
        }

        if session.budget_remaining.risk_remaining <= 0.0 {
            return Some(PolicyCheckResult {
                allowed: false,
                reason: "budget exhausted: risk budget depleted".to_string(),
                blocked_by_policy: true,
                waiting_for_input: None,
            });
        }

        None
    }

    /// Check whether a skill involves destructive operations, logging warnings
    /// and enforcing confirmation requirements.
    fn check_destructive(skill: &SkillSpec) -> Option<PolicyCheckResult> {
        let side_effect = Self::derive_side_effect_class(skill);

        match side_effect {
            SideEffectClass::Irreversible => {
                tracing::warn!(
                    skill_id = %skill.skill_id,
                    "policy: irreversible destructive operation requested — denying"
                );
                Some(PolicyCheckResult {
                    allowed: false,
                    reason: format!(
                        "irreversible destructive operation '{}' requires explicit host policy",
                        skill.skill_id,
                    ),
                    blocked_by_policy: true,
                    waiting_for_input: None,
                })
            }
            SideEffectClass::Destructive => {
                tracing::warn!(
                    skill_id = %skill.skill_id,
                    "policy: destructive operation requested — requires confirmation"
                );
                Some(PolicyCheckResult {
                    allowed: false,
                    reason: format!(
                        "destructive operation '{}' requires confirmation",
                        skill.skill_id,
                    ),
                    blocked_by_policy: true,
                    waiting_for_input: Some(format!(
                        "confirm destructive operation: {}",
                        skill.skill_id,
                    )),
                })
            }
            _ => None,
        }
    }

    /// Log a warning for write/delete filesystem operations.
    fn warn_write_operations(skill: &SkillSpec) {
        let is_write_op = skill.expected_effects.iter().any(|e| {
            matches!(
                e.effect_type,
                EffectType::Creation | EffectType::Update | EffectType::Deletion
            )
        });

        if is_write_op {
            tracing::warn!(
                skill_id = %skill.skill_id,
                effects = ?skill.expected_effects.iter().map(|e| format!("{:?}", e.effect_type)).collect::<Vec<_>>(),
                "policy: skill performs write/delete operations"
            );
        }
    }
}

impl PolicyEngine for PolicyEngineAdapter {
    fn check_skill_execution(
        &self,
        skill: &SkillSpec,
        session: &ControlSession,
    ) -> PolicyCheckResult {
        // Budget check first — hard deny.
        if let Some(denied) = Self::check_budget(session, self.max_steps) {
            return denied;
        }

        // Destructive operation checks.
        if let Some(denied) = Self::check_destructive(skill) {
            // Before returning the deny, check whether the full policy engine
            // has an explicit host Allow rule that overrides the default.
            let context = Self::build_context(skill, session);
            let decision = self.inner.check_destructive_operation(&skill.skill_id, &context);
            if decision.allowed {
                // Host policy explicitly permits this destructive action.
                Self::warn_write_operations(skill);
                return PolicyCheckResult {
                    allowed: true,
                    reason: format!(
                        "destructive operation allowed by host policy: {}",
                        decision.reason,
                    ),
                    blocked_by_policy: false,
                    waiting_for_input: None,
                };
            }
            return denied;
        }

        // Log write operations even when allowed.
        Self::warn_write_operations(skill);

        // Read-only skills pass without consulting the rule engine —
        // the wildcard RequireConfirmation rule is only for destructive ops.
        let side_effect = Self::derive_side_effect_class(skill);
        if matches!(side_effect, SideEffectClass::None | SideEffectClass::ReadOnly) {
            return PolicyCheckResult {
                allowed: true,
                reason: "read-only skill, no policy restriction".to_string(),
                blocked_by_policy: false,
                waiting_for_input: None,
            };
        }

        // Delegate to the full policy engine for rule-based evaluation.
        let context = Self::build_context(skill, session);
        match self.inner.check_skill(&skill.skill_id, &context) {
            Ok(decision) => PolicyCheckResult {
                allowed: decision.allowed,
                reason: decision.reason,
                blocked_by_policy: !decision.allowed,
                waiting_for_input: None,
            },
            Err(e) => {
                tracing::error!(error = %e, "policy evaluation failed, denying as safety fallback");
                PolicyCheckResult {
                    allowed: false,
                    reason: format!("policy evaluation error: {}", e),
                    blocked_by_policy: true,
                    waiting_for_input: None,
                }
            }
        }
    }

    fn check_hook(
        &self,
        hook: PolicyHook,
        skill: &SkillSpec,
        session: &ControlSession,
    ) -> PolicyCheckResult {
        // Budget check applies at every hook.
        if let Some(denied) = Self::check_budget(session, self.max_steps) {
            return denied;
        }

        // Hook-specific enforcement.
        match hook {
            PolicyHook::BeforeSideEffectingStep => {
                // Check destructive ops specifically at this hook.
                if let Some(denied) = Self::check_destructive(skill) {
                    let context = Self::build_context(skill, session);
                    let decision =
                        self.inner.check_destructive_operation(&skill.skill_id, &context);
                    if decision.allowed {
                        Self::warn_write_operations(skill);
                        return PolicyCheckResult {
                            allowed: true,
                            reason: format!(
                                "destructive operation allowed by host policy: {}",
                                decision.reason,
                            ),
                            blocked_by_policy: false,
                            waiting_for_input: None,
                        };
                    }
                    return denied;
                }
                Self::warn_write_operations(skill);
            }
            PolicyHook::BeforeExecutionBegins => {
                // Read-only skills skip rule evaluation — the wildcard
                // RequireConfirmation rule is only meant for destructive ops.
                let side_effect = Self::derive_side_effect_class(skill);
                if !matches!(side_effect, SideEffectClass::None | SideEffectClass::ReadOnly) {
                    let context = Self::build_context(skill, session);
                    match self.inner.check_skill(&skill.skill_id, &context) {
                        Ok(decision) if !decision.allowed => {
                            return PolicyCheckResult {
                                allowed: false,
                                reason: decision.reason,
                                blocked_by_policy: true,
                                waiting_for_input: None,
                            };
                        }
                        Err(e) => {
                            return PolicyCheckResult {
                                allowed: false,
                                reason: format!("policy evaluation error: {}", e),
                                blocked_by_policy: true,
                                waiting_for_input: None,
                            };
                        }
                        _ => {}
                    }
                }
            }
            PolicyHook::BeforeDelegation => {
                // Delegation to remote peers: check with elevated context.
                let context = Self::build_context(skill, session);
                match self
                    .inner
                    .evaluate(&crate::runtime::policy::PolicyRequest {
                        action: format!("delegate:{}", skill.skill_id),
                        target_type: PolicyTargetType::Skill,
                        target_id: skill.skill_id.clone(),
                        context,
                    }) {
                    Ok(decision) if !decision.allowed => {
                        return PolicyCheckResult {
                            allowed: false,
                            reason: decision.reason,
                            blocked_by_policy: true,
                            waiting_for_input: None,
                        };
                    }
                    Err(e) => {
                        return PolicyCheckResult {
                            allowed: false,
                            reason: format!("policy evaluation error: {}", e),
                            blocked_by_policy: true,
                            waiting_for_input: None,
                        };
                    }
                    _ => {}
                }
            }
            // Other hooks: default allow (budget check already passed above).
            _ => {}
        }

        PolicyCheckResult {
            allowed: true,
            reason: format!("policy check passed for hook {:?}", hook),
            blocked_by_policy: false,
            waiting_for_input: None,
        }
    }
}

// ---------------------------------------------------------------------------
// PermissivePolicyEngine
// ---------------------------------------------------------------------------

/// Allows everything. Kept as a fallback for tests that don't need policy enforcement.
pub struct PermissivePolicyEngine;

impl PolicyEngine for PermissivePolicyEngine {
    fn check_skill_execution(
        &self,
        _skill: &SkillSpec,
        _session: &ControlSession,
    ) -> PolicyCheckResult {
        PolicyCheckResult {
            allowed: true,
            reason: "permissive policy: all actions allowed".to_string(),
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
            reason: "permissive policy: all hooks allowed".to_string(),
            blocked_by_policy: false,
            waiting_for_input: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::episodes::DefaultEpisodeStore;
    use crate::memory::schemas::DefaultSchemaStore;
    use crate::memory::routines::DefaultRoutineStore;

    #[test]
    fn simple_belief_source_creates_belief() {
        let src = SimpleBeliefSource::new();
        let goal = test_goal();
        let belief = src.build_initial_belief(&goal).unwrap();
        assert!(belief.resources.is_empty());
    }

    #[test]
    fn empty_episode_memory_returns_empty() {
        let mem = EmptyEpisodeMemory;
        let goal = test_goal();
        let belief = DefaultBeliefRuntime::new()
            .create_belief(Uuid::new_v4())
            .unwrap();
        assert!(mem.retrieve_nearest(&goal, &belief, 5).is_empty());
    }

    #[test]
    fn empty_schema_memory_returns_empty() {
        let mem = EmptySchemaMemory;
        let goal = test_goal();
        let belief = DefaultBeliefRuntime::new()
            .create_belief(Uuid::new_v4())
            .unwrap();
        assert!(mem.retrieve_matching(&goal, &belief).is_empty());
    }

    #[test]
    fn empty_routine_memory_returns_empty() {
        let mem = EmptyRoutineMemory;
        let goal = test_goal();
        let belief = DefaultBeliefRuntime::new()
            .create_belief(Uuid::new_v4())
            .unwrap();
        assert!(mem.retrieve_matching(&goal, &belief).is_empty());
    }

    #[test]
    fn simple_predictor_scores_by_relevance() {
        let predictor = SimpleCandidatePredictor::new();
        let skills = vec![test_skill("a"), test_skill("b"), test_skill("c")];
        let goal = test_goal();
        let belief = DefaultBeliefRuntime::new()
            .create_belief(Uuid::new_v4())
            .unwrap();
        let scores = predictor.score(&skills, &goal, &belief, &[]);
        assert_eq!(scores.len(), 3);
        // All test skills have the same relevance to "test goal"
        // so they should all have the same score
    }

    #[test]
    fn simple_predictor_predict_top_limits() {
        let predictor = SimpleCandidatePredictor::new();
        let skills = vec![test_skill("a"), test_skill("b"), test_skill("c")];
        let goal = test_goal();
        let belief = DefaultBeliefRuntime::new()
            .create_belief(Uuid::new_v4())
            .unwrap();
        let scores = predictor.score(&skills, &goal, &belief, &[]);
        let top = predictor.predict_top(&scores, 1);
        assert_eq!(top.len(), 1);
    }

    #[test]
    fn simple_predictor_favors_readdir_for_list_files() {
        
        let predictor = SimpleCandidatePredictor::new();

        let mut readdir_skill = test_skill("reference.readdir");
        readdir_skill.name = "Read Directory".to_string();
        readdir_skill.description = "List entries in a directory".to_string();
        readdir_skill.capability_requirements = vec!["port:filesystem/readdir".to_string()];

        let mut rm_skill = test_skill("reference.rm");
        rm_skill.name = "Remove File".to_string();
        rm_skill.description = "Remove a file".to_string();
        rm_skill.capability_requirements = vec!["port:filesystem/rm".to_string()];

        let skills = vec![rm_skill, readdir_skill];

        let mut goal = test_goal();
        goal.objective.description = "list files in /tmp".to_string();

        let belief = DefaultBeliefRuntime::new()
            .create_belief(Uuid::new_v4())
            .unwrap();
        let scores = predictor.score(&skills, &goal, &belief, &[]);
        let top = predictor.predict_top(&scores, 1);

        assert_eq!(top[0].skill_id, "reference.readdir");
    }

    #[test]
    fn simple_critic_stops_on_success() {
        let critic = SimpleSessionCritic::new();
        let goal = test_goal();
        let belief = DefaultBeliefRuntime::new()
            .create_belief(Uuid::new_v4())
            .unwrap();
        let obs = Observation {
            observation_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            skill_id: None,
            port_calls: vec![],
            raw_result: serde_json::Value::Null,
            structured_result: serde_json::Value::Null,
            effect_patch: None,
            success: true,
            failure_class: None,
            latency_ms: 0,
            resource_cost: default_cost_profile(),
            confidence: 1.0,
            timestamp: Utc::now(),
        };
        let budget = Budget {
            risk_remaining: 0.5,
            latency_remaining_ms: 30000,
            resource_remaining: 100.0,
            steps_remaining: 100,
        };
        let decision = critic.evaluate(&goal, &belief, &obs, &budget, 0);
        assert_eq!(decision, CriticDecision::Stop);
    }

    #[test]
    fn permissive_policy_allows_everything() {
        let policy = PermissivePolicyEngine;
        let skill = test_skill("test");
        let session = test_session();
        let result = policy.check_skill_execution(&skill, &session);
        assert!(result.allowed);
    }

    #[test]
    fn resolve_port_capability_parses_format() {
        let mut skill = test_skill("test");
        skill.capability_requirements = vec!["port:filesystem/readdir".to_string()];
        let (port, cap) = PortBackedSkillExecutor::resolve_port_capability(&skill).unwrap();
        assert_eq!(port, "filesystem");
        assert_eq!(cap, "readdir");
    }

    #[test]
    fn resolve_port_capability_without_prefix() {
        let mut skill = test_skill("test");
        skill.capability_requirements = vec!["filesystem/readdir".to_string()];
        let (port, cap) = PortBackedSkillExecutor::resolve_port_capability(&skill).unwrap();
        assert_eq!(port, "filesystem");
        assert_eq!(cap, "readdir");
    }

    #[test]
    fn resolve_port_capability_fails_on_bad_format() {
        let mut skill = test_skill("test");
        skill.capability_requirements = vec!["no-slash-here".to_string()];
        assert!(PortBackedSkillExecutor::resolve_port_capability(&skill).is_err());
    }

    // --- test helpers ---

    fn test_goal() -> GoalSpec {
        use crate::types::goal::*;
        GoalSpec {
            goal_id: Uuid::new_v4(),
            source: GoalSource {
                source_type: GoalSourceType::User,
                identity: None,
                session_id: None,
                peer_id: None,
            },
            objective: Objective {
                description: "test goal".to_string(),
                structured: None,
            },
            constraints: vec![],
            success_conditions: vec![SuccessCondition {
                description: "done".to_string(),
                expression: serde_json::json!({"status": "done"}),
            }],
            risk_budget: 0.5,
            latency_budget_ms: 30000,
            resource_budget: 100.0,
            deadline: None,
            permissions_scope: vec!["default".to_string()],
            priority: Priority::Normal,
        }
    }

    fn test_skill(id: &str) -> SkillSpec {
        use crate::types::common::*;
        use crate::types::skill::*;
        SkillSpec {
            skill_id: id.to_string(),
            namespace: "test".to_string(),
            pack: "test".to_string(),
            kind: SkillKind::Primitive,
            name: id.to_string(),
            description: "test skill".to_string(),
            version: "0.1.0".to_string(),
            inputs: SchemaRef { schema: serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}) },
            outputs: SchemaRef { schema: serde_json::json!({"type": "object"}) },
            required_resources: vec![],
            preconditions: vec![],
            expected_effects: vec![],
            observables: vec![ObservableDecl { field: "result".to_string(), role: ObservableRole::ConfirmSuccess }],
            termination_conditions: vec![TerminationCondition { condition_type: TerminationType::Success, expression: serde_json::json!(true), description: "success".to_string() }],
            rollback_or_compensation: RollbackSpec { support: RollbackSupport::Irreversible, compensation_skill: None, description: "none".to_string() },
            cost_prior: CostPrior { latency: LatencyProfile { expected_latency_ms: 10, p95_latency_ms: 100, max_latency_ms: 1000 }, resource_cost: CostProfile { cpu_cost_class: CostClass::Negligible, memory_cost_class: CostClass::Negligible, io_cost_class: CostClass::Low, network_cost_class: CostClass::Negligible, energy_cost_class: CostClass::Negligible } },
            risk_class: RiskClass::Negligible,
            determinism: DeterminismClass::Deterministic,
            remote_exposure: RemoteExposureDecl { remote_scope: CapabilityScope::Local, peer_trust_requirements: "none".to_string(), serialization_requirements: "json".to_string(), rate_limits: "none".to_string(), replay_protection: false, observation_streaming: false, delegation_support: false, enabled: false },
            tags: vec![],
            aliases: vec![],
            capability_requirements: vec!["port:filesystem/readdir".to_string()],
            subskills: vec![],
            guard_conditions: vec![],
            match_conditions: vec![],
            telemetry_fields: vec![],
            policy_overrides: vec![],
            confidence_threshold: None,
            locality: None,
            remote_endpoint: None,
            remote_trust_requirement: None,
            remote_capability_contract: None,
            fallback_skill: None,
            invalidation_conditions: vec![],
            nondeterminism_sources: vec![],
            partial_success_behavior: None,
        }
    }

    fn test_session() -> ControlSession {
        use crate::types::session::*;
        ControlSession {
            session_id: Uuid::new_v4(),
            goal: test_goal(),
            belief: DefaultBeliefRuntime::new().create_belief(Uuid::new_v4()).unwrap(),
            working_memory: WorkingMemory {
                active_bindings: vec![],
                unresolved_slots: vec![],
                current_subgoal: None,
                recent_observations: vec![],
                candidate_shortlist: vec![],
                current_branch_state: None,
                budget_deltas: vec![],
                output_bindings: vec![],
                active_plan: None,
                plan_step: 0,
            },
            status: SessionStatus::Created,
            trace: SessionTrace { steps: vec![] },
            budget_remaining: Budget {
                risk_remaining: 0.5,
                latency_remaining_ms: 30000,
                resource_remaining: 100.0,
                steps_remaining: 100,
            },
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    // --- SchemaMemoryAdapter tests ---

    #[test]
    fn schema_memory_adapter_returns_empty_initially() {
        let store: Arc<Mutex<dyn SchemaStore + Send>> = Arc::new(Mutex::new(DefaultSchemaStore::new()));
        let adapter = SchemaMemoryAdapter::new(store);
        let goal = test_goal();
        let belief = DefaultBeliefRuntime::new()
            .create_belief(Uuid::new_v4())
            .unwrap();
        assert!(adapter.retrieve_matching(&goal, &belief).is_empty());
    }

    #[test]
    fn schema_memory_adapter_finds_matching_schema() {
        use crate::types::common::Precondition;
        use crate::types::schema::{RollbackBias, Schema};

        let store: Arc<Mutex<dyn SchemaStore + Send>> = Arc::new(Mutex::new(DefaultSchemaStore::new()));
        {
            let mut s = store.lock().unwrap();
            let schema = Schema {
                schema_id: "test_schema".to_string(),
                namespace: String::new(),
                pack: String::new(),
                name: "Test Schema".to_string(),
                version: semver::Version::new(0, 1, 0),
                trigger_conditions: vec![Precondition {
                    condition_type: "goal_fingerprint".to_string(),
                    expression: serde_json::json!({ "goal_fingerprint": "test goal" }),
                    description: "goal matches test goal".to_string(),
                }],
                resource_requirements: Vec::new(),
                subgoal_structure: Vec::new(),
                candidate_skill_ordering: vec!["a".into()],
                stop_conditions: Vec::new(),
                rollback_bias: RollbackBias::Cautious,
                confidence: 0.8,
            };
            s.register(schema).unwrap();
        }

        let adapter = SchemaMemoryAdapter::new(store);
        let goal = test_goal();
        let belief = DefaultBeliefRuntime::new()
            .create_belief(Uuid::new_v4())
            .unwrap();
        let results = adapter.retrieve_matching(&goal, &belief);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].schema_id, "test_schema");
    }

    // --- RoutineMemoryAdapter tests ---

    #[test]
    fn routine_memory_adapter_returns_empty_initially() {
        let store: Arc<Mutex<dyn RoutineStore + Send>> = Arc::new(Mutex::new(DefaultRoutineStore::new()));
        let adapter = RoutineMemoryAdapter::new(store);
        let goal = test_goal();
        let belief = DefaultBeliefRuntime::new()
            .create_belief(Uuid::new_v4())
            .unwrap();
        assert!(adapter.retrieve_matching(&goal, &belief).is_empty());
    }

    #[test]
    fn routine_memory_adapter_finds_matching_routine() {
        use crate::types::common::Precondition;
        use crate::types::routine::{Routine, RoutineOrigin};

        let store: Arc<Mutex<dyn RoutineStore + Send>> = Arc::new(Mutex::new(DefaultRoutineStore::new()));
        {
            let mut s = store.lock().unwrap();
            let routine = Routine {
                routine_id: "test_routine".to_string(),
                namespace: String::new(),
                origin: RoutineOrigin::SchemaCompiled,
                match_conditions: vec![Precondition {
                    condition_type: "goal_fingerprint".to_string(),
                    expression: serde_json::json!({ "goal_fingerprint": "test goal" }),
                    description: "goal matches test goal".to_string(),
                }],
                compiled_skill_path: vec!["a".into(), "b".into()],
                guard_conditions: Vec::new(),
                expected_cost: 0.1,
                expected_effect: Vec::new(),
                confidence: 0.9,
            };
            s.register(routine).unwrap();
        }

        let adapter = RoutineMemoryAdapter::new(store);
        let goal = test_goal();
        let belief = DefaultBeliefRuntime::new()
            .create_belief(Uuid::new_v4())
            .unwrap();
        let results = adapter.retrieve_matching(&goal, &belief);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].routine_id, "test_routine");
    }

    // --- EpisodeMemoryAdapter tests ---

    #[test]
    fn episode_memory_adapter_stores_and_retrieves() {
        use crate::types::episode::{Episode, EpisodeOutcome};

        let store: Arc<Mutex<dyn EpisodeStore + Send>> = Arc::new(Mutex::new(DefaultEpisodeStore::new()));
        let embedder: Arc<dyn crate::memory::embedder::GoalEmbedder + Send + Sync> = Arc::new(crate::memory::embedder::HashEmbedder::new());
        let adapter = EpisodeMemoryAdapter::new(Arc::clone(&store), embedder);

        let episode = Episode {
            episode_id: Uuid::new_v4(),
            goal_fingerprint: "test goal".to_string(),
            initial_belief_summary: serde_json::json!({}),
            steps: Vec::new(),
            observations: Vec::new(),
            outcome: EpisodeOutcome::Success,
            total_cost: 0.1,
            success: true,
            tags: Vec::new(),
            embedding: None,
            created_at: Utc::now(),
        };
        adapter.store(episode).unwrap();

        let mut goal = test_goal();
        goal.objective.description = "test goal".to_string();
        let belief = DefaultBeliefRuntime::new()
            .create_belief(Uuid::new_v4())
            .unwrap();
        let results = adapter.retrieve_nearest(&goal, &belief, 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].goal_fingerprint, "test goal");
    }

    // --- PolicyEngineAdapter tests ---

    fn make_policy_adapter(max_steps: u32) -> PolicyEngineAdapter {
        PolicyEngineAdapter::new(DefaultPolicyRuntime::new(), max_steps)
    }

    /// Helper: create a skill with specific side effects.
    fn test_skill_with_effects(
        id: &str,
        effects: Vec<crate::types::common::EffectDescriptor>,
        rollback: RollbackSupport,
    ) -> SkillSpec {
        
        use crate::types::skill::*;
        let mut skill = test_skill(id);
        skill.expected_effects = effects;
        skill.rollback_or_compensation = RollbackSpec {
            support: rollback,
            compensation_skill: None,
            description: "test".to_string(),
        };
        skill
    }

    #[test]
    fn policy_adapter_allows_read_only_skill() {
        let adapter = make_policy_adapter(100);
        let skill = test_skill("reference.readdir");
        let session = test_session();
        let result = adapter.check_skill_execution(&skill, &session);
        assert!(result.allowed, "read-only skill should be allowed: {}", result.reason);
        assert!(!result.blocked_by_policy);
    }

    #[test]
    fn policy_adapter_denies_when_budget_exhausted() {
        let adapter = make_policy_adapter(100);
        let skill = test_skill("reference.readdir");
        let mut session = test_session();
        session.budget_remaining.resource_remaining = 0.0;
        let result = adapter.check_skill_execution(&skill, &session);
        assert!(!result.allowed, "should be denied when budget exhausted");
        assert!(result.blocked_by_policy);
        assert!(result.reason.contains("budget exhausted"));
    }

    #[test]
    fn policy_adapter_denies_when_risk_exhausted() {
        let adapter = make_policy_adapter(100);
        let skill = test_skill("reference.readdir");
        let mut session = test_session();
        session.budget_remaining.risk_remaining = 0.0;
        let result = adapter.check_skill_execution(&skill, &session);
        assert!(!result.allowed, "should be denied when risk exhausted");
        assert!(result.blocked_by_policy);
        assert!(result.reason.contains("risk budget"));
    }

    #[test]
    fn policy_adapter_denies_when_steps_exhausted() {
        let adapter = make_policy_adapter(5);
        let skill = test_skill("reference.readdir");
        let mut session = test_session();
        // Add 5 steps to the trace to exceed max_steps=5.
        use crate::types::session::TraceStep;
        for _ in 0..5 {
            session.trace.steps.push(TraceStep {
                step_index: 0,
                belief_summary_before: serde_json::Value::Null,
                retrieved_episodes: vec![],
                retrieved_schemas: vec![],
                retrieved_routines: vec![],
                candidate_skills: vec![],
                predicted_scores: vec![],
                selected_skill: "x".to_string(),
                port_calls: vec![],
                observation_id: Uuid::new_v4(),
                belief_patch: serde_json::Value::Null,
                progress_delta: 0.0,
                critic_decision: "Continue".to_string(),
                policy_decisions: vec![],
                bound_inputs: vec![],
                precondition_results: vec![],
                termination_reason: None,
                rollback_invoked: false,
                timestamp: Utc::now(),
            });
        }
        let result = adapter.check_skill_execution(&skill, &session);
        assert!(!result.allowed, "should be denied when step limit exceeded");
        assert!(result.reason.contains("budget exhausted"));
    }

    #[test]
    fn policy_adapter_blocks_destructive_deletion() {
        use crate::types::common::EffectDescriptor;
        let adapter = make_policy_adapter(100);
        let skill = test_skill_with_effects(
            "reference.rm",
            vec![EffectDescriptor {
                effect_type: EffectType::Deletion,
                target_resource: Some("file".to_string()),
                description: "delete a file".to_string(),
                patch: None,
            }],
            RollbackSupport::CompensatingAction,
        );
        let session = test_session();
        let result = adapter.check_skill_execution(&skill, &session);
        assert!(!result.allowed, "destructive op should be blocked: {}", result.reason);
        assert!(
            result.reason.contains("destructive") || result.reason.contains("confirmation"),
            "reason should mention destructive: {}",
            result.reason,
        );
    }

    #[test]
    fn policy_adapter_blocks_irreversible_deletion() {
        use crate::types::common::EffectDescriptor;
        let adapter = make_policy_adapter(100);
        let skill = test_skill_with_effects(
            "reference.rm_irreversible",
            vec![EffectDescriptor {
                effect_type: EffectType::Deletion,
                target_resource: Some("file".to_string()),
                description: "irreversibly delete a file".to_string(),
                patch: None,
            }],
            RollbackSupport::Irreversible,
        );
        let session = test_session();
        let result = adapter.check_skill_execution(&skill, &session);
        assert!(!result.allowed, "irreversible op should be blocked: {}", result.reason);
        assert!(result.blocked_by_policy);
    }

    #[test]
    fn policy_adapter_allows_creation_effects() {
        use crate::types::common::EffectDescriptor;
        let adapter = make_policy_adapter(100);
        let skill = test_skill_with_effects(
            "reference.writefile",
            vec![EffectDescriptor {
                effect_type: EffectType::Creation,
                target_resource: Some("file".to_string()),
                description: "create a file".to_string(),
                patch: None,
            }],
            RollbackSupport::FullReversal,
        );
        let session = test_session();
        let result = adapter.check_skill_execution(&skill, &session);
        assert!(result.allowed, "creation effect should be allowed: {}", result.reason);
    }

    #[test]
    fn policy_adapter_hook_check_budget() {
        let adapter = make_policy_adapter(100);
        let skill = test_skill("reference.readdir");
        let mut session = test_session();
        session.budget_remaining.resource_remaining = 0.0;
        let result = adapter.check_hook(
            PolicyHook::BeforeExecutionBegins,
            &skill,
            &session,
        );
        assert!(!result.allowed, "hook should deny when budget exhausted");
    }

    #[test]
    fn policy_adapter_hook_before_side_effecting_blocks_destructive() {
        use crate::types::common::EffectDescriptor;
        let adapter = make_policy_adapter(100);
        let skill = test_skill_with_effects(
            "reference.rm",
            vec![EffectDescriptor {
                effect_type: EffectType::Deletion,
                target_resource: Some("file".to_string()),
                description: "delete".to_string(),
                patch: None,
            }],
            RollbackSupport::CompensatingAction,
        );
        let session = test_session();
        let result = adapter.check_hook(
            PolicyHook::BeforeSideEffectingStep,
            &skill,
            &session,
        );
        assert!(!result.allowed, "BeforeSideEffectingStep should block destructive op");
    }

    #[test]
    fn policy_adapter_hook_before_candidate_allows_readonly() {
        let adapter = make_policy_adapter(100);
        let skill = test_skill("reference.readdir");
        let session = test_session();
        let result = adapter.check_hook(
            PolicyHook::BeforeCandidateSelection,
            &skill,
            &session,
        );
        assert!(result.allowed, "BeforeCandidateSelection should allow read-only: {}", result.reason);
    }

    #[test]
    fn derive_side_effect_class_readonly_for_no_effects() {
        let skill = test_skill("reference.readdir");
        assert_eq!(
            PolicyEngineAdapter::derive_side_effect_class(&skill),
            SideEffectClass::ReadOnly,
        );
    }

    #[test]
    fn derive_side_effect_class_destructive_for_deletion() {
        use crate::types::common::EffectDescriptor;
        let skill = test_skill_with_effects(
            "reference.rm",
            vec![EffectDescriptor {
                effect_type: EffectType::Deletion,
                target_resource: None,
                description: "delete".to_string(),
                patch: None,
            }],
            RollbackSupport::CompensatingAction,
        );
        assert_eq!(
            PolicyEngineAdapter::derive_side_effect_class(&skill),
            SideEffectClass::Destructive,
        );
    }

    #[test]
    fn derive_side_effect_class_irreversible_for_deletion_plus_irreversible_rollback() {
        use crate::types::common::EffectDescriptor;
        let skill = test_skill_with_effects(
            "reference.rm",
            vec![EffectDescriptor {
                effect_type: EffectType::Deletion,
                target_resource: None,
                description: "delete permanently".to_string(),
                patch: None,
            }],
            RollbackSupport::Irreversible,
        );
        assert_eq!(
            PolicyEngineAdapter::derive_side_effect_class(&skill),
            SideEffectClass::Irreversible,
        );
    }

    #[test]
    fn derive_side_effect_class_local_mutation_for_creation() {
        use crate::types::common::EffectDescriptor;
        let skill = test_skill_with_effects(
            "reference.writefile",
            vec![EffectDescriptor {
                effect_type: EffectType::Creation,
                target_resource: None,
                description: "create file".to_string(),
                patch: None,
            }],
            RollbackSupport::FullReversal,
        );
        assert_eq!(
            PolicyEngineAdapter::derive_side_effect_class(&skill),
            SideEffectClass::LocalStateMutation,
        );
    }
}
