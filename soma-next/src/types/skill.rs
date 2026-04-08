use serde::{Deserialize, Serialize};

use super::common::{
    CapabilityScope, CostProfile, DeterminismClass, EffectDescriptor, LatencyProfile, Precondition,
    RiskClass, RollbackSupport, SchemaRef, TerminationCondition,
};

/// SkillKind — the four skill categories from spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillKind {
    Primitive,
    Composite,
    Routine,
    Delegated,
}

/// SkillSpec — the canonical skill declaration.
/// Every field follows skill-spec.md Section 5.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSpec {
    pub skill_id: String,
    pub namespace: String,
    pub pack: String,
    pub kind: SkillKind,
    pub name: String,
    pub description: String,
    pub version: String,

    pub inputs: SchemaRef,
    pub outputs: SchemaRef,
    pub required_resources: Vec<String>,
    pub preconditions: Vec<Precondition>,
    pub expected_effects: Vec<EffectDescriptor>,
    pub observables: Vec<ObservableDecl>,
    pub termination_conditions: Vec<TerminationCondition>,
    pub rollback_or_compensation: RollbackSpec,
    pub cost_prior: CostPrior,
    pub risk_class: RiskClass,
    pub determinism: DeterminismClass,
    pub remote_exposure: RemoteExposureDecl,

    // Optional fields (skill-spec.md Section 5.3)
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub capability_requirements: Vec<String>,
    #[serde(default)]
    pub subskills: Vec<SubskillRef>,
    #[serde(default)]
    pub guard_conditions: Vec<Precondition>,
    #[serde(default)]
    pub match_conditions: Vec<Precondition>,
    #[serde(default)]
    pub telemetry_fields: Vec<String>,
    #[serde(default)]
    pub policy_overrides: Vec<String>,
    pub confidence_threshold: Option<f64>,
    pub locality: Option<SkillLocality>,
    pub remote_endpoint: Option<String>,

    // --- Delegated skill fields (Section 12) ---
    pub remote_trust_requirement: Option<String>,
    pub remote_capability_contract: Option<String>,

    // --- Routine skill fields (Section 11) ---
    pub fallback_skill: Option<String>,
    #[serde(default)]
    pub invalidation_conditions: Vec<String>,

    // --- Determinism declaration (Section 6.3) ---
    /// What makes this skill nondeterministic — required when determinism is
    /// Stochastic or PartiallyDeterministic so callers can reason about
    /// reproducibility and the runtime can decide whether to retry.
    #[serde(default)]
    pub nondeterminism_sources: Vec<String>,

    // --- Partial success declaration (Section 17.3) ---
    /// Declares what constitutes partial success for this skill.
    /// A skill that may produce a PartialSuccess outcome must declare this;
    /// without it, the runtime will not accept partial success as a valid result.
    pub partial_success_behavior: Option<PartialSuccessDetail>,
}

/// Typed observable declaration — distinguishes the role each observable plays
/// during execution monitoring and outcome assessment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservableDecl {
    pub field: String,
    pub role: ObservableRole,
}

/// The role an observable plays during skill execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservableRole {
    ConfirmSuccess,
    DetectPartialSuccess,
    DetectAmbiguity,
    UpdateConfidence,
    General,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackSpec {
    pub support: RollbackSupport,
    pub compensation_skill: Option<String>,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostPrior {
    pub latency: LatencyProfile,
    pub resource_cost: CostProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubskillRef {
    pub skill_id: String,
    pub ordering: SubskillOrdering,
    pub required: bool,
    /// Branch condition expression: under what conditions this subskill is taken.
    /// MUST be declared for composite skills (Section 10).
    #[serde(default)]
    pub branch_condition: Option<serde_json::Value>,
    /// Stop condition: when to terminate this branch.
    /// MUST be declared for composite skills (Section 10).
    #[serde(default)]
    pub stop_condition: Option<serde_json::Value>,
}

/// Partial success detail (Section 17.3).
/// Required when a skill declares partial success as a possible outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartialSuccessDetail {
    /// Which effects occurred.
    pub effects_occurred: Vec<String>,
    /// Which effects did not occur.
    pub effects_missing: Vec<String>,
    /// Whether compensation is possible for missing effects.
    pub compensation_possible: bool,
    /// Whether downstream execution may continue despite partial success.
    pub downstream_continuation: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubskillOrdering {
    Sequential,
    Parallel,
    Conditional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillLocality {
    LocalOnly,
    RemoteAllowed,
    RemotePreferred,
}

/// Remote exposure declaration for a skill (pack-spec.md Section "Remote Exposure Requirements").
/// Every remotely exposable capability MUST declare these fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteExposureDecl {
    /// The scope at which this skill is remotely accessible.
    pub remote_scope: CapabilityScope,
    /// Minimum peer trust level required to invoke remotely.
    pub peer_trust_requirements: String,
    /// Serialization format required for remote invocation.
    pub serialization_requirements: String,
    /// Rate limits for remote callers.
    pub rate_limits: String,
    /// Whether replay protection is required.
    pub replay_protection: bool,
    /// Whether observation streaming is supported for remote callers.
    pub observation_streaming: bool,
    /// Whether delegation to further peers is allowed.
    pub delegation_support: bool,
    /// Whether remote exposure is enabled at all (default deny).
    pub enabled: bool,
}
