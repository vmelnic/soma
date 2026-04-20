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
    pub use crate::semver;
    pub use crate::{
        AuthMethod, AuthRequirements, CostClass, CostProfile, DeterminismClass, IdempotenceClass,
        LazyConn, LatencyProfile, Port, PortCallRecord, PortCapabilitySpec, PortError,
        PortFailureClass, PortKind, PortLifecycleState, PortSpec, RiskClass, RollbackSupport,
        SandboxRequirements, SchemaRef, SideEffectClass, TrustLevel, ValidationRule,
    };
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthRequirements {
    pub methods: Vec<AuthMethod>,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
///
/// # Init contract
///
/// `soma_port_init()` MUST return immediately. Never block on network I/O,
/// service connections, or resource acquisition during construction. Use
/// [`LazyConn`] or `OnceLock` to defer connections to first invoke.
/// A port whose backing service is down MUST still load — it reports
/// `DependencyUnavailable` at invoke time, not at init time.
pub trait Port: Send + Sync {
    /// The declared specification for this port.
    fn spec(&self) -> &PortSpec;

    /// Serialize the port spec to JSON. Used by the runtime to safely
    /// transfer PortSpec data across the dylib ABI boundary. The default
    /// implementation calls `spec()` and serializes — ports should NOT
    /// override this unless they have a custom serialization need.
    fn spec_json(&self) -> String {
        serde_json::to_string(self.spec()).unwrap_or_else(|e| {
            format!("{{\"error\":\"spec serialization failed: {e}\"}}")
        })
    }

    /// Execute a capability, returning a fully-populated `PortCallRecord`.
    ///
    /// The runtime calls `validate_input` before dispatching here, so
    /// implementations may assume the input passed schema validation.
    fn invoke(&self, capability_id: &str, input: serde_json::Value) -> Result<PortCallRecord>;

    /// ABI-safe invoke: input and output are both JSON strings so no Rust
    /// structs cross the dylib boundary. The runtime passes serialized input;
    /// this default deserializes, calls invoke(), and re-serializes the result.
    fn invoke_json(&self, capability_id: &str, input_json: &str) -> std::result::Result<String, String> {
        let input: serde_json::Value = serde_json::from_str(input_json)
            .map_err(|e| format!("input deserialization failed: {e}"))?;
        match self.invoke(capability_id, input) {
            Ok(record) => serde_json::to_string(&record).map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Validate input against the capability's declared schema.
    ///
    /// Called by the runtime before `invoke`. A port MAY add domain-specific
    /// checks beyond pure schema conformance.
    fn validate_input(&self, capability_id: &str, input: &serde_json::Value) -> Result<()>;

    /// ABI-safe validation: input arrives as a JSON string so no Value crosses
    /// the dylib boundary. Default deserializes and delegates to validate_input.
    fn validate_input_json(&self, capability_id: &str, input_json: &str) -> std::result::Result<(), String> {
        let input: serde_json::Value = serde_json::from_str(input_json)
            .map_err(|e| format!("input deserialization failed: {e}"))?;
        self.validate_input(capability_id, &input).map_err(|e| e.to_string())
    }

    /// Current lifecycle state as seen by the adapter itself.
    fn lifecycle_state(&self) -> PortLifecycleState;
}

// ---------------------------------------------------------------------------
// LazyConn -- SDK helper for deferred, non-blocking connections
// ---------------------------------------------------------------------------

use std::sync::Mutex;
use std::time::Duration;

/// Thread-safe lazy connection wrapper for port adapters.
///
/// Defers connection establishment to first use. If the backing service is
/// unreachable, returns `PortError::DependencyUnavailable` without blocking
/// the runtime startup.
///
/// ```rust,ignore
/// struct MyPort {
///     conn: LazyConn<MyClient>,
///     // ...
/// }
///
/// impl MyPort {
///     fn new() -> Self {
///         Self {
///             conn: LazyConn::new(Duration::from_secs(3), || {
///                 MyClient::connect(&url)
///             }),
///         }
///     }
/// }
/// ```
pub struct LazyConn<C> {
    inner: Mutex<Option<C>>,
    factory: Box<dyn Fn() -> std::result::Result<C, String> + Send + Sync>,
    timeout: Duration,
}

impl<C: Clone + Send> LazyConn<C> {
    pub fn new<F>(timeout: Duration, factory: F) -> Self
    where
        F: Fn() -> std::result::Result<C, String> + Send + Sync + 'static,
    {
        Self {
            inner: Mutex::new(None),
            factory: Box::new(factory),
            timeout,
        }
    }

    /// Get or establish the connection. Returns DependencyUnavailable on failure.
    pub fn get(&self) -> Result<C> {
        let mut guard = self.inner.lock()
            .map_err(|e| PortError::Internal(format!("lock poisoned: {e}")))?;

        if let Some(ref conn) = *guard {
            return Ok(conn.clone());
        }

        let conn = (self.factory)()
            .map_err(|e| PortError::DependencyUnavailable(e))?;
        *guard = Some(conn.clone());
        Ok(conn)
    }

    /// Reset the cached connection (e.g., after a disconnect).
    pub fn reset(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            *guard = None;
        }
    }

    /// Whether a connection is currently cached.
    pub fn is_connected(&self) -> bool {
        self.inner.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    /// The configured connect timeout.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}
