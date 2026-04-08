use serde::{Deserialize, Serialize};

// --- Side Effects ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectClass {
    None,
    ReadOnly,
    LocalStateMutation,
    ExternalStateMutation,
    Destructive,
    Irreversible,
}

// --- Failure Classes (Port-level) ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortFailureClass {
    ValidationError,
    AuthorizationDenied,
    SandboxViolation,
    PolicyDenied,
    Timeout,
    DependencyUnavailable,
    TransportError,
    ExternalError,
    PartialSuccess,
    RollbackFailed,
    Unknown,
}

// --- Failure Classes (Skill-level) ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillFailureClass {
    ValidationFailure,
    PreconditionFailure,
    PolicyDenial,
    BindingFailure,
    PortFailure,
    RemoteFailure,
    Timeout,
    BudgetExhaustion,
    PartialSuccess,
    RollbackFailure,
    Unknown,
}

// --- Determinism ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeterminismClass {
    Deterministic,
    PartiallyDeterministic,
    Stochastic,
    DelegatedVariant,
}

// --- Risk ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskClass {
    Negligible,
    Low,
    Medium,
    High,
    Critical,
}

// --- Trust ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    Untrusted,
    Restricted,
    Verified,
    Trusted,
    BuiltIn,
}

// --- Capability Scope ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityScope {
    Local,
    Session,
    Tenant,
    Device,
    Peer,
    Public,
}

impl CapabilityScope {
    /// Return a numeric rank for scope ordering.
    /// Local (narrowest, 0) < Session < Tenant < Device < Peer < Public (broadest, 5).
    pub fn rank(self) -> u8 {
        match self {
            CapabilityScope::Local => 0,
            CapabilityScope::Session => 1,
            CapabilityScope::Tenant => 2,
            CapabilityScope::Device => 3,
            CapabilityScope::Peer => 4,
            CapabilityScope::Public => 5,
        }
    }

    /// Return true if `self` is at least as broad as `other`.
    pub fn is_at_least(&self, other: CapabilityScope) -> bool {
        self.rank() >= other.rank()
    }
}

impl PartialOrd for CapabilityScope {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CapabilityScope {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank().cmp(&other.rank())
    }
}

// --- Critic Decision ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriticDecision {
    Continue,
    Revise,
    Backtrack,
    Delegate,
    Stop,
}

// --- Idempotence ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdempotenceClass {
    Idempotent,
    NonIdempotent,
    ConditionallyIdempotent,
}

// --- Rollback Support ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollbackSupport {
    FullReversal,
    CompensatingAction,
    LogicalUndo,
    Irreversible,
}

// --- Latency Profile ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatencyProfile {
    pub expected_latency_ms: u64,
    pub p95_latency_ms: u64,
    pub max_latency_ms: u64,
}

// --- Cost Profile ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostProfile {
    pub cpu_cost_class: CostClass,
    pub memory_cost_class: CostClass,
    pub io_cost_class: CostClass,
    pub network_cost_class: CostClass,
    pub energy_cost_class: CostClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostClass {
    Negligible,
    Low,
    Medium,
    High,
    Extreme,
}

// --- Schema Reference ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaRef {
    pub schema: serde_json::Value,
}

// --- Validation Rule ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationRule {
    pub field: String,
    pub rule_type: String,
    pub constraint: serde_json::Value,
}

// --- Auth Requirements ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequirements {
    pub methods: Vec<AuthMethod>,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    BearerToken,
    ApiKey,
    MTls,
    SignedCapabilityToken,
    LocalProcessTrust,
    DeviceAttestation,
    PeerIdentityTrust,
}

// --- Sandbox Requirements ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxRequirements {
    pub filesystem_access: bool,
    pub network_access: bool,
    pub device_access: bool,
    pub process_access: bool,
    pub memory_limit_mb: Option<u64>,
    pub cpu_limit_percent: Option<u32>,
    pub time_limit_ms: Option<u64>,
    /// Syscall limit where applicable (port-spec.md: 8 sandbox dimensions).
    pub syscall_limit: Option<u64>,
}

// --- Fact Provenance ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactProvenance {
    Asserted,
    Observed,
    Inferred,
    Stale,
    Remote,
}

// --- Effect Descriptor ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectDescriptor {
    pub effect_type: EffectType,
    pub target_resource: Option<String>,
    pub description: String,
    pub patch: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectType {
    Creation,
    Update,
    Deletion,
    Emission,
    Scheduling,
    Notification,
    Delegation,
    Synchronization,
}

// --- Budget ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budget {
    pub risk_remaining: f64,
    pub latency_remaining_ms: u64,
    pub resource_remaining: f64,
    pub steps_remaining: u32,
}

// --- Precondition ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Precondition {
    pub condition_type: String,
    pub expression: serde_json::Value,
    pub description: String,
}

// --- Termination Condition ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminationCondition {
    pub condition_type: TerminationType,
    pub expression: serde_json::Value,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminationType {
    Success,
    Failure,
    Timeout,
    BudgetExhaustion,
    PolicyDenial,
    ExternalError,
    ExplicitAbort,
}

// --- Port invocation outcomes (tracing obligations) ---

/// Result of the auth check performed before a port invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum AuthOutcome {
    Passed,
    Failed { reason: String },
    NotRequired,
}

/// Result of the policy check performed before a port invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum PolicyOutcome {
    Allowed,
    Denied { reason: String },
    RequiresConfirmation { reason: String },
}

/// Result of the sandbox constraint check for a port invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum SandboxOutcome {
    Satisfied,
    Violated { dimension: String, reason: String },
    NotEnforced,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_scope_ordering() {
        assert!(CapabilityScope::Local < CapabilityScope::Session);
        assert!(CapabilityScope::Session < CapabilityScope::Tenant);
        assert!(CapabilityScope::Tenant < CapabilityScope::Device);
        assert!(CapabilityScope::Device < CapabilityScope::Peer);
        assert!(CapabilityScope::Peer < CapabilityScope::Public);
    }

    #[test]
    fn capability_scope_rank_values() {
        assert_eq!(CapabilityScope::Local.rank(), 0);
        assert_eq!(CapabilityScope::Session.rank(), 1);
        assert_eq!(CapabilityScope::Tenant.rank(), 2);
        assert_eq!(CapabilityScope::Device.rank(), 3);
        assert_eq!(CapabilityScope::Peer.rank(), 4);
        assert_eq!(CapabilityScope::Public.rank(), 5);
    }

    #[test]
    fn capability_scope_is_at_least() {
        assert!(CapabilityScope::Public.is_at_least(CapabilityScope::Local));
        assert!(CapabilityScope::Public.is_at_least(CapabilityScope::Public));
        assert!(CapabilityScope::Session.is_at_least(CapabilityScope::Local));
        assert!(CapabilityScope::Session.is_at_least(CapabilityScope::Session));
        assert!(!CapabilityScope::Local.is_at_least(CapabilityScope::Session));
        assert!(!CapabilityScope::Local.is_at_least(CapabilityScope::Peer));
        assert!(!CapabilityScope::Session.is_at_least(CapabilityScope::Peer));
    }
}
