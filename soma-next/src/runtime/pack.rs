use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use semver::{Version, VersionReq};
use tracing::{info, warn};

use crate::errors::{Result, SomaError};
use crate::memory::routines::{InvalidationReason, RoutineStore};
use crate::runtime::port::{DefaultPortRuntime, PortRuntime};
use crate::types::pack::{PackLifecycleState, PackSpec};
use crate::types::policy::{PolicyRuleType, PolicySpec};
use crate::types::port::PortSpec;
use crate::types::resource::ResourceSpec;
use crate::types::routine::Routine;
use crate::types::schema::Schema;
use crate::types::common::{CapabilityScope, RollbackSupport};
use crate::types::skill::{SkillKind, SkillSpec};

// --- Validation outcome ---

/// The outcome of pack validation (pack-spec.md Section "Validation Outcomes").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationOutcome {
    Accepted,
    Rejected,
    Quarantined,
    Degraded,
}

/// Result of running all 11 validation stages on a PackSpec.
#[derive(Debug, Clone)]
pub struct PackValidationResult {
    pub outcome: ValidationOutcome,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

// --- Capability health tracking ---

/// Per-capability health information, tracking availability and failure metrics.
#[derive(Debug, Clone)]
pub struct CapabilityHealth {
    pub fqn: String,
    pub enabled: bool,
    pub failure_count: u64,
    pub last_latency_ms: u64,
}

/// Aggregate health report for a single pack, including observability metadata.
#[derive(Debug, Clone)]
pub struct PackHealthReport {
    pub pack_id: String,
    pub state: PackLifecycleState,
    pub capabilities: Vec<CapabilityHealth>,
    pub total_failures: u64,
    pub health_checks: Vec<String>,
    pub expected_failure_modes: Vec<String>,
    pub metric_names: Vec<String>,
    pub trace_categories: Vec<String>,
}

// --- Pack entry ---

/// A loaded pack with its spec, current lifecycle state, and metadata.
#[derive(Debug, Clone)]
pub struct PackEntry {
    pub spec: PackSpec,
    pub state: PackLifecycleState,
    pub loaded_at: DateTime<Utc>,
    pub capability_health: HashMap<String, CapabilityHealth>,
}

// --- Runtime registries ---

/// Registries holding all capabilities contributed by loaded packs.
/// Each entry is keyed by fully-qualified name (namespace.id).
#[derive(Debug, Default)]
struct Registries {
    skills: HashMap<String, SkillSpec>,
    resources: HashMap<String, ResourceSpec>,
    schemas: HashMap<String, Schema>,
    routines: HashMap<String, Routine>,
    policies: HashMap<String, PolicySpec>,
    ports: HashMap<String, PortSpec>,
}

// --- PackRuntime trait ---

/// The Pack Runtime: discovers, validates, loads, and manages pack lifecycles.
///
/// Supports all 11 lifecycle transitions from pack-spec.md:
/// discover, validate, stage (via load), activate, suspend, resume,
/// quarantine, unload, reload, upgrade, rollback.
pub trait PackRuntime: Send + Sync {
    /// Read and parse a pack manifest from a path.
    fn discover(&self, path: &str) -> Result<PackSpec>;

    /// Run all 11 validation stages against the current runtime state.
    fn validate(&self, spec: &PackSpec) -> Result<PackValidationResult>;

    /// Load a validated pack into the runtime (stage). Returns the pack id.
    fn load(&mut self, spec: PackSpec) -> Result<String>;

    /// Transition a loaded pack to Active state.
    fn activate(&mut self, pack_id: &str) -> Result<()>;

    /// Suspend an active or degraded pack.
    fn suspend(&mut self, pack_id: &str) -> Result<()>;

    /// Resume a suspended pack back to Active.
    fn resume(&mut self, pack_id: &str) -> Result<()>;

    /// Quarantine a pack (isolation due to failure or policy violation).
    fn quarantine(&mut self, pack_id: &str) -> Result<()>;

    /// Unload a pack entirely, removing all its registrations.
    fn unload(&mut self, pack_id: &str) -> Result<()>;

    /// Reload a pack by unloading the current version and loading the replacement.
    /// A pack SHOULD be able to be revalidated on reload (pack-spec.md).
    fn reload(&mut self, pack_id: &str, new_spec: PackSpec) -> Result<()>;

    /// Upgrade a pack to a new version. Backward-compatible updates MAY be hot-loaded;
    /// breaking changes MUST require a version bump (pack-spec.md).
    fn upgrade(&mut self, pack_id: &str, new_spec: PackSpec) -> Result<()>;

    /// Rollback a pack to the previous version after a failed upgrade.
    /// Downgrade MUST be treated as a compatibility event (pack-spec.md).
    fn rollback(&mut self, pack_id: &str, old_spec: PackSpec) -> Result<()>;

    /// Get an immutable reference to a pack entry by id.
    fn get_pack(&self, pack_id: &str) -> Option<&PackEntry>;

    /// List all loaded pack entries.
    fn list_packs(&self) -> Vec<&PackEntry>;

    /// Get the current lifecycle state of a pack.
    fn get_lifecycle_state(&self, pack_id: &str) -> Option<PackLifecycleState>;

    /// Enumerate all registered port specs, optionally filtered by namespace.
    ///
    /// The runtime MUST be able to enumerate active ports and their capabilities
    /// (port-spec.md Pack Registration).
    fn list_ports(&self, namespace: Option<&str>) -> Vec<&PortSpec>;

    /// Get a specific registered port spec by fully-qualified port id.
    fn get_port_spec(&self, fqn: &str) -> Option<&PortSpec>;

    /// Get an aggregate health report for a pack, including per-capability health.
    fn get_pack_health(&self, pack_id: &str) -> Option<PackHealthReport>;

    /// Disable a single capability within a pack, keeping other capabilities active.
    fn disable_capability(&mut self, pack_id: &str, capability_fqn: &str) -> Result<()>;

    /// Re-enable a previously disabled capability within a pack.
    fn enable_capability(&mut self, pack_id: &str, capability_fqn: &str) -> Result<()>;

    /// Record the outcome of a capability invocation for health tracking.
    fn record_capability_outcome(&mut self, pack_id: &str, capability_fqn: &str, success: bool, latency_ms: u64);
}

// --- Current runtime version (for compatibility checks) ---

const RUNTIME_VERSION: &str = "0.1.0";

// --- DefaultPackRuntime ---

/// Default implementation of the Pack Runtime.
///
/// Maintains:
/// - A map of loaded packs by pack id.
/// - A set of registered namespaces for collision detection.
/// - Registries for skills, resources, schemas, routines, and policies.
/// - A scope map for each capability FQN to its declared CapabilityScope.
pub struct DefaultPackRuntime {
    packs: HashMap<String, PackEntry>,
    namespaces: HashSet<String>,
    registries: Registries,
    /// Maps each capability FQN to the CapabilityScope declared in its
    /// owning CapabilityGroup. Populated during `register_capabilities`
    /// and used for scope enforcement at dispatch time.
    capability_scopes: HashMap<String, CapabilityScope>,
    /// Number of failures a capability can accumulate before it is
    /// automatically disabled. When all capabilities of a pack are
    /// disabled the pack transitions to Quarantined.
    failure_threshold: u64,
}

/// Default failure threshold used when none is specified.
const DEFAULT_FAILURE_THRESHOLD: u64 = 10;

impl DefaultPackRuntime {
    pub fn new() -> Self {
        Self {
            packs: HashMap::new(),
            namespaces: HashSet::new(),
            registries: Registries::default(),
            capability_scopes: HashMap::new(),
            failure_threshold: DEFAULT_FAILURE_THRESHOLD,
        }
    }

    /// Create a runtime with a custom failure threshold for auto-quarantine.
    pub fn with_failure_threshold(failure_threshold: u64) -> Self {
        Self {
            failure_threshold,
            ..Self::new()
        }
    }

    fn namespace_has_active_pack(&self, namespace: &str) -> bool {
        self.packs.values().any(|entry| {
            entry.spec.namespace == namespace
                && matches!(
                    entry.state,
                    PackLifecycleState::Active | PackLifecycleState::Degraded
                )
        })
    }

    // --- Validation stages ---

    /// Stage 1: Manifest integrity — required fields must be present and non-empty.
    fn validate_manifest_integrity(spec: &PackSpec, errors: &mut Vec<String>) {
        if spec.id.trim().is_empty() {
            errors.push("manifest: id is empty".to_string());
        }
        if spec.name.trim().is_empty() {
            errors.push("manifest: name is empty".to_string());
        }
        if spec.namespace.trim().is_empty() {
            errors.push("manifest: namespace is empty".to_string());
        }
        // A loadable pack must provide at least one skill, resource, schema, or routine.
        if spec.skills.is_empty()
            && spec.resources.is_empty()
            && spec.schemas.is_empty()
            && spec.routines.is_empty()
        {
            errors.push(
                "manifest: pack must provide at least one skill, resource, schema, or routine"
                    .to_string(),
            );
        }
    }

    /// Stage 2: Namespace uniqueness — the pack's namespace must not collide with loaded packs.
    fn validate_namespace_uniqueness(
        &self,
        spec: &PackSpec,
        errors: &mut Vec<String>,
    ) {
        if self.namespaces.contains(&spec.namespace) {
            // Allow reload of the same pack id (same namespace owner).
            let same_pack = self
                .packs
                .get(&spec.id)
                .map(|e| e.spec.namespace == spec.namespace)
                .unwrap_or(false);
            if !same_pack {
                errors.push(format!(
                    "namespace collision: '{}' is already registered",
                    spec.namespace
                ));
            }
        }
    }

    /// Stage 3: Dependency availability — all required dependencies must be loaded.
    fn validate_dependency_availability(
        &self,
        spec: &PackSpec,
        errors: &mut Vec<String>,
        warnings: &mut Vec<String>,
    ) {
        for dep in &spec.dependencies {
            match self.packs.get(&dep.pack_id) {
                Some(entry) => {
                    // Verify the dependency is in a usable state.
                    match entry.state {
                        PackLifecycleState::Active | PackLifecycleState::Degraded => {}
                        _ => {
                            let msg = format!(
                                "dependency '{}' is loaded but in state {:?}, not usable",
                                dep.pack_id, entry.state
                            );
                            if dep.required {
                                errors.push(msg);
                            } else {
                                warnings.push(msg);
                            }
                        }
                    }
                }
                None => {
                    if dep.required {
                        errors.push(format!(
                            "required dependency '{}' is not loaded",
                            dep.pack_id
                        ));
                    } else {
                        warnings.push(format!(
                            "optional dependency '{}' is not loaded",
                            dep.pack_id
                        ));
                    }
                }
            }
        }
    }

    /// Stage 4: Version compatibility — pack's runtime_compatibility must match runtime version,
    /// and dependency versions must match.
    fn validate_version_compatibility(
        &self,
        spec: &PackSpec,
        errors: &mut Vec<String>,
    ) {
        // Check runtime compatibility.
        if let Ok(runtime_ver) = Version::parse(RUNTIME_VERSION)
            && !spec.runtime_compatibility.matches(&runtime_ver) {
                errors.push(format!(
                    "runtime version {} does not satisfy pack requirement '{}'",
                    RUNTIME_VERSION, spec.runtime_compatibility
                ));
            }

        // Check dependency version ranges.
        for dep in &spec.dependencies {
            if let Some(entry) = self.packs.get(&dep.pack_id) {
                if let Ok(req) = VersionReq::parse(&dep.version_range) {
                    if !req.matches(&entry.spec.version) {
                        errors.push(format!(
                            "dependency '{}' version {} does not satisfy requirement '{}'",
                            dep.pack_id, entry.spec.version, dep.version_range
                        ));
                    }
                } else {
                    errors.push(format!(
                        "invalid version_range '{}' for dependency '{}'",
                        dep.version_range, dep.pack_id
                    ));
                }
            }
            // Missing deps are caught in stage 3.
        }
    }

    /// Stage 5: Resource schemas — each resource must have a non-empty schema and valid namespace.
    fn validate_resource_schemas(
        spec: &PackSpec,
        errors: &mut Vec<String>,
        warnings: &mut Vec<String>,
    ) {
        for res in &spec.resources {
            if res.resource_id.trim().is_empty() {
                errors.push("resource: resource_id is empty".to_string());
            }
            if res.namespace.trim().is_empty() {
                errors.push(format!(
                    "resource '{}': namespace is empty",
                    res.resource_id
                ));
            } else if res.namespace != spec.namespace {
                warnings.push(format!(
                    "resource '{}': namespace '{}' differs from pack namespace '{}'",
                    res.resource_id, res.namespace, spec.namespace
                ));
            }
            if res.schema.is_null() {
                errors.push(format!(
                    "resource '{}': schema is null",
                    res.resource_id
                ));
            }
            if res.type_name.trim().is_empty() {
                errors.push(format!(
                    "resource '{}': type_name is empty",
                    res.resource_id
                ));
            }
        }
    }

    /// Stage 6: Skill schemas — each skill must have valid input/output schemas and all required
    /// fields from pack-spec.md Section "SkillSpec Requirements" (14 required fields).
    fn validate_skill_schemas(
        spec: &PackSpec,
        errors: &mut Vec<String>,
        warnings: &mut Vec<String>,
    ) {
        for skill in &spec.skills {
            let sid = &skill.skill_id;
            if sid.trim().is_empty() {
                errors.push("skill: skill_id is empty".to_string());
            }
            if skill.namespace.trim().is_empty() {
                errors.push(format!("skill '{}': namespace is empty", sid));
            } else if skill.namespace != spec.namespace {
                warnings.push(format!(
                    "skill '{}': namespace '{}' differs from pack namespace '{}'",
                    sid, skill.namespace, spec.namespace
                ));
            }
            if skill.name.trim().is_empty() {
                errors.push(format!("skill '{}': name is empty", sid));
            }
            if skill.inputs.schema.is_null() {
                errors.push(format!("skill '{}': input schema is null", sid));
            }
            if skill.outputs.schema.is_null() {
                errors.push(format!("skill '{}': output schema is null", sid));
            }
            // Composite skills MUST declare subskills and evaluation boundaries.
            if skill.kind == SkillKind::Composite && skill.subskills.is_empty() {
                errors.push(format!(
                    "skill '{}': composite skill must declare subskills",
                    sid
                ));
            }
            // Routine skills MUST declare guard conditions for shortcut safety.
            if skill.kind == SkillKind::Routine && skill.guard_conditions.is_empty() {
                errors.push(format!(
                    "skill '{}': routine skill must declare guard_conditions",
                    sid
                ));
            }
            // Delegated skills MUST declare remote execution constraints.
            if skill.kind == SkillKind::Delegated && skill.remote_endpoint.is_none() {
                errors.push(format!(
                    "skill '{}': delegated skill must declare remote_endpoint",
                    sid
                ));
            }
            // Every skill MUST be associated with at least one resource or port dependency.
            if skill.required_resources.is_empty() && skill.capability_requirements.is_empty() {
                errors.push(format!(
                    "skill '{}': must be associated with at least one resource or port dependency",
                    sid
                ));
            }
            if skill.observables.is_empty() {
                errors.push(format!(
                    "skill '{}': observables is required and must not be empty",
                    sid
                ));
            }
            if skill.termination_conditions.is_empty() {
                errors.push(format!(
                    "skill '{}': termination_conditions is required and must not be empty",
                    sid
                ));
            }
            if skill.preconditions.is_empty() {
                errors.push(format!(
                    "skill '{}': preconditions is required and must not be empty",
                    sid
                ));
            }
            if skill.expected_effects.is_empty() {
                errors.push(format!(
                    "skill '{}': expected_effects is required and must not be empty",
                    sid
                ));
            }
            // Compensating rollback must declare the compensation skill.
            if skill.rollback_or_compensation.support == RollbackSupport::CompensatingAction
                && skill.rollback_or_compensation.compensation_skill.is_none()
            {
                errors.push(format!(
                    "skill '{}': rollback support is CompensatingAction but no compensation_skill is declared",
                    sid
                ));
            }
        }
    }

    /// Stage 7: Schema schemas — each schema must have all required fields from
    /// pack-spec.md Section "SchemaSpec Requirements" (9 required fields).
    fn validate_schema_schemas(
        spec: &PackSpec,
        errors: &mut Vec<String>,
        warnings: &mut Vec<String>,
    ) {
        for schema in &spec.schemas {
            let sid = &schema.schema_id;
            if sid.trim().is_empty() {
                errors.push("schema: schema_id is empty".to_string());
            }
            if schema.namespace.trim().is_empty() {
                errors.push(format!("schema '{}': namespace is empty", sid));
            } else if schema.namespace != spec.namespace {
                warnings.push(format!(
                    "schema '{}': namespace '{}' differs from pack namespace '{}'",
                    sid, schema.namespace, spec.namespace
                ));
            }
            if schema.name.trim().is_empty() {
                errors.push(format!("schema '{}': name is empty", sid));
            }
            if schema.trigger_conditions.is_empty() {
                errors.push(format!(
                    "schema '{}': trigger_conditions is required and must not be empty",
                    sid
                ));
            }
            if schema.subgoal_structure.is_empty() {
                errors.push(format!(
                    "schema '{}': subgoal_structure is required and must not be empty",
                    sid
                ));
            }
            if schema.stop_conditions.is_empty() {
                errors.push(format!(
                    "schema '{}': stop_conditions is required and must not be empty",
                    sid
                ));
            }
            if schema.candidate_skill_ordering.is_empty() {
                errors.push(format!(
                    "schema '{}': candidate_skill_ordering is required and must not be empty",
                    sid
                ));
            }
            if schema.resource_requirements.is_empty() {
                errors.push(format!(
                    "schema '{}': resource_requirements is required and must not be empty",
                    sid
                ));
            }
            if schema.confidence < 0.0 || schema.confidence > 1.0 {
                errors.push(format!(
                    "schema '{}': confidence {} is out of range [0.0, 1.0]",
                    sid, schema.confidence
                ));
            }
        }
    }

    /// Stage 8: Routine schemas — each routine must have all required fields from
    /// pack-spec.md Section "RoutineSpec Requirements" (8 required fields).
    fn validate_routine_schemas(
        spec: &PackSpec,
        errors: &mut Vec<String>,
        warnings: &mut Vec<String>,
    ) {
        for routine in &spec.routines {
            let rid = &routine.routine_id;
            if rid.trim().is_empty() {
                errors.push("routine: routine_id is empty".to_string());
            }
            if routine.namespace.trim().is_empty() {
                errors.push(format!("routine '{}': namespace is empty", rid));
            } else if routine.namespace != spec.namespace {
                warnings.push(format!(
                    "routine '{}': namespace '{}' differs from pack namespace '{}'",
                    rid, routine.namespace, spec.namespace
                ));
            }
            if routine.match_conditions.is_empty() {
                errors.push(format!(
                    "routine '{}': match_conditions is required and must not be empty",
                    rid
                ));
            }
            if routine.compiled_skill_path.is_empty() && routine.compiled_steps.is_empty() {
                errors.push(format!(
                    "routine '{}': both compiled_skill_path and compiled_steps are empty",
                    rid
                ));
            }
            if routine.guard_conditions.is_empty() {
                errors.push(format!(
                    "routine '{}': guard_conditions is required (routine must only be used when guards pass)",
                    rid
                ));
            }
            if routine.expected_cost < 0.0 {
                errors.push(format!(
                    "routine '{}': expected_cost {} is negative",
                    rid, routine.expected_cost
                ));
            }
            if routine.confidence < 0.0 || routine.confidence > 1.0 {
                errors.push(format!(
                    "routine '{}': confidence {} is out of range [0.0, 1.0]",
                    rid, routine.confidence
                ));
            }
        }
    }

    /// Check whether a capability identifier matches sensitive patterns
    /// (destructive, credential, secret, device-actuation, policy-mutation)
    /// or is a wildcard that implicitly covers all of them.
    fn is_sensitive_capability(name: &str) -> bool {
        if name == "*" {
            return true;
        }
        let lower = name.to_lowercase();
        lower.contains("destructive")
            || lower.contains("destroy")
            || lower.contains("credential")
            || lower.contains("secret")
            || lower.contains("device-actuat")
            || lower.contains("device_actuat")
            || lower.contains("policy-mutat")
            || lower.contains("policy_mutat")
            || lower.contains("delete")
            || lower.contains("drop")
            || lower.contains("admin")
    }

    /// Stage 9: Policy constraints — each policy must have valid structure,
    /// and pack Allow rules targeting sensitive capabilities without
    /// scope_limits are rejected to prevent privilege widening.
    fn validate_policy_constraints(
        spec: &PackSpec,
        errors: &mut Vec<String>,
        warnings: &mut Vec<String>,
    ) {
        for policy in &spec.policies {
            let pid = &policy.policy_id;
            if pid.trim().is_empty() {
                errors.push("policy: policy_id is empty".to_string());
            }
            if policy.namespace.trim().is_empty() {
                errors.push(format!("policy '{}': namespace is empty", pid));
            }
            if policy.rules.is_empty()
                && policy.allowed_capabilities.is_empty()
                && policy.denied_capabilities.is_empty()
            {
                errors.push(format!(
                    "policy '{}': no rules, allowed_capabilities, or denied_capabilities defined",
                    pid
                ));
            }
            // Warn if scope_limits or trust_classification not set.
            if policy.scope_limits.is_none() {
                warnings.push(format!(
                    "policy '{}': no scope_limits declared",
                    pid
                ));
            }
            if policy.trust_classification.is_none() {
                warnings.push(format!(
                    "policy '{}': no trust_classification declared",
                    pid
                ));
            }

            // Detect pack Allow rules that target sensitive capabilities without
            // scope_limits. Without host rules available at validation time, we
            // reject the most dangerous patterns statically: a pack policy
            // granting Allow on destructive/credential/secret/device-actuation/
            // policy-mutation capabilities without scope_limits would almost
            // certainly conflict with any reasonable host deny.
            let has_scope_limits = policy.scope_limits.is_some();

            for rule in &policy.rules {
                if rule.rule_type != PolicyRuleType::Allow {
                    continue;
                }
                // Wildcard allow (empty identifiers) without scope_limits
                // attempts to allow everything.
                if rule.target.identifiers.is_empty() && !has_scope_limits {
                    errors.push(format!(
                        "policy '{}': rule '{}' is a wildcard Allow (no identifiers) without \
                         scope_limits — this would widen privilege for all capabilities",
                        pid, rule.rule_id,
                    ));
                    continue;
                }
                for ident in &rule.target.identifiers {
                    if Self::is_sensitive_capability(ident) && !has_scope_limits {
                        errors.push(format!(
                            "policy '{}': rule '{}' allows sensitive capability '{}' without \
                             scope_limits — pack policies must declare scope_limits when \
                             granting access to destructive, credential, secret, \
                             device-actuation, or policy-mutation capabilities",
                            pid, rule.rule_id, ident,
                        ));
                    }
                }
            }

            // Check allowed_capabilities list for sensitive patterns.
            if !has_scope_limits {
                for cap in &policy.allowed_capabilities {
                    if Self::is_sensitive_capability(cap) {
                        errors.push(format!(
                            "policy '{}': allowed_capabilities includes sensitive capability \
                             '{}' without scope_limits — pack policies must declare \
                             scope_limits when granting access to destructive, credential, \
                             secret, device-actuation, or policy-mutation capabilities",
                            pid, cap,
                        ));
                    }
                }
            }
        }
    }

    /// Stage 10: Exposure rules — exposed items must exist in the pack, remote exposure
    /// must satisfy 7 requirements and 3 constraints, default deny for destructive.
    fn validate_exposure_rules(
        spec: &PackSpec,
        errors: &mut Vec<String>,
        warnings: &mut Vec<String>,
    ) {
        let skill_ids: HashSet<&str> = spec.skills.iter().map(|s| s.skill_id.as_str()).collect();
        let resource_ids: HashSet<&str> = spec.resources.iter().map(|r| r.resource_id.as_str()).collect();

        // Local skill exposure: must exist.
        for exposed in &spec.exposure.local_skills {
            if !skill_ids.contains(exposed.as_str()) {
                errors.push(format!(
                    "exposure: local skill '{}' not found in pack skills",
                    exposed
                ));
            }
        }
        // Remote skill exposure: must exist, 7 requirements enforced.
        for entry in &spec.exposure.remote_skills {
            if !skill_ids.contains(entry.capability_id.as_str()) {
                errors.push(format!(
                    "exposure: remote skill '{}' not found in pack skills",
                    entry.capability_id
                ));
            }
            // Validate 7 remote exposure requirements.
            if entry.peer_trust_requirements.trim().is_empty() {
                errors.push(format!(
                    "exposure: remote skill '{}' missing peer_trust_requirements",
                    entry.capability_id
                ));
            }
            if entry.serialization_requirements.trim().is_empty() {
                errors.push(format!(
                    "exposure: remote skill '{}' missing serialization_requirements",
                    entry.capability_id
                ));
            }
            if entry.rate_limits.trim().is_empty() {
                warnings.push(format!(
                    "exposure: remote skill '{}' has no rate_limits declared",
                    entry.capability_id
                ));
            }
        }
        // Local resource exposure: must exist.
        for exposed in &spec.exposure.local_resources {
            if !resource_ids.contains(exposed.as_str()) {
                errors.push(format!(
                    "exposure: local resource '{}' not found in pack resources",
                    exposed
                ));
            }
        }
        // Remote resource exposure: must exist.
        for entry in &spec.exposure.remote_resources {
            if !resource_ids.contains(entry.capability_id.as_str()) {
                errors.push(format!(
                    "exposure: remote resource '{}' not found in pack resources",
                    entry.capability_id
                ));
            }
            if entry.peer_trust_requirements.trim().is_empty() {
                errors.push(format!(
                    "exposure: remote resource '{}' missing peer_trust_requirements",
                    entry.capability_id
                ));
            }
        }
        // Remote safety: default deny destructive must be true if not overridden.
        if !spec.exposure.default_deny_destructive {
            errors.push(
                "exposure: default_deny_destructive must be true; sensitive capabilities require explicit policy metadata"
                    .to_string(),
            );
        }
    }

    /// Stage 11: Observability metadata — all 9 fields from pack-spec.md must be present.
    fn validate_observability_metadata(
        spec: &PackSpec,
        errors: &mut Vec<String>,
    ) {
        if spec.observability.health_checks.is_empty() {
            errors.push("observability: no health_checks defined".to_string());
        }
        if spec.observability.version_metadata.is_null() {
            errors.push("observability: version_metadata is null".to_string());
        }
        if spec.observability.dependency_status.is_empty() && !spec.dependencies.is_empty() {
            errors.push("observability: dependency_status is empty but pack has dependencies".to_string());
        }
        if spec.observability.capability_inventory.is_empty() {
            errors.push("observability: no capability_inventory defined".to_string());
        }
        if spec.observability.expected_latency_classes.is_empty() {
            errors.push("observability: no expected_latency_classes defined".to_string());
        }
        if spec.observability.expected_failure_modes.is_empty() {
            errors.push("observability: no expected_failure_modes defined".to_string());
        }
        if spec.observability.trace_categories.is_empty() {
            errors.push("observability: no trace_categories defined".to_string());
        }
        if spec.observability.metric_names.is_empty() {
            errors.push("observability: no metric_names defined".to_string());
        }
        if spec.observability.pack_load_state.trim().is_empty() {
            errors.push("observability: pack_load_state is empty".to_string());
        }
    }

    /// Stage 12: Port spec validation — each PortSpec in the pack must be
    /// structurally valid per port-spec.md requirements.
    fn validate_port_specs(
        spec: &PackSpec,
        errors: &mut Vec<String>,
        _warnings: &mut Vec<String>,
    ) {
        // Use a standalone runtime to run validate_port without requiring
        // a mutable self reference.
        let probe = DefaultPortRuntime::new();
        for port in &spec.ports {
            if let Err(e) = probe.validate_port(port) {
                errors.push(format!("port '{}' failed spec validation: {}", port.port_id, e));
            }
        }
    }

    /// Stage 13: Port dependency version range checking — each declared
    /// port_dependency must have a matching registered port at the required version.
    fn validate_port_version_ranges(
        &self,
        spec: &PackSpec,
        errors: &mut Vec<String>,
        _warnings: &mut Vec<String>,
    ) {
        for dep in &spec.port_dependencies {
            // Ports are stored under their fully-qualified key "namespace.port_id".
            // Check FQN-scoped key first to prevent false matches across namespaces.
            // Fall back to bare port_id key only for globally-unscoped dependencies.
            let fqn_key = format!("{}.{}", spec.namespace, dep.port_id);
            let found = if let Some(port) = self.registries.ports.get(&fqn_key) {
                dep.version_range.matches(&port.version)
            } else if let Some(port) = self.registries.ports.get(&dep.port_id) {
                dep.version_range.matches(&port.version)
            } else {
                false
            };

            if !found {
                let msg = format!(
                    "port dependency '{}' version '{}' is not satisfied by any registered port",
                    dep.port_id, dep.version_range,
                );
                if dep.required {
                    errors.push(msg);
                } else {
                    // Non-required unsatisfied port dependency marks the port as
                    // unavailable for this dependency path but does not reject the pack.
                    // Record as warning — callers should check port availability before use.
                    _warnings.push(format!("optional port dependency: {}", msg));
                }
            }
        }
    }

    /// Determine the validation outcome from collected errors and warnings.
    fn determine_outcome(errors: &[String], warnings: &[String]) -> ValidationOutcome {
        if errors.is_empty() && warnings.is_empty() {
            ValidationOutcome::Accepted
        } else if errors.is_empty() {
            // Warnings only — accepted with notes, but if many warnings, degrade.
            if warnings.len() > 5 {
                ValidationOutcome::Degraded
            } else {
                ValidationOutcome::Accepted
            }
        } else {
            // Any errors present.
            // Namespace collision or dependency errors that are partial -> quarantine.
            let has_namespace_collision = errors.iter().any(|e| e.starts_with("namespace collision"));
            if has_namespace_collision {
                ValidationOutcome::Quarantined
            } else {
                ValidationOutcome::Rejected
            }
        }
    }

    /// Register all capabilities from a pack into the runtime registries.
    /// Also populates `capability_scopes` by mapping each capability FQN
    /// to the scope declared in its CapabilityGroup.
    fn register_capabilities(&mut self, spec: &PackSpec) -> HashMap<String, CapabilityHealth> {
        let ns = &spec.namespace;

        // Build a lookup from capability name to its group's scope so we
        // can tag each FQN with the right scope during registration.
        let mut cap_name_to_scope: HashMap<&str, CapabilityScope> = HashMap::new();
        for group in &spec.capabilities {
            for cap in &group.capabilities {
                cap_name_to_scope.insert(cap.as_str(), group.scope);
            }
        }

        for skill in &spec.skills {
            let fqn = format!("{}.{}", ns, skill.skill_id);
            if let Some(&scope) = cap_name_to_scope.get(skill.skill_id.as_str()) {
                self.capability_scopes.insert(fqn.clone(), scope);
            }
            self.registries.skills.insert(fqn, skill.clone());
        }
        for resource in &spec.resources {
            let fqn = format!("{}.{}", ns, resource.resource_id);
            if let Some(&scope) = cap_name_to_scope.get(resource.resource_id.as_str()) {
                self.capability_scopes.insert(fqn.clone(), scope);
            }
            self.registries.resources.insert(fqn, resource.clone());
        }
        for schema in &spec.schemas {
            let fqn = format!("{}.{}", ns, schema.schema_id);
            if let Some(&scope) = cap_name_to_scope.get(schema.schema_id.as_str()) {
                self.capability_scopes.insert(fqn.clone(), scope);
            }
            self.registries.schemas.insert(fqn, schema.clone());
        }
        for routine in &spec.routines {
            let fqn = format!("{}.{}", ns, routine.routine_id);
            if let Some(&scope) = cap_name_to_scope.get(routine.routine_id.as_str()) {
                self.capability_scopes.insert(fqn.clone(), scope);
            }
            self.registries.routines.insert(fqn, routine.clone());
        }
        for policy in &spec.policies {
            let fqn = format!("{}.{}", ns, policy.policy_id);
            if let Some(&scope) = cap_name_to_scope.get(policy.policy_id.as_str()) {
                self.capability_scopes.insert(fqn.clone(), scope);
            }
            self.registries.policies.insert(fqn, policy.clone());
        }
        for port in &spec.ports {
            let fqn = format!("{}.{}", ns, port.port_id);
            if let Some(&scope) = cap_name_to_scope.get(port.port_id.as_str()) {
                self.capability_scopes.insert(fqn.clone(), scope);
            }
            self.registries.ports.insert(fqn, port.clone());
        }

        // Build initial health entries for every registered capability.
        let all_fqns = Self::collect_capability_fqns(spec);
        all_fqns
            .into_iter()
            .map(|fqn| {
                let health = CapabilityHealth {
                    fqn: fqn.clone(),
                    enabled: true,
                    failure_count: 0,
                    last_latency_ms: 0,
                };
                (fqn, health)
            })
            .collect()
    }

    /// Unregister all capabilities for a given pack namespace.
    fn unregister_capabilities(&mut self, spec: &PackSpec) {
        let prefix = format!("{}.", spec.namespace);

        self.registries.skills.retain(|k, _| !k.starts_with(&prefix));
        self.registries.resources.retain(|k, _| !k.starts_with(&prefix));
        self.registries.schemas.retain(|k, _| !k.starts_with(&prefix));
        self.registries.routines.retain(|k, _| !k.starts_with(&prefix));
        self.registries.policies.retain(|k, _| !k.starts_with(&prefix));
        self.registries.ports.retain(|k, _| !k.starts_with(&prefix));
        self.capability_scopes.retain(|k, _| !k.starts_with(&prefix));
    }

    /// Collect the set of all capability FQNs that a pack contributes.
    fn collect_capability_fqns(spec: &PackSpec) -> HashSet<String> {
        let ns = &spec.namespace;
        let mut fqns = HashSet::new();
        for s in &spec.skills {
            fqns.insert(format!("{}.{}", ns, s.skill_id));
        }
        for r in &spec.resources {
            fqns.insert(format!("{}.{}", ns, r.resource_id));
        }
        for s in &spec.schemas {
            fqns.insert(format!("{}.{}", ns, s.schema_id));
        }
        for r in &spec.routines {
            fqns.insert(format!("{}.{}", ns, r.routine_id));
        }
        for p in &spec.policies {
            fqns.insert(format!("{}.{}", ns, p.policy_id));
        }
        for p in &spec.ports {
            fqns.insert(format!("{}.{}", ns, p.port_id));
        }
        fqns
    }

    /// Collect only skill FQNs from a pack spec.
    fn collect_skill_fqns(spec: &PackSpec) -> HashSet<String> {
        let ns = &spec.namespace;
        spec.skills
            .iter()
            .map(|s| format!("{}.{}", ns, s.skill_id))
            .collect()
    }

    /// Compute the set of skill FQNs that were present in `old_spec` but absent
    /// from `new_spec`. Used to drive routine invalidation after reload/upgrade.
    pub fn compute_removed_skill_fqns(old_spec: &PackSpec, new_spec: &PackSpec) -> HashSet<String> {
        let old_skills = Self::collect_skill_fqns(old_spec);
        let new_skills = Self::collect_skill_fqns(new_spec);
        old_skills.difference(&new_skills).cloned().collect()
    }

    /// After a reload/upgrade, warn about capabilities that were removed and are
    /// still referenced by skills, routines, or schemas in other packs.
    fn warn_orphaned_capability_refs(
        &self,
        removed_fqns: &HashSet<String>,
        reloaded_namespace: &str,
    ) {
        if removed_fqns.is_empty() {
            return;
        }

        let own_prefix = format!("{}.", reloaded_namespace);

        // Scan skills in other packs for references to removed capabilities.
        for (skill_fqn, skill) in &self.registries.skills {
            if skill_fqn.starts_with(&own_prefix) {
                continue;
            }
            for req in &skill.required_resources {
                if removed_fqns.contains(req) {
                    warn!(
                        skill = %skill_fqn,
                        removed_capability = %req,
                        "skill references a resource that was removed during pack reload"
                    );
                }
            }
            for cap in &skill.capability_requirements {
                if removed_fqns.contains(cap) {
                    warn!(
                        skill = %skill_fqn,
                        removed_capability = %cap,
                        "skill references a capability that was removed during pack reload"
                    );
                }
            }
            for sub in &skill.subskills {
                if removed_fqns.contains(&sub.skill_id) {
                    warn!(
                        skill = %skill_fqn,
                        removed_capability = %sub.skill_id,
                        "skill references a subskill that was removed during pack reload"
                    );
                }
            }
            if let Some(ref fb) = skill.fallback_skill
                && removed_fqns.contains(fb) {
                    warn!(
                        skill = %skill_fqn,
                        removed_capability = %fb,
                        "skill references a fallback skill that was removed during pack reload"
                    );
                }
        }

        // Scan routines in other packs for references to removed skills.
        for (routine_fqn, routine) in &self.registries.routines {
            if routine_fqn.starts_with(&own_prefix) {
                continue;
            }
            for step in &routine.compiled_skill_path {
                if removed_fqns.contains(step) {
                    warn!(
                        routine = %routine_fqn,
                        removed_capability = %step,
                        "routine references a skill that was removed during pack reload"
                    );
                }
            }
        }

        // Scan schemas in other packs for references to removed capabilities.
        for (schema_fqn, schema) in &self.registries.schemas {
            if schema_fqn.starts_with(&own_prefix) {
                continue;
            }
            for res in &schema.resource_requirements {
                if removed_fqns.contains(res) {
                    warn!(
                        schema = %schema_fqn,
                        removed_capability = %res,
                        "schema references a resource that was removed during pack reload"
                    );
                }
            }
            for candidate in &schema.candidate_skill_ordering {
                if removed_fqns.contains(candidate) {
                    warn!(
                        schema = %schema_fqn,
                        removed_capability = %candidate,
                        "schema references a skill that was removed during pack reload"
                    );
                }
            }
            for subgoal in &schema.subgoal_structure {
                for candidate in &subgoal.skill_candidates {
                    if removed_fqns.contains(candidate) {
                        warn!(
                            schema = %schema_fqn,
                            subgoal = %subgoal.subgoal_id,
                            removed_capability = %candidate,
                            "schema subgoal references a skill that was removed during pack reload"
                        );
                    }
                }
            }
        }
    }

    /// Enforce valid lifecycle state transitions.
    /// Returns an error if the transition is not allowed.
    fn check_transition(
        from: PackLifecycleState,
        to: PackLifecycleState,
        pack_id: &str,
    ) -> Result<()> {
        use PackLifecycleState::*;

        let allowed = match from {
            Discovered => matches!(to, Validated | Staged | Quarantined | Degraded | Failed),
            Validated => matches!(to, Staged | Quarantined | Failed),
            Staged => matches!(to, Active | Quarantined | Failed),
            Active => matches!(to, Suspended | Degraded | Quarantined | Unloaded),
            Degraded => matches!(to, Active | Suspended | Quarantined | Unloaded),
            Quarantined => matches!(to, Unloaded | Validated),
            Suspended => matches!(to, Active | Unloaded | Quarantined),
            Unloaded => false, // terminal
            Failed => matches!(to, Unloaded | Discovered),
        };

        if allowed {
            Ok(())
        } else {
            Err(SomaError::Pack(format!(
                "invalid state transition for pack '{}': {:?} -> {:?}",
                pack_id, from, to
            )))
        }
    }

    /// Check whether `calling_pack` is allowed to access `target_fqn`.
    ///
    /// Cross-pack access is allowed when:
    /// 1. The target capability belongs to the same pack (self-access), OR
    /// 2. The calling pack has declared the target's owning pack as a dependency.
    ///
    /// If neither condition holds, the call is rejected. This enforces the
    /// isolation boundary: packs can only reach capabilities they explicitly
    /// depend on.
    pub fn check_cross_pack_access(&self, calling_pack: &str, target_fqn: &str) -> Result<()> {
        // Extract the namespace portion of the target FQN ("ns.capability_id" -> "ns").
        let target_namespace = target_fqn
            .split_once('.')
            .map(|(ns, _)| ns)
            .unwrap_or(target_fqn);

        // Find the pack that owns the target namespace.
        let target_pack_id = self
            .packs
            .iter()
            .find(|(_, entry)| entry.spec.namespace == target_namespace)
            .map(|(id, _)| id.as_str());

        let target_pack_id = match target_pack_id {
            Some(id) => id,
            None => {
                return Err(SomaError::Pack(format!(
                    "cross-pack access denied: no pack owns namespace '{}' for capability '{}'",
                    target_namespace, target_fqn
                )));
            }
        };

        // Same pack: always allowed.
        if calling_pack == target_pack_id {
            return Ok(());
        }

        // Look up the calling pack's declared dependencies.
        let calling_entry = self.packs.get(calling_pack).ok_or_else(|| {
            SomaError::Pack(format!(
                "cross-pack access denied: calling pack '{}' is not loaded",
                calling_pack
            ))
        })?;

        let has_dependency = calling_entry
            .spec
            .dependencies
            .iter()
            .any(|dep| dep.pack_id == target_pack_id);

        if has_dependency {
            Ok(())
        } else {
            Err(SomaError::Pack(format!(
                "cross-pack access denied: pack '{}' has not declared '{}' as a dependency, \
                 cannot access capability '{}'",
                calling_pack, target_pack_id, target_fqn
            )))
        }
    }

    /// Compare capability scopes between two versions of a pack and return
    /// descriptions of any capabilities whose scope was broadened.
    ///
    /// Scope ordering from narrowest to broadest:
    /// Local < Session < Tenant < Device < Peer < Public
    ///
    /// A broadened scope is a potential security escalation — callers should
    /// treat these as upgrade warnings or errors depending on policy.
    pub fn detect_scope_widening(old: &PackSpec, new: &PackSpec) -> Vec<String> {
        fn scope_rank(s: CapabilityScope) -> u8 {
            match s {
                CapabilityScope::Local => 0,
                CapabilityScope::Session => 1,
                CapabilityScope::Tenant => 2,
                CapabilityScope::Device => 3,
                CapabilityScope::Peer => 4,
                CapabilityScope::Public => 5,
            }
        }

        // Build capability -> scope maps for both old and new.
        let mut old_scopes: HashMap<&str, CapabilityScope> = HashMap::new();
        for group in &old.capabilities {
            for cap in &group.capabilities {
                old_scopes.insert(cap.as_str(), group.scope);
            }
        }

        let mut new_scopes: HashMap<&str, CapabilityScope> = HashMap::new();
        for group in &new.capabilities {
            for cap in &group.capabilities {
                new_scopes.insert(cap.as_str(), group.scope);
            }
        }

        let mut widened = Vec::new();
        for (cap_name, &new_scope) in &new_scopes {
            if let Some(&old_scope) = old_scopes.get(cap_name)
                && scope_rank(new_scope) > scope_rank(old_scope) {
                    widened.push(format!(
                        "capability '{}' scope widened from {:?} to {:?}",
                        cap_name, old_scope, new_scope
                    ));
                }
        }

        widened
    }

    /// Look up the scope for a given capability FQN.
    pub fn get_capability_scope(&self, fqn: &str) -> Option<CapabilityScope> {
        self.capability_scopes.get(fqn).copied()
    }

    /// Check whether a capability can be invoked from the given scope context.
    ///
    /// A capability declared at scope X can only be invoked from a context
    /// whose scope is X or narrower. For example, a Local-scoped capability
    /// cannot be invoked from a Peer context because Peer is broader than
    /// Local. Conversely, a Public-scoped capability can be invoked from
    /// any context.
    ///
    /// Returns true if:
    ///   - The FQN is registered and its declared scope is at least as broad
    ///     as `invocation_scope`.
    ///   - The FQN is not registered (unknown capabilities are not blocked
    ///     by scope; other validation layers handle that).
    pub fn check_capability_scope(&self, fqn: &str, invocation_scope: CapabilityScope) -> bool {
        match self.capability_scopes.get(fqn) {
            Some(&declared_scope) => declared_scope.is_at_least(invocation_scope),
            None => true,
        }
    }

    /// Check if a capability FQN is disabled in any loaded pack.
    fn is_capability_disabled(&self, fqn: &str) -> bool {
        for entry in self.packs.values() {
            if let Some(ch) = entry.capability_health.get(fqn)
                && !ch.enabled {
                    return true;
                }
        }
        false
    }
}

impl Default for DefaultPackRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultPackRuntime {
    /// Reload a pack and invalidate routines whose compiled_skill_path references
    /// any skill that was removed between the old and new pack versions.
    ///
    /// This is the preferred entry point when a routine store is available.
    /// Returns the list of invalidated routine IDs on success.
    pub fn reload_and_invalidate_routines(
        &mut self,
        pack_id: &str,
        new_spec: PackSpec,
        routine_store: &mut dyn RoutineStore,
    ) -> Result<Vec<String>> {
        let old_spec = self
            .packs
            .get(pack_id)
            .ok_or_else(|| SomaError::Pack(format!("pack '{}' not found", pack_id)))?
            .spec
            .clone();

        self.reload(pack_id, new_spec.clone())?;

        let removed_skills = Self::compute_removed_skill_fqns(&old_spec, &new_spec);
        if removed_skills.is_empty() {
            return Ok(Vec::new());
        }

        let removed_vec: Vec<String> = removed_skills.into_iter().collect();
        let reason = InvalidationReason::PackVersionBreak {
            removed_skills: removed_vec,
        };
        let invalidated = routine_store.invalidate_by_condition(&reason);

        for id in &invalidated {
            warn!(
                routine_id = %id,
                pack_id = %pack_id,
                "routine invalidated due to pack version break"
            );
        }

        Ok(invalidated)
    }

    /// Upgrade a pack and invalidate routines whose compiled_skill_path references
    /// any skill that was removed in the new version.
    ///
    /// Returns the list of invalidated routine IDs on success.
    pub fn upgrade_and_invalidate_routines(
        &mut self,
        pack_id: &str,
        new_spec: PackSpec,
        routine_store: &mut dyn RoutineStore,
    ) -> Result<Vec<String>> {
        let old_spec = self
            .packs
            .get(pack_id)
            .ok_or_else(|| SomaError::Pack(format!("pack '{}' not found", pack_id)))?
            .spec
            .clone();

        self.upgrade(pack_id, new_spec.clone())?;

        let removed_skills = Self::compute_removed_skill_fqns(&old_spec, &new_spec);
        if removed_skills.is_empty() {
            return Ok(Vec::new());
        }

        let removed_vec: Vec<String> = removed_skills.into_iter().collect();
        let reason = InvalidationReason::PackVersionBreak {
            removed_skills: removed_vec,
        };
        let invalidated = routine_store.invalidate_by_condition(&reason);

        for id in &invalidated {
            warn!(
                routine_id = %id,
                pack_id = %pack_id,
                "routine invalidated due to pack upgrade removing skills"
            );
        }

        Ok(invalidated)
    }
}

impl PackRuntime for DefaultPackRuntime {
    fn discover(&self, path: &str) -> Result<PackSpec> {
        let data = std::fs::read_to_string(path).map_err(|e| {
            SomaError::Pack(format!("failed to read pack manifest at '{}': {}", path, e))
        })?;
        let spec: PackSpec = serde_json::from_str(&data).map_err(|e| {
            SomaError::Pack(format!(
                "failed to parse pack manifest at '{}': {}",
                path, e
            ))
        })?;
        Ok(spec)
    }

    fn validate(&self, spec: &PackSpec) -> Result<PackValidationResult> {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // Stage 1: manifest integrity
        Self::validate_manifest_integrity(spec, &mut errors);

        // Stage 2: namespace uniqueness
        self.validate_namespace_uniqueness(spec, &mut errors);

        // Stage 3: dependency availability
        self.validate_dependency_availability(spec, &mut errors, &mut warnings);

        // Stage 4: version compatibility
        self.validate_version_compatibility(spec, &mut errors);

        // Stage 5: resource schemas
        Self::validate_resource_schemas(spec, &mut errors, &mut warnings);

        // Stage 6: skill schemas
        Self::validate_skill_schemas(spec, &mut errors, &mut warnings);

        // Stage 7: schema schemas
        Self::validate_schema_schemas(spec, &mut errors, &mut warnings);

        // Stage 8: routine schemas
        Self::validate_routine_schemas(spec, &mut errors, &mut warnings);

        // Stage 9: policy constraints
        Self::validate_policy_constraints(spec, &mut errors, &mut warnings);

        // Stage 10: exposure rules
        Self::validate_exposure_rules(spec, &mut errors, &mut warnings);

        // Stage 11: observability metadata
        Self::validate_observability_metadata(spec, &mut errors);

        // Stage 12: port spec validation
        // Each PortSpec declared in the pack must pass PortRuntime::validate_port.
        Self::validate_port_specs(spec, &mut errors, &mut warnings);

        // Stage 13: port dependency version range checking
        // If a port_dependency declares a version range that no registered port
        // satisfies, the port is considered unavailable for this dependency path.
        self.validate_port_version_ranges(spec, &mut errors, &mut warnings);

        let outcome = Self::determine_outcome(&errors, &warnings);

        Ok(PackValidationResult {
            outcome,
            errors,
            warnings,
        })
    }

    fn load(&mut self, spec: PackSpec) -> Result<String> {
        let pack_id = spec.id.clone();
        let now = Utc::now();

        // Validate before inserting into the registry so validation sees the
        // true runtime state without self-interference.
        let result = self.validate(&spec)?;
        match result.outcome {
            ValidationOutcome::Rejected => {
                // Insert the pack in Failed state so it can be inspected
                // later via get_pack / get_lifecycle_state.
                self.packs.insert(
                    pack_id.clone(),
                    PackEntry {
                        spec,
                        state: PackLifecycleState::Failed,
                        loaded_at: now,
                        capability_health: HashMap::new(),
                    },
                );
                return Err(SomaError::PackValidation {
                    pack_id,
                    reason: format!(
                        "validation rejected: {}",
                        result.errors.join("; ")
                    ),
                });
            }
            ValidationOutcome::Quarantined => {
                // Insert in Quarantined state — do not register capabilities.
                self.namespaces.insert(spec.namespace.clone());
                self.packs.insert(
                    pack_id.clone(),
                    PackEntry {
                        spec,
                        state: PackLifecycleState::Quarantined,
                        loaded_at: now,
                        capability_health: HashMap::new(),
                    },
                );
                return Ok(pack_id);
            }
            ValidationOutcome::Accepted | ValidationOutcome::Degraded => {
                // Proceed with full registration.
            }
        }

        let initial_state = if result.outcome == ValidationOutcome::Degraded {
            PackLifecycleState::Degraded
        } else {
            PackLifecycleState::Staged
        };

        // Register namespace but defer capability registration until activate().
        self.namespaces.insert(spec.namespace.clone());

        // Build initial health entries without registering into the global registries.
        let all_fqns = Self::collect_capability_fqns(&spec);
        let capability_health: HashMap<String, CapabilityHealth> = all_fqns
            .into_iter()
            .map(|fqn| {
                let health = CapabilityHealth {
                    fqn: fqn.clone(),
                    enabled: true,
                    failure_count: 0,
                    last_latency_ms: 0,
                };
                (fqn, health)
            })
            .collect();

        // Insert pack entry.
        self.packs.insert(
            pack_id.clone(),
            PackEntry {
                spec,
                state: initial_state,
                loaded_at: now,
                capability_health,
            },
        );

        Ok(pack_id)
    }

    fn activate(&mut self, pack_id: &str) -> Result<()> {
        let entry = self.packs.get(pack_id).ok_or_else(|| {
            SomaError::Pack(format!("pack '{}' not found", pack_id))
        })?;
        Self::check_transition(entry.state, PackLifecycleState::Active, pack_id)?;

        // Register capabilities into the global registries on activation.
        let spec = entry.spec.clone();
        self.register_capabilities(&spec);

        let entry = self.packs.get_mut(pack_id).unwrap();
        entry.state = PackLifecycleState::Active;
        Ok(())
    }

    fn suspend(&mut self, pack_id: &str) -> Result<()> {
        let entry = self.packs.get(pack_id).ok_or_else(|| {
            SomaError::Pack(format!("pack '{}' not found", pack_id))
        })?;
        Self::check_transition(entry.state, PackLifecycleState::Suspended, pack_id)?;

        // Remove capabilities from global registries while suspended.
        let spec = entry.spec.clone();
        self.unregister_capabilities(&spec);

        let entry = self.packs.get_mut(pack_id).unwrap();
        entry.state = PackLifecycleState::Suspended;
        Ok(())
    }

    fn quarantine(&mut self, pack_id: &str) -> Result<()> {
        let entry = self.packs.get_mut(pack_id).ok_or_else(|| {
            SomaError::Pack(format!("pack '{}' not found", pack_id))
        })?;
        Self::check_transition(entry.state, PackLifecycleState::Quarantined, pack_id)?;
        entry.state = PackLifecycleState::Quarantined;
        Ok(())
    }

    fn unload(&mut self, pack_id: &str) -> Result<()> {
        let entry = self.packs.get(pack_id).ok_or_else(|| {
            SomaError::Pack(format!("pack '{}' not found", pack_id))
        })?;
        Self::check_transition(entry.state, PackLifecycleState::Unloaded, pack_id)?;

        // Remove capabilities and namespace.
        let spec = entry.spec.clone();
        self.unregister_capabilities(&spec);
        self.namespaces.remove(&spec.namespace);
        self.packs.remove(pack_id);

        Ok(())
    }

    fn resume(&mut self, pack_id: &str) -> Result<()> {
        let entry = self.packs.get(pack_id).ok_or_else(|| {
            SomaError::Pack(format!("pack '{}' not found", pack_id))
        })?;
        // Resume is only valid from Suspended state.
        if entry.state != PackLifecycleState::Suspended {
            return Err(SomaError::Pack(format!(
                "cannot resume pack '{}': current state is {:?}, expected Suspended",
                pack_id, entry.state
            )));
        }

        // Re-register capabilities into the global registries on resume.
        let spec = entry.spec.clone();
        self.register_capabilities(&spec);

        let entry = self.packs.get_mut(pack_id).unwrap();
        entry.state = PackLifecycleState::Active;
        Ok(())
    }

    fn reload(&mut self, pack_id: &str, new_spec: PackSpec) -> Result<()> {
        // The pack must exist and be in a reloadable state.
        let entry = self.packs.get(pack_id).ok_or_else(|| {
            SomaError::Pack(format!("pack '{}' not found", pack_id))
        })?;
        let old_spec = entry.spec.clone();
        let old_fqns = Self::collect_capability_fqns(&old_spec);
        let new_fqns = Self::collect_capability_fqns(&new_spec);

        // Detect scope widening before applying the reload. If any
        // capability has a broader scope in the new version, reject the
        // reload as a potential security escalation.
        let widened = Self::detect_scope_widening(&old_spec, &new_spec);
        if !widened.is_empty() {
            return Err(SomaError::Pack(format!(
                "reload rejected for pack '{}': scope widening detected: {}",
                pack_id,
                widened.join("; ")
            )));
        }

        // Unregister old capabilities.
        self.unregister_capabilities(&old_spec);
        self.namespaces.remove(&old_spec.namespace);
        self.packs.remove(pack_id);

        // Load the replacement. If validation fails, restore the old pack.
        match self.load(new_spec) {
            Ok(_) => {
                // Detect capabilities that existed in the old version but are
                // absent from the new one — other packs may still reference them.
                let removed: HashSet<String> =
                    old_fqns.difference(&new_fqns).cloned().collect();
                self.warn_orphaned_capability_refs(&removed, &old_spec.namespace);
                Ok(())
            }
            Err(e) => {
                // Rollback: re-register old pack in Failed state.
                self.namespaces.insert(old_spec.namespace.clone());
                self.packs.insert(
                    pack_id.to_string(),
                    PackEntry {
                        spec: old_spec,
                        state: PackLifecycleState::Failed,
                        loaded_at: Utc::now(),
                        capability_health: HashMap::new(),
                    },
                );
                Err(e)
            }
        }
    }

    fn upgrade(&mut self, pack_id: &str, new_spec: PackSpec) -> Result<()> {
        let entry = self.packs.get(pack_id).ok_or_else(|| {
            SomaError::Pack(format!("pack '{}' not found", pack_id))
        })?;

        // Breaking change requires version bump.
        if new_spec.version <= entry.spec.version {
            return Err(SomaError::Pack(format!(
                "upgrade for pack '{}' requires a version bump (current: {}, new: {})",
                pack_id, entry.spec.version, new_spec.version
            )));
        }

        // Detect scope widening before validation so we catch escalations
        // even if the new spec is otherwise structurally valid.
        let widened = Self::detect_scope_widening(&entry.spec, &new_spec);
        if !widened.is_empty() {
            return Err(SomaError::Pack(format!(
                "upgrade rejected for pack '{}': scope widening detected: {}",
                pack_id,
                widened.join("; ")
            )));
        }

        // Validate the new spec against current runtime state.
        let result = self.validate(&new_spec)?;
        if result.outcome == ValidationOutcome::Rejected {
            return Err(SomaError::PackValidation {
                pack_id: pack_id.to_string(),
                reason: format!("upgrade validation rejected: {}", result.errors.join("; ")),
            });
        }

        // Hot-load: unregister old, register new.
        self.reload(pack_id, new_spec)
    }

    fn rollback(&mut self, pack_id: &str, old_spec: PackSpec) -> Result<()> {
        // Downgrade MUST be treated as a compatibility event and validated like a fresh load.
        // Unload current version if present.
        if self.packs.contains_key(pack_id) {
            let entry = self.packs.get(pack_id).unwrap();
            let spec = entry.spec.clone();
            self.unregister_capabilities(&spec);
            self.namespaces.remove(&spec.namespace);
            self.packs.remove(pack_id);
        }

        // Load old spec as fresh (full validation).
        self.load(old_spec)?;
        Ok(())
    }

    fn get_pack(&self, pack_id: &str) -> Option<&PackEntry> {
        self.packs.get(pack_id)
    }

    fn list_packs(&self) -> Vec<&PackEntry> {
        self.packs.values().collect()
    }

    fn get_lifecycle_state(&self, pack_id: &str) -> Option<PackLifecycleState> {
        self.packs.get(pack_id).map(|e| e.state)
    }

    fn list_ports(&self, namespace: Option<&str>) -> Vec<&PortSpec> {
        self.registries
            .ports
            .iter()
            .filter(|(fqn, _)| {
                if self.is_capability_disabled(fqn) {
                    return false;
                }
                match fqn.split_once('.') {
                    Some((ns, _)) => {
                        self.namespace_has_active_pack(ns)
                            && match namespace {
                                Some(filter_ns) => ns == filter_ns,
                                None => true,
                            }
                    }
                    None => false,
                }
            })
            .map(|(_, spec)| spec)
            .collect()
    }

    fn get_port_spec(&self, fqn: &str) -> Option<&PortSpec> {
        if self.is_capability_disabled(fqn) {
            return None;
        }
        let (namespace, _) = fqn.split_once('.')?;
        if !self.namespace_has_active_pack(namespace) {
            return None;
        }
        self.registries.ports.get(fqn)
    }

    fn get_pack_health(&self, pack_id: &str) -> Option<PackHealthReport> {
        let entry = self.packs.get(pack_id)?;
        let capabilities: Vec<CapabilityHealth> =
            entry.capability_health.values().cloned().collect();
        let total_failures = capabilities.iter().map(|c| c.failure_count).sum();
        Some(PackHealthReport {
            pack_id: pack_id.to_string(),
            state: entry.state,
            capabilities,
            total_failures,
            health_checks: entry.spec.observability.health_checks.clone(),
            expected_failure_modes: entry.spec.observability.expected_failure_modes.clone(),
            metric_names: entry.spec.observability.metric_names.clone(),
            trace_categories: entry.spec.observability.trace_categories.clone(),
        })
    }

    fn disable_capability(&mut self, pack_id: &str, capability_fqn: &str) -> Result<()> {
        let entry = self.packs.get_mut(pack_id).ok_or_else(|| {
            SomaError::Pack(format!("pack '{}' not found", pack_id))
        })?;
        let ch = entry.capability_health.get_mut(capability_fqn).ok_or_else(|| {
            SomaError::Pack(format!(
                "capability '{}' not found in pack '{}'",
                capability_fqn, pack_id
            ))
        })?;
        ch.enabled = false;
        Ok(())
    }

    fn enable_capability(&mut self, pack_id: &str, capability_fqn: &str) -> Result<()> {
        let entry = self.packs.get_mut(pack_id).ok_or_else(|| {
            SomaError::Pack(format!("pack '{}' not found", pack_id))
        })?;
        let ch = entry.capability_health.get_mut(capability_fqn).ok_or_else(|| {
            SomaError::Pack(format!(
                "capability '{}' not found in pack '{}'",
                capability_fqn, pack_id
            ))
        })?;
        ch.enabled = true;
        Ok(())
    }

    fn record_capability_outcome(
        &mut self,
        pack_id: &str,
        capability_fqn: &str,
        success: bool,
        latency_ms: u64,
    ) {
        // Structured log for every outcome so external collectors can scrape it.
        info!(
            pack_id = %pack_id,
            capability = %capability_fqn,
            success = success,
            latency_ms = latency_ms,
            "capability_outcome"
        );

        let threshold = self.failure_threshold;

        if let Some(entry) = self.packs.get_mut(pack_id) {
            let health = entry
                .capability_health
                .entry(capability_fqn.to_string())
                .or_insert_with(|| CapabilityHealth {
                    fqn: capability_fqn.to_string(),
                    enabled: true,
                    failure_count: 0,
                    last_latency_ms: 0,
                });
            if !success {
                health.failure_count += 1;
            }
            health.last_latency_ms = latency_ms;

            // Auto-disable a capability once its failures exceed the threshold.
            if health.failure_count > threshold && health.enabled {
                health.enabled = false;
                warn!(
                    pack_id = %pack_id,
                    capability = %capability_fqn,
                    failure_count = health.failure_count,
                    threshold = threshold,
                    "capability auto-disabled: failure count exceeded threshold"
                );
            }

            // If every capability in this pack is now disabled, quarantine the pack.
            let all_disabled = !entry.capability_health.is_empty()
                && entry.capability_health.values().all(|ch| !ch.enabled);
            if all_disabled {
                // Only transition if the current state allows it.
                if Self::check_transition(entry.state, PackLifecycleState::Quarantined, pack_id)
                    .is_ok()
                {
                    entry.state = PackLifecycleState::Quarantined;
                    warn!(
                        pack_id = %pack_id,
                        "pack auto-quarantined: all capabilities disabled due to failures"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::common::*;
    use crate::types::pack::*;

    /// Build a minimal valid PackSpec for testing.
    fn make_valid_pack_spec(id: &str, namespace: &str) -> PackSpec {
        use crate::types::common::*;
        use crate::types::skill::*;

        PackSpec {
            id: id.to_string(),
            name: format!("{} pack", id),
            version: Version::new(1, 0, 0),
            runtime_compatibility: VersionReq::parse(">=0.1.0").unwrap(),
            namespace: namespace.to_string(),
            capabilities: vec![CapabilityGroup {
                group_name: "core".to_string(),
                scope: CapabilityScope::Local,
                capabilities: vec!["read".to_string()],
            }],
            dependencies: vec![],
            resources: vec![],
            skills: vec![SkillSpec {
                skill_id: "test_skill".to_string(),
                namespace: namespace.to_string(),
                pack: id.to_string(),
                kind: SkillKind::Primitive,
                name: "Test Skill".to_string(),
                description: "A test skill".to_string(),
                version: "1.0.0".to_string(),
                inputs: SchemaRef {
                    schema: serde_json::json!({"type": "object"}),
                },
                outputs: SchemaRef {
                    schema: serde_json::json!({"type": "object"}),
                },
                required_resources: vec![],
                preconditions: vec![Precondition {
                    condition_type: "resource_available".to_string(),
                    expression: serde_json::json!({"resource": "test"}),
                    description: "test resource must be available".to_string(),
                }],
                expected_effects: vec![EffectDescriptor {
                    effect_type: EffectType::Emission,
                    target_resource: None,
                    description: "emits result".to_string(),
                    patch: None,
                }],
                observables: vec![ObservableDecl {
                    field: "result".to_string(),
                    role: ObservableRole::ConfirmSuccess,
                }],
                termination_conditions: vec![TerminationCondition {
                    condition_type: TerminationType::Success,
                    expression: serde_json::json!({"result": "ok"}),
                    description: "succeeds when result is ok".to_string(),
                }],
                rollback_or_compensation: RollbackSpec {
                    support: RollbackSupport::Irreversible,
                    compensation_skill: None,
                    description: "none".to_string(),
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
                        io_cost_class: CostClass::Negligible,
                        network_cost_class: CostClass::Negligible,
                        energy_cost_class: CostClass::Negligible,
                    },
                },
                risk_class: RiskClass::Negligible,
                determinism: DeterminismClass::Deterministic,
                remote_exposure: RemoteExposureDecl {
                    remote_scope: CapabilityScope::Local,
                    peer_trust_requirements: "none".to_string(),
                    serialization_requirements: "json".to_string(),
                    rate_limits: "none".to_string(),
                    replay_protection: false,
                    observation_streaming: false,
                    delegation_support: false,
                    enabled: false,
                },
                tags: vec![],
                aliases: vec![],
                capability_requirements: vec!["read".to_string()],
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
            }],
            schemas: vec![],
            routines: vec![],
            policies: vec![],
            exposure: ExposureSpec {
                local_skills: vec!["test_skill".to_string()],
                remote_skills: vec![],
                local_resources: vec![],
                remote_resources: vec![],
                default_deny_destructive: true,
            },
            observability: ObservabilitySpec {
                health_checks: vec!["ping".to_string()],
                version_metadata: serde_json::json!({"version": "1.0.0"}),
                dependency_status: vec![],
                capability_inventory: vec!["test_skill".to_string()],
                expected_latency_classes: vec!["fast".to_string()],
                expected_failure_modes: vec!["timeout".to_string()],
                trace_categories: vec!["skill_execution".to_string()],
                metric_names: vec!["skill_latency_ms".to_string()],
                pack_load_state: "staged".to_string(),
            },
            description: Some("A test pack".to_string()),
            authors: vec!["test".to_string()],
            license: Some("MIT".to_string()),
            homepage: None,
            repository: None,
            targets: vec![],
            build: None,
            checksum: None,
            signature: None,
            entrypoints: vec![],
            tags: vec![],
            deprecation: None,
            ports: vec![],
            port_dependencies: vec![],
        }
    }

    // --- Validation tests ---

    #[test]
    fn validate_valid_pack_accepted() {
        let rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("test-pack", "test");
        let result = rt.validate(&spec).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Accepted);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn validate_empty_id_rejected() {
        let rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("", "test");
        spec.id = "".to_string();
        let result = rt.validate(&spec).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Rejected);
        assert!(result.errors.iter().any(|e| e.contains("id is empty")));
    }

    #[test]
    fn validate_empty_namespace_rejected() {
        let rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "");
        spec.namespace = "".to_string();
        let result = rt.validate(&spec).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Rejected);
        assert!(result.errors.iter().any(|e| e.contains("namespace is empty")));
    }

    #[test]
    fn validate_no_capabilities_rejected() {
        let rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "test");
        spec.skills.clear();
        spec.resources.clear();
        spec.schemas.clear();
        spec.routines.clear();
        spec.exposure.local_skills.clear();
        let result = rt.validate(&spec).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Rejected);
        assert!(result.errors.iter().any(|e| e.contains("at least one")));
    }

    #[test]
    fn validate_namespace_collision_quarantined() {
        let mut rt = DefaultPackRuntime::new();
        let spec1 = make_valid_pack_spec("pack-a", "shared_ns");
        rt.load(spec1).unwrap();
        rt.activate("pack-a").unwrap();

        let spec2 = make_valid_pack_spec("pack-b", "shared_ns");
        let result = rt.validate(&spec2).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Quarantined);
        assert!(result.errors.iter().any(|e| e.contains("namespace collision")));
    }

    #[test]
    fn validate_incompatible_runtime_rejected() {
        let rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "test");
        spec.runtime_compatibility = VersionReq::parse(">=99.0.0").unwrap();
        let result = rt.validate(&spec).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Rejected);
        assert!(result.errors.iter().any(|e| e.contains("runtime version")));
    }

    #[test]
    fn validate_missing_required_dependency_rejected() {
        let rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "test");
        spec.dependencies.push(DependencySpec {
            pack_id: "nonexistent".to_string(),
            version_range: ">=1.0.0".to_string(),
            required: true,
            capabilities_needed: vec![],
            feature_flags: vec![],
        });
        let result = rt.validate(&spec).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Rejected);
        assert!(result.errors.iter().any(|e| e.contains("nonexistent")));
    }

    #[test]
    fn validate_optional_dependency_produces_warning() {
        let rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "test");
        spec.dependencies.push(DependencySpec {
            pack_id: "optional_dep".to_string(),
            version_range: ">=1.0.0".to_string(),
            required: false,
            capabilities_needed: vec![],
            feature_flags: vec![],
        });
        spec.observability.dependency_status.push(DependencyStatusEntry {
            pack_id: "optional_dep".to_string(),
            status: "unknown".to_string(),
        });
        let result = rt.validate(&spec).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Accepted);
        assert!(result.warnings.iter().any(|w| w.contains("optional_dep")));
    }

    #[test]
    fn validate_exposure_nonexistent_skill_rejected() {
        let rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "test");
        spec.exposure.local_skills.push("ghost_skill".to_string());
        let result = rt.validate(&spec).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Rejected);
        assert!(result.errors.iter().any(|e| e.contains("ghost_skill")));
    }

    #[test]
    fn validate_skill_null_input_schema_rejected() {
        let rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "test");
        spec.skills[0].inputs.schema = serde_json::Value::Null;
        let result = rt.validate(&spec).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Rejected);
        assert!(result.errors.iter().any(|e| e.contains("input schema is null")));
    }

    #[test]
    fn validate_missing_observability_produces_errors() {
        let rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "test");
        spec.observability.health_checks.clear();
        spec.observability.metric_names.clear();
        spec.observability.trace_categories.clear();
        let result = rt.validate(&spec).unwrap();
        // Missing observability fields are errors — pack is rejected.
        assert_eq!(result.outcome, ValidationOutcome::Rejected);
        assert!(!result.errors.is_empty());
    }

    // --- Load tests ---

    #[test]
    fn load_valid_pack() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("pack-1", "ns1");
        let id = rt.load(spec).unwrap();
        assert_eq!(id, "pack-1");
        assert_eq!(
            rt.get_lifecycle_state("pack-1"),
            Some(PackLifecycleState::Staged)
        );
        assert!(rt.namespaces.contains("ns1"));
    }

    #[test]
    fn load_does_not_register_skills_before_activation() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("pack-1", "ns1");
        rt.load(spec).unwrap();
        // Capabilities are deferred until activate().
        assert!(!rt.registries.skills.contains_key("ns1.test_skill"));
    }

    #[test]
    fn activate_registers_skills() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("pack-1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("pack-1").unwrap();
        assert!(rt.registries.skills.contains_key("ns1.test_skill"));
    }

    #[test]
    fn load_rejected_pack_fails() {
        let mut rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("bad-pack", "");
        spec.namespace = "".to_string();
        let result = rt.load(spec);
        assert!(result.is_err());
        // Rejected packs are retained in Failed state for inspection.
        assert_eq!(
            rt.get_lifecycle_state("bad-pack"),
            Some(PackLifecycleState::Failed)
        );
    }

    #[test]
    fn load_quarantined_pack_no_capabilities() {
        let mut rt = DefaultPackRuntime::new();
        // Load first pack to claim namespace.
        let spec1 = make_valid_pack_spec("pack-a", "collision");
        rt.load(spec1).unwrap();
        rt.activate("pack-a").unwrap();

        // Second pack with same namespace gets quarantined.
        let spec2 = make_valid_pack_spec("pack-b", "collision");
        let id = rt.load(spec2).unwrap();
        assert_eq!(id, "pack-b");
        assert_eq!(
            rt.get_lifecycle_state("pack-b"),
            Some(PackLifecycleState::Quarantined)
        );
        // Quarantined pack should NOT overwrite pack-a's registration.
        // The skill still belongs to pack-a (the original owner).
        let skill = rt.registries.skills.get("collision.test_skill").unwrap();
        assert_eq!(skill.pack, "pack-a");
    }

    // --- Lifecycle transition tests ---

    #[test]
    fn activate_from_staged() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        assert!(rt.activate("p1").is_ok());
        assert_eq!(rt.get_lifecycle_state("p1"), Some(PackLifecycleState::Active));
    }

    #[test]
    fn suspend_from_active() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();
        assert!(rt.suspend("p1").is_ok());
        assert_eq!(
            rt.get_lifecycle_state("p1"),
            Some(PackLifecycleState::Suspended)
        );
    }

    #[test]
    fn quarantine_from_active() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();
        assert!(rt.quarantine("p1").is_ok());
        assert_eq!(
            rt.get_lifecycle_state("p1"),
            Some(PackLifecycleState::Quarantined)
        );
    }

    #[test]
    fn unload_from_active() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();
        assert!(rt.unload("p1").is_ok());
        assert!(rt.get_pack("p1").is_none());
        assert!(!rt.namespaces.contains("ns1"));
        assert!(!rt.registries.skills.contains_key("ns1.test_skill"));
    }

    #[test]
    fn invalid_transition_staged_to_suspended_fails() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        // Staged -> Suspended is not a valid transition.
        assert!(rt.suspend("p1").is_err());
    }

    #[test]
    fn invalid_transition_staged_to_unloaded_fails() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        // Staged -> Unloaded is not valid (must go through Active first).
        assert!(rt.unload("p1").is_err());
    }

    #[test]
    fn activate_nonexistent_pack_fails() {
        let mut rt = DefaultPackRuntime::new();
        assert!(rt.activate("ghost").is_err());
    }

    #[test]
    fn suspend_nonexistent_pack_fails() {
        let mut rt = DefaultPackRuntime::new();
        assert!(rt.suspend("ghost").is_err());
    }

    // --- Query tests ---

    #[test]
    fn list_packs_returns_all() {
        let mut rt = DefaultPackRuntime::new();
        let spec1 = make_valid_pack_spec("p1", "ns1");
        let spec2 = make_valid_pack_spec("p2", "ns2");
        rt.load(spec1).unwrap();
        rt.load(spec2).unwrap();
        assert_eq!(rt.list_packs().len(), 2);
    }

    #[test]
    fn get_pack_returns_correct_entry() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        let entry = rt.get_pack("p1").unwrap();
        assert_eq!(entry.spec.id, "p1");
        assert_eq!(entry.spec.namespace, "ns1");
    }

    #[test]
    fn get_pack_missing_returns_none() {
        let rt = DefaultPackRuntime::new();
        assert!(rt.get_pack("nonexistent").is_none());
    }

    #[test]
    fn get_lifecycle_state_missing_returns_none() {
        let rt = DefaultPackRuntime::new();
        assert!(rt.get_lifecycle_state("nonexistent").is_none());
    }

    // --- Full lifecycle test ---

    #[test]
    fn full_lifecycle_load_activate_suspend_resume_unload() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("lifecycle", "lc");

        // Load -> Staged
        rt.load(spec).unwrap();
        assert_eq!(
            rt.get_lifecycle_state("lifecycle"),
            Some(PackLifecycleState::Staged)
        );

        // Staged -> Active
        rt.activate("lifecycle").unwrap();
        assert_eq!(
            rt.get_lifecycle_state("lifecycle"),
            Some(PackLifecycleState::Active)
        );

        // Active -> Suspended
        rt.suspend("lifecycle").unwrap();
        assert_eq!(
            rt.get_lifecycle_state("lifecycle"),
            Some(PackLifecycleState::Suspended)
        );

        // Suspended -> Active (resume)
        rt.activate("lifecycle").unwrap();
        assert_eq!(
            rt.get_lifecycle_state("lifecycle"),
            Some(PackLifecycleState::Active)
        );

        // Active -> Unloaded
        rt.unload("lifecycle").unwrap();
        assert!(rt.get_pack("lifecycle").is_none());
    }

    // --- Dependency satisfaction test ---

    #[test]
    fn validate_with_satisfied_dependency() {
        let mut rt = DefaultPackRuntime::new();

        // Load and activate the dependency.
        let dep_spec = make_valid_pack_spec("dep-pack", "dep_ns");
        rt.load(dep_spec).unwrap();
        rt.activate("dep-pack").unwrap();

        // Now validate a pack that depends on it.
        let mut spec = make_valid_pack_spec("consumer", "consumer_ns");
        spec.dependencies.push(DependencySpec {
            pack_id: "dep-pack".to_string(),
            version_range: ">=1.0.0".to_string(),
            required: true,
            capabilities_needed: vec![],
            feature_flags: vec![],
        });
        spec.observability.dependency_status.push(DependencyStatusEntry {
            pack_id: "dep-pack".to_string(),
            status: "active".to_string(),
        });
        let result = rt.validate(&spec).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Accepted);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn validate_dependency_version_mismatch() {
        let mut rt = DefaultPackRuntime::new();

        // Load and activate the dependency at version 1.0.0.
        let dep_spec = make_valid_pack_spec("dep-pack", "dep_ns");
        rt.load(dep_spec).unwrap();
        rt.activate("dep-pack").unwrap();

        // Consumer requires >=2.0.0.
        let mut spec = make_valid_pack_spec("consumer", "consumer_ns");
        spec.dependencies.push(DependencySpec {
            pack_id: "dep-pack".to_string(),
            version_range: ">=2.0.0".to_string(),
            required: true,
            capabilities_needed: vec![],
            feature_flags: vec![],
        });
        let result = rt.validate(&spec).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Rejected);
        assert!(result.errors.iter().any(|e| e.contains("does not satisfy")));
    }

    // --- Degraded state transition tests ---

    #[test]
    fn activate_from_degraded_back_to_active() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();

        // Manually set to degraded for testing.
        rt.packs.get_mut("p1").unwrap().state = PackLifecycleState::Degraded;
        assert_eq!(
            rt.get_lifecycle_state("p1"),
            Some(PackLifecycleState::Degraded)
        );

        // Degraded -> Active is valid.
        rt.activate("p1").unwrap();
        assert_eq!(
            rt.get_lifecycle_state("p1"),
            Some(PackLifecycleState::Active)
        );
    }

    #[test]
    fn unload_from_degraded() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();
        rt.packs.get_mut("p1").unwrap().state = PackLifecycleState::Degraded;
        assert!(rt.unload("p1").is_ok());
    }

    // --- Discover test (filesystem) ---

    #[test]
    fn discover_nonexistent_path_fails() {
        let rt = DefaultPackRuntime::new();
        let result = rt.discover("/nonexistent/path/manifest.json");
        assert!(result.is_err());
    }

    // --- Resume transition tests ---

    #[test]
    fn resume_from_suspended() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();
        rt.suspend("p1").unwrap();
        assert!(rt.resume("p1").is_ok());
        assert_eq!(
            rt.get_lifecycle_state("p1"),
            Some(PackLifecycleState::Active)
        );
    }

    #[test]
    fn suspend_unregisters_capabilities() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();
        assert!(rt.registries.skills.contains_key("ns1.test_skill"));

        rt.suspend("p1").unwrap();
        assert!(!rt.registries.skills.contains_key("ns1.test_skill"));
    }

    #[test]
    fn resume_re_registers_capabilities() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();
        rt.suspend("p1").unwrap();
        assert!(!rt.registries.skills.contains_key("ns1.test_skill"));

        rt.resume("p1").unwrap();
        assert!(rt.registries.skills.contains_key("ns1.test_skill"));
    }

    #[test]
    fn resume_from_quarantined_fails() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();
        rt.quarantine("p1").unwrap();
        // Quarantined -> Active is not a valid transition (must go through Validated first).
        assert!(rt.resume("p1").is_err());
    }

    #[test]
    fn resume_nonexistent_fails() {
        let mut rt = DefaultPackRuntime::new();
        assert!(rt.resume("ghost").is_err());
    }

    // --- Upgrade tests ---

    #[test]
    fn upgrade_with_version_bump() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();

        let mut new_spec = make_valid_pack_spec("p1", "ns1");
        new_spec.version = Version::new(2, 0, 0);
        assert!(rt.upgrade("p1", new_spec).is_ok());
        let entry = rt.get_pack("p1").unwrap();
        assert_eq!(entry.spec.version, Version::new(2, 0, 0));
    }

    #[test]
    fn upgrade_without_version_bump_fails() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();

        let new_spec = make_valid_pack_spec("p1", "ns1"); // same version
        assert!(rt.upgrade("p1", new_spec).is_err());
    }

    #[test]
    fn upgrade_nonexistent_fails() {
        let mut rt = DefaultPackRuntime::new();
        let new_spec = make_valid_pack_spec("ghost", "ns1");
        assert!(rt.upgrade("ghost", new_spec).is_err());
    }

    // --- Rollback tests ---

    #[test]
    fn rollback_restores_old_version() {
        let mut rt = DefaultPackRuntime::new();
        let old_spec = make_valid_pack_spec("p1", "ns1");
        rt.load(old_spec.clone()).unwrap();
        rt.activate("p1").unwrap();

        // Upgrade to v2.
        let mut new_spec = make_valid_pack_spec("p1", "ns1");
        new_spec.version = Version::new(2, 0, 0);
        rt.upgrade("p1", new_spec).unwrap();

        // Rollback to v1. Treated as fresh load.
        rt.activate("p1").unwrap();
        rt.rollback("p1", old_spec).unwrap();
        let entry = rt.get_pack("p1").unwrap();
        assert_eq!(entry.spec.version, Version::new(1, 0, 0));
    }

    // --- Reload tests ---

    #[test]
    fn reload_replaces_pack() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();

        let mut new_spec = make_valid_pack_spec("p1", "ns1");
        new_spec.description = Some("reloaded".to_string());
        assert!(rt.reload("p1", new_spec).is_ok());
        let entry = rt.get_pack("p1").unwrap();
        assert_eq!(entry.spec.description.as_deref(), Some("reloaded"));
    }

    #[test]
    fn reload_nonexistent_fails() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("ghost", "ns1");
        assert!(rt.reload("ghost", spec).is_err());
    }

    // --- Composite skill validation tests ---

    #[test]
    fn validate_composite_skill_without_subskills_rejected() {
        let rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "test");
        spec.skills[0].kind = SkillKind::Composite;
        spec.skills[0].subskills.clear();
        let result = rt.validate(&spec).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Rejected);
        assert!(result.errors.iter().any(|e| e.contains("composite skill must declare subskills")));
    }

    #[test]
    fn validate_delegated_skill_without_remote_endpoint_rejected() {
        let rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "test");
        spec.skills[0].kind = SkillKind::Delegated;
        spec.skills[0].remote_endpoint = None;
        let result = rt.validate(&spec).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Rejected);
        assert!(result.errors.iter().any(|e| e.contains("delegated skill must declare remote_endpoint")));
    }

    #[test]
    fn validate_routine_skill_without_guard_conditions_rejected() {
        let rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "test");
        spec.skills[0].kind = SkillKind::Routine;
        spec.skills[0].guard_conditions.clear();
        let result = rt.validate(&spec).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Rejected);
        assert!(result.errors.iter().any(|e| e.contains("routine skill must declare guard_conditions")));
    }

    // --- Skill namespace validation ---

    #[test]
    fn validate_skill_empty_namespace_rejected() {
        let rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "test");
        spec.skills[0].namespace = "".to_string();
        let result = rt.validate(&spec).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Rejected);
        assert!(result.errors.iter().any(|e| e.contains("skill") && e.contains("namespace is empty")));
    }

    // --- Remote exposure validation ---

    #[test]
    fn validate_remote_exposure_missing_peer_trust_rejected() {
        use crate::types::common::CapabilityScope;
        use crate::types::pack::RemoteExposureEntry;

        let rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "test");
        spec.exposure.remote_skills.push(RemoteExposureEntry {
            capability_id: "test_skill".to_string(),
            remote_scope: CapabilityScope::Peer,
            peer_trust_requirements: "".to_string(), // empty = invalid
            serialization_requirements: "json".to_string(),
            rate_limits: "100/min".to_string(),
            replay_protection: true,
            observation_streaming: false,
            delegation_support: false,
        });
        let result = rt.validate(&spec).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Rejected);
        assert!(result.errors.iter().any(|e| e.contains("missing peer_trust_requirements")));
    }

    // --- Port registration test ---

    #[test]
    fn load_registers_ports() {
        use crate::types::common::*;
        use crate::types::port::*;

        let mut rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "ns1");
        spec.ports.push(PortSpec {
            port_id: "test_port".to_string(),
            name: "Test Port".to_string(),
            version: Version::new(1, 0, 0),
            kind: PortKind::Http,
            description: "A test port".to_string(),
            namespace: "ns1".to_string(),
            trust_level: TrustLevel::Trusted,
            capabilities: vec![PortCapabilitySpec {
                capability_id: "fetch".to_string(),
                name: "Fetch".to_string(),
                purpose: "Fetch data from remote".to_string(),
                input_schema: SchemaRef { schema: serde_json::json!({"type": "object"}) },
                output_schema: SchemaRef { schema: serde_json::json!({"type": "object"}) },
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile { expected_latency_ms: 10, p95_latency_ms: 50, max_latency_ms: 200 },
                cost_profile: CostProfile {
                    cpu_cost_class: CostClass::Low,
                    memory_cost_class: CostClass::Low,
                    io_cost_class: CostClass::Low,
                    network_cost_class: CostClass::Low,
                    energy_cost_class: CostClass::Negligible,
                },
                remote_exposable: false,
                auth_override: None,
            }],
            input_schema: SchemaRef {
                schema: serde_json::json!({"type": "object"}),
            },
            output_schema: SchemaRef {
                schema: serde_json::json!({"type": "object"}),
            },
            failure_modes: vec![PortFailureClass::Timeout],
            side_effect_class: SideEffectClass::ReadOnly,
            latency_profile: LatencyProfile {
                expected_latency_ms: 10,
                p95_latency_ms: 50,
                max_latency_ms: 200,
            },
            cost_profile: CostProfile {
                cpu_cost_class: CostClass::Low,
                memory_cost_class: CostClass::Low,
                io_cost_class: CostClass::Low,
                network_cost_class: CostClass::Low,
                energy_cost_class: CostClass::Negligible,
            },
            auth_requirements: AuthRequirements {
                methods: vec![],
                required: false,
            },
            sandbox_requirements: SandboxRequirements {
                filesystem_access: false,
                network_access: true,
                device_access: false,
                process_access: false,
                memory_limit_mb: None,
                cpu_limit_percent: None,
                time_limit_ms: None,
                syscall_limit: None,
            },
            observable_fields: vec![],
            validation_rules: vec![],
            remote_exposure: false,
            backend: crate::types::port::PortBackend::default(),
        });
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();
        assert!(rt.registries.ports.contains_key("ns1.test_port"));
    }

    // --- Policy validation tests ---

    #[test]
    fn validate_policy_empty_rules_and_no_capabilities_rejected() {
        let rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "test");
        spec.policies.push(PolicySpec {
            policy_id: "p_empty".to_string(),
            namespace: "test".to_string(),
            rules: vec![],
            allowed_capabilities: vec![],
            denied_capabilities: vec![],
            ..Default::default()
        });
        let result = rt.validate(&spec).unwrap();
        assert_eq!(result.outcome, ValidationOutcome::Rejected);
        assert!(result.errors.iter().any(|e| e.contains("no rules")));
    }

    // --- Pack failure class enum exists ---

    #[test]
    fn pack_failure_class_has_all_nine_variants() {
        use crate::types::pack::PackFailureClass;
        // Ensure all 9 failure classes compile and are distinct.
        let classes = vec![
            PackFailureClass::ManifestFailure,
            PackFailureClass::SchemaFailure,
            PackFailureClass::DependencyFailure,
            PackFailureClass::NamespaceCollision,
            PackFailureClass::PolicyFailure,
            PackFailureClass::PortFailure,
            PackFailureClass::SkillExecutionFailure,
            PackFailureClass::RemotePeerFailure,
            PackFailureClass::IntegrityFailure,
        ];
        assert_eq!(classes.len(), 9);
    }

    // --- Observability all 9 fields checked ---

    #[test]
    fn validate_observability_rejects_missing_capability_inventory() {
        let rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "test");
        spec.observability.capability_inventory.clear();
        let result = rt.validate(&spec).unwrap();
        assert!(result.errors.iter().any(|e| e.contains("capability_inventory")));
    }

    #[test]
    fn validate_observability_rejects_empty_pack_load_state() {
        let rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "test");
        spec.observability.pack_load_state = "".to_string();
        let result = rt.validate(&spec).unwrap();
        assert!(result.errors.iter().any(|e| e.contains("pack_load_state")));
    }

    // --- Capability scope on groups ---

    #[test]
    fn capability_group_has_scope() {
        use crate::types::common::CapabilityScope;
        let group = CapabilityGroup {
            group_name: "test".to_string(),
            scope: CapabilityScope::Session,
            capabilities: vec!["read".to_string()],
        };
        assert_eq!(group.scope, CapabilityScope::Session);
    }

    #[test]
    fn list_ports_only_returns_active_or_degraded_pack_ports() {
        let mut rt = DefaultPackRuntime::new();

        let mut staged = make_valid_pack_spec("staged", "staged_ns");
        let mut active = make_valid_pack_spec("active", "active_ns");
        for spec in [&mut staged, &mut active] {
            spec.ports.push(PortSpec {
                port_id: "test_port".to_string(),
                name: "Test Port".to_string(),
                version: Version::new(1, 0, 0),
                kind: crate::types::port::PortKind::Http,
                description: "test".to_string(),
                namespace: spec.namespace.clone(),
                trust_level: TrustLevel::Verified,
                capabilities: vec![crate::types::port::PortCapabilitySpec {
                    capability_id: "read".to_string(),
                    name: "Read".to_string(),
                    purpose: "test".to_string(),
                    input_schema: SchemaRef { schema: serde_json::json!({"type": "object"}) },
                    output_schema: SchemaRef { schema: serde_json::json!({"type": "object"}) },
                    effect_class: SideEffectClass::ReadOnly,
                    rollback_support: RollbackSupport::Irreversible,
                    determinism_class: DeterminismClass::Deterministic,
                    idempotence_class: IdempotenceClass::Idempotent,
                    risk_class: RiskClass::Low,
                    latency_profile: LatencyProfile {
                        expected_latency_ms: 10,
                        p95_latency_ms: 50,
                        max_latency_ms: 100,
                    },
                    cost_profile: CostProfile {
                        cpu_cost_class: CostClass::Low,
                        memory_cost_class: CostClass::Low,
                        io_cost_class: CostClass::Low,
                        network_cost_class: CostClass::Low,
                        energy_cost_class: CostClass::Negligible,
                    },
                    remote_exposable: false,
                    auth_override: None,
                }],
                input_schema: SchemaRef { schema: serde_json::json!({"type": "object"}) },
                output_schema: SchemaRef { schema: serde_json::json!({"type": "object"}) },
                failure_modes: vec![PortFailureClass::Timeout],
                side_effect_class: SideEffectClass::ReadOnly,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 10,
                    p95_latency_ms: 50,
                    max_latency_ms: 100,
                },
                cost_profile: CostProfile {
                    cpu_cost_class: CostClass::Low,
                    memory_cost_class: CostClass::Low,
                    io_cost_class: CostClass::Low,
                    network_cost_class: CostClass::Low,
                    energy_cost_class: CostClass::Negligible,
                },
                auth_requirements: AuthRequirements {
                    methods: vec![AuthMethod::BearerToken],
                    required: true,
                },
                sandbox_requirements: SandboxRequirements {
                    filesystem_access: false,
                    network_access: true,
                    device_access: false,
                    process_access: false,
                    memory_limit_mb: None,
                    cpu_limit_percent: None,
                    time_limit_ms: None,
                    syscall_limit: None,
                },
                observable_fields: vec![],
                validation_rules: vec![],
                remote_exposure: false,
                backend: crate::types::port::PortBackend::default(),
            });
        }

        rt.load(staged).unwrap();
        rt.load(active).unwrap();
        rt.activate("active").unwrap();

        let ports = rt.list_ports(None);
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].namespace, "active_ns");
    }

    #[test]
    fn get_port_spec_hides_non_active_pack_ports() {
        let mut rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "ns1");
        spec.ports.push(PortSpec {
            port_id: "test_port".to_string(),
            name: "Test Port".to_string(),
            version: Version::new(1, 0, 0),
            kind: crate::types::port::PortKind::Http,
            description: "test".to_string(),
            namespace: "ns1".to_string(),
            trust_level: TrustLevel::Verified,
            capabilities: vec![crate::types::port::PortCapabilitySpec {
                capability_id: "read".to_string(),
                name: "Read".to_string(),
                purpose: "test".to_string(),
                input_schema: SchemaRef { schema: serde_json::json!({"type": "object"}) },
                output_schema: SchemaRef { schema: serde_json::json!({"type": "object"}) },
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Low,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 10,
                    p95_latency_ms: 50,
                    max_latency_ms: 100,
                },
                cost_profile: CostProfile {
                    cpu_cost_class: CostClass::Low,
                    memory_cost_class: CostClass::Low,
                    io_cost_class: CostClass::Low,
                    network_cost_class: CostClass::Low,
                    energy_cost_class: CostClass::Negligible,
                },
                remote_exposable: false,
                auth_override: None,
            }],
            input_schema: SchemaRef { schema: serde_json::json!({"type": "object"}) },
            output_schema: SchemaRef { schema: serde_json::json!({"type": "object"}) },
            failure_modes: vec![PortFailureClass::Timeout],
            side_effect_class: SideEffectClass::ReadOnly,
            latency_profile: LatencyProfile {
                expected_latency_ms: 10,
                p95_latency_ms: 50,
                max_latency_ms: 100,
            },
            cost_profile: CostProfile {
                cpu_cost_class: CostClass::Low,
                memory_cost_class: CostClass::Low,
                io_cost_class: CostClass::Low,
                network_cost_class: CostClass::Low,
                energy_cost_class: CostClass::Negligible,
            },
            auth_requirements: AuthRequirements {
                methods: vec![AuthMethod::BearerToken],
                required: true,
            },
            sandbox_requirements: SandboxRequirements {
                filesystem_access: false,
                network_access: true,
                device_access: false,
                process_access: false,
                memory_limit_mb: None,
                cpu_limit_percent: None,
                time_limit_ms: None,
                syscall_limit: None,
            },
            observable_fields: vec![],
            validation_rules: vec![],
            remote_exposure: false,
            backend: crate::types::port::PortBackend::default(),
        });
        rt.load(spec).unwrap();

        assert!(rt.get_port_spec("ns1.test_port").is_none());

        rt.activate("p1").unwrap();
        assert!(rt.get_port_spec("ns1.test_port").is_some());

        rt.suspend("p1").unwrap();
        assert!(rt.get_port_spec("ns1.test_port").is_none());
    }

    // --- Cross-pack access tests ---

    #[test]
    fn cross_pack_access_same_pack_allowed() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();

        // A pack accessing its own capability is always allowed.
        assert!(rt.check_cross_pack_access("p1", "ns1.test_skill").is_ok());
    }

    #[test]
    fn cross_pack_access_with_dependency_allowed() {
        let mut rt = DefaultPackRuntime::new();

        // Load target pack.
        let target = make_valid_pack_spec("target-pack", "target_ns");
        rt.load(target).unwrap();
        rt.activate("target-pack").unwrap();

        // Load calling pack that declares target-pack as a dependency.
        let mut caller = make_valid_pack_spec("caller-pack", "caller_ns");
        caller.dependencies.push(DependencySpec {
            pack_id: "target-pack".to_string(),
            version_range: ">=1.0.0".to_string(),
            required: true,
            capabilities_needed: vec![],
            feature_flags: vec![],
        });
        caller.observability.dependency_status.push(DependencyStatusEntry {
            pack_id: "target-pack".to_string(),
            status: "active".to_string(),
        });
        rt.load(caller).unwrap();
        rt.activate("caller-pack").unwrap();

        // Caller can access target's capability because it declared the dependency.
        assert!(rt.check_cross_pack_access("caller-pack", "target_ns.test_skill").is_ok());
    }

    #[test]
    fn cross_pack_access_without_dependency_denied() {
        let mut rt = DefaultPackRuntime::new();

        // Load two unrelated packs (no dependency between them).
        let pack_a = make_valid_pack_spec("pack-a", "ns_a");
        rt.load(pack_a).unwrap();
        rt.activate("pack-a").unwrap();

        let pack_b = make_valid_pack_spec("pack-b", "ns_b");
        rt.load(pack_b).unwrap();
        rt.activate("pack-b").unwrap();

        // pack-b tries to access pack-a's capability without declaring it as a dependency.
        let result = rt.check_cross_pack_access("pack-b", "ns_a.test_skill");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cross-pack access denied"));
        assert!(err.contains("has not declared"));
    }

    #[test]
    fn cross_pack_access_unknown_namespace_denied() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();

        // Target FQN references a namespace that no pack owns.
        let result = rt.check_cross_pack_access("p1", "unknown_ns.some_cap");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no pack owns namespace"));
    }

    #[test]
    fn cross_pack_access_calling_pack_not_loaded_denied() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();

        // Calling pack doesn't exist.
        let result = rt.check_cross_pack_access("ghost", "ns1.test_skill");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("is not loaded"));
    }

    // --- Scope widening detection tests ---

    #[test]
    fn detect_scope_widening_no_change() {
        let old = make_valid_pack_spec("p1", "ns1");
        let new_spec = make_valid_pack_spec("p1", "ns1");
        let widened = DefaultPackRuntime::detect_scope_widening(&old, &new_spec);
        assert!(widened.is_empty());
    }

    #[test]
    fn detect_scope_widening_broadened() {
        let old = make_valid_pack_spec("p1", "ns1");
        // old has capability "read" at scope Local

        let mut new_spec = make_valid_pack_spec("p1", "ns1");
        // Broaden "read" from Local to Public.
        new_spec.capabilities = vec![CapabilityGroup {
            group_name: "core".to_string(),
            scope: CapabilityScope::Public,
            capabilities: vec!["read".to_string()],
        }];
        let widened = DefaultPackRuntime::detect_scope_widening(&old, &new_spec);
        assert_eq!(widened.len(), 1);
        assert!(widened[0].contains("read"));
        assert!(widened[0].contains("Local"));
        assert!(widened[0].contains("Public"));
    }

    #[test]
    fn detect_scope_widening_narrowed_is_ok() {
        let mut old = make_valid_pack_spec("p1", "ns1");
        old.capabilities = vec![CapabilityGroup {
            group_name: "core".to_string(),
            scope: CapabilityScope::Public,
            capabilities: vec!["read".to_string()],
        }];

        let mut new_spec = make_valid_pack_spec("p1", "ns1");
        new_spec.capabilities = vec![CapabilityGroup {
            group_name: "core".to_string(),
            scope: CapabilityScope::Local,
            capabilities: vec!["read".to_string()],
        }];

        // Narrowing is not a widening — should be empty.
        let widened = DefaultPackRuntime::detect_scope_widening(&old, &new_spec);
        assert!(widened.is_empty());
    }

    #[test]
    fn detect_scope_widening_new_capability_not_flagged() {
        let old = make_valid_pack_spec("p1", "ns1");

        let mut new_spec = make_valid_pack_spec("p1", "ns1");
        new_spec.capabilities.push(CapabilityGroup {
            group_name: "extra".to_string(),
            scope: CapabilityScope::Public,
            capabilities: vec!["new_cap".to_string()],
        });

        // New capabilities that didn't exist before aren't considered widened.
        let widened = DefaultPackRuntime::detect_scope_widening(&old, &new_spec);
        assert!(widened.is_empty());
    }

    // --- Scope widening enforcement in reload/upgrade ---

    #[test]
    fn reload_rejects_scope_widening() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();

        let mut widened_spec = make_valid_pack_spec("p1", "ns1");
        widened_spec.capabilities = vec![CapabilityGroup {
            group_name: "core".to_string(),
            scope: CapabilityScope::Peer,
            capabilities: vec!["read".to_string()],
        }];

        let result = rt.reload("p1", widened_spec);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("scope widening detected"));
    }

    #[test]
    fn upgrade_rejects_scope_widening() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();

        let mut widened_spec = make_valid_pack_spec("p1", "ns1");
        widened_spec.version = Version::new(2, 0, 0);
        widened_spec.capabilities = vec![CapabilityGroup {
            group_name: "core".to_string(),
            scope: CapabilityScope::Public,
            capabilities: vec!["read".to_string()],
        }];

        let result = rt.upgrade("p1", widened_spec);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("scope widening detected"));
    }

    // --- Capability scope registration tests ---

    #[test]
    fn register_capabilities_populates_scope_map() {
        let mut rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "ns1");
        spec.capabilities = vec![
            CapabilityGroup {
                group_name: "core".to_string(),
                scope: CapabilityScope::Session,
                capabilities: vec!["test_skill".to_string()],
            },
        ];
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();

        // The skill FQN should be registered with Session scope after activation.
        assert_eq!(
            rt.get_capability_scope("ns1.test_skill"),
            Some(CapabilityScope::Session)
        );
    }

    #[test]
    fn unregister_capabilities_clears_scope_map() {
        let mut rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "ns1");
        // The default spec has "read" in the capability group but skill_id is "test_skill".
        // Add "test_skill" to the capability group so it gets a scope entry.
        spec.capabilities = vec![CapabilityGroup {
            group_name: "core".to_string(),
            scope: CapabilityScope::Local,
            capabilities: vec!["read".to_string(), "test_skill".to_string()],
        }];
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();

        // Scope is present after load.
        assert!(rt.get_capability_scope("ns1.test_skill").is_some());

        // After unload, scope should be gone.
        rt.unload("p1").unwrap();
        assert!(rt.get_capability_scope("ns1.test_skill").is_none());
    }

    #[test]
    fn check_capability_scope_allows_broad_scope() {
        let mut rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "ns1");
        spec.capabilities = vec![CapabilityGroup {
            group_name: "core".to_string(),
            scope: CapabilityScope::Public,
            capabilities: vec!["test_skill".to_string()],
        }];
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();

        // Public-scoped capability should be allowed from any invocation context.
        assert!(rt.check_capability_scope("ns1.test_skill", CapabilityScope::Local));
        assert!(rt.check_capability_scope("ns1.test_skill", CapabilityScope::Session));
        assert!(rt.check_capability_scope("ns1.test_skill", CapabilityScope::Peer));
        assert!(rt.check_capability_scope("ns1.test_skill", CapabilityScope::Public));
    }

    #[test]
    fn check_capability_scope_denies_narrow_scope_from_broad_context() {
        let mut rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "ns1");
        spec.capabilities = vec![CapabilityGroup {
            group_name: "core".to_string(),
            scope: CapabilityScope::Local,
            capabilities: vec!["test_skill".to_string()],
        }];
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();

        // Local-scoped capability should be allowed from Local context.
        assert!(rt.check_capability_scope("ns1.test_skill", CapabilityScope::Local));

        // Local-scoped capability should be denied from broader contexts.
        assert!(!rt.check_capability_scope("ns1.test_skill", CapabilityScope::Session));
        assert!(!rt.check_capability_scope("ns1.test_skill", CapabilityScope::Peer));
        assert!(!rt.check_capability_scope("ns1.test_skill", CapabilityScope::Public));
    }

    #[test]
    fn check_capability_scope_allows_unknown_fqn() {
        let rt = DefaultPackRuntime::new();
        // Unregistered FQN should be allowed (scope enforcement is not
        // responsible for capability existence validation).
        assert!(rt.check_capability_scope("nonexistent.skill", CapabilityScope::Peer));
    }

    #[test]
    fn check_capability_scope_same_scope_allowed() {
        let mut rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "ns1");
        spec.capabilities = vec![CapabilityGroup {
            group_name: "core".to_string(),
            scope: CapabilityScope::Session,
            capabilities: vec!["test_skill".to_string()],
        }];
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();

        // Session-scoped capability invoked from Session context: exact match.
        assert!(rt.check_capability_scope("ns1.test_skill", CapabilityScope::Session));
        // Narrower context is also fine.
        assert!(rt.check_capability_scope("ns1.test_skill", CapabilityScope::Local));
        // Broader context should be denied.
        assert!(!rt.check_capability_scope("ns1.test_skill", CapabilityScope::Peer));
    }

    // --- Routine invalidation integration tests ---

    #[test]
    fn reload_and_invalidate_removes_routines_referencing_removed_skills() {
        use crate::memory::routines::{DefaultRoutineStore, RoutineStore};
        use crate::types::routine::{Routine, RoutineOrigin};

        let mut rt = DefaultPackRuntime::new();
        let mut routine_store = DefaultRoutineStore::new();

        let spec_v1 = make_valid_pack_spec("p1", "ns1");
        rt.load(spec_v1).unwrap();
        rt.activate("p1").unwrap();

        // Routine that references ns1.test_skill (will be removed in v2).
        routine_store
            .register(Routine {
                routine_id: "r_uses_test_skill".to_string(),
                namespace: "ns1".to_string(),
                origin: RoutineOrigin::SchemaCompiled,
                match_conditions: vec![Precondition {
                    condition_type: "goal".to_string(),
                    expression: serde_json::json!({"goal": "test"}),
                    description: "test".to_string(),
                }],
                compiled_skill_path: vec!["ns1.test_skill".to_string()],
                compiled_steps: vec![],
                guard_conditions: Vec::new(),
                expected_cost: 0.1,
                expected_effect: Vec::new(),
                confidence: 0.9,
                autonomous: false,
                priority: 0,
                exclusive: false,
                policy_scope: None,
            })
            .unwrap();

        // Unrelated routine.
        routine_store
            .register(Routine {
                routine_id: "r_unrelated".to_string(),
                namespace: "other".to_string(),
                origin: RoutineOrigin::PackAuthored,
                match_conditions: vec![Precondition {
                    condition_type: "goal".to_string(),
                    expression: serde_json::json!({"goal": "other"}),
                    description: "other".to_string(),
                }],
                compiled_skill_path: vec!["other.ping".to_string()],
                compiled_steps: vec![],
                guard_conditions: Vec::new(),
                expected_cost: 0.05,
                expected_effect: Vec::new(),
                confidence: 0.85,
                autonomous: false,
                priority: 0,
                exclusive: false,
                policy_scope: None,
            })
            .unwrap();

        // Reload with a spec that renames the skill (test_skill -> new_skill).
        let mut spec_v2 = make_valid_pack_spec("p1", "ns1");
        spec_v2.skills[0].skill_id = "new_skill".to_string();
        spec_v2.exposure.local_skills = vec!["new_skill".to_string()];

        let invalidated = rt
            .reload_and_invalidate_routines("p1", spec_v2, &mut routine_store)
            .unwrap();

        assert_eq!(invalidated.len(), 1);
        assert!(invalidated.contains(&"r_uses_test_skill".to_string()));
        assert!(routine_store.get("r_uses_test_skill").is_none());
        assert!(routine_store.get("r_unrelated").is_some());
    }

    #[test]
    fn reload_and_invalidate_no_removed_skills_preserves_routines() {
        use crate::memory::routines::{DefaultRoutineStore, RoutineStore};
        use crate::types::routine::{Routine, RoutineOrigin};

        let mut rt = DefaultPackRuntime::new();
        let mut routine_store = DefaultRoutineStore::new();

        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();

        routine_store
            .register(Routine {
                routine_id: "r1".to_string(),
                namespace: "ns1".to_string(),
                origin: RoutineOrigin::SchemaCompiled,
                match_conditions: vec![Precondition {
                    condition_type: "goal".to_string(),
                    expression: serde_json::json!({"goal": "test"}),
                    description: "test".to_string(),
                }],
                compiled_skill_path: vec!["ns1.test_skill".to_string()],
                compiled_steps: vec![],
                guard_conditions: Vec::new(),
                expected_cost: 0.1,
                expected_effect: Vec::new(),
                confidence: 0.9,
                autonomous: false,
                priority: 0,
                exclusive: false,
                policy_scope: None,
            })
            .unwrap();

        // Reload with the same skills (no removals).
        let spec_same = make_valid_pack_spec("p1", "ns1");
        let invalidated = rt
            .reload_and_invalidate_routines("p1", spec_same, &mut routine_store)
            .unwrap();

        assert!(invalidated.is_empty());
        assert!(routine_store.get("r1").is_some());
    }

    #[test]
    fn upgrade_and_invalidate_removes_routines_referencing_removed_skills() {
        use crate::memory::routines::{DefaultRoutineStore, RoutineStore};
        use crate::types::routine::{Routine, RoutineOrigin};

        let mut rt = DefaultPackRuntime::new();
        let mut routine_store = DefaultRoutineStore::new();

        let spec_v1 = make_valid_pack_spec("p1", "ns1");
        rt.load(spec_v1).unwrap();
        rt.activate("p1").unwrap();

        routine_store
            .register(Routine {
                routine_id: "r_affected".to_string(),
                namespace: "ns1".to_string(),
                origin: RoutineOrigin::SchemaCompiled,
                match_conditions: vec![Precondition {
                    condition_type: "goal".to_string(),
                    expression: serde_json::json!({"goal": "test"}),
                    description: "test".to_string(),
                }],
                compiled_skill_path: vec!["ns1.test_skill".to_string()],
                compiled_steps: vec![],
                guard_conditions: Vec::new(),
                expected_cost: 0.1,
                expected_effect: Vec::new(),
                confidence: 0.9,
                autonomous: false,
                priority: 0,
                exclusive: false,
                policy_scope: None,
            })
            .unwrap();

        // Upgrade to v2 with the skill renamed.
        let mut spec_v2 = make_valid_pack_spec("p1", "ns1");
        spec_v2.version = Version::new(2, 0, 0);
        spec_v2.skills[0].skill_id = "replacement_skill".to_string();
        spec_v2.exposure.local_skills = vec!["replacement_skill".to_string()];

        let invalidated = rt
            .upgrade_and_invalidate_routines("p1", spec_v2, &mut routine_store)
            .unwrap();

        assert_eq!(invalidated.len(), 1);
        assert!(invalidated.contains(&"r_affected".to_string()));
        assert!(routine_store.get("r_affected").is_none());
    }

    #[test]
    fn compute_removed_skill_fqns_detects_removals() {
        let old_spec = make_valid_pack_spec("p1", "ns1");
        let mut new_spec = make_valid_pack_spec("p1", "ns1");
        new_spec.skills[0].skill_id = "other_skill".to_string();

        let removed = DefaultPackRuntime::compute_removed_skill_fqns(&old_spec, &new_spec);
        assert_eq!(removed.len(), 1);
        assert!(removed.contains("ns1.test_skill"));
    }

    #[test]
    fn compute_removed_skill_fqns_empty_when_same() {
        let spec = make_valid_pack_spec("p1", "ns1");
        let removed = DefaultPackRuntime::compute_removed_skill_fqns(&spec, &spec);
        assert!(removed.is_empty());
    }

    // --- Capability health tracking tests ---

    #[test]
    fn load_initializes_capability_health() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();

        let entry = rt.get_pack("p1").unwrap();
        assert!(entry.capability_health.contains_key("ns1.test_skill"));
        let ch = &entry.capability_health["ns1.test_skill"];
        assert!(ch.enabled);
        assert_eq!(ch.failure_count, 0);
        assert_eq!(ch.last_latency_ms, 0);
    }

    #[test]
    fn disable_capability_marks_it_disabled() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();

        rt.disable_capability("p1", "ns1.test_skill").unwrap();

        let entry = rt.get_pack("p1").unwrap();
        assert!(!entry.capability_health["ns1.test_skill"].enabled);
    }

    #[test]
    fn enable_capability_restores_it() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();

        rt.disable_capability("p1", "ns1.test_skill").unwrap();
        rt.enable_capability("p1", "ns1.test_skill").unwrap();

        let entry = rt.get_pack("p1").unwrap();
        assert!(entry.capability_health["ns1.test_skill"].enabled);
    }

    #[test]
    fn disable_unknown_capability_returns_error() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();

        let result = rt.disable_capability("p1", "ns1.nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn enable_unknown_capability_returns_error() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();

        let result = rt.enable_capability("p1", "ns1.nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn disable_capability_on_missing_pack_returns_error() {
        let mut rt = DefaultPackRuntime::new();
        assert!(rt.disable_capability("ghost", "ns1.test_skill").is_err());
    }

    #[test]
    fn record_capability_outcome_tracks_failures() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();

        rt.record_capability_outcome("p1", "ns1.test_skill", false, 42);
        rt.record_capability_outcome("p1", "ns1.test_skill", false, 55);
        rt.record_capability_outcome("p1", "ns1.test_skill", true, 10);

        let entry = rt.get_pack("p1").unwrap();
        let ch = &entry.capability_health["ns1.test_skill"];
        assert_eq!(ch.failure_count, 2);
        assert_eq!(ch.last_latency_ms, 10);
    }

    #[test]
    fn get_pack_health_report() {
        let mut rt = DefaultPackRuntime::new();
        let spec = make_valid_pack_spec("p1", "ns1");
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();

        rt.record_capability_outcome("p1", "ns1.test_skill", false, 100);

        let report = rt.get_pack_health("p1").unwrap();
        assert_eq!(report.pack_id, "p1");
        assert_eq!(report.state, PackLifecycleState::Active);
        assert_eq!(report.total_failures, 1);
        assert!(!report.capabilities.is_empty());
        assert_eq!(report.health_checks, vec!["ping".to_string()]);
        assert_eq!(report.expected_failure_modes, vec!["timeout".to_string()]);
        assert_eq!(report.metric_names, vec!["skill_latency_ms".to_string()]);
        assert_eq!(report.trace_categories, vec!["skill_execution".to_string()]);
    }

    #[test]
    fn get_pack_health_missing_pack_returns_none() {
        let rt = DefaultPackRuntime::new();
        assert!(rt.get_pack_health("ghost").is_none());
    }

    #[test]
    fn disabled_port_excluded_from_list_ports() {
        use crate::types::common::*;
        use crate::types::port::*;

        let mut rt = DefaultPackRuntime::new();
        let mut spec = make_valid_pack_spec("p1", "ns1");
        spec.ports.push(PortSpec {
            port_id: "test_port".to_string(),
            name: "Test Port".to_string(),
            version: Version::new(1, 0, 0),
            kind: PortKind::Http,
            description: "test".to_string(),
            namespace: "ns1".to_string(),
            trust_level: TrustLevel::Verified,
            capabilities: vec![PortCapabilitySpec {
                capability_id: "read".to_string(),
                name: "Read".to_string(),
                purpose: "test".to_string(),
                input_schema: SchemaRef { schema: serde_json::json!({"type": "object"}) },
                output_schema: SchemaRef { schema: serde_json::json!({"type": "object"}) },
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 10,
                    p95_latency_ms: 50,
                    max_latency_ms: 200,
                },
                cost_profile: CostProfile {
                    cpu_cost_class: CostClass::Low,
                    memory_cost_class: CostClass::Low,
                    io_cost_class: CostClass::Low,
                    network_cost_class: CostClass::Low,
                    energy_cost_class: CostClass::Negligible,
                },
                remote_exposable: false,
                auth_override: None,
            }],
            input_schema: SchemaRef { schema: serde_json::json!({"type": "object"}) },
            output_schema: SchemaRef { schema: serde_json::json!({"type": "object"}) },
            failure_modes: vec![PortFailureClass::Timeout],
            side_effect_class: SideEffectClass::ReadOnly,
            latency_profile: LatencyProfile {
                expected_latency_ms: 10,
                p95_latency_ms: 50,
                max_latency_ms: 200,
            },
            cost_profile: CostProfile {
                cpu_cost_class: CostClass::Low,
                memory_cost_class: CostClass::Low,
                io_cost_class: CostClass::Low,
                network_cost_class: CostClass::Low,
                energy_cost_class: CostClass::Negligible,
            },
            auth_requirements: AuthRequirements {
                methods: vec![],
                required: false,
            },
            sandbox_requirements: SandboxRequirements {
                filesystem_access: false,
                network_access: true,
                device_access: false,
                process_access: false,
                memory_limit_mb: None,
                cpu_limit_percent: None,
                time_limit_ms: None,
                syscall_limit: None,
            },
            observable_fields: vec![],
            validation_rules: vec![],
            remote_exposure: false,
            backend: crate::types::port::PortBackend::default(),
        });
        rt.load(spec).unwrap();
        rt.activate("p1").unwrap();

        // Port is visible before disabling.
        assert_eq!(rt.list_ports(None).len(), 1);
        assert!(rt.get_port_spec("ns1.test_port").is_some());

        // Disable the port capability.
        rt.disable_capability("p1", "ns1.test_port").unwrap();

        // Port should now be hidden from list_ports and get_port_spec.
        assert_eq!(rt.list_ports(None).len(), 0);
        assert!(rt.get_port_spec("ns1.test_port").is_none());

        // Re-enable it.
        rt.enable_capability("p1", "ns1.test_port").unwrap();
        assert_eq!(rt.list_ports(None).len(), 1);
        assert!(rt.get_port_spec("ns1.test_port").is_some());
    }
}
