use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use semver::Version;
use uuid::Uuid;

use super::common::{
    AuthRequirements, CostProfile, DeterminismClass, IdempotenceClass, LatencyProfile,
    PortFailureClass, RiskClass, RollbackSupport, SandboxRequirements, SchemaRef, SideEffectClass,
    TrustLevel, ValidationRule,
};

/// PortKind — classification of port type.
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

/// Port lifecycle states from port-spec.md.
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

/// PortSpec — the required declaration for every port.
/// Full compliance with port-spec.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortSpec {
    pub port_id: String,
    pub name: String,
    pub version: Version,
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
    /// How this port's implementation is loaded by the bootstrap layer.
    /// Defaults to `Dylib`, preserving the original naming-convention behavior
    /// for every manifest authored before the field existed.
    #[serde(default)]
    pub backend: PortBackend,
}

/// How a port's implementation is loaded by the runtime.
///
/// Defaults to `Dylib` so existing pack manifests load unchanged. When a
/// manifest specifies `backend` explicitly, bootstrap dispatches accordingly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PortBackend {
    /// Native Rust `cdylib` loaded via libloading and the C-ABI SDK vtable.
    /// `library_name` defaults to `soma_port_{port_id}` when not set.
    Dylib {
        #[serde(default)]
        library_name: Option<String>,
    },
    /// Consume an MCP server as a port. Capabilities map onto `tools/call`,
    /// and are discovered at load time via `tools/list` unless the manifest
    /// declares them statically.
    McpClient {
        transport: McpTransport,
    },
}

impl Default for PortBackend {
    fn default() -> Self {
        PortBackend::Dylib { library_name: None }
    }
}

/// Transport used by `PortBackend::McpClient` to reach the MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransport {
    /// Spawn a local subprocess and exchange JSON-RPC 2.0 messages over
    /// stdin/stdout (newline-delimited per the MCP stdio transport). This is
    /// how ports written in any language — Node, Python, Bun, PHP, Go,
    /// Ruby — are loaded: ship an MCP server executable, point at it here.
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
        #[serde(default)]
        working_dir: Option<String>,
    },
    /// POST JSON-RPC 2.0 requests to a remote MCP server over HTTP.
    Http {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

/// Context passed into every port invocation for tracing and auth.
///
/// Callers supply this so the port runtime can record session provenance,
/// enforce auth/policy checks, and populate the tracing obligation fields on
/// each `PortCallRecord` without requiring the port adapter to know about
/// higher-level session state.
#[derive(Debug, Clone, Default)]
pub struct InvocationContext {
    /// Session that triggered this invocation.
    pub session_id: Option<Uuid>,
    /// Goal associated with the session at invocation time.
    pub goal_id: Option<String>,
    /// Identity string for the caller (local session ID or remote peer ID).
    pub caller_identity: Option<String>,
    /// Whether this invocation originates from a remote peer rather than a
    /// local session. Used to enforce remote_exposure restrictions.
    pub remote_caller: bool,
    /// Pack that owns the skill triggering this invocation.
    /// Required for per-pack policy override enforcement.
    pub pack_id: Option<String>,
    /// Pack that is making this invocation (the caller side).
    /// Used for cross-pack isolation: the runtime verifies that the
    /// calling pack has declared the target pack as a dependency
    /// before allowing cross-pack capability access.
    pub calling_pack_id: Option<String>,
}

impl InvocationContext {
    /// Convenience constructor for local (non-remote) invocations with no
    /// session context — useful in tests and direct port invocations.
    pub fn local() -> Self {
        Self::default()
    }

    /// Constructor for session-scoped invocations.
    pub fn for_session(session_id: Uuid, goal_id: Option<String>, caller_identity: Option<String>) -> Self {
        Self {
            session_id: Some(session_id),
            goal_id,
            caller_identity,
            remote_caller: false,
            pack_id: None,
            calling_pack_id: None,
        }
    }

    /// Constructor for remote peer invocations.
    pub fn remote(peer_identity: String) -> Self {
        Self {
            session_id: None,
            goal_id: None,
            caller_identity: Some(peer_identity),
            remote_caller: true,
            pack_id: None,
            calling_pack_id: None,
        }
    }
}

/// PortCapabilitySpec — each capability within a port.
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
    /// Per-capability auth override. When set, the runtime uses these
    /// requirements instead of the port-level auth_requirements, allowing
    /// individual capabilities to demand stricter auth than the port default.
    #[serde(default)]
    pub auth_override: Option<AuthRequirements>,
}
