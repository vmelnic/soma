use std::collections::HashMap;

use tracing::{debug, warn};

use crate::errors::{Result, SomaError};
use crate::types::common::{EffectType, RollbackSupport, TerminationType};
use crate::types::session::{BindingSource, WorkingBinding};
use crate::types::skill::{SkillKind, SkillSpec};

// ---------------------------------------------------------------------------
// SkillQuery — structured filter for enumerate_candidates
// ---------------------------------------------------------------------------

/// Query struct for filtering skill candidates.
///
/// All fields are optional; a `None` field imposes no constraint.
/// Candidates must satisfy every non-None field.
#[derive(Debug, Clone, Default)]
pub struct SkillQuery {
    /// Filter by skill kind (primitive, composite, routine, delegated).
    pub kind: Option<SkillKind>,
    /// Candidate must declare all of these required resources.
    pub required_resources: Option<Vec<String>>,
    /// Candidate must belong to this pack.
    pub pack: Option<String>,
    /// Candidate must carry at least one of these tags.
    pub tags: Option<Vec<String>>,
    /// Candidate must support at least this risk class or lower.
    pub max_risk: Option<crate::types::common::RiskClass>,
    /// Free-text name substring match.
    pub name_contains: Option<String>,
}

// ---------------------------------------------------------------------------
// SemanticWarning — non-fatal issues from semantic validation (Section 13.2)
// ---------------------------------------------------------------------------

/// A non-fatal warning from semantic validation.
/// Skills with warnings MAY be loaded but SHOULD be flagged for review.
#[derive(Debug, Clone)]
pub struct SemanticWarning {
    pub skill_id: String,
    pub code: SemanticWarningCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SemanticWarningCode {
    /// Preconditions and effects appear inconsistent.
    InconsistentPreconditionsEffects,
    /// Observables cannot confirm success.
    ObservablesCannotConfirmSuccess,
    /// Termination conditions are incomplete (missing some of the 7 types).
    IncompleteTermination,
    /// Cost prior is unrealistically undefined.
    UndefinedCostPrior,
    /// Resource requirements are under-declared.
    UnderdeclaredResources,
    /// Stochastic/partially-deterministic skill has no declared nondeterminism_sources.
    UndeclaredNondeterminismSources,
}

// ---------------------------------------------------------------------------
// RuntimeValidationContext — inputs to pre-execution revalidation (Section 13.3)
// ---------------------------------------------------------------------------

/// Context for runtime revalidation before each execution.
#[derive(Debug, Clone)]
pub struct RuntimeValidationContext {
    /// Currently available resources by type/id.
    pub available_resources: Vec<String>,
    /// Current permission scope for the session.
    pub permission_scope: Vec<String>,
    /// Whether policy currently allows this skill.
    pub policy_allows: bool,
    /// Current resource versions (resource_id -> version).
    pub resource_versions: std::collections::HashMap<String, u64>,
    /// Remaining budget.
    pub budget_remaining: Option<f64>,
    /// Remote trust state for delegated skills (peer_id -> trusted).
    pub remote_trust_state: std::collections::HashMap<String, bool>,
}

// ---------------------------------------------------------------------------
// SkillRuntime trait
// ---------------------------------------------------------------------------

/// The primary interface for skill management and pre-execution validation.
///
/// Covers loading, validation, querying, input binding, and precondition
/// checking.  Actual execution (port dispatch, observation collection,
/// effect application, termination) belongs to the session controller;
/// this trait owns the skill catalog and the preparatory contract.
pub trait SkillRuntime {
    /// Register a validated skill into the runtime catalog.
    /// Rejects duplicates (same `skill_id`).
    fn register_skill(&mut self, spec: SkillSpec) -> Result<()>;

    /// Static validation of a skill spec (Section 13.1 of skill-spec.md).
    fn validate_skill(&self, spec: &SkillSpec) -> Result<()>;

    /// Semantic validation of a skill spec (Section 13.2 of skill-spec.md).
    /// Returns a list of non-fatal warnings. Skills with warnings MAY still
    /// be loaded but SHOULD be reviewed or demoted.
    fn validate_semantic(&self, spec: &SkillSpec) -> Vec<SemanticWarning>;

    /// Runtime validation before each execution (Section 13.3 of skill-spec.md).
    /// Checks resource availability, permissions, policy, versions, budget,
    /// and remote trust state.
    fn validate_runtime(
        &self,
        skill: &SkillSpec,
        context: &RuntimeValidationContext,
    ) -> Result<()>;

    /// Look up a skill by its canonical `skill_id`.
    fn get_skill(&self, skill_id: &str) -> Option<&SkillSpec>;

    /// List skills, optionally filtered by pack namespace.
    fn list_skills(&self, namespace: Option<&str>) -> Vec<&SkillSpec>;

    /// Return all skills matching the query constraints.
    fn enumerate_candidates(&self, requirements: &SkillQuery) -> Vec<&SkillSpec>;

    /// Bind skill inputs from a context value (belief state / goal fields).
    ///
    /// For every key declared in `skill.inputs.schema.properties`, this
    /// extracts the corresponding value from `context` and wraps it in a
    /// `WorkingBinding` that records the provenance of the bound value.
    /// Missing required keys are a binding failure.
    fn bind_inputs(
        &self,
        skill: &SkillSpec,
        context: &serde_json::Value,
    ) -> Result<Vec<WorkingBinding>>;

    /// Evaluate all preconditions against the current belief state.
    ///
    /// A precondition with `condition_type == "belief_contains"` checks that
    /// `expression.field` exists in `belief`.  Other types pass through
    /// (future policy / resource conditions are not enforced here).
    fn check_preconditions(
        &self,
        skill: &SkillSpec,
        belief: &serde_json::Value,
    ) -> Result<()>;
}

// ---------------------------------------------------------------------------
// DefaultSkillRuntime
// ---------------------------------------------------------------------------

/// Default in-memory implementation of `SkillRuntime`.
///
/// Skills are stored in a `HashMap` keyed by `skill_id`.
#[derive(Debug, Default)]
pub struct DefaultSkillRuntime {
    skills: HashMap<String, SkillSpec>,
}

impl DefaultSkillRuntime {
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
        }
    }

    // -- internal helpers --------------------------------------------------

    /// Validate that required string fields are non-empty.
    fn validate_required_fields(spec: &SkillSpec) -> Result<()> {
        let id = &spec.skill_id;

        if spec.skill_id.is_empty() {
            return Err(SomaError::SkillValidation {
                skill_id: id.clone(),
                reason: "skill_id is empty".into(),
            });
        }
        if spec.pack.is_empty() {
            return Err(SomaError::SkillValidation {
                skill_id: id.clone(),
                reason: "pack is empty".into(),
            });
        }
        if spec.name.is_empty() {
            return Err(SomaError::SkillValidation {
                skill_id: id.clone(),
                reason: "name is empty".into(),
            });
        }
        if spec.description.is_empty() {
            return Err(SomaError::SkillValidation {
                skill_id: id.clone(),
                reason: "description is empty".into(),
            });
        }
        if spec.version.is_empty() {
            return Err(SomaError::SkillValidation {
                skill_id: id.clone(),
                reason: "version is empty".into(),
            });
        }
        Ok(())
    }

    /// Validate input/output schemas are JSON objects (or explicitly null for
    /// skills with no inputs/outputs, which is unusual but possible).
    fn validate_schemas(spec: &SkillSpec) -> Result<()> {
        let id = &spec.skill_id;

        if let Some(obj) = spec.inputs.schema.as_object() {
            // If there is a "type" key it must be "object".
            if let Some(t) = obj.get("type")
                && t.as_str() != Some("object")
            {
                return Err(SomaError::SkillValidation {
                    skill_id: id.clone(),
                    reason: format!(
                        "inputs schema type must be \"object\", got {:?}",
                        t
                    ),
                });
            }
        } else if !spec.inputs.schema.is_null() {
            return Err(SomaError::SkillValidation {
                skill_id: id.clone(),
                reason: "inputs schema must be a JSON object or null".into(),
            });
        }

        if let Some(obj) = spec.outputs.schema.as_object() {
            if let Some(t) = obj.get("type")
                && t.as_str() != Some("object")
            {
                return Err(SomaError::SkillValidation {
                    skill_id: id.clone(),
                    reason: format!(
                        "outputs schema type must be \"object\", got {:?}",
                        t
                    ),
                });
            }
        } else if !spec.outputs.schema.is_null() {
            return Err(SomaError::SkillValidation {
                skill_id: id.clone(),
                reason: "outputs schema must be a JSON object or null".into(),
            });
        }

        // Structural consistency: every field listed in "required" must appear
        // in "properties".  A required field absent from properties is a
        // schema authoring error that would make runtime validation impossible.
        Self::check_required_properties_consistency(id, "inputs", &spec.inputs.schema)?;
        Self::check_required_properties_consistency(id, "outputs", &spec.outputs.schema)?;

        Ok(())
    }

    /// Verify that all fields listed in a schema's "required" array also
    /// appear in its "properties" object.  A mismatch indicates a malformed
    /// schema where the author declared a field as required but never
    /// defined it, which would make downstream validation or binding
    /// against that schema impossible.
    fn check_required_properties_consistency(
        skill_id: &str,
        label: &str,
        schema: &serde_json::Value,
    ) -> Result<()> {
        let obj = match schema.as_object() {
            Some(o) => o,
            None => return Ok(()),
        };

        let required = match obj.get("required").and_then(|r| r.as_array()) {
            Some(arr) => arr,
            None => return Ok(()),
        };

        let properties = obj.get("properties").and_then(|p| p.as_object());

        for req_val in required {
            if let Some(field_name) = req_val.as_str() {
                let declared = properties
                    .map(|props| props.contains_key(field_name))
                    .unwrap_or(false);
                if !declared {
                    return Err(SomaError::SkillValidation {
                        skill_id: skill_id.to_string(),
                        reason: format!(
                            "{} schema lists \"{}\" as required but it is not \
                             defined in properties",
                            label, field_name
                        ),
                    });
                }
            }
        }

        Ok(())
    }

    /// Validate that every precondition has a non-empty condition_type and a
    /// non-null expression.  Malformed preconditions caught here avoid
    /// confusing failures at runtime when check_preconditions tries to
    /// interpret them.
    fn validate_preconditions(spec: &SkillSpec) -> Result<()> {
        let id = &spec.skill_id;

        for (i, precondition) in spec.preconditions.iter().enumerate() {
            if precondition.condition_type.is_empty() {
                return Err(SomaError::SkillValidation {
                    skill_id: id.clone(),
                    reason: format!(
                        "precondition[{}] has an empty condition_type",
                        i
                    ),
                });
            }
            if precondition.expression.is_null() {
                return Err(SomaError::SkillValidation {
                    skill_id: id.clone(),
                    reason: format!(
                        "precondition[{}] ({}) has a null expression",
                        i, precondition.condition_type
                    ),
                });
            }
        }

        Ok(())
    }

    /// Rollback or compensation must be declared for destructive side effects.  Any effect whose type is Deletion or
    /// Update (state-mutating) with `Irreversible` rollback support
    /// that does NOT declare the risk explicitly is rejected.
    ///
    /// Concrete rule: if any expected_effect has an effect_type that is
    /// state-mutating (Deletion, Update, Creation), the rollback_or_compensation
    /// MUST NOT be Irreversible unless it is explicitly acknowledged via a
    /// non-empty description.
    fn validate_rollback(spec: &SkillSpec) -> Result<()> {
        let has_destructive_effect = spec.expected_effects.iter().any(|e| {
            matches!(
                e.effect_type,
                EffectType::Deletion | EffectType::Update | EffectType::Creation
            )
        });

        if has_destructive_effect
            && spec.rollback_or_compensation.support == RollbackSupport::Irreversible
            && spec.rollback_or_compensation.description.is_empty()
        {
            return Err(SomaError::SkillValidation {
                skill_id: spec.skill_id.clone(),
                reason: "rollback/compensation is declared Irreversible for \
                         destructive effects but has no description — \
                         destructive skills must explicitly acknowledge \
                         irreversibility"
                    .into(),
            });
        }

        Ok(())
    }

    /// Composite skills must declare at least one subskill, and every subskill
    /// must carry explicit branch and stop conditions so the runtime knows
    /// when to take each branch and when to end it.
    fn validate_composite(spec: &SkillSpec) -> Result<()> {
        if spec.kind == SkillKind::Composite {
            if spec.subskills.is_empty() {
                return Err(SomaError::SkillValidation {
                    skill_id: spec.skill_id.clone(),
                    reason: "composite skill must declare at least one subskill".into(),
                });
            }
            for sub in &spec.subskills {
                if sub.branch_condition.is_none() {
                    return Err(SomaError::SkillValidation {
                        skill_id: spec.skill_id.clone(),
                        reason: format!(
                            "composite subskill '{}' must declare a branch_condition",
                            sub.skill_id
                        ),
                    });
                }
                if sub.stop_condition.is_none() {
                    return Err(SomaError::SkillValidation {
                        skill_id: spec.skill_id.clone(),
                        reason: format!(
                            "composite subskill '{}' must declare a stop_condition",
                            sub.skill_id
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    /// Routine skills must declare match_conditions, confidence_threshold,
    /// and fallback_skill (Section 11).
    fn validate_routine(spec: &SkillSpec) -> Result<()> {
        if spec.kind == SkillKind::Routine {
            if spec.match_conditions.is_empty() {
                return Err(SomaError::SkillValidation {
                    skill_id: spec.skill_id.clone(),
                    reason: "routine skill must declare at least one match_condition".into(),
                });
            }
            if spec.confidence_threshold.is_none() {
                return Err(SomaError::SkillValidation {
                    skill_id: spec.skill_id.clone(),
                    reason: "routine skill must declare confidence_threshold".into(),
                });
            }
            if spec.fallback_skill.is_none() {
                return Err(SomaError::SkillValidation {
                    skill_id: spec.skill_id.clone(),
                    reason: "routine skill must declare fallback_skill (Section 11)".into(),
                });
            }
        }
        Ok(())
    }

    /// Delegated skills must declare non-empty remote_endpoint,
    /// remote_trust_requirement, and remote_capability_contract (Section 12).
    fn validate_delegated(spec: &SkillSpec) -> Result<()> {
        if spec.kind == SkillKind::Delegated {
            match spec.remote_endpoint.as_deref() {
                None | Some("") => {
                    return Err(SomaError::SkillValidation {
                        skill_id: spec.skill_id.clone(),
                        reason: "delegated skill must declare a non-empty remote_endpoint".into(),
                    });
                }
                _ => {}
            }
            match spec.remote_trust_requirement.as_deref() {
                None | Some("") => {
                    return Err(SomaError::SkillValidation {
                        skill_id: spec.skill_id.clone(),
                        reason: "delegated skill must declare a non-empty remote_trust_requirement (Section 12)".into(),
                    });
                }
                _ => {}
            }
            match spec.remote_capability_contract.as_deref() {
                None | Some("") => {
                    return Err(SomaError::SkillValidation {
                        skill_id: spec.skill_id.clone(),
                        reason: "delegated skill must declare a non-empty remote_capability_contract (Section 12)".into(),
                    });
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Determine the provenance of a bound value by inspecting which sub-object
    /// of the context it was found in. The context is checked in priority order:
    /// goal fields, belief resources, prior observations, remote observations,
    /// working memory, and finally falls back to pack default.
    fn determine_binding_source(key: &str, context: &serde_json::Value) -> BindingSource {
        if context.get("goal_fields").and_then(|g| g.get(key)).is_some() {
            BindingSource::GoalField
        } else if context.get("belief_resources").and_then(|b| b.get(key)).is_some() {
            BindingSource::BeliefResource
        } else if context.get("prior_observations").and_then(|p| p.get(key)).is_some() {
            BindingSource::PriorObservation
        } else if context.get("remote_observations").and_then(|r| r.get(key)).is_some() {
            BindingSource::RemoteObservation
        } else if context.get("working_memory").and_then(|w| w.get(key)).is_some() {
            BindingSource::WorkingMemory
        } else {
            BindingSource::PackDefault
        }
    }

    /// Check that a JSON value is type-compatible with a JSON Schema type string.
    /// Returns `false` if the type is known and the value does not match.
    /// Unknown or missing type strings pass through as `true`.
    fn check_type_compat(value: &serde_json::Value, expected_type: &str) -> bool {
        match expected_type {
            "string" => value.is_string(),
            "number" => value.is_number(),
            "integer" => value.is_i64() || value.is_u64(),
            "boolean" => value.is_boolean(),
            "array" => value.is_array(),
            "object" => value.is_object(),
            "null" => value.is_null(),
            _ => true,
        }
    }

    /// Return a human-readable type name for a JSON value, used in binding
    /// error messages to describe what type was actually received.
    fn json_type_name(value: &serde_json::Value) -> &'static str {
        match value {
            serde_json::Value::Null => "null",
            serde_json::Value::Bool(_) => "boolean",
            serde_json::Value::Number(_) => "number",
            serde_json::Value::String(_) => "string",
            serde_json::Value::Array(_) => "array",
            serde_json::Value::Object(_) => "object",
        }
    }

    /// Every skill must be bounded: it must either declare at least one
    /// termination condition or have a budget constraint (non-zero
    /// max_latency_ms in cost_prior).  An unbounded skill could run
    /// forever, which the runtime cannot allow.
    fn validate_boundedness(spec: &SkillSpec) -> Result<()> {
        if spec.termination_conditions.is_empty() {
            let has_time_bound = spec.cost_prior.latency.max_latency_ms > 0;
            if !has_time_bound {
                return Err(SomaError::SkillValidation {
                    skill_id: spec.skill_id.clone(),
                    reason: "skill must declare at least one termination condition \
                             or a budget constraint (non-zero max_latency_ms)"
                        .into(),
                });
            }
        }
        Ok(())
    }

    /// Termination conditions must include at least one success and one
    /// failure condition.
    fn validate_termination(spec: &SkillSpec) -> Result<()> {
        use crate::types::common::TerminationType;

        let has_success = spec
            .termination_conditions
            .iter()
            .any(|t| t.condition_type == TerminationType::Success);
        let has_failure = spec.termination_conditions.iter().any(|t| {
            matches!(
                t.condition_type,
                TerminationType::Failure | TerminationType::Timeout | TerminationType::BudgetExhaustion
            )
        });

        if !has_success {
            return Err(SomaError::SkillValidation {
                skill_id: spec.skill_id.clone(),
                reason: "termination_conditions must include at least one Success condition".into(),
            });
        }
        if !has_failure {
            return Err(SomaError::SkillValidation {
                skill_id: spec.skill_id.clone(),
                reason: "termination_conditions must include at least one failure/timeout/budget condition".into(),
            });
        }

        Ok(())
    }

    /// Version must be valid semver (Section 13.1 — version/compatibility metadata).
    fn validate_version(spec: &SkillSpec) -> Result<()> {
        // Simple semver check: must have at least major.minor.patch format.
        let parts: Vec<&str> = spec.version.split('.').collect();
        if parts.len() < 2 {
            return Err(SomaError::SkillValidation {
                skill_id: spec.skill_id.clone(),
                reason: format!(
                    "version '{}' is not valid semver (expected at least major.minor)",
                    spec.version
                ),
            });
        }
        for (i, part) in parts.iter().enumerate() {
            if part.parse::<u64>().is_err() {
                // Allow pre-release suffixes on the last segment (e.g. "1.0.0-alpha").
                if i == parts.len() - 1 {
                    let base = part.split('-').next().unwrap_or("");
                    if base.parse::<u64>().is_err() {
                        return Err(SomaError::SkillValidation {
                            skill_id: spec.skill_id.clone(),
                            reason: format!(
                                "version '{}' contains non-numeric segment '{}'",
                                spec.version, part
                            ),
                        });
                    }
                } else {
                    return Err(SomaError::SkillValidation {
                        skill_id: spec.skill_id.clone(),
                        reason: format!(
                            "version '{}' contains non-numeric segment '{}'",
                            spec.version, part
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    /// Risk class must be declared for destructive behavior (Section 13.1).
    fn validate_risk_class(spec: &SkillSpec) -> Result<()> {
        use crate::types::common::RiskClass;

        let has_destructive = spec.expected_effects.iter().any(|e| {
            matches!(e.effect_type, EffectType::Deletion)
        });

        if has_destructive && spec.risk_class == RiskClass::Negligible {
            return Err(SomaError::SkillValidation {
                skill_id: spec.skill_id.clone(),
                reason: "skill with Deletion effects must not have Negligible risk_class \
                         (Section 13.1)"
                    .into(),
            });
        }
        Ok(())
    }

    /// Check whether a skill matches a SkillQuery.
    fn matches_query(spec: &SkillSpec, query: &SkillQuery) -> bool {
        if let Some(kind) = &query.kind
            && spec.kind != *kind
        {
            return false;
        }

        if let Some(pack) = &query.pack
            && spec.pack != *pack
        {
            return false;
        }

        if let Some(required) = &query.required_resources {
            for r in required {
                if !spec.required_resources.contains(r) {
                    return false;
                }
            }
        }

        if let Some(tags) = &query.tags
            && !tags.iter().any(|t| spec.tags.contains(t))
        {
            return false;
        }

        if let Some(max_risk) = &query.max_risk
            && !risk_within_budget(&spec.risk_class, max_risk)
        {
            return false;
        }

        if let Some(name_sub) = &query.name_contains {
            let lower = spec.name.to_lowercase();
            if !lower.contains(&name_sub.to_lowercase()) {
                return false;
            }
        }

        true
    }
}

impl SkillRuntime for DefaultSkillRuntime {
    fn register_skill(&mut self, spec: SkillSpec) -> Result<()> {
        self.validate_skill(&spec)?;

        if self.skills.contains_key(&spec.skill_id) {
            return Err(SomaError::NamespaceCollision(format!(
                "skill already registered: {}",
                spec.skill_id
            )));
        }

        // Warn if skill_id is not properly namespaced.  A well-formed
        // skill_id should contain a dot separator (e.g. "pack.skill_name").
        // When a namespace is set, the id should start with that namespace
        // followed by a dot.
        if !spec.namespace.is_empty() && !spec.skill_id.contains('.') {
            warn!(
                skill_id = %spec.skill_id,
                namespace = %spec.namespace,
                "skill_id is not dot-namespaced; expected format \
                 \"<namespace>.<skill_name>\" when namespace is set"
            );
        } else if !spec.namespace.is_empty()
            && !spec.skill_id.starts_with(&format!("{}.", spec.namespace))
        {
            warn!(
                skill_id = %spec.skill_id,
                namespace = %spec.namespace,
                "skill_id does not start with its declared namespace prefix"
            );
        }

        debug!(skill_id = %spec.skill_id, pack = %spec.pack, "registered skill");
        self.skills.insert(spec.skill_id.clone(), spec);
        Ok(())
    }

    fn validate_skill(&self, spec: &SkillSpec) -> Result<()> {
        Self::validate_required_fields(spec)?;
        Self::validate_schemas(spec)?;
        Self::validate_preconditions(spec)?;
        Self::validate_version(spec)?;
        Self::validate_rollback(spec)?;
        Self::validate_risk_class(spec)?;
        Self::validate_composite(spec)?;
        Self::validate_routine(spec)?;
        Self::validate_delegated(spec)?;
        Self::validate_boundedness(spec)?;
        Self::validate_termination(spec)?;
        Ok(())
    }

    fn validate_semantic(&self, spec: &SkillSpec) -> Vec<SemanticWarning> {
        let mut warnings = Vec::new();
        let id = &spec.skill_id;

        // A skill that demands resource preconditions but declares no effects
        // is suspicious — it reads state but changes nothing, which usually
        // means the effect list is incomplete.
        let has_resource_preconditions = spec.preconditions.iter().any(|p| {
            p.condition_type == "resource_available" || p.condition_type == "resource_version"
        });
        let has_effects = !spec.expected_effects.is_empty();
        if has_resource_preconditions && !has_effects {
            warnings.push(SemanticWarning {
                skill_id: id.clone(),
                code: SemanticWarningCode::InconsistentPreconditionsEffects,
                message: "skill has resource preconditions but declares no effects".into(),
            });
        }

        // Without observables the runtime cannot verify whether execution
        // succeeded or update belief state confidence.
        if spec.observables.is_empty() {
            warnings.push(SemanticWarning {
                skill_id: id.clone(),
                code: SemanticWarningCode::ObservablesCannotConfirmSuccess,
                message: "skill declares no observables — cannot confirm success".into(),
            });
        }

        // A complete termination declaration covers all seven exit paths:
        // success, failure, timeout, budget exhaustion, policy denial,
        // external error, and explicit abort.  Missing any of them means
        // the session controller has no guidance for that exit case.
        let has_type = |t: TerminationType| {
            spec.termination_conditions.iter().any(|tc| tc.condition_type == t)
        };
        let covered = [
            has_type(TerminationType::Success),
            has_type(TerminationType::Failure),
            has_type(TerminationType::Timeout),
            has_type(TerminationType::BudgetExhaustion),
            has_type(TerminationType::PolicyDenial),
            has_type(TerminationType::ExternalError),
            has_type(TerminationType::ExplicitAbort),
        ];
        let coverage_count = covered.iter().filter(|&&b| b).count();
        if coverage_count < 7 {
            warnings.push(SemanticWarning {
                skill_id: id.clone(),
                code: SemanticWarningCode::IncompleteTermination,
                message: format!(
                    "termination conditions cover only {}/7 required exit types",
                    coverage_count
                ),
            });
        }

        // All-zero latency means the author left cost_prior at its default
        // without filling it in, which makes scheduling and budget checks
        // meaningless for this skill.
        if spec.cost_prior.latency.expected_latency_ms == 0
            && spec.cost_prior.latency.p95_latency_ms == 0
            && spec.cost_prior.latency.max_latency_ms == 0
        {
            warnings.push(SemanticWarning {
                skill_id: id.clone(),
                code: SemanticWarningCode::UndefinedCostPrior,
                message: "cost_prior latency is all zeros — unrealistically undefined".into(),
            });
        }

        // If a skill targets resources in its effects but does not list those
        // resources in required_resources, the runtime cannot gate execution
        // on resource availability or version checks.
        let effects_target_resources = spec.expected_effects.iter().any(|e| {
            e.target_resource.is_some()
        });
        if effects_target_resources && spec.required_resources.is_empty() {
            warnings.push(SemanticWarning {
                skill_id: id.clone(),
                code: SemanticWarningCode::UnderdeclaredResources,
                message: "skill has effects targeting resources but declares no required_resources"
                    .into(),
            });
        }

        // Stochastic and partially-deterministic skills must name the sources
        // of their nondeterminism so callers can reason about reproducibility
        // and so the runtime can decide when to retry vs. when to give up.
        if matches!(
            spec.determinism,
            crate::types::common::DeterminismClass::Stochastic
                | crate::types::common::DeterminismClass::PartiallyDeterministic
        ) && spec.nondeterminism_sources.is_empty()
        {
            warnings.push(SemanticWarning {
                skill_id: id.clone(),
                code: SemanticWarningCode::UndeclaredNondeterminismSources,
                message: "stochastic/partially-deterministic skill must declare nondeterminism_sources"
                    .into(),
            });
        }

        warnings
    }

    fn validate_runtime(
        &self,
        skill: &SkillSpec,
        context: &RuntimeValidationContext,
    ) -> Result<()> {
        // Every resource the skill needs must exist in the current session
        // context before execution is allowed to start.
        for res in &skill.required_resources {
            if !context.available_resources.contains(res) {
                return Err(SomaError::SkillExecution {
                    skill_id: skill.skill_id.clone(),
                    failure_class: crate::types::common::SkillFailureClass::PreconditionFailure,
                    details: format!("required resource '{}' not available in session context", res),
                });
            }
        }

        // The session's permission scope must cover every resource the skill
        // requires; anything out of scope is a policy denial, not a missing
        // resource.
        for res in &skill.required_resources {
            if !context.permission_scope.contains(res) {
                return Err(SomaError::SkillExecution {
                    skill_id: skill.skill_id.clone(),
                    failure_class: crate::types::common::SkillFailureClass::PolicyDenial,
                    details: format!("resource '{}' is outside session permission scope", res),
                });
            }
        }

        // Policy engine has already ruled — if it said no, stop here.
        if !context.policy_allows {
            return Err(SomaError::SkillExecution {
                skill_id: skill.skill_id.clone(),
                failure_class: crate::types::common::SkillFailureClass::PolicyDenial,
                details: "policy engine denied execution".into(),
            });
        }

        // Check resource versions when the context provides them. If the
        // required resource exists but the version doesn't match what the
        // context knows, the skill may be operating on stale data.
        for res in &skill.required_resources {
            if let Some(&ctx_version) = context.resource_versions.get(res.as_str())
                && ctx_version == 0 {
                    warn!(
                        skill_id = %skill.skill_id,
                        resource = %res,
                        "resource version is 0 — may be uninitialized"
                    );
                }
        }

        // Refuse to start a skill when the session has already run out of budget.
        if let Some(budget) = context.budget_remaining
            && budget <= 0.0
        {
            return Err(SomaError::SkillExecution {
                skill_id: skill.skill_id.clone(),
                failure_class: crate::types::common::SkillFailureClass::BudgetExhaustion,
                details: "session budget exhausted before skill start".into(),
            });
        }

        // Delegated skills route to a remote peer — verify that peer is
        // currently trusted before committing to the remote call.
        if skill.kind == SkillKind::Delegated
            && let Some(endpoint) = &skill.remote_endpoint
        {
            let trusted = context
                .remote_trust_state
                .get(endpoint)
                .copied()
                .unwrap_or(false);
            if !trusted {
                return Err(SomaError::SkillExecution {
                    skill_id: skill.skill_id.clone(),
                    failure_class: crate::types::common::SkillFailureClass::RemoteFailure,
                    details: format!("remote endpoint '{}' is not trusted", endpoint),
                });
            }
        }

        Ok(())
    }

    fn get_skill(&self, skill_id: &str) -> Option<&SkillSpec> {
        self.skills.get(skill_id)
    }

    fn list_skills(&self, namespace: Option<&str>) -> Vec<&SkillSpec> {
        match namespace {
            Some(ns) => self.skills.values().filter(|s| s.pack == ns).collect(),
            None => self.skills.values().collect(),
        }
    }

    fn enumerate_candidates(&self, requirements: &SkillQuery) -> Vec<&SkillSpec> {
        self.skills
            .values()
            .filter(|s| Self::matches_query(s, requirements))
            .collect()
    }

    fn bind_inputs(
        &self,
        skill: &SkillSpec,
        context: &serde_json::Value,
    ) -> Result<Vec<WorkingBinding>> {
        let schema_obj = match skill.inputs.schema.as_object() {
            Some(obj) => obj,
            None => {
                // Null schema means no inputs required.
                return Ok(Vec::new());
            }
        };

        let properties = match schema_obj.get("properties").and_then(|p| p.as_object()) {
            Some(props) => props,
            None => {
                // Schema with no properties: nothing to bind.
                return Ok(Vec::new());
            }
        };

        let required_fields: Vec<&str> = schema_obj
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect()
            })
            .unwrap_or_default();

        let context_obj = context.as_object();

        let mut bindings = Vec::new();

        for (key, property_schema) in properties {
            let value = context_obj.and_then(|obj| obj.get(key.as_str()));

            match value {
                Some(v) => {
                    // If the property schema declares a type, the bound value
                    // must match it.  A mismatch is a binding failure, not a
                    // missing-field failure.
                    if let Some(expected_type) =
                        property_schema.get("type").and_then(|t| t.as_str())
                        && !Self::check_type_compat(v, expected_type)
                    {
                        return Err(SomaError::SkillExecution {
                            skill_id: skill.skill_id.clone(),
                            failure_class:
                                crate::types::common::SkillFailureClass::BindingFailure,
                            details: format!(
                                "input '{}' type mismatch: expected {}, got {}",
                                key, expected_type, Self::json_type_name(v)
                            ),
                        });
                    }
                    let source = Self::determine_binding_source(key, context);
                    bindings.push(WorkingBinding {
                        name: key.clone(),
                        value: v.clone(),
                        source,
                    });
                }
                None => {
                    if required_fields.contains(&key.as_str()) {
                        return Err(SomaError::SkillExecution {
                            skill_id: skill.skill_id.clone(),
                            failure_class: crate::types::common::SkillFailureClass::BindingFailure,
                            details: format!("required input '{}' not present in context", key),
                        });
                    }
                    // Optional field — omit from bound result.
                    debug!(
                        skill_id = %skill.skill_id,
                        field = %key,
                        "optional input field not present in context, skipping"
                    );
                }
            }
        }

        Ok(bindings)
    }

    fn check_preconditions(
        &self,
        skill: &SkillSpec,
        belief: &serde_json::Value,
    ) -> Result<()> {
        for precondition in &skill.preconditions {
            match precondition.condition_type.as_str() {
                "belief_contains" => {
                    // expression must have a "field" key; the value at
                    // that path must exist in belief.
                    let field = precondition
                        .expression
                        .get("field")
                        .and_then(|f| f.as_str())
                        .ok_or_else(|| SomaError::SkillValidation {
                            skill_id: skill.skill_id.clone(),
                            reason: format!(
                                "precondition '{}' has condition_type belief_contains \
                                 but expression lacks a 'field' string",
                                precondition.description
                            ),
                        })?;

                    let found = belief
                        .as_object()
                        .map(|obj| obj.contains_key(field))
                        .unwrap_or(false);

                    if !found {
                        warn!(
                            skill_id = %skill.skill_id,
                            field = %field,
                            "precondition failed: belief does not contain required field"
                        );
                        return Err(SomaError::SkillExecution {
                            skill_id: skill.skill_id.clone(),
                            failure_class:
                                crate::types::common::SkillFailureClass::PreconditionFailure,
                            details: format!(
                                "belief_contains precondition failed: belief missing field '{}' ({})",
                                field, precondition.description
                            ),
                        });
                    }
                }
                "belief_equals" => {
                    // expression: { "field": "...", "value": ... }
                    let field = precondition
                        .expression
                        .get("field")
                        .and_then(|f| f.as_str())
                        .ok_or_else(|| SomaError::SkillValidation {
                            skill_id: skill.skill_id.clone(),
                            reason: format!(
                                "precondition '{}' has condition_type belief_equals \
                                 but expression lacks a 'field' string",
                                precondition.description
                            ),
                        })?;

                    let expected = precondition
                        .expression
                        .get("value")
                        .ok_or_else(|| SomaError::SkillValidation {
                            skill_id: skill.skill_id.clone(),
                            reason: format!(
                                "precondition '{}' has condition_type belief_equals \
                                 but expression lacks a 'value'",
                                precondition.description
                            ),
                        })?;

                    let actual = belief
                        .as_object()
                        .and_then(|obj| obj.get(field));

                    if actual != Some(expected) {
                        warn!(
                            skill_id = %skill.skill_id,
                            field = %field,
                            "precondition failed: belief field does not match expected value"
                        );
                        return Err(SomaError::SkillExecution {
                            skill_id: skill.skill_id.clone(),
                            failure_class:
                                crate::types::common::SkillFailureClass::PreconditionFailure,
                            details: format!(
                                "belief_equals precondition failed: field '{}' does not match expected value ({})",
                                field, precondition.description
                            ),
                        });
                    }
                }
                other => {
                    // Unknown condition types are logged and deferred to the
                    // policy or resource subsystems. If no other subsystem
                    // handles them, they are a latent failure — callers must
                    // ensure all custom condition types have an evaluator.
                    warn!(
                        skill_id = %skill.skill_id,
                        condition_type = %other,
                        "precondition type not enforced by skill runtime, deferring to policy/resource subsystem"
                    );
                }
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns true if `actual` is at most `max` on the risk ordering:
/// Negligible < Low < Medium < High < Critical
fn risk_within_budget(
    actual: &crate::types::common::RiskClass,
    max: &crate::types::common::RiskClass,
) -> bool {
    use crate::types::common::RiskClass;

    fn ordinal(r: &RiskClass) -> u8 {
        match r {
            RiskClass::Negligible => 0,
            RiskClass::Low => 1,
            RiskClass::Medium => 2,
            RiskClass::High => 3,
            RiskClass::Critical => 4,
        }
    }

    ordinal(actual) <= ordinal(max)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::common::*;
    use crate::types::skill::*;
    use serde_json::json;

    /// Build a minimal valid SkillSpec for testing.
    fn minimal_skill(id: &str) -> SkillSpec {
        SkillSpec {
            skill_id: id.to_string(),
            namespace: "test-pack".to_string(),
            pack: "test-pack".to_string(),
            kind: SkillKind::Primitive,
            name: "Test Skill".to_string(),
            description: "A test skill".to_string(),
            version: "0.1.0".to_string(),
            inputs: SchemaRef {
                schema: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    },
                    "required": ["path"]
                }),
            },
            outputs: SchemaRef {
                schema: json!({ "type": "object", "properties": {} }),
            },
            required_resources: vec![],
            preconditions: vec![],
            expected_effects: vec![],
            observables: vec![ObservableDecl {
                field: "result".to_string(),
                role: ObservableRole::General,
            }],
            termination_conditions: vec![
                TerminationCondition {
                    condition_type: TerminationType::Success,
                    expression: json!({}),
                    description: "completed".to_string(),
                },
                TerminationCondition {
                    condition_type: TerminationType::Failure,
                    expression: json!({}),
                    description: "failed".to_string(),
                },
            ],
            rollback_or_compensation: RollbackSpec {
                support: RollbackSupport::FullReversal,
                compensation_skill: None,
                description: "reverse the action".to_string(),
            },
            cost_prior: CostPrior {
                latency: LatencyProfile {
                    expected_latency_ms: 10,
                    p95_latency_ms: 50,
                    max_latency_ms: 200,
                },
                resource_cost: CostProfile {
                    cpu_cost_class: CostClass::Low,
                    memory_cost_class: CostClass::Low,
                    io_cost_class: CostClass::Low,
                    network_cost_class: CostClass::Negligible,
                    energy_cost_class: CostClass::Negligible,
                },
            },
            risk_class: RiskClass::Low,
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
            tags: vec!["test".to_string()],
            aliases: vec![],
            capability_requirements: vec![],
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

    #[test]
    fn register_and_get() {
        let mut rt = DefaultSkillRuntime::new();
        let spec = minimal_skill("s1");
        rt.register_skill(spec).unwrap();
        assert!(rt.get_skill("s1").is_some());
        assert!(rt.get_skill("s2").is_none());
    }

    #[test]
    fn reject_duplicate_registration() {
        let mut rt = DefaultSkillRuntime::new();
        rt.register_skill(minimal_skill("dup")).unwrap();
        let err = rt.register_skill(minimal_skill("dup"));
        assert!(err.is_err());
    }

    #[test]
    fn reject_empty_skill_id() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("");
        spec.skill_id = String::new();
        assert!(rt.register_skill(spec).is_err());
    }

    #[test]
    fn reject_empty_name() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("x");
        spec.name = String::new();
        assert!(rt.register_skill(spec).is_err());
    }

    #[test]
    fn reject_empty_version() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("x");
        spec.version = String::new();
        assert!(rt.register_skill(spec).is_err());
    }

    #[test]
    fn reject_destructive_irreversible_without_description() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("d1");
        spec.expected_effects = vec![EffectDescriptor {
            effect_type: EffectType::Deletion,
            target_resource: None,
            description: "deletes data".to_string(),
            patch: None,
        }];
        spec.rollback_or_compensation = RollbackSpec {
            support: RollbackSupport::Irreversible,
            compensation_skill: None,
            description: String::new(), // empty => rejected
        };
        assert!(rt.register_skill(spec).is_err());
    }

    #[test]
    fn accept_destructive_irreversible_with_description() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("d2");
        spec.expected_effects = vec![EffectDescriptor {
            effect_type: EffectType::Deletion,
            target_resource: None,
            description: "deletes data".to_string(),
            patch: None,
        }];
        spec.rollback_or_compensation = RollbackSpec {
            support: RollbackSupport::Irreversible,
            compensation_skill: None,
            description: "acknowledged: data deletion is permanent".to_string(),
        };
        assert!(rt.register_skill(spec).is_ok());
    }

    #[test]
    fn reject_composite_without_subskills() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("c1");
        spec.kind = SkillKind::Composite;
        spec.subskills = vec![]; // empty => rejected
        assert!(rt.register_skill(spec).is_err());
    }

    #[test]
    fn accept_composite_with_branch_and_stop_conditions() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("c2");
        spec.kind = SkillKind::Composite;
        spec.subskills = vec![SubskillRef {
            skill_id: "s1".to_string(),
            ordering: SubskillOrdering::Sequential,
            required: true,
            branch_condition: Some(json!({"when": "resource_available"})),
            stop_condition: Some(json!({"on": "observation.success"})),
        }];
        assert!(rt.register_skill(spec).is_ok());
    }

    #[test]
    fn reject_composite_subskill_without_branch_condition() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("c3");
        spec.kind = SkillKind::Composite;
        spec.subskills = vec![SubskillRef {
            skill_id: "s1".to_string(),
            ordering: SubskillOrdering::Sequential,
            required: true,
            branch_condition: None,
            stop_condition: Some(json!({"on": "observation.success"})),
        }];
        assert!(rt.register_skill(spec).is_err());
    }

    #[test]
    fn reject_composite_subskill_without_stop_condition() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("c4");
        spec.kind = SkillKind::Composite;
        spec.subskills = vec![SubskillRef {
            skill_id: "s1".to_string(),
            ordering: SubskillOrdering::Sequential,
            required: true,
            branch_condition: Some(json!({"when": "resource_available"})),
            stop_condition: None,
        }];
        assert!(rt.register_skill(spec).is_err());
    }

    #[test]
    fn reject_routine_without_match_conditions() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("r1");
        spec.kind = SkillKind::Routine;
        spec.match_conditions = vec![];
        spec.confidence_threshold = Some(0.9);
        assert!(rt.register_skill(spec).is_err());
    }

    #[test]
    fn reject_routine_without_confidence_threshold() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("r2");
        spec.kind = SkillKind::Routine;
        spec.match_conditions = vec![Precondition {
            condition_type: "belief_contains".to_string(),
            expression: json!({ "field": "x" }),
            description: "x present".to_string(),
        }];
        spec.confidence_threshold = None;
        assert!(rt.register_skill(spec).is_err());
    }

    #[test]
    fn reject_delegated_without_remote_endpoint() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("del1");
        spec.kind = SkillKind::Delegated;
        spec.remote_endpoint = None;
        assert!(rt.register_skill(spec).is_err());
    }

    #[test]
    fn reject_missing_success_termination() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("t1");
        spec.termination_conditions = vec![TerminationCondition {
            condition_type: TerminationType::Failure,
            expression: json!({}),
            description: "failed".to_string(),
        }];
        assert!(rt.register_skill(spec).is_err());
    }

    #[test]
    fn reject_missing_failure_termination() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("t2");
        spec.termination_conditions = vec![TerminationCondition {
            condition_type: TerminationType::Success,
            expression: json!({}),
            description: "done".to_string(),
        }];
        assert!(rt.register_skill(spec).is_err());
    }

    #[test]
    fn reject_unbounded_skill_no_termination_no_budget() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("unbound1");
        spec.termination_conditions = vec![];
        spec.cost_prior.latency.max_latency_ms = 0;
        assert!(rt.register_skill(spec).is_err());
    }

    #[test]
    fn accept_skill_with_empty_termination_but_time_bound() {
        let _rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("bound1");
        spec.termination_conditions = vec![];
        spec.cost_prior.latency.max_latency_ms = 5000;
        // Passes boundedness check but will fail validate_termination
        // (no Success condition), so test boundedness directly.
        let result = DefaultSkillRuntime::validate_boundedness(&spec);
        assert!(result.is_ok());
    }

    #[test]
    fn accept_skill_with_termination_conditions() {
        let _rt = DefaultSkillRuntime::new();
        let spec = minimal_skill("bound2");
        // minimal_skill has Success + Failure termination conditions
        let result = DefaultSkillRuntime::validate_boundedness(&spec);
        assert!(result.is_ok());
    }

    #[test]
    fn reject_precondition_with_empty_condition_type() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("pre1");
        spec.preconditions = vec![Precondition {
            condition_type: String::new(),
            expression: json!({ "field": "x" }),
            description: "bad precondition".to_string(),
        }];
        assert!(rt.register_skill(spec).is_err());
    }

    #[test]
    fn reject_precondition_with_null_expression() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("pre2");
        spec.preconditions = vec![Precondition {
            condition_type: "belief_contains".to_string(),
            expression: serde_json::Value::Null,
            description: "bad precondition".to_string(),
        }];
        assert!(rt.register_skill(spec).is_err());
    }

    #[test]
    fn accept_valid_preconditions() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("pre3");
        spec.preconditions = vec![Precondition {
            condition_type: "belief_contains".to_string(),
            expression: json!({ "field": "user_id" }),
            description: "user must be known".to_string(),
        }];
        assert!(rt.register_skill(spec).is_ok());
    }

    #[test]
    fn list_skills_all() {
        let mut rt = DefaultSkillRuntime::new();
        rt.register_skill(minimal_skill("a")).unwrap();
        rt.register_skill(minimal_skill("b")).unwrap();
        assert_eq!(rt.list_skills(None).len(), 2);
    }

    #[test]
    fn list_skills_by_namespace() {
        let mut rt = DefaultSkillRuntime::new();
        rt.register_skill(minimal_skill("a")).unwrap();
        let mut b = minimal_skill("b");
        b.pack = "other-pack".to_string();
        rt.register_skill(b).unwrap();
        assert_eq!(rt.list_skills(Some("test-pack")).len(), 1);
        assert_eq!(rt.list_skills(Some("other-pack")).len(), 1);
        assert_eq!(rt.list_skills(Some("nope")).len(), 0);
    }

    #[test]
    fn enumerate_by_kind() {
        let mut rt = DefaultSkillRuntime::new();
        rt.register_skill(minimal_skill("p1")).unwrap();
        let mut c = minimal_skill("c1");
        c.kind = SkillKind::Composite;
        c.subskills = vec![SubskillRef {
            skill_id: "p1".to_string(),
            ordering: SubskillOrdering::Sequential,
            required: true,
            branch_condition: Some(json!({"when": "always"})),
            stop_condition: Some(json!({"on": "success"})),
        }];
        rt.register_skill(c).unwrap();

        let query = SkillQuery {
            kind: Some(SkillKind::Primitive),
            ..Default::default()
        };
        assert_eq!(rt.enumerate_candidates(&query).len(), 1);
    }

    #[test]
    fn enumerate_by_tags() {
        let mut rt = DefaultSkillRuntime::new();
        let mut a = minimal_skill("a");
        a.tags = vec!["fs".to_string()];
        rt.register_skill(a).unwrap();

        let mut b = minimal_skill("b");
        b.tags = vec!["net".to_string()];
        rt.register_skill(b).unwrap();

        let query = SkillQuery {
            tags: Some(vec!["fs".to_string()]),
            ..Default::default()
        };
        assert_eq!(rt.enumerate_candidates(&query).len(), 1);
    }

    #[test]
    fn enumerate_by_risk() {
        let mut rt = DefaultSkillRuntime::new();
        let mut low = minimal_skill("low");
        low.risk_class = RiskClass::Low;
        rt.register_skill(low).unwrap();

        let mut high = minimal_skill("high");
        high.risk_class = RiskClass::High;
        rt.register_skill(high).unwrap();

        let query = SkillQuery {
            max_risk: Some(RiskClass::Medium),
            ..Default::default()
        };
        let candidates = rt.enumerate_candidates(&query);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].skill_id, "low");
    }

    #[test]
    fn bind_inputs_success() {
        let rt = DefaultSkillRuntime::new();
        let spec = minimal_skill("s1");
        let ctx = json!({ "path": "/tmp/foo" });
        let bound = rt.bind_inputs(&spec, &ctx).unwrap();
        assert_eq!(bound.len(), 1);
        assert_eq!(bound[0].name, "path");
        assert_eq!(bound[0].value, json!("/tmp/foo"));
        // Flat context without sub-objects falls back to PackDefault.
        assert_eq!(bound[0].source, BindingSource::PackDefault);
    }

    #[test]
    fn bind_inputs_missing_required() {
        let rt = DefaultSkillRuntime::new();
        let spec = minimal_skill("s1");
        let ctx = json!({});
        let result = rt.bind_inputs(&spec, &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn bind_inputs_optional_ok() {
        let rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("s1");
        spec.inputs = SchemaRef {
            schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "mode": { "type": "string" }
                },
                "required": ["path"]
            }),
        };
        let ctx = json!({ "path": "/tmp" });
        let bound = rt.bind_inputs(&spec, &ctx).unwrap();
        assert_eq!(bound.len(), 1);
        assert_eq!(bound[0].name, "path");
        assert_eq!(bound[0].value, json!("/tmp"));
        assert!(!bound.iter().any(|b| b.name == "mode"));
    }

    #[test]
    fn bind_inputs_null_schema() {
        let rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("s1");
        spec.inputs = SchemaRef {
            schema: serde_json::Value::Null,
        };
        let ctx = json!({});
        let bound = rt.bind_inputs(&spec, &ctx).unwrap();
        assert!(bound.is_empty());
    }

    #[test]
    fn bind_inputs_provenance_goal_field() {
        let rt = DefaultSkillRuntime::new();
        let spec = minimal_skill("s1");
        let ctx = json!({
            "path": "/tmp/foo",
            "goal_fields": { "path": "/tmp/foo" }
        });
        let bound = rt.bind_inputs(&spec, &ctx).unwrap();
        assert_eq!(bound.len(), 1);
        assert_eq!(bound[0].source, BindingSource::GoalField);
    }

    #[test]
    fn bind_inputs_provenance_belief_resource() {
        let rt = DefaultSkillRuntime::new();
        let spec = minimal_skill("s1");
        let ctx = json!({
            "path": "/data/store",
            "belief_resources": { "path": "/data/store" }
        });
        let bound = rt.bind_inputs(&spec, &ctx).unwrap();
        assert_eq!(bound.len(), 1);
        assert_eq!(bound[0].source, BindingSource::BeliefResource);
    }

    #[test]
    fn bind_inputs_provenance_prior_observation() {
        let rt = DefaultSkillRuntime::new();
        let spec = minimal_skill("s1");
        let ctx = json!({
            "path": "/observed",
            "prior_observations": { "path": "/observed" }
        });
        let bound = rt.bind_inputs(&spec, &ctx).unwrap();
        assert_eq!(bound.len(), 1);
        assert_eq!(bound[0].source, BindingSource::PriorObservation);
    }

    #[test]
    fn bind_inputs_provenance_working_memory() {
        let rt = DefaultSkillRuntime::new();
        let spec = minimal_skill("s1");
        let ctx = json!({
            "path": "/cached",
            "working_memory": { "path": "/cached" }
        });
        let bound = rt.bind_inputs(&spec, &ctx).unwrap();
        assert_eq!(bound.len(), 1);
        assert_eq!(bound[0].source, BindingSource::WorkingMemory);
    }

    #[test]
    fn bind_inputs_provenance_priority_order() {
        let rt = DefaultSkillRuntime::new();
        let spec = minimal_skill("s1");
        // When a key appears in multiple sub-objects, goal_fields wins.
        let ctx = json!({
            "path": "/val",
            "goal_fields": { "path": "/val" },
            "belief_resources": { "path": "/val" },
            "working_memory": { "path": "/val" }
        });
        let bound = rt.bind_inputs(&spec, &ctx).unwrap();
        assert_eq!(bound[0].source, BindingSource::GoalField);
    }

    #[test]
    fn check_preconditions_belief_contains_pass() {
        let rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("s1");
        spec.preconditions = vec![Precondition {
            condition_type: "belief_contains".to_string(),
            expression: json!({ "field": "user_id" }),
            description: "user must be known".to_string(),
        }];
        let belief = json!({ "user_id": "abc" });
        assert!(rt.check_preconditions(&spec, &belief).is_ok());
    }

    #[test]
    fn check_preconditions_belief_contains_fail() {
        let rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("s1");
        spec.preconditions = vec![Precondition {
            condition_type: "belief_contains".to_string(),
            expression: json!({ "field": "user_id" }),
            description: "user must be known".to_string(),
        }];
        let belief = json!({});
        assert!(rt.check_preconditions(&spec, &belief).is_err());
    }

    #[test]
    fn check_preconditions_belief_equals_pass() {
        let rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("s1");
        spec.preconditions = vec![Precondition {
            condition_type: "belief_equals".to_string(),
            expression: json!({ "field": "status", "value": "active" }),
            description: "status must be active".to_string(),
        }];
        let belief = json!({ "status": "active" });
        assert!(rt.check_preconditions(&spec, &belief).is_ok());
    }

    #[test]
    fn check_preconditions_belief_equals_fail() {
        let rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("s1");
        spec.preconditions = vec![Precondition {
            condition_type: "belief_equals".to_string(),
            expression: json!({ "field": "status", "value": "active" }),
            description: "status must be active".to_string(),
        }];
        let belief = json!({ "status": "suspended" });
        assert!(rt.check_preconditions(&spec, &belief).is_err());
    }

    #[test]
    fn check_preconditions_unknown_type_passes() {
        let rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("s1");
        spec.preconditions = vec![Precondition {
            condition_type: "resource_available".to_string(),
            expression: json!({ "resource": "db" }),
            description: "database available".to_string(),
        }];
        let belief = json!({});
        // Unknown types are deferred, so this passes.
        assert!(rt.check_preconditions(&spec, &belief).is_ok());
    }

    #[test]
    fn reject_bad_input_schema_type() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("bad_schema");
        spec.inputs = SchemaRef {
            schema: json!({ "type": "array" }),
        };
        assert!(rt.register_skill(spec).is_err());
    }

    #[test]
    fn reject_output_schema_required_field_not_in_properties() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("bad_out_schema");
        spec.outputs = SchemaRef {
            schema: json!({
                "type": "object",
                "properties": {
                    "status": { "type": "string" }
                },
                "required": ["status", "missing_field"]
            }),
        };
        let err = rt.register_skill(spec);
        assert!(err.is_err());
        let msg = format!("{}", err.unwrap_err());
        assert!(msg.contains("missing_field"));
    }

    #[test]
    fn reject_input_schema_required_field_not_in_properties() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("bad_in_schema");
        spec.inputs = SchemaRef {
            schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path", "ghost"]
            }),
        };
        let err = rt.register_skill(spec);
        assert!(err.is_err());
        let msg = format!("{}", err.unwrap_err());
        assert!(msg.contains("ghost"));
    }

    #[test]
    fn accept_schema_with_consistent_required_and_properties() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("good_schema");
        spec.inputs = SchemaRef {
            schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "mode": { "type": "string" }
                },
                "required": ["path", "mode"]
            }),
        };
        spec.outputs = SchemaRef {
            schema: json!({
                "type": "object",
                "properties": {
                    "result": { "type": "string" }
                },
                "required": ["result"]
            }),
        };
        assert!(rt.register_skill(spec).is_ok());
    }

    #[test]
    fn accept_schema_with_no_required_array() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("no_req");
        spec.outputs = SchemaRef {
            schema: json!({
                "type": "object",
                "properties": {
                    "data": { "type": "string" }
                }
            }),
        };
        assert!(rt.register_skill(spec).is_ok());
    }

    #[test]
    fn reject_output_schema_required_with_no_properties() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("req_no_props");
        spec.outputs = SchemaRef {
            schema: json!({
                "type": "object",
                "required": ["phantom"]
            }),
        };
        let err = rt.register_skill(spec);
        assert!(err.is_err());
        let msg = format!("{}", err.unwrap_err());
        assert!(msg.contains("phantom"));
    }

    #[test]
    fn enumerate_by_name_contains() {
        let mut rt = DefaultSkillRuntime::new();
        let mut a = minimal_skill("a");
        a.name = "List Files".to_string();
        rt.register_skill(a).unwrap();

        let mut b = minimal_skill("b");
        b.name = "Delete Files".to_string();
        rt.register_skill(b).unwrap();

        let query = SkillQuery {
            name_contains: Some("list".to_string()),
            ..Default::default()
        };
        let candidates = rt.enumerate_candidates(&query);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].skill_id, "a");
    }

    #[test]
    fn enumerate_by_required_resources() {
        let mut rt = DefaultSkillRuntime::new();
        let mut a = minimal_skill("a");
        a.required_resources = vec!["postgres".to_string(), "redis".to_string()];
        rt.register_skill(a).unwrap();

        let mut b = minimal_skill("b");
        b.required_resources = vec!["postgres".to_string()];
        rt.register_skill(b).unwrap();

        let query = SkillQuery {
            required_resources: Some(vec!["postgres".to_string(), "redis".to_string()]),
            ..Default::default()
        };
        let candidates = rt.enumerate_candidates(&query);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].skill_id, "a");
    }

    // -----------------------------------------------------------------------
    // Section 5.3 — Optional fields: telemetry_fields, policy_overrides
    // -----------------------------------------------------------------------

    #[test]
    fn optional_fields_default_empty() {
        let spec = minimal_skill("opt1");
        assert!(spec.telemetry_fields.is_empty());
        assert!(spec.policy_overrides.is_empty());
        assert!(spec.invalidation_conditions.is_empty());
        assert!(spec.remote_trust_requirement.is_none());
        assert!(spec.remote_capability_contract.is_none());
        assert!(spec.fallback_skill.is_none());
    }

    // -----------------------------------------------------------------------
    // Section 11 — Routine: fallback_skill required
    // -----------------------------------------------------------------------

    #[test]
    fn reject_routine_without_fallback_skill() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("r3");
        spec.kind = SkillKind::Routine;
        spec.match_conditions = vec![Precondition {
            condition_type: "belief_contains".to_string(),
            expression: json!({ "field": "x" }),
            description: "x present".to_string(),
        }];
        spec.confidence_threshold = Some(0.9);
        spec.fallback_skill = None; // missing fallback => rejected
        assert!(rt.register_skill(spec).is_err());
    }

    #[test]
    fn accept_routine_with_all_required_fields() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("r4");
        spec.kind = SkillKind::Routine;
        spec.match_conditions = vec![Precondition {
            condition_type: "belief_contains".to_string(),
            expression: json!({ "field": "x" }),
            description: "x present".to_string(),
        }];
        spec.confidence_threshold = Some(0.9);
        spec.fallback_skill = Some("fallback-skill-id".to_string());
        assert!(rt.register_skill(spec).is_ok());
    }

    // -----------------------------------------------------------------------
    // Section 12 — Delegated: trust + capability contract required
    // -----------------------------------------------------------------------

    #[test]
    fn reject_delegated_without_trust_requirement() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("del2");
        spec.kind = SkillKind::Delegated;
        spec.remote_endpoint = Some("peer://other".to_string());
        spec.remote_trust_requirement = None;
        spec.remote_capability_contract = Some("contract-v1".to_string());
        assert!(rt.register_skill(spec).is_err());
    }

    #[test]
    fn reject_delegated_without_capability_contract() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("del3");
        spec.kind = SkillKind::Delegated;
        spec.remote_endpoint = Some("peer://other".to_string());
        spec.remote_trust_requirement = Some("verified".to_string());
        spec.remote_capability_contract = None;
        assert!(rt.register_skill(spec).is_err());
    }

    #[test]
    fn accept_delegated_with_all_fields() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("del4");
        spec.kind = SkillKind::Delegated;
        spec.remote_endpoint = Some("peer://other".to_string());
        spec.remote_trust_requirement = Some("verified".to_string());
        spec.remote_capability_contract = Some("contract-v1".to_string());
        assert!(rt.register_skill(spec).is_ok());
    }

    // -----------------------------------------------------------------------
    // Section 13.1 — Version validation
    // -----------------------------------------------------------------------

    #[test]
    fn reject_invalid_version_format() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("v1");
        spec.version = "abc".to_string();
        assert!(rt.register_skill(spec).is_err());
    }

    #[test]
    fn accept_semver_with_prerelease() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("v2");
        spec.version = "1.0.0-alpha".to_string();
        assert!(rt.register_skill(spec).is_ok());
    }

    #[test]
    fn accept_two_part_version() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("v3");
        spec.version = "1.0".to_string();
        assert!(rt.register_skill(spec).is_ok());
    }

    // -----------------------------------------------------------------------
    // Section 13.1 — Risk class validation for destructive skills
    // -----------------------------------------------------------------------

    #[test]
    fn reject_destructive_with_negligible_risk() {
        let mut rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("risk1");
        spec.expected_effects = vec![EffectDescriptor {
            effect_type: EffectType::Deletion,
            target_resource: None,
            description: "deletes data".to_string(),
            patch: None,
        }];
        spec.risk_class = RiskClass::Negligible;
        spec.rollback_or_compensation = RollbackSpec {
            support: RollbackSupport::CompensatingAction,
            compensation_skill: Some("undo-skill".to_string()),
            description: "compensating action".to_string(),
        };
        assert!(rt.register_skill(spec).is_err());
    }

    // -----------------------------------------------------------------------
    // Section 13.2 — Semantic validation
    // -----------------------------------------------------------------------

    #[test]
    fn semantic_warns_no_observables() {
        let rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("sem1");
        spec.observables = vec![];
        let warnings = rt.validate_semantic(&spec);
        assert!(warnings.iter().any(|w| w.code == SemanticWarningCode::ObservablesCannotConfirmSuccess));
    }

    #[test]
    fn semantic_warns_incomplete_termination() {
        let rt = DefaultSkillRuntime::new();
        let spec = minimal_skill("sem2");
        // minimal_skill only has Success + Failure, not all 7
        let warnings = rt.validate_semantic(&spec);
        assert!(warnings.iter().any(|w| w.code == SemanticWarningCode::IncompleteTermination));
    }

    #[test]
    fn semantic_warns_zero_cost_prior() {
        let rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("sem3");
        spec.cost_prior.latency.expected_latency_ms = 0;
        spec.cost_prior.latency.p95_latency_ms = 0;
        spec.cost_prior.latency.max_latency_ms = 0;
        let warnings = rt.validate_semantic(&spec);
        assert!(warnings.iter().any(|w| w.code == SemanticWarningCode::UndefinedCostPrior));
    }

    #[test]
    fn semantic_warns_underdeclared_resources() {
        let rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("sem4");
        spec.expected_effects = vec![EffectDescriptor {
            effect_type: EffectType::Update,
            target_resource: Some("postgres.users".to_string()),
            description: "updates user record".to_string(),
            patch: None,
        }];
        spec.required_resources = vec![];
        let warnings = rt.validate_semantic(&spec);
        assert!(warnings.iter().any(|w| w.code == SemanticWarningCode::UnderdeclaredResources));
    }

    #[test]
    fn semantic_no_warnings_for_good_spec() {
        let rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("sem5");
        spec.termination_conditions = vec![
            TerminationCondition { condition_type: TerminationType::Success, expression: json!({}), description: "ok".into() },
            TerminationCondition { condition_type: TerminationType::Failure, expression: json!({}), description: "fail".into() },
            TerminationCondition { condition_type: TerminationType::Timeout, expression: json!({}), description: "timeout".into() },
            TerminationCondition { condition_type: TerminationType::BudgetExhaustion, expression: json!({}), description: "budget".into() },
            TerminationCondition { condition_type: TerminationType::PolicyDenial, expression: json!({}), description: "policy".into() },
            TerminationCondition { condition_type: TerminationType::ExternalError, expression: json!({}), description: "ext".into() },
            TerminationCondition { condition_type: TerminationType::ExplicitAbort, expression: json!({}), description: "abort".into() },
        ];
        let warnings = rt.validate_semantic(&spec);
        // Should have no warnings about incomplete termination
        assert!(!warnings.iter().any(|w| w.code == SemanticWarningCode::IncompleteTermination));
    }

    // -----------------------------------------------------------------------
    // Section 13.3 — Runtime validation
    // -----------------------------------------------------------------------

    #[test]
    fn runtime_validate_missing_resource() {
        let rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("rt1");
        spec.required_resources = vec!["postgres".to_string()];
        let ctx = RuntimeValidationContext {
            available_resources: vec![],
            permission_scope: vec![],
            policy_allows: true,
            resource_versions: HashMap::new(),
            budget_remaining: Some(100.0),
            remote_trust_state: HashMap::new(),
        };
        assert!(rt.validate_runtime(&spec, &ctx).is_err());
    }

    #[test]
    fn runtime_validate_missing_permission() {
        let rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("rt2");
        spec.required_resources = vec!["postgres".to_string()];
        let ctx = RuntimeValidationContext {
            available_resources: vec!["postgres".to_string()],
            permission_scope: vec![], // no permission
            policy_allows: true,
            resource_versions: HashMap::new(),
            budget_remaining: Some(100.0),
            remote_trust_state: HashMap::new(),
        };
        assert!(rt.validate_runtime(&spec, &ctx).is_err());
    }

    #[test]
    fn runtime_validate_policy_denied() {
        let rt = DefaultSkillRuntime::new();
        let spec = minimal_skill("rt3");
        let ctx = RuntimeValidationContext {
            available_resources: vec![],
            permission_scope: vec![],
            policy_allows: false,
            resource_versions: HashMap::new(),
            budget_remaining: Some(100.0),
            remote_trust_state: HashMap::new(),
        };
        assert!(rt.validate_runtime(&spec, &ctx).is_err());
    }

    #[test]
    fn runtime_validate_budget_exhausted() {
        let rt = DefaultSkillRuntime::new();
        let spec = minimal_skill("rt4");
        let ctx = RuntimeValidationContext {
            available_resources: vec![],
            permission_scope: vec![],
            policy_allows: true,
            resource_versions: HashMap::new(),
            budget_remaining: Some(0.0),
            remote_trust_state: HashMap::new(),
        };
        assert!(rt.validate_runtime(&spec, &ctx).is_err());
    }

    #[test]
    fn runtime_validate_delegated_untrusted_peer() {
        let rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("rt5");
        spec.kind = SkillKind::Delegated;
        spec.remote_endpoint = Some("peer://remote-soma".to_string());
        spec.remote_trust_requirement = Some("verified".to_string());
        spec.remote_capability_contract = Some("contract-v1".to_string());
        let ctx = RuntimeValidationContext {
            available_resources: vec![],
            permission_scope: vec![],
            policy_allows: true,
            resource_versions: HashMap::new(),
            budget_remaining: Some(100.0),
            remote_trust_state: HashMap::new(), // peer not in trust state
        };
        assert!(rt.validate_runtime(&spec, &ctx).is_err());
    }

    #[test]
    fn runtime_validate_delegated_trusted_peer() {
        let rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("rt6");
        spec.kind = SkillKind::Delegated;
        spec.remote_endpoint = Some("peer://remote-soma".to_string());
        spec.remote_trust_requirement = Some("verified".to_string());
        spec.remote_capability_contract = Some("contract-v1".to_string());
        let mut trust = HashMap::new();
        trust.insert("peer://remote-soma".to_string(), true);
        let ctx = RuntimeValidationContext {
            available_resources: vec![],
            permission_scope: vec![],
            policy_allows: true,
            resource_versions: HashMap::new(),
            budget_remaining: Some(100.0),
            remote_trust_state: trust,
        };
        assert!(rt.validate_runtime(&spec, &ctx).is_ok());
    }

    #[test]
    fn runtime_validate_all_pass() {
        let rt = DefaultSkillRuntime::new();
        let mut spec = minimal_skill("rt7");
        spec.required_resources = vec!["postgres".to_string()];
        let ctx = RuntimeValidationContext {
            available_resources: vec!["postgres".to_string()],
            permission_scope: vec!["postgres".to_string()],
            policy_allows: true,
            resource_versions: HashMap::new(),
            budget_remaining: Some(100.0),
            remote_trust_state: HashMap::new(),
        };
        assert!(rt.validate_runtime(&spec, &ctx).is_ok());
    }

    // -----------------------------------------------------------------------
    // Section 10 — Composite SubskillRef has branch/stop conditions
    // -----------------------------------------------------------------------

    #[test]
    fn subskill_ref_has_branch_and_stop_conditions() {
        let subskill = SubskillRef {
            skill_id: "sub1".to_string(),
            ordering: SubskillOrdering::Conditional,
            required: true,
            branch_condition: Some(json!({"when": "resource_available"})),
            stop_condition: Some(json!({"on": "observation.success"})),
        };
        assert!(subskill.branch_condition.is_some());
        assert!(subskill.stop_condition.is_some());
    }

    // -----------------------------------------------------------------------
    // Section 17.3 — PartialSuccessDetail
    // -----------------------------------------------------------------------

    #[test]
    fn partial_success_detail_construction() {
        use crate::types::skill::PartialSuccessDetail;
        let detail = PartialSuccessDetail {
            effects_occurred: vec!["user_created".to_string()],
            effects_missing: vec!["notification_sent".to_string()],
            compensation_possible: true,
            downstream_continuation: true,
        };
        assert_eq!(detail.effects_occurred.len(), 1);
        assert_eq!(detail.effects_missing.len(), 1);
        assert!(detail.compensation_possible);
        assert!(detail.downstream_continuation);
    }
}
