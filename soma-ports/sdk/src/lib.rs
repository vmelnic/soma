//! SOMA Port SDK -- shared types and Port trait for building external port packs.
//!
//! This crate defines the contract between the SOMA runtime (`soma-next`) and
//! dynamically loaded port adapters. Each port pack is compiled as a `cdylib`
//! crate and must export a C-ABI init function:
//!
//! ```rust,ignore
//! #[no_mangle]
//! pub extern "C" fn soma_port_init() -> *mut dyn Port {
//!     Box::into_raw(Box::new(MyPort::new()))
//! }
//! ```
//!
//! The types here mirror the canonical types in `soma-next/src/types/` so that
//! dynamically loaded ports are ABI-compatible with the runtime.

/// Re-export semver so port packs can use it without adding a direct dependency.
pub use semver;

pub mod prelude {
    pub use crate::{
        AuthMethod, AuthRequirements, CostClass, CostProfile, DeterminismClass,
        IdempotenceClass, LatencyProfile, Port, PortCallRecord, PortCapabilitySpec,
        PortError, PortFailureClass, PortKind, PortLifecycleState, PortSpec, RiskClass,
        RollbackSupport, SandboxRequirements, SchemaRef, SideEffectClass, TrustLevel,
        ValidationRule,
    };
    pub use crate::semver;
}

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Port error type
// ---------------------------------------------------------------------------

/// Structured error returned by port invocations.
#[derive(Debug, thiserror::Error)]
pub enum PortError {
    #[error("validation error: {0}")]
    Validation(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("authorization denied: {0}")]
    AuthorizationDenied(String),

    #[error("timeout: {0}")]
    Timeout(String),

    #[error("dependency unavailable: {0}")]
    DependencyUnavailable(String),

    #[error("transport error: {0}")]
    TransportError(String),

    #[error("external error: {0}")]
    ExternalError(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl PortError {
    /// Map this error to a failure class for the PortCallRecord.
    pub fn failure_class(&self) -> PortFailureClass {
        match self {
            Self::Validation(_) => PortFailureClass::ValidationError,
            Self::NotFound(_) => PortFailureClass::ValidationError,
            Self::AuthorizationDenied(_) => PortFailureClass::AuthorizationDenied,
            Self::Timeout(_) => PortFailureClass::Timeout,
            Self::DependencyUnavailable(_) => PortFailureClass::DependencyUnavailable,
            Self::TransportError(_) => PortFailureClass::TransportError,
            Self::ExternalError(_) => PortFailureClass::ExternalError,
            Self::Internal(_) => PortFailureClass::Unknown,
        }
    }
}

pub type Result<T> = std::result::Result<T, PortError>;

// ---------------------------------------------------------------------------
// Enums mirroring soma-next/src/types/common.rs
// ---------------------------------------------------------------------------

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeterminismClass {
    Deterministic,
    PartiallyDeterministic,
    Stochastic,
    DelegatedVariant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskClass {
    Negligible,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    Untrusted,
    Restricted,
    Verified,
    Trusted,
    BuiltIn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdempotenceClass {
    Idempotent,
    NonIdempotent,
    ConditionallyIdempotent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollbackSupport {
    FullReversal,
    CompensatingAction,
    LogicalUndo,
    Irreversible,
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

// ---------------------------------------------------------------------------
// Struct types mirroring soma-next/src/types/common.rs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatencyProfile {
    pub expected_latency_ms: u64,
    pub p95_latency_ms: u64,
    pub max_latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostProfile {
    pub cpu_cost_class: CostClass,
    pub memory_cost_class: CostClass,
    pub io_cost_class: CostClass,
    pub network_cost_class: CostClass,
    pub energy_cost_class: CostClass,
}

impl Default for CostProfile {
    fn default() -> Self {
        Self {
            cpu_cost_class: CostClass::Negligible,
            memory_cost_class: CostClass::Negligible,
            io_cost_class: CostClass::Negligible,
            network_cost_class: CostClass::Negligible,
            energy_cost_class: CostClass::Negligible,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaRef {
    pub schema: serde_json::Value,
}

impl SchemaRef {
    /// Convenience constructor for a JSON Schema object type with properties.
    pub fn object(properties: serde_json::Value) -> Self {
        Self {
            schema: serde_json::json!({
                "type": "object",
                "properties": properties,
            }),
        }
    }

    /// Empty schema that accepts anything.
    pub fn any() -> Self {
        Self {
            schema: serde_json::json!({}),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationRule {
    pub field: String,
    pub rule_type: String,
    pub constraint: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequirements {
    pub methods: Vec<AuthMethod>,
    pub required: bool,
}

impl Default for AuthRequirements {
    fn default() -> Self {
        Self {
            methods: vec![],
            required: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxRequirements {
    pub filesystem_access: bool,
    pub network_access: bool,
    pub device_access: bool,
    pub process_access: bool,
    pub memory_limit_mb: Option<u64>,
    pub cpu_limit_percent: Option<u32>,
    pub time_limit_ms: Option<u64>,
    pub syscall_limit: Option<u64>,
}

impl Default for SandboxRequirements {
    fn default() -> Self {
        Self {
            filesystem_access: false,
            network_access: false,
            device_access: false,
            process_access: false,
            memory_limit_mb: None,
            cpu_limit_percent: None,
            time_limit_ms: None,
            syscall_limit: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Port types mirroring soma-next/src/types/port.rs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortKind {
    Filesystem,
    Database,
    Http,
    Queue,
    Renderer,
    Sensor,
    Actuator,
    Messaging,
    DeviceTransport,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortLifecycleState {
    Declared,
    Loaded,
    Validated,
    Active,
    Degraded,
    Quarantined,
    Retired,
}

/// PortSpec -- the required declaration for every port.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortSpec {
    pub port_id: String,
    pub name: String,
    pub version: semver::Version,
    pub kind: PortKind,
    pub description: String,
    pub namespace: String,
    pub trust_level: TrustLevel,
    pub capabilities: Vec<PortCapabilitySpec>,
    pub input_schema: SchemaRef,
    pub output_schema: SchemaRef,
    pub failure_modes: Vec<PortFailureClass>,
    pub side_effect_class: SideEffectClass,
    pub latency_profile: LatencyProfile,
    pub cost_profile: CostProfile,
    pub auth_requirements: AuthRequirements,
    pub sandbox_requirements: SandboxRequirements,
    pub observable_fields: Vec<String>,
    pub validation_rules: Vec<ValidationRule>,
    pub remote_exposure: bool,
}

/// Per-capability specification within a port.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortCapabilitySpec {
    pub capability_id: String,
    pub name: String,
    pub purpose: String,
    pub input_schema: SchemaRef,
    pub output_schema: SchemaRef,
    pub effect_class: SideEffectClass,
    pub rollback_support: RollbackSupport,
    pub determinism_class: DeterminismClass,
    pub idempotence_class: IdempotenceClass,
    pub risk_class: RiskClass,
    pub latency_profile: LatencyProfile,
    pub cost_profile: CostProfile,
    pub remote_exposable: bool,
    #[serde(default)]
    pub auth_override: Option<AuthRequirements>,
}

// ---------------------------------------------------------------------------
// PortCallRecord mirroring soma-next/src/types/observation.rs
// ---------------------------------------------------------------------------

/// Structured result of a single port capability invocation.
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
    pub retry_safe: bool,
    pub input_hash: Option<String>,
    pub session_id: Option<Uuid>,
    pub goal_id: Option<String>,
    pub caller_identity: Option<String>,
    pub auth_result: Option<serde_json::Value>,
    pub policy_result: Option<serde_json::Value>,
    pub sandbox_result: Option<serde_json::Value>,
}

impl PortCallRecord {
    /// Convenience constructor for a successful invocation.
    pub fn success(
        port_id: &str,
        capability_id: &str,
        result: serde_json::Value,
        latency_ms: u64,
    ) -> Self {
        Self {
            observation_id: Uuid::new_v4(),
            port_id: port_id.to_string(),
            capability_id: capability_id.to_string(),
            invocation_id: Uuid::new_v4(),
            success: true,
            failure_class: None,
            raw_result: result.clone(),
            structured_result: result,
            effect_patch: None,
            side_effect_summary: None,
            latency_ms,
            resource_cost: 0.0,
            confidence: 1.0,
            timestamp: Utc::now(),
            retry_safe: true,
            input_hash: None,
            session_id: None,
            goal_id: None,
            caller_identity: None,
            auth_result: None,
            policy_result: None,
            sandbox_result: None,
        }
    }

    /// Convenience constructor for a failed invocation.
    pub fn failure(
        port_id: &str,
        capability_id: &str,
        failure_class: PortFailureClass,
        error_msg: &str,
        latency_ms: u64,
    ) -> Self {
        Self {
            observation_id: Uuid::new_v4(),
            port_id: port_id.to_string(),
            capability_id: capability_id.to_string(),
            invocation_id: Uuid::new_v4(),
            success: false,
            failure_class: Some(failure_class),
            raw_result: serde_json::json!({ "error": error_msg }),
            structured_result: serde_json::json!({ "error": error_msg }),
            effect_patch: None,
            side_effect_summary: None,
            latency_ms,
            resource_cost: 0.0,
            confidence: 0.0,
            timestamp: Utc::now(),
            retry_safe: false,
            input_hash: None,
            session_id: None,
            goal_id: None,
            caller_identity: None,
            auth_result: None,
            policy_result: None,
            sandbox_result: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Port trait -- the interface each port adapter implements
// ---------------------------------------------------------------------------

/// A port adapter: typed interface to one external system.
///
/// Implementors wrap a single integration boundary (database, filesystem,
/// HTTP endpoint, device, etc.) and expose one or more capabilities through
/// a stable, validated contract.
pub trait Port: Send + Sync {
    /// The declared specification for this port.
    fn spec(&self) -> &PortSpec;

    /// Execute a capability, returning a fully-populated `PortCallRecord`.
    ///
    /// The runtime calls `validate_input` before dispatching here, so
    /// implementations may assume the input passed schema validation.
    fn invoke(
        &self,
        capability_id: &str,
        input: serde_json::Value,
    ) -> Result<PortCallRecord>;

    /// Validate input against the capability's declared schema.
    ///
    /// Called by the runtime before `invoke`. A port MAY add domain-specific
    /// checks beyond pure schema conformance.
    fn validate_input(
        &self,
        capability_id: &str,
        input: &serde_json::Value,
    ) -> Result<()>;

    /// Current lifecycle state as seen by the adapter itself.
    fn lifecycle_state(&self) -> PortLifecycleState;
}
