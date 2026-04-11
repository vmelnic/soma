use std::collections::HashMap;
use std::time::Instant;

use chrono::Utc;
use sha2::{Digest, Sha256};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::errors::{Result, SomaError};
use crate::types::common::{AuthOutcome, CostClass, PolicyOutcome, PortFailureClass, SandboxOutcome, SideEffectClass};
use crate::types::observation::PortCallRecord;
use crate::types::port::{InvocationContext, PortCapabilitySpec, PortLifecycleState, PortSpec};

/// Runtime sandbox capabilities used to decide whether a declared port sandbox
/// can actually be satisfied before execution begins.
#[derive(Debug, Clone)]
#[derive(Default)]
pub struct RuntimeSandboxProfile {
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
// Port trait — the interface each port adapter implements
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
    /// Implementations MUST still handle external failures and classify them
    /// into the appropriate `PortFailureClass`.
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

// ---------------------------------------------------------------------------
// PortPolicyChecker — policy gate for port invocations
// ---------------------------------------------------------------------------

/// Policy check performed before a port capability is dispatched to the adapter.
///
/// The policy runtime implements this to enforce per-session, per-pack, and
/// per-peer capability restrictions. Decisions are recorded in the
/// `PortCallRecord.policy_result` field so they appear in trace.
pub trait PortPolicyChecker: Send + Sync {
    /// Check whether invoking `capability_id` on `port_id` is allowed given
    /// the current invocation context. Returns a `PolicyOutcome`.
    fn check_port_invocation(
        &self,
        port_id: &str,
        capability_id: &str,
        ctx: &InvocationContext,
    ) -> PolicyOutcome;
}

/// Permissive policy checker that allows all invocations — used when no
/// policy checker is configured.
pub struct AllowAllPortPolicy;

impl PortPolicyChecker for AllowAllPortPolicy {
    fn check_port_invocation(
        &self,
        _port_id: &str,
        _capability_id: &str,
        _ctx: &InvocationContext,
    ) -> PolicyOutcome {
        PolicyOutcome::Allowed
    }
}

// ---------------------------------------------------------------------------
// PortAuthChecker — auth gate for port invocations
// ---------------------------------------------------------------------------

/// Auth check performed before a port capability is dispatched to the adapter.
///
/// Determines whether the caller in `InvocationContext` satisfies the auth
/// requirements declared on the port spec. The result is recorded in
/// `PortCallRecord.auth_result` for every invocation.
pub trait PortAuthChecker: Send + Sync {
    fn check_port_auth(
        &self,
        port_id: &str,
        capability_id: &str,
        ctx: &InvocationContext,
        auth_requirements: &crate::types::common::AuthRequirements,
    ) -> AuthOutcome;
}

/// Default auth checker: passes when auth is not required; when auth is
/// required on a remote call, verifies that caller_identity is present.
/// Full credential verification is out of scope for the port runtime layer.
pub struct DefaultPortAuthChecker;

impl PortAuthChecker for DefaultPortAuthChecker {
    fn check_port_auth(
        &self,
        _port_id: &str,
        _capability_id: &str,
        ctx: &InvocationContext,
        auth_requirements: &crate::types::common::AuthRequirements,
    ) -> AuthOutcome {
        if !auth_requirements.required {
            return AuthOutcome::NotRequired;
        }
        // Auth is required. For remote callers, a missing identity is a hard
        // failure — the port must fail closed.
        if ctx.remote_caller && ctx.caller_identity.is_none() {
            return AuthOutcome::Failed {
                reason: "remote caller has no identity but port requires authentication".to_string(),
            };
        }
        AuthOutcome::Passed
    }
}

// ---------------------------------------------------------------------------
// PortEntry — internal bookkeeping for a registered port
// ---------------------------------------------------------------------------

/// Runtime-managed entry wrapping a port adapter and its lifecycle state.
struct PortEntry {
    spec: PortSpec,
    adapter: Box<dyn Port>,
    state: PortLifecycleState,
}

// ---------------------------------------------------------------------------
// PortRuntime trait — manages all registered ports
// ---------------------------------------------------------------------------

/// Runtime that owns, validates, and dispatches to ports.
pub trait PortRuntime: Send + Sync {
    /// Declare a port spec without attaching an adapter.
    ///
    /// The port enters `Declared` state — the spec is acknowledged but the
    /// port is not yet callable. Call `load_port` next to attach the adapter
    /// and advance through `Loaded → Validated`.
    fn declare_port(&mut self, spec: PortSpec, adapter: Box<dyn Port>) -> Result<()>;

    /// Advance a declared port to `Loaded` then `Validated` by attaching the
    /// given adapter and running full spec validation.
    ///
    /// The port must currently be in `Declared` state.
    fn load_port(&mut self, port_id: &str) -> Result<()>;

    /// Register a new port with its adapter in one step.
    ///
    /// Convenience that calls `declare_port` then `load_port` atomically,
    /// transitioning through `Declared → Loaded → Validated`. Namespace
    /// collisions are rejected.
    fn register_port(&mut self, spec: PortSpec, adapter: Box<dyn Port>) -> Result<()>;

    /// Look up a port by its `port_id`.
    fn get_port(&self, port_id: &str) -> Option<&dyn Port>;

    /// Invoke a capability on a registered port.
    ///
    /// Validates input, checks lifecycle state, dispatches to the adapter,
    /// and guarantees a `PortCallRecord` is produced even on failure.
    /// The `ctx` supplies session provenance and caller identity for tracing
    /// obligations and remote-exposure enforcement.
    fn invoke(
        &self,
        port_id: &str,
        capability_id: &str,
        input: serde_json::Value,
        ctx: &InvocationContext,
    ) -> Result<PortCallRecord>;

    /// Validate a `PortSpec` for completeness and internal consistency
    /// per port-spec.md requirements.
    fn validate_port(&self, spec: &PortSpec) -> Result<()>;

    /// List all registered port specs, optionally filtered by namespace.
    fn list_ports(&self, namespace: Option<&str>) -> Vec<&PortSpec>;

    /// Move a port to the `Quarantined` state.
    ///
    /// Quarantined ports MUST NOT be callable except for explicit recovery
    /// or inspection operations.
    fn quarantine(&mut self, port_id: &str) -> Result<()>;

    /// Move a port to the `Retired` state.
    ///
    /// Retired ports MUST be removed from normal dispatch.
    fn retire(&mut self, port_id: &str) -> Result<()>;
}

// ---------------------------------------------------------------------------
// DefaultPortRuntime — production implementation
// ---------------------------------------------------------------------------

/// Default implementation of `PortRuntime`.
///
/// Stores ports in a `HashMap<String, PortEntry>` keyed by `port_id`.
/// Enforces validation, lifecycle transitions, namespace uniqueness, and
/// capability-id uniqueness within each port.
pub struct DefaultPortRuntime {
    ports: HashMap<String, PortEntry>,
    policy_checker: Box<dyn PortPolicyChecker>,
    auth_checker: Box<dyn PortAuthChecker>,
    sandbox_profile: RuntimeSandboxProfile,
}

impl DefaultPortRuntime {
    pub fn new() -> Self {
        Self {
            ports: HashMap::new(),
            policy_checker: Box::new(AllowAllPortPolicy),
            auth_checker: Box::new(DefaultPortAuthChecker),
            sandbox_profile: RuntimeSandboxProfile::default(),
        }
    }

    /// Create a runtime with a custom policy checker.
    pub fn with_policy(policy_checker: Box<dyn PortPolicyChecker>) -> Self {
        Self {
            ports: HashMap::new(),
            policy_checker,
            auth_checker: Box::new(DefaultPortAuthChecker),
            sandbox_profile: RuntimeSandboxProfile::default(),
        }
    }

    /// Create a runtime with custom policy and auth checkers.
    pub fn with_auth(
        policy_checker: Box<dyn PortPolicyChecker>,
        auth_checker: Box<dyn PortAuthChecker>,
    ) -> Self {
        Self {
            ports: HashMap::new(),
            policy_checker,
            auth_checker,
            sandbox_profile: RuntimeSandboxProfile::default(),
        }
    }

    /// Create a runtime with explicit sandbox capabilities.
    pub fn with_sandbox_profile(sandbox_profile: RuntimeSandboxProfile) -> Self {
        Self {
            ports: HashMap::new(),
            policy_checker: Box::new(AllowAllPortPolicy),
            auth_checker: Box::new(DefaultPortAuthChecker),
            sandbox_profile,
        }
    }

    /// Activate a port that is currently in `Validated` state.
    pub fn activate(&mut self, port_id: &str) -> Result<()> {
        let entry = self
            .ports
            .get_mut(port_id)
            .ok_or_else(|| SomaError::PortNotFound(port_id.to_string()))?;

        match entry.state {
            PortLifecycleState::Validated => {
                entry.state = PortLifecycleState::Active;
                debug!(port_id, "port activated");
                Ok(())
            }
            other => Err(SomaError::Port(format!(
                "cannot activate port '{}': current state is {:?}, expected Validated",
                port_id, other
            ))),
        }
    }

    /// Mark a port as degraded. A degraded port may still serve some
    /// capabilities with reduced confidence.
    pub fn degrade(&mut self, port_id: &str) -> Result<()> {
        let entry = self
            .ports
            .get_mut(port_id)
            .ok_or_else(|| SomaError::PortNotFound(port_id.to_string()))?;

        match entry.state {
            PortLifecycleState::Active => {
                entry.state = PortLifecycleState::Degraded;
                warn!(port_id, "port degraded");
                Ok(())
            }
            other => Err(SomaError::Port(format!(
                "cannot degrade port '{}': current state is {:?}, expected Active",
                port_id, other
            ))),
        }
    }

    /// Get the runtime-managed lifecycle state for a port.
    pub fn lifecycle_state(&self, port_id: &str) -> Option<PortLifecycleState> {
        self.ports.get(port_id).map(|e| e.state)
    }

    /// Check that a capability_id exists within a port spec.
    fn find_capability<'a>(
        spec: &'a PortSpec,
        capability_id: &str,
    ) -> Option<&'a PortCapabilitySpec> {
        spec.capabilities
            .iter()
            .find(|c| c.capability_id == capability_id)
    }

    /// Compute SHA-256 hex digest of a JSON value for tracing obligations.
    fn hash_input(input: &serde_json::Value) -> String {
        let bytes = serde_json::to_vec(input).unwrap_or_default();
        let digest = Sha256::digest(&bytes);
        format!("{:x}", digest)
    }

    /// Determine whether retry is safe for a given failure class.
    ///
    /// Per port-spec.md failure model: "A port MUST report whether recovery
    /// or retry is safe." Retry is safe for transient failures; unsafe for
    /// authorization, sandbox violations, or rollback failures.
    fn retry_safe_for(failure_class: PortFailureClass) -> bool {
        matches!(
            failure_class,
            PortFailureClass::Timeout
                | PortFailureClass::DependencyUnavailable
                | PortFailureClass::TransportError
                | PortFailureClass::ExternalError
                | PortFailureClass::Unknown
        )
    }

    fn validate_json_type(value: &serde_json::Value, expected: &str) -> bool {
        match expected {
            "string" => value.is_string(),
            "number" => value.is_number(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "boolean" => value.is_boolean(),
            "object" => value.is_object(),
            "array" => value.is_array(),
            "null" => value.is_null(),
            _ => true,
        }
    }

    fn validate_schema(
        value: &serde_json::Value,
        schema: &serde_json::Value,
        path: &str,
    ) -> std::result::Result<(), String> {
        if let Some(expected_type) = schema.get("type").and_then(|v| v.as_str())
            && !Self::validate_json_type(value, expected_type)
        {
            return Err(format!(
                "{} expected type '{}' but got {}",
                path,
                expected_type,
                match value {
                    serde_json::Value::Null => "null",
                    serde_json::Value::Bool(_) => "boolean",
                    serde_json::Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
                    serde_json::Value::Number(_) => "number",
                    serde_json::Value::String(_) => "string",
                    serde_json::Value::Array(_) => "array",
                    serde_json::Value::Object(_) => "object",
                }
            ));
        }

        if let Some(allowed) = schema.get("enum").and_then(|v| v.as_array())
            && !allowed.contains(value)
        {
            return Err(format!("{} is not one of the allowed enum values", path));
        }

        if let Some(s) = value.as_str() {
            if let Some(min_len) = schema.get("minLength").and_then(|v| v.as_u64())
                && (s.len() as u64) < min_len
            {
                return Err(format!("{} is shorter than minLength {}", path, min_len));
            }
            if let Some(max_len) = schema.get("maxLength").and_then(|v| v.as_u64())
                && s.len() as u64 > max_len
            {
                return Err(format!("{} is longer than maxLength {}", path, max_len));
            }
        }

        if let Some(n) = value.as_f64() {
            if let Some(min) = schema.get("minimum").and_then(|v| v.as_f64())
                && n < min
            {
                return Err(format!("{} is below minimum {}", path, min));
            }
            if let Some(max) = schema.get("maximum").and_then(|v| v.as_f64())
                && n > max
            {
                return Err(format!("{} exceeds maximum {}", path, max));
            }
        }

        if let Some(arr) = value.as_array() {
            if let Some(min_items) = schema.get("minItems").and_then(|v| v.as_u64())
                && (arr.len() as u64) < min_items
            {
                return Err(format!("{} has fewer than minItems {}", path, min_items));
            }
            if let Some(max_items) = schema.get("maxItems").and_then(|v| v.as_u64())
                && arr.len() as u64 > max_items
            {
                return Err(format!("{} has more than maxItems {}", path, max_items));
            }
            if let Some(item_schema) = schema.get("items") {
                for (index, item) in arr.iter().enumerate() {
                    Self::validate_schema(item, item_schema, &format!("{}[{}]", path, index))?;
                }
            }
        }

        if let Some(obj) = value.as_object() {
            if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
                for key in required.iter().filter_map(|v| v.as_str()) {
                    if !obj.contains_key(key) {
                        return Err(format!("{} missing required field '{}'", path, key));
                    }
                }
            }

            if let Some(properties) = schema.get("properties").and_then(|v| v.as_object()) {
                for (key, field_schema) in properties {
                    if let Some(field_value) = obj.get(key) {
                        Self::validate_schema(
                            field_value,
                            field_schema,
                            &format!("{}.{}", path, key),
                        )?;
                    }
                }
            }

            if schema
                .get("additionalProperties")
                .and_then(|v| v.as_bool())
                == Some(false)
                && let Some(properties) = schema.get("properties").and_then(|v| v.as_object())
            {
                for key in obj.keys() {
                    if !properties.contains_key(key) {
                        return Err(format!("{} contains unexpected field '{}'", path, key));
                    }
                }
            }
        }

        Ok(())
    }

    fn validate_input_contract(
        spec: &PortSpec,
        capability: &PortCapabilitySpec,
        input: &serde_json::Value,
    ) -> std::result::Result<(), String> {
        Self::validate_schema(input, &spec.input_schema.schema, "input")?;
        Self::validate_schema(input, &capability.input_schema.schema, "input")?;
        Ok(())
    }

    fn validate_output_contract(
        spec: &PortSpec,
        capability: &PortCapabilitySpec,
        output: &serde_json::Value,
    ) -> std::result::Result<(), String> {
        Self::validate_schema(output, &spec.output_schema.schema, "structured_result")?;
        Self::validate_schema(output, &capability.output_schema.schema, "structured_result")?;
        Ok(())
    }

    fn schema_declares_field(schema: &serde_json::Value, field_path: &str) -> bool {
        let mut current = schema;
        for segment in field_path.split('.') {
            let Some(properties) = current.get("properties").and_then(|v| v.as_object()) else {
                return false;
            };
            let Some(next) = properties.get(segment) else {
                return false;
            };
            current = next;
        }
        true
    }

    fn validate_output_record(
        spec: &PortSpec,
        capability: &PortCapabilitySpec,
        record: &PortCallRecord,
    ) -> std::result::Result<(), String> {
        if record.success && record.failure_class.is_some() {
            return Err("successful invocation reported a failure_class".to_string());
        }
        if !record.success && record.failure_class.is_none() {
            return Err("failed invocation did not report a failure_class".to_string());
        }
        if !(0.0..=1.0).contains(&record.confidence) {
            return Err(format!(
                "confidence {} is outside the required 0..=1 range",
                record.confidence
            ));
        }

        Self::validate_output_contract(spec, capability, &record.structured_result)?;

        if record.success {
            for field in &spec.observable_fields {
                let pointer = format!("/{}", field.replace('.', "/"));
                if record.structured_result.pointer(&pointer).is_none() {
                    return Err(format!(
                        "observable field '{}' is missing from structured_result",
                        field,
                    ));
                }
            }
        }

        Ok(())
    }

    fn default_side_effect_summary(effect_class: SideEffectClass) -> &'static str {
        match effect_class {
            SideEffectClass::None => "none",
            SideEffectClass::ReadOnly => "read_only",
            SideEffectClass::LocalStateMutation => "local_state_mutation",
            SideEffectClass::ExternalStateMutation => "external_state_mutation",
            SideEffectClass::Destructive => "destructive",
            SideEffectClass::Irreversible => "irreversible",
        }
    }

    fn check_sandbox(
        &self,
        spec: &PortSpec,
    ) -> std::result::Result<SandboxOutcome, SandboxOutcome> {
        let req = &spec.sandbox_requirements;
        let available = &self.sandbox_profile;

        if req.filesystem_access && !available.filesystem_access {
            return Err(SandboxOutcome::Violated {
                dimension: "filesystem_access".to_string(),
                reason: "runtime sandbox does not permit filesystem access".to_string(),
            });
        }
        if req.network_access && !available.network_access {
            return Err(SandboxOutcome::Violated {
                dimension: "network_access".to_string(),
                reason: "runtime sandbox does not permit network access".to_string(),
            });
        }
        if req.device_access && !available.device_access {
            return Err(SandboxOutcome::Violated {
                dimension: "device_access".to_string(),
                reason: "runtime sandbox does not permit device access".to_string(),
            });
        }
        if req.process_access && !available.process_access {
            return Err(SandboxOutcome::Violated {
                dimension: "process_access".to_string(),
                reason: "runtime sandbox does not permit process access".to_string(),
            });
        }

        if let Some(required) = req.memory_limit_mb {
            match available.memory_limit_mb {
                Some(max) if required > max => {
                    return Err(SandboxOutcome::Violated {
                        dimension: "memory_limit_mb".to_string(),
                        reason: format!("required {}MB exceeds runtime limit {}MB", required, max),
                    });
                }
                None => {
                    return Err(SandboxOutcome::Violated {
                        dimension: "memory_limit_mb".to_string(),
                        reason: "runtime sandbox has no configurable memory limit".to_string(),
                    });
                }
                _ => {}
            }
        }
        if let Some(required) = req.cpu_limit_percent {
            match available.cpu_limit_percent {
                Some(max) if required > max => {
                    return Err(SandboxOutcome::Violated {
                        dimension: "cpu_limit_percent".to_string(),
                        reason: format!("required {}% exceeds runtime limit {}%", required, max),
                    });
                }
                None => {
                    return Err(SandboxOutcome::Violated {
                        dimension: "cpu_limit_percent".to_string(),
                        reason: "runtime sandbox has no configurable CPU limit".to_string(),
                    });
                }
                _ => {}
            }
        }
        if let (Some(required), Some(max)) = (req.time_limit_ms, available.time_limit_ms)
            && required > max
        {
            return Err(SandboxOutcome::Violated {
                dimension: "time_limit_ms".to_string(),
                reason: format!("required {}ms exceeds runtime limit {}ms", required, max),
            });
        }
        if let Some(required) = req.syscall_limit {
            match available.syscall_limit {
                Some(max) if required > max => {
                    return Err(SandboxOutcome::Violated {
                        dimension: "syscall_limit".to_string(),
                        reason: format!("required {} exceeds runtime limit {}", required, max),
                    });
                }
                None => {
                    return Err(SandboxOutcome::Violated {
                        dimension: "syscall_limit".to_string(),
                        reason: "runtime sandbox has no configurable syscall limit".to_string(),
                    });
                }
                _ => {}
            }
        }

        Ok(SandboxOutcome::Satisfied)
    }

    /// Build a failure `PortCallRecord` — guarantees observation emission
    /// even when the invocation cannot proceed.
    ///
    /// `policy_outcome` should be `Some` for all failures that occur after
    /// the policy gate (capability not found, validation failures, adapter
    /// errors). It is `None` only for pre-policy failures (port not found,
    /// lifecycle denials, remote exposure denials).
    #[allow(clippy::too_many_arguments)]
    fn failure_record(
        port_id: &str,
        capability_id: &str,
        failure_class: PortFailureClass,
        message: &str,
        latency_ms: u64,
        input_hash: Option<String>,
        ctx: &InvocationContext,
        policy_outcome: Option<PolicyOutcome>,
    ) -> PortCallRecord {
        let retry_safe = Self::retry_safe_for(failure_class);
        PortCallRecord {
            observation_id: Uuid::new_v4(),
            port_id: port_id.to_string(),
            capability_id: capability_id.to_string(),
            invocation_id: Uuid::new_v4(),
            success: false,
            failure_class: Some(failure_class),
            raw_result: serde_json::Value::Null,
            structured_result: serde_json::json!({ "error": message }),
            effect_patch: None,
            // Pre-execution denials have no side effects — the adapter was
            // never called. "none" is the correct classification.
            side_effect_summary: Some("none".to_string()),
            latency_ms,
            resource_cost: 0.0,
            confidence: 0.0,
            timestamp: Utc::now(),
            retry_safe,
            input_hash,
            session_id: ctx.session_id,
            goal_id: ctx.goal_id.clone(),
            caller_identity: ctx.caller_identity.clone(),
            auth_result: None,
            policy_result: policy_outcome,
            sandbox_result: None,
        }
    }
}

impl Default for DefaultPortRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl PortRuntime for DefaultPortRuntime {
    fn declare_port(&mut self, spec: PortSpec, adapter: Box<dyn Port>) -> Result<()> {
        // Namespace collision checks before inserting.
        if self.ports.contains_key(&spec.port_id) {
            return Err(SomaError::NamespaceCollision(format!(
                "port_id '{}' is already registered",
                spec.port_id,
            )));
        }
        let has_name_collision = self.ports.values().any(|entry| {
            entry.spec.namespace == spec.namespace && entry.spec.name == spec.name
        });
        if has_name_collision {
            return Err(SomaError::NamespaceCollision(format!(
                "port name '{}' already exists in namespace '{}'",
                spec.name, spec.namespace,
            )));
        }

        let port_id = spec.port_id.clone();
        self.ports.insert(
            port_id.clone(),
            PortEntry {
                spec,
                adapter,
                state: PortLifecycleState::Declared,
            },
        );
        debug!(port_id = %port_id, "port declared");
        Ok(())
    }

    fn load_port(&mut self, port_id: &str) -> Result<()> {
        let entry = self
            .ports
            .get_mut(port_id)
            .ok_or_else(|| SomaError::PortNotFound(port_id.to_string()))?;

        if entry.state != PortLifecycleState::Declared {
            return Err(SomaError::Port(format!(
                "cannot load port '{}': must be in Declared state, currently {:?}",
                port_id, entry.state,
            )));
        }

        // Transition Declared → Loaded: adapter is now attached.
        entry.state = PortLifecycleState::Loaded;
        debug!(port_id, "port loaded");

        // Validate the spec (runs all completeness/consistency checks).
        // Borrow the spec out for validation, then advance state.
        let spec = entry.spec.clone();
        let _ = entry;
        self.validate_port(&spec)?;

        let entry = self.ports.get_mut(port_id).unwrap();
        // Transition Loaded → Validated: spec is fully validated.
        entry.state = PortLifecycleState::Validated;
        debug!(port_id, "port validated");
        Ok(())
    }

    fn register_port(&mut self, spec: PortSpec, adapter: Box<dyn Port>) -> Result<()> {
        // Pre-validate before touching state.
        self.validate_port(&spec)?;
        let port_id = spec.port_id.clone();
        // Declare → Load → Validate atomically.
        self.declare_port(spec, adapter)?;
        self.load_port(&port_id)?;
        debug!(port_id = %port_id, "port registered");
        Ok(())
    }

    fn get_port(&self, port_id: &str) -> Option<&dyn Port> {
        // Quarantined and Retired ports must not be accessible through any
        // path that bypasses invoke(). All other states (Declared, Loaded,
        // Validated, Active, Degraded) may still expose the adapter for
        // inspection, but callers MUST go through invoke() for actual
        // capability execution to satisfy policy, auth, and tracing obligations.
        self.ports.get(port_id).and_then(|entry| {
            match entry.state {
                PortLifecycleState::Quarantined | PortLifecycleState::Retired => None,
                _ => Some(&*entry.adapter),
            }
        })
    }

    fn invoke(
        &self,
        port_id: &str,
        capability_id: &str,
        input: serde_json::Value,
        ctx: &InvocationContext,
    ) -> Result<PortCallRecord> {
        let start = Instant::now();
        let input_hash = Some(Self::hash_input(&input));

        // Look up the port entry. Unknown-port invocations still produce a
        // PortCallRecord so observation emission is guaranteed even on failure.
        let entry = match self.ports.get(port_id) {
            Some(e) => e,
            None => {
                let elapsed = start.elapsed().as_millis() as u64;
                return Ok(Self::failure_record(
                    port_id,
                    capability_id,
                    PortFailureClass::DependencyUnavailable,
                    &format!("port '{}' is not registered", port_id),
                    elapsed,
                    input_hash,
                    ctx,
                    None,
                ));
            }
        };

        // Lifecycle gate: only Active or Degraded ports may be invoked.
        // Quarantined/retired ports produce a failure observation record so
        // observation emission is guaranteed even on denial.
        match entry.state {
            PortLifecycleState::Active | PortLifecycleState::Degraded => {}
            PortLifecycleState::Quarantined => {
                let elapsed = start.elapsed().as_millis() as u64;
                return Ok(Self::failure_record(
                    port_id,
                    capability_id,
                    PortFailureClass::PolicyDenied,
                    &format!("port '{}' is quarantined and cannot be invoked", port_id),
                    elapsed,
                    input_hash,
                    ctx,
                    None,
                ));
            }
            PortLifecycleState::Retired => {
                let elapsed = start.elapsed().as_millis() as u64;
                return Ok(Self::failure_record(
                    port_id,
                    capability_id,
                    PortFailureClass::PolicyDenied,
                    &format!("port '{}' is retired and cannot be invoked", port_id),
                    elapsed,
                    input_hash,
                    ctx,
                    None,
                ));
            }
            other => {
                let elapsed = start.elapsed().as_millis() as u64;
                return Ok(Self::failure_record(
                    port_id,
                    capability_id,
                    PortFailureClass::PolicyDenied,
                    &format!("port '{}' is in {:?} state and cannot be invoked", port_id, other),
                    elapsed,
                    input_hash,
                    ctx,
                    None,
                ));
            }
        }

        // Remote exposure gate: if the caller is remote, the port and the
        // specific capability must both opt in to remote exposure.
        if ctx.remote_caller {
            if !entry.spec.remote_exposure {
                let elapsed = start.elapsed().as_millis() as u64;
                return Ok(Self::failure_record(
                    port_id,
                    capability_id,
                    PortFailureClass::PolicyDenied,
                    &format!("port '{}' does not allow remote invocation (remote_exposure=false)", port_id),
                    elapsed,
                    input_hash,
                    ctx,
                    None,
                ));
            }
            // Check the specific capability's remote_exposable flag.
            if let Some(cap) = Self::find_capability(&entry.spec, capability_id)
                && !cap.remote_exposable {
                    let elapsed = start.elapsed().as_millis() as u64;
                    return Ok(Self::failure_record(
                        port_id,
                        capability_id,
                        PortFailureClass::PolicyDenied,
                        &format!(
                            "capability '{}' on port '{}' is not remote-exposable",
                            capability_id, port_id,
                        ),
                        elapsed,
                        input_hash,
                        ctx,
                        None,
                    ));
                }
        }

        // Policy admission gate: must happen before external execution.
        // Decision is recorded in the PortCallRecord for tracing obligations.
        // RequiresConfirmation is treated as a denial — the runtime does not
        // have a confirmation mechanism at this layer, so it fails safely.
        let policy_outcome = self.policy_checker.check_port_invocation(port_id, capability_id, ctx);
        match &policy_outcome {
            PolicyOutcome::Denied { reason } => {
                let elapsed = start.elapsed().as_millis() as u64;
                return Ok(Self::failure_record(
                    port_id,
                    capability_id,
                    PortFailureClass::PolicyDenied,
                    &format!("policy denied invocation: {}", reason),
                    elapsed,
                    input_hash,
                    ctx,
                    Some(policy_outcome),
                ));
            }
            PolicyOutcome::RequiresConfirmation { reason } => {
                let elapsed = start.elapsed().as_millis() as u64;
                return Ok(Self::failure_record(
                    port_id,
                    capability_id,
                    PortFailureClass::PolicyDenied,
                    &format!("policy requires confirmation before invocation: {}", reason),
                    elapsed,
                    input_hash,
                    ctx,
                    Some(policy_outcome),
                ));
            }
            PolicyOutcome::Allowed => {}
        }

        // Verify the requested capability exists.
        let capability = if let Some(capability) = Self::find_capability(&entry.spec, capability_id) {
            capability
        } else {
            let elapsed = start.elapsed().as_millis() as u64;
            return Ok(Self::failure_record(
                port_id,
                capability_id,
                PortFailureClass::ValidationError,
                &format!("capability '{}' not found on port '{}'", capability_id, port_id),
                elapsed,
                input_hash,
                ctx,
                Some(policy_outcome),
            ));
        };

        if let Err(reason) = Self::validate_input_contract(&entry.spec, capability, &input) {
            let elapsed = start.elapsed().as_millis() as u64;
            return Ok(Self::failure_record(
                port_id,
                capability_id,
                PortFailureClass::ValidationError,
                &format!("input schema validation failed: {}", reason),
                elapsed,
                input_hash,
                ctx,
                Some(policy_outcome),
            ));
        }

        // Evaluate declared validation_rules against the input.
        // Each rule specifies a field, rule_type, and constraint value.
        // A rule whose constraint type does not match the field's type is a
        // validation failure — the field must have the right type to pass.
        for rule in &entry.spec.validation_rules {
            if let Some(field_value) = input.get(&rule.field) {
                let passes = match rule.rule_type.as_str() {
                    "required" => !field_value.is_null(),
                    "min_length" => {
                        if let (Some(s), Some(n)) = (field_value.as_str(), rule.constraint.as_u64()) {
                            s.len() as u64 >= n
                        } else {
                            false // type mismatch: field is not a string
                        }
                    }
                    "max_length" => {
                        if let (Some(s), Some(n)) = (field_value.as_str(), rule.constraint.as_u64()) {
                            s.len() as u64 <= n
                        } else {
                            false // type mismatch: field is not a string
                        }
                    }
                    "min" => {
                        if let (Some(v), Some(n)) = (field_value.as_f64(), rule.constraint.as_f64()) {
                            v >= n
                        } else {
                            false // type mismatch: field is not numeric
                        }
                    }
                    "max" => {
                        if let (Some(v), Some(n)) = (field_value.as_f64(), rule.constraint.as_f64()) {
                            v <= n
                        } else {
                            false // type mismatch: field is not numeric
                        }
                    }
                    "enum" => {
                        if let Some(allowed) = rule.constraint.as_array() {
                            allowed.contains(field_value)
                        } else {
                            false // malformed constraint: not an array
                        }
                    }
                    _ => true, // Unknown rule types: forward-compatible pass.
                };
                if !passes {
                    let elapsed = start.elapsed().as_millis() as u64;
                    return Ok(Self::failure_record(
                        port_id,
                        capability_id,
                        PortFailureClass::ValidationError,
                        &format!(
                            "validation rule '{}' failed for field '{}': value {:?} violates constraint {:?}",
                            rule.rule_type, rule.field, field_value, rule.constraint,
                        ),
                        elapsed,
                        input_hash,
                        ctx,
                        Some(policy_outcome),
                    ));
                }
            } else if rule.rule_type == "required" {
                let elapsed = start.elapsed().as_millis() as u64;
                return Ok(Self::failure_record(
                    port_id,
                    capability_id,
                    PortFailureClass::ValidationError,
                    &format!("required field '{}' is missing from input", rule.field),
                    elapsed,
                    input_hash,
                    ctx,
                    Some(policy_outcome),
                ));
            }
        }

        // Validate input before external execution.
        if let Err(e) = entry.adapter.validate_input(capability_id, &input) {
            let elapsed = start.elapsed().as_millis() as u64;
            return Ok(Self::failure_record(
                port_id,
                capability_id,
                PortFailureClass::ValidationError,
                &format!("input validation failed: {}", e),
                elapsed,
                input_hash,
                ctx,
                Some(policy_outcome),
            ));
        }

        // Auth check: use the capability's auth_override if present,
        // otherwise fall back to the port-level auth_requirements.
        let effective_auth = capability
            .auth_override
            .as_ref()
            .unwrap_or(&entry.spec.auth_requirements);
        let auth_result = self.auth_checker.check_port_auth(
            port_id,
            capability_id,
            ctx,
            effective_auth,
        );
        if let AuthOutcome::Failed { ref reason } = auth_result {
            let elapsed = start.elapsed().as_millis() as u64;
            let mut record = Self::failure_record(
                port_id,
                capability_id,
                PortFailureClass::AuthorizationDenied,
                &format!("auth check failed: {}", reason),
                elapsed,
                input_hash,
                ctx,
                Some(policy_outcome),
            );
            record.auth_result = Some(auth_result);
            return Ok(record);
        }

        let sandbox_result = match self.check_sandbox(&entry.spec) {
            Ok(outcome) => outcome,
            Err(outcome) => {
                let elapsed = start.elapsed().as_millis() as u64;
                let mut record = Self::failure_record(
                    port_id,
                    capability_id,
                    PortFailureClass::SandboxViolation,
                    &format!("sandbox requirements not satisfiable: {:?}", outcome),
                    elapsed,
                    input_hash,
                    ctx,
                    Some(policy_outcome),
                );
                record.auth_result = Some(auth_result);
                record.sandbox_result = Some(outcome);
                return Ok(record);
            }
        };

        // Dispatch to the adapter. On failure, produce a failure record
        // so observation emission is guaranteed.
        match entry.adapter.invoke(capability_id, input) {
            Ok(mut record) => {
                // Overwrite adapter-reported latency with the actual wall-clock
                // elapsed time measured by the runtime.
                let elapsed = start.elapsed().as_millis() as u64;

                // Post-hoc timeout check: if time_limit_ms is declared and
                // elapsed time exceeds it, replace this result with a Timeout
                // failure record. Preemptive cancellation requires async adapters;
                // this check enforces the contract boundary after the fact.
                if let Some(limit_ms) = entry.spec.sandbox_requirements.time_limit_ms
                    && elapsed >= limit_ms {
                        let mut timeout_record = Self::failure_record(
                            port_id,
                            capability_id,
                            PortFailureClass::Timeout,
                            &format!(
                                "port invocation exceeded declared time_limit_ms ({} >= {})",
                                elapsed, limit_ms,
                            ),
                            elapsed,
                            input_hash,
                            ctx,
                            Some(policy_outcome),
                        );
                        timeout_record.auth_result = Some(auth_result);
                        timeout_record.sandbox_result = Some(SandboxOutcome::Violated {
                            dimension: "time_limit_ms".to_string(),
                            reason: format!("elapsed {}ms exceeded limit {}ms", elapsed, limit_ms),
                        });
                        return Ok(timeout_record);
                    }

                if let Err(reason) = Self::validate_output_record(&entry.spec, capability, &record)
                {
                    let mut output_record = Self::failure_record(
                        port_id,
                        capability_id,
                        PortFailureClass::ExternalError,
                        &format!("output contract validation failed: {}", reason),
                        elapsed,
                        input_hash,
                        ctx,
                        Some(policy_outcome),
                    );
                    output_record.auth_result = Some(auth_result);
                    output_record.sandbox_result = Some(sandbox_result);
                    return Ok(output_record);
                }

                record.latency_ms = elapsed;
                record.input_hash = input_hash;
                if record.side_effect_summary.is_none() {
                    record.side_effect_summary = Some(
                        Self::default_side_effect_summary(capability.effect_class).to_string(),
                    );
                }
                // Populate tracing obligation fields from the invocation context
                // and the policy outcome recorded above.
                record.session_id = ctx.session_id;
                record.goal_id = ctx.goal_id.clone();
                record.caller_identity = ctx.caller_identity.clone();
                record.policy_result = Some(policy_outcome);
                record.auth_result = Some(auth_result);
                record.sandbox_result = Some(sandbox_result);
                Ok(record)
            }
            Err(e) => {
                let elapsed = start.elapsed().as_millis() as u64;
                let failure_class = match &e {
                    SomaError::PortInvocation { failure_class, .. } => *failure_class,
                    _ => PortFailureClass::Unknown,
                };
                let mut record = Self::failure_record(
                    port_id,
                    capability_id,
                    failure_class,
                    &format!("{}", e),
                    elapsed,
                    input_hash,
                    ctx,
                    Some(policy_outcome),
                );
                record.auth_result = Some(auth_result);
                record.sandbox_result = Some(sandbox_result);
                Ok(record)
            }
        }
    }

    fn validate_port(&self, spec: &PortSpec) -> Result<()> {
        // PortSpec completeness: required fields must be non-empty.
        if spec.port_id.is_empty() {
            return Err(SomaError::Port("port_id must not be empty".to_string()));
        }
        if spec.name.is_empty() {
            return Err(SomaError::Port("name must not be empty".to_string()));
        }
        if spec.description.is_empty() {
            return Err(SomaError::Port("description must not be empty".to_string()));
        }
        if spec.namespace.is_empty() {
            return Err(SomaError::Port("namespace must not be empty".to_string()));
        }
        if spec.capabilities.is_empty() {
            return Err(SomaError::Port(
                "capabilities must not be empty: a port must expose at least one capability"
                    .to_string(),
            ));
        }
        if spec.failure_modes.is_empty() {
            return Err(SomaError::Port(
                "failure_modes must not be empty: a port must declare explicit failure classes"
                    .to_string(),
            ));
        }

        // Capability uniqueness: no duplicate capability_ids within the port.
        let mut seen_ids = std::collections::HashSet::new();
        for cap in &spec.capabilities {
            if cap.capability_id.is_empty() {
                return Err(SomaError::Port(
                    "capability_id must not be empty".to_string(),
                ));
            }
            if cap.name.is_empty() {
                return Err(SomaError::Port(format!(
                    "capability '{}' has an empty name",
                    cap.capability_id,
                )));
            }
            if cap.purpose.is_empty() {
                return Err(SomaError::Port(format!(
                    "capability '{}' has an empty purpose",
                    cap.capability_id,
                )));
            }
            if !seen_ids.insert(&cap.capability_id) {
                return Err(SomaError::Port(format!(
                    "duplicate capability_id '{}' in port '{}'",
                    cap.capability_id, spec.port_id,
                )));
            }
        }

        // Observable fields: the spec says observable_fields is required.
        // An empty list is permitted (the port simply has no observable fields),
        // but we validate that each entry is non-empty if present.
        for field in &spec.observable_fields {
            if field.is_empty() {
                return Err(SomaError::Port(
                    "observable_fields entries must not be empty strings".to_string(),
                ));
            }
            let declared_by_port = Self::schema_declares_field(&spec.output_schema.schema, field);
            let declared_by_capability = spec
                .capabilities
                .iter()
                .any(|cap| Self::schema_declares_field(&cap.output_schema.schema, field));
            if !declared_by_port && !declared_by_capability {
                return Err(SomaError::Port(format!(
                    "observable field '{}' is not declared by any output schema",
                    field,
                )));
            }
        }

        // Latency profile sanity.
        if spec.latency_profile.expected_latency_ms > spec.latency_profile.p95_latency_ms {
            return Err(SomaError::Port(
                "latency_profile: expected_latency_ms must not exceed p95_latency_ms".to_string(),
            ));
        }
        if spec.latency_profile.p95_latency_ms > spec.latency_profile.max_latency_ms {
            return Err(SomaError::Port(
                "latency_profile: p95_latency_ms must not exceed max_latency_ms".to_string(),
            ));
        }

        // Validate each capability's latency profile.
        for cap in &spec.capabilities {
            if cap.latency_profile.expected_latency_ms > cap.latency_profile.p95_latency_ms {
                return Err(SomaError::Port(format!(
                    "capability '{}': expected_latency_ms must not exceed p95_latency_ms",
                    cap.capability_id,
                )));
            }
            if cap.latency_profile.p95_latency_ms > cap.latency_profile.max_latency_ms {
                return Err(SomaError::Port(format!(
                    "capability '{}': p95_latency_ms must not exceed max_latency_ms",
                    cap.capability_id,
                )));
            }
        }

        // Validate each capability's cost profile against its effect class.
        // A destructive or irreversible capability that claims negligible cost
        // on every dimension is suspect — such operations inherently carry
        // resource cost, so all-negligible is almost certainly a misconfiguration.
        for cap in &spec.capabilities {
            let is_high_impact = matches!(
                cap.effect_class,
                SideEffectClass::Destructive | SideEffectClass::Irreversible
            );
            if is_high_impact {
                let cp = &cap.cost_profile;
                let all_negligible = cp.cpu_cost_class == CostClass::Negligible
                    && cp.memory_cost_class == CostClass::Negligible
                    && cp.io_cost_class == CostClass::Negligible
                    && cp.network_cost_class == CostClass::Negligible
                    && cp.energy_cost_class == CostClass::Negligible;
                if all_negligible {
                    return Err(SomaError::Port(format!(
                        "capability '{}': cost_profile has all dimensions set to Negligible \
                         but effect_class is {:?} — destructive or irreversible operations \
                         cannot plausibly have negligible cost on every dimension",
                        cap.capability_id, cap.effect_class,
                    )));
                }
            }
        }

        // Auth metadata: if auth is required, at least one method must be
        // declared. Per port-spec.md: "A port MUST fail closed when required
        // auth is missing."
        if spec.auth_requirements.required && spec.auth_requirements.methods.is_empty() {
            return Err(SomaError::Port(
                "auth_requirements.required is true but no auth methods are declared".to_string(),
            ));
        }

        // Schema reference validation: input_schema and output_schema must not
        // be empty (a null or empty-object schema provides no contract).
        if spec.input_schema.schema.is_null() || spec.input_schema.schema == serde_json::json!({}) {
            return Err(SomaError::Port(format!(
                "port '{}': input_schema must declare a non-empty schema",
                spec.port_id,
            )));
        }
        if spec.output_schema.schema.is_null() || spec.output_schema.schema == serde_json::json!({}) {
            return Err(SomaError::Port(format!(
                "port '{}': output_schema must declare a non-empty schema",
                spec.port_id,
            )));
        }
        for cap in &spec.capabilities {
            if cap.input_schema.schema.is_null() || cap.input_schema.schema == serde_json::json!({}) {
                return Err(SomaError::Port(format!(
                    "capability '{}': input_schema must declare a non-empty schema",
                    cap.capability_id,
                )));
            }
            if cap.output_schema.schema.is_null() || cap.output_schema.schema == serde_json::json!({}) {
                return Err(SomaError::Port(format!(
                    "capability '{}': output_schema must declare a non-empty schema",
                    cap.capability_id,
                )));
            }
        }

        // Sandbox metadata consistency: at minimum, a port that declares no
        // network access must not set a nonzero network-dependent time limit
        // as its only limit. More importantly, all limits must be internally
        // coherent (no zero-ms time limit, which would make the port unusable).
        if let Some(t) = spec.sandbox_requirements.time_limit_ms
            && t == 0 {
                return Err(SomaError::Port(format!(
                    "port '{}': sandbox_requirements.time_limit_ms must not be zero",
                    spec.port_id,
                )));
            }
        if let Some(m) = spec.sandbox_requirements.memory_limit_mb
            && m == 0 {
                return Err(SomaError::Port(format!(
                    "port '{}': sandbox_requirements.memory_limit_mb must not be zero",
                    spec.port_id,
                )));
            }

        // Trust level adequacy: ports with high-risk side effects must declare
        // a correspondingly high trust level so policy can enforce correctly.
        match spec.side_effect_class {
            SideEffectClass::Irreversible => {
                if spec.trust_level < crate::types::common::TrustLevel::Verified {
                    return Err(SomaError::Port(format!(
                        "port '{}': irreversible side effects require trust_level >= Verified",
                        spec.port_id,
                    )));
                }
            }
            SideEffectClass::Destructive => {
                if spec.trust_level < crate::types::common::TrustLevel::Restricted {
                    return Err(SomaError::Port(format!(
                        "port '{}': destructive side effects require trust_level >= Restricted",
                        spec.port_id,
                    )));
                }
            }
            _ => {}
        }

        // Remote exposure eligibility (port-spec.md Remote Exposure Constraints):
        // A port MAY be exposed remotely only if:
        // - its PortSpec explicitly allows remote exposure (checked here)
        // - its auth requirements are satisfiable remotely
        // - its side effects are declared (we verify they are not under-declared)
        if spec.remote_exposure {
            // Remote-exposed ports MUST have auth requirements.
            if !spec.auth_requirements.required {
                return Err(SomaError::Port(
                    "remote_exposure is true but auth_requirements.required is false: \
                     remote-exposed ports must require authentication"
                        .to_string(),
                ));
            }

            // Sandbox dimensions that require local-only enforcement cannot be
            // guaranteed on a remote host. Reject ports that claim remote exposure
            // while depending on process or device isolation.
            if spec.sandbox_requirements.process_access {
                return Err(SomaError::Port(format!(
                    "port '{}': remote_exposure is true but sandbox requires process_access, \
                     which cannot be enforced remotely",
                    spec.port_id,
                )));
            }
            if spec.sandbox_requirements.device_access {
                return Err(SomaError::Port(format!(
                    "port '{}': remote_exposure is true but sandbox requires device_access, \
                     which cannot be enforced remotely",
                    spec.port_id,
                )));
            }

            // All remote-exposable capabilities must have their side effects
            // declared (i.e., not under-declared). We verify that each
            // remote_exposable capability has an explicit effect_class.
            for cap in &spec.capabilities {
                if cap.remote_exposable {
                    // Irreversible side effects on a remote capability are high-risk;
                    // the port must have trust_level >= Verified.
                    if cap.effect_class == SideEffectClass::Irreversible
                        && spec.trust_level
                            < crate::types::common::TrustLevel::Verified
                    {
                        return Err(SomaError::Port(format!(
                            "capability '{}' is remote_exposable with irreversible side effects \
                             but port trust_level is below Verified",
                            cap.capability_id,
                        )));
                    }
                }
            }
        }

        // Namespace collision: verify no other port with the same port_id or
        // (namespace, name) pair is already registered. Excludes self — when
        // called from load_port(), the spec under validation is already in the
        // map, so we count matches and reject only if another port collides.
        let id_collisions = self
            .ports
            .values()
            .filter(|e| e.spec.port_id == spec.port_id)
            .count();
        if id_collisions > 1 {
            return Err(SomaError::NamespaceCollision(format!(
                "port_id '{}' is already registered by another port",
                spec.port_id,
            )));
        }
        let name_collisions = self.ports.values().filter(|e| {
            e.spec.namespace == spec.namespace
                && e.spec.name == spec.name
                && e.spec.port_id != spec.port_id
        }).count();
        if name_collisions > 0 {
            return Err(SomaError::NamespaceCollision(format!(
                "port name '{}' already exists in namespace '{}'",
                spec.name, spec.namespace,
            )));
        }

        Ok(())
    }

    fn list_ports(&self, namespace: Option<&str>) -> Vec<&PortSpec> {
        self.ports
            .values()
            .filter(|entry| {
                matches!(
                    entry.state,
                    PortLifecycleState::Active | PortLifecycleState::Degraded
                ) && match namespace {
                    Some(ns) => entry.spec.namespace == ns,
                    None => true,
                }
            })
            .map(|entry| &entry.spec)
            .collect()
    }

    fn quarantine(&mut self, port_id: &str) -> Result<()> {
        let entry = self
            .ports
            .get_mut(port_id)
            .ok_or_else(|| SomaError::PortNotFound(port_id.to_string()))?;

        match entry.state {
            PortLifecycleState::Active | PortLifecycleState::Degraded => {
                entry.state = PortLifecycleState::Quarantined;
                warn!(port_id, "port quarantined");
                Ok(())
            }
            PortLifecycleState::Quarantined => {
                // Already quarantined — idempotent.
                Ok(())
            }
            other => Err(SomaError::Port(format!(
                "cannot quarantine port '{}': current state is {:?}",
                port_id, other,
            ))),
        }
    }

    fn retire(&mut self, port_id: &str) -> Result<()> {
        let entry = self
            .ports
            .get_mut(port_id)
            .ok_or_else(|| SomaError::PortNotFound(port_id.to_string()))?;

        match entry.state {
            PortLifecycleState::Active
            | PortLifecycleState::Degraded
            | PortLifecycleState::Quarantined => {
                entry.state = PortLifecycleState::Retired;
                warn!(port_id, "port retired");
                Ok(())
            }
            PortLifecycleState::Retired => {
                // Already retired — idempotent.
                Ok(())
            }
            other => Err(SomaError::Port(format!(
                "cannot retire port '{}': current state is {:?}",
                port_id, other,
            ))),
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
    use crate::types::port::*;
    use semver::Version;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Build a minimal valid PortCapabilitySpec for testing.
    fn test_capability(id: &str) -> PortCapabilitySpec {
        PortCapabilitySpec {
            capability_id: id.to_string(),
            name: format!("cap-{}", id),
            purpose: "testing".to_string(),
            input_schema: SchemaRef {
                schema: serde_json::json!({"type": "object"}),
            },
            output_schema: SchemaRef {
                schema: serde_json::json!({"type": "object"}),
            },
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
        }
    }

    /// Build a minimal valid PortSpec for testing.
    fn test_port_spec(id: &str) -> PortSpec {
        PortSpec {
            port_id: id.to_string(),
            name: format!("test-port-{}", id),
            version: Version::new(1, 0, 0),
            kind: PortKind::Database,
            description: "A test port".to_string(),
            namespace: "test".to_string(),
            trust_level: TrustLevel::Trusted,
            capabilities: vec![test_capability("read"), test_capability("write")],
            input_schema: SchemaRef {
                schema: serde_json::json!({"type": "object"}),
            },
            output_schema: SchemaRef {
                schema: serde_json::json!({"type": "object"}),
            },
            failure_modes: vec![PortFailureClass::Timeout],
            side_effect_class: SideEffectClass::ExternalStateMutation,
            latency_profile: LatencyProfile {
                expected_latency_ms: 10,
                p95_latency_ms: 50,
                max_latency_ms: 200,
            },
            cost_profile: CostProfile {
                cpu_cost_class: CostClass::Low,
                memory_cost_class: CostClass::Low,
                io_cost_class: CostClass::Medium,
                network_cost_class: CostClass::Low,
                energy_cost_class: CostClass::Negligible,
            },
            auth_requirements: AuthRequirements {
                methods: vec![AuthMethod::BearerToken],
                required: true,
            },
            sandbox_requirements: SandboxRequirements {
                filesystem_access: false,
                network_access: false,
                device_access: false,
                process_access: false,
                memory_limit_mb: None,
                cpu_limit_percent: None,
                time_limit_ms: Some(5000),
                syscall_limit: None,
            },
            observable_fields: vec![],
            validation_rules: vec![],
            remote_exposure: false,
            backend: crate::types::port::PortBackend::default(),
        }
    }

    /// A trivial port adapter for testing.
    struct StubPort {
        spec: PortSpec,
    }

    impl StubPort {
        fn new(spec: PortSpec) -> Self {
            Self { spec }
        }
    }

    impl Port for StubPort {
        fn spec(&self) -> &PortSpec {
            &self.spec
        }

        fn invoke(
            &self,
            capability_id: &str,
            _input: serde_json::Value,
        ) -> Result<PortCallRecord> {
            Ok(PortCallRecord {
                observation_id: Uuid::new_v4(),
                port_id: self.spec.port_id.clone(),
                capability_id: capability_id.to_string(),
                invocation_id: Uuid::new_v4(),
                success: true,
                failure_class: None,
                raw_result: serde_json::json!({"ok": true, "rows_affected": 0}),
                structured_result: serde_json::json!({"ok": true, "rows_affected": 0}),
                effect_patch: None,
                side_effect_summary: Some("none".to_string()),
                latency_ms: 1,
                resource_cost: 0.01,
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
            })
        }

        fn validate_input(
            &self,
            _capability_id: &str,
            _input: &serde_json::Value,
        ) -> Result<()> {
            Ok(())
        }

        fn lifecycle_state(&self) -> PortLifecycleState {
            PortLifecycleState::Active
        }
    }

    // -- Registration tests --

    #[test]
    fn register_and_get_port() {
        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("pg");
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec.clone())))
            .unwrap();

        let port = rt.get_port("pg").expect("port should exist");
        assert_eq!(port.spec().port_id, "pg");
    }

    #[test]
    fn register_duplicate_port_id_fails() {
        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("pg");
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec.clone())))
            .unwrap();

        let result = rt.register_port(spec.clone(), Box::new(StubPort::new(spec)));
        assert!(result.is_err());
    }

    #[test]
    fn register_namespace_name_collision_fails() {
        let mut rt = DefaultPortRuntime::new();
        let spec1 = test_port_spec("pg1");
        let mut spec2 = test_port_spec("pg2");
        // Same namespace + name as spec1.
        spec2.name = spec1.name.clone();
        spec2.namespace = spec1.namespace.clone();

        rt.register_port(spec1.clone(), Box::new(StubPort::new(spec1)))
            .unwrap();
        let result = rt.register_port(spec2.clone(), Box::new(StubPort::new(spec2)));
        assert!(result.is_err());
    }

    // -- Validation tests --

    #[test]
    fn validate_empty_port_id_fails() {
        let rt = DefaultPortRuntime::new();
        let mut spec = test_port_spec("pg");
        spec.port_id = String::new();
        assert!(rt.validate_port(&spec).is_err());
    }

    #[test]
    fn validate_empty_capabilities_fails() {
        let rt = DefaultPortRuntime::new();
        let mut spec = test_port_spec("pg");
        spec.capabilities.clear();
        assert!(rt.validate_port(&spec).is_err());
    }

    #[test]
    fn validate_empty_failure_modes_fails() {
        let rt = DefaultPortRuntime::new();
        let mut spec = test_port_spec("pg");
        spec.failure_modes.clear();
        assert!(rt.validate_port(&spec).is_err());
    }

    #[test]
    fn validate_duplicate_capability_ids_fails() {
        let rt = DefaultPortRuntime::new();
        let mut spec = test_port_spec("pg");
        spec.capabilities = vec![test_capability("dup"), test_capability("dup")];
        assert!(rt.validate_port(&spec).is_err());
    }

    #[test]
    fn validate_bad_latency_profile_fails() {
        let rt = DefaultPortRuntime::new();
        let mut spec = test_port_spec("pg");
        spec.latency_profile = LatencyProfile {
            expected_latency_ms: 100,
            p95_latency_ms: 50, // expected > p95, invalid
            max_latency_ms: 200,
        };
        assert!(rt.validate_port(&spec).is_err());
    }

    // -- Lifecycle tests --

    #[test]
    fn activate_validated_port() {
        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("pg");
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec)))
            .unwrap();

        assert_eq!(
            rt.lifecycle_state("pg"),
            Some(PortLifecycleState::Validated)
        );
        rt.activate("pg").unwrap();
        assert_eq!(rt.lifecycle_state("pg"), Some(PortLifecycleState::Active));
    }

    #[test]
    fn quarantine_active_port() {
        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("pg");
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec)))
            .unwrap();
        rt.activate("pg").unwrap();
        rt.quarantine("pg").unwrap();
        assert_eq!(
            rt.lifecycle_state("pg"),
            Some(PortLifecycleState::Quarantined)
        );
    }

    #[test]
    fn retire_quarantined_port() {
        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("pg");
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec)))
            .unwrap();
        rt.activate("pg").unwrap();
        rt.quarantine("pg").unwrap();
        rt.retire("pg").unwrap();
        assert_eq!(rt.lifecycle_state("pg"), Some(PortLifecycleState::Retired));
    }

    #[test]
    fn degrade_active_port() {
        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("pg");
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec)))
            .unwrap();
        rt.activate("pg").unwrap();
        rt.degrade("pg").unwrap();
        assert_eq!(
            rt.lifecycle_state("pg"),
            Some(PortLifecycleState::Degraded)
        );
    }

    // -- Invocation tests --

    #[test]
    fn invoke_active_port_succeeds() {
        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("pg");
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec)))
            .unwrap();
        rt.activate("pg").unwrap();

        let record = rt
            .invoke("pg", "read", serde_json::json!({}), &InvocationContext::local())
            .unwrap();
        assert!(record.success);
        assert_eq!(record.port_id, "pg");
        assert_eq!(record.capability_id, "read");
    }

    #[test]
    fn invoke_degraded_port_still_works() {
        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("pg");
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec)))
            .unwrap();
        rt.activate("pg").unwrap();
        rt.degrade("pg").unwrap();

        let record = rt
            .invoke("pg", "read", serde_json::json!({}), &InvocationContext::local())
            .unwrap();
        assert!(record.success);
    }

    #[test]
    fn invoke_quarantined_port_returns_policy_denied_observation() {
        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("pg");
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec)))
            .unwrap();
        rt.activate("pg").unwrap();
        rt.quarantine("pg").unwrap();

        // Per port-spec.md: observation emission MUST happen even when
        // policy denies execution. Returns Ok(failure_record) not Err.
        let record = rt.invoke("pg", "read", serde_json::json!({}), &InvocationContext::local()).unwrap();
        assert!(!record.success);
        assert_eq!(record.failure_class, Some(PortFailureClass::PolicyDenied));
        assert!(!record.retry_safe);
        assert!(record.input_hash.is_some());
    }

    #[test]
    fn invoke_retired_port_returns_policy_denied_observation() {
        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("pg");
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec)))
            .unwrap();
        rt.activate("pg").unwrap();
        rt.retire("pg").unwrap();

        // Per port-spec.md: observation emission MUST happen even when
        // policy denies execution. Returns Ok(failure_record) not Err.
        let record = rt.invoke("pg", "read", serde_json::json!({}), &InvocationContext::local()).unwrap();
        assert!(!record.success);
        assert_eq!(record.failure_class, Some(PortFailureClass::PolicyDenied));
        assert!(!record.retry_safe);
    }

    #[test]
    fn invoke_unknown_capability_returns_failure_record() {
        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("pg");
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec)))
            .unwrap();
        rt.activate("pg").unwrap();

        let record = rt
            .invoke("pg", "nonexistent", serde_json::json!({}), &InvocationContext::local())
            .unwrap();
        assert!(!record.success);
        assert_eq!(
            record.failure_class,
            Some(PortFailureClass::ValidationError)
        );
    }

    #[test]
    fn invoke_nonexistent_port_returns_failure_record() {
        let rt = DefaultPortRuntime::new();
        let record = rt
            .invoke("no-such-port", "read", serde_json::json!({}), &InvocationContext::local())
            .unwrap();
        // Observation MUST be emitted even when the port is not registered.
        assert!(!record.success);
        assert_eq!(record.port_id, "no-such-port");
        assert_eq!(record.failure_class, Some(PortFailureClass::DependencyUnavailable));
    }

    #[test]
    fn invoke_with_validation_failure_returns_failure_record() {
        /// A port whose validate_input always rejects.
        struct RejectingPort {
            spec: PortSpec,
        }
        impl Port for RejectingPort {
            fn spec(&self) -> &PortSpec {
                &self.spec
            }
            fn invoke(
                &self,
                _cap: &str,
                _input: serde_json::Value,
            ) -> Result<PortCallRecord> {
                unreachable!("should not be called after validation failure")
            }
            fn validate_input(
                &self,
                _cap: &str,
                _input: &serde_json::Value,
            ) -> Result<()> {
                Err(SomaError::Port("bad input".to_string()))
            }
            fn lifecycle_state(&self) -> PortLifecycleState {
                PortLifecycleState::Active
            }
        }

        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("rej");
        rt.register_port(spec.clone(), Box::new(RejectingPort { spec }))
            .unwrap();
        rt.activate("rej").unwrap();

        let record = rt
            .invoke("rej", "read", serde_json::json!({"bad": true}), &InvocationContext::local())
            .unwrap();
        assert!(!record.success);
        assert_eq!(
            record.failure_class,
            Some(PortFailureClass::ValidationError)
        );
    }

    #[test]
    fn invoke_adapter_error_produces_failure_record() {
        /// A port whose invoke always fails.
        struct FailingPort {
            spec: PortSpec,
        }
        impl Port for FailingPort {
            fn spec(&self) -> &PortSpec {
                &self.spec
            }
            fn invoke(
                &self,
                cap: &str,
                _input: serde_json::Value,
            ) -> Result<PortCallRecord> {
                Err(SomaError::PortInvocation {
                    port_id: self.spec.port_id.clone(),
                    capability_id: cap.to_string(),
                    failure_class: PortFailureClass::ExternalError,
                })
            }
            fn validate_input(
                &self,
                _cap: &str,
                _input: &serde_json::Value,
            ) -> Result<()> {
                Ok(())
            }
            fn lifecycle_state(&self) -> PortLifecycleState {
                PortLifecycleState::Active
            }
        }

        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("fail");
        rt.register_port(spec.clone(), Box::new(FailingPort { spec }))
            .unwrap();
        rt.activate("fail").unwrap();

        // The runtime catches the error and wraps it in a failure record
        // to guarantee observation emission.
        let record = rt
            .invoke("fail", "read", serde_json::json!({}), &InvocationContext::local())
            .unwrap();
        assert!(!record.success);
        assert_eq!(
            record.failure_class,
            Some(PortFailureClass::ExternalError)
        );
    }

    // -- Listing tests --

    #[test]
    fn list_ports_all() {
        let mut rt = DefaultPortRuntime::new();
        let spec1 = test_port_spec("pg");
        let mut spec2 = test_port_spec("redis");
        spec2.namespace = "cache".to_string();
        spec2.name = "redis-port".to_string();

        rt.register_port(spec1.clone(), Box::new(StubPort::new(spec1)))
            .unwrap();
        rt.register_port(spec2.clone(), Box::new(StubPort::new(spec2)))
            .unwrap();
        rt.activate("pg").unwrap();
        rt.activate("redis").unwrap();

        assert_eq!(rt.list_ports(None).len(), 2);
    }

    #[test]
    fn list_ports_by_namespace() {
        let mut rt = DefaultPortRuntime::new();
        let spec1 = test_port_spec("pg");
        let mut spec2 = test_port_spec("redis");
        spec2.namespace = "cache".to_string();
        spec2.name = "redis-port".to_string();

        rt.register_port(spec1.clone(), Box::new(StubPort::new(spec1)))
            .unwrap();
        rt.register_port(spec2.clone(), Box::new(StubPort::new(spec2)))
            .unwrap();
        rt.activate("pg").unwrap();
        rt.activate("redis").unwrap();

        let test_ports = rt.list_ports(Some("test"));
        assert_eq!(test_ports.len(), 1);
        assert_eq!(test_ports[0].port_id, "pg");

        let cache_ports = rt.list_ports(Some("cache"));
        assert_eq!(cache_ports.len(), 1);
        assert_eq!(cache_ports[0].port_id, "redis");
    }

    #[test]
    fn list_ports_hides_non_callable_states() {
        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("pg");
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec)))
            .unwrap();

        assert!(rt.list_ports(None).is_empty());

        rt.activate("pg").unwrap();
        assert_eq!(rt.list_ports(None).len(), 1);

        rt.quarantine("pg").unwrap();
        assert!(rt.list_ports(None).is_empty());
    }

    #[test]
    fn get_nonexistent_port_returns_none() {
        let rt = DefaultPortRuntime::new();
        assert!(rt.get_port("nope").is_none());
    }

    // -- Edge cases --

    #[test]
    fn quarantine_is_idempotent() {
        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("pg");
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec)))
            .unwrap();
        rt.activate("pg").unwrap();
        rt.quarantine("pg").unwrap();
        // Second quarantine should be idempotent.
        rt.quarantine("pg").unwrap();
    }

    #[test]
    fn retire_is_idempotent() {
        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("pg");
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec)))
            .unwrap();
        rt.activate("pg").unwrap();
        rt.retire("pg").unwrap();
        // Second retire should be idempotent.
        rt.retire("pg").unwrap();
    }

    #[test]
    fn activate_non_validated_port_fails() {
        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("pg");
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec)))
            .unwrap();
        rt.activate("pg").unwrap();
        // Already active — cannot re-activate.
        assert!(rt.activate("pg").is_err());
    }

    // -- Observation model compliance --

    #[test]
    fn observation_has_all_required_fields() {
        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("pg");
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec)))
            .unwrap();
        rt.activate("pg").unwrap();

        let record = rt
            .invoke("pg", "read", serde_json::json!({}), &InvocationContext::local())
            .unwrap();

        // Per port-spec.md Observation Model: 14 required fields.
        assert!(!record.observation_id.is_nil());
        assert_eq!(record.port_id, "pg");
        assert_eq!(record.capability_id, "read");
        assert!(!record.invocation_id.is_nil());
        assert!(record.success);
        assert!(record.failure_class.is_none());
        // raw_result and structured_result are present.
        assert!(!record.raw_result.is_null());
        assert!(!record.structured_result.is_null());
        // latency_ms, resource_cost, confidence, timestamp are set.
        assert!(record.confidence >= 0.0);
        assert!(record.timestamp <= Utc::now());
        assert_eq!(record.sandbox_result, Some(SandboxOutcome::Satisfied));
    }

    #[test]
    fn failure_record_contains_retry_safety() {
        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("pg");
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec)))
            .unwrap();
        rt.activate("pg").unwrap();

        // Unknown capability => ValidationError => retry is NOT safe.
        let record = rt
            .invoke("pg", "nonexistent", serde_json::json!({}), &InvocationContext::local())
            .unwrap();
        assert!(!record.success);
        assert!(!record.retry_safe);
    }

    #[test]
    fn failure_record_contains_input_hash() {
        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("pg");
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec)))
            .unwrap();
        rt.activate("pg").unwrap();

        let record = rt
            .invoke("pg", "nonexistent", serde_json::json!({"key": "value"}), &InvocationContext::local())
            .unwrap();
        assert!(record.input_hash.is_some());
        let hash = record.input_hash.as_ref().unwrap();
        // SHA-256 produces 64 hex characters.
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn successful_invocation_contains_input_hash() {
        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("pg");
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec)))
            .unwrap();
        rt.activate("pg").unwrap();

        let record = rt
            .invoke("pg", "read", serde_json::json!({"key": "value"}), &InvocationContext::local())
            .unwrap();
        assert!(record.success);
        assert!(record.input_hash.is_some());
        assert_eq!(record.input_hash.as_ref().unwrap().len(), 64);
    }

    // -- Auth metadata validation --

    #[test]
    fn validate_auth_required_but_no_methods_fails() {
        let rt = DefaultPortRuntime::new();
        let mut spec = test_port_spec("pg");
        spec.auth_requirements.required = true;
        spec.auth_requirements.methods = vec![];
        assert!(rt.validate_port(&spec).is_err());
    }

    #[test]
    fn validate_auth_not_required_with_no_methods_passes() {
        let rt = DefaultPortRuntime::new();
        let mut spec = test_port_spec("pg");
        spec.auth_requirements.required = false;
        spec.auth_requirements.methods = vec![];
        assert!(rt.validate_port(&spec).is_ok());
    }

    // -- Remote exposure eligibility --

    #[test]
    fn validate_remote_exposure_requires_auth() {
        let rt = DefaultPortRuntime::new();
        let mut spec = test_port_spec("pg");
        spec.remote_exposure = true;
        spec.auth_requirements.required = false;
        let result = rt.validate_port(&spec);
        assert!(result.is_err());
    }

    #[test]
    fn validate_remote_exposure_with_auth_passes() {
        let rt = DefaultPortRuntime::new();
        let mut spec = test_port_spec("pg");
        spec.remote_exposure = true;
        spec.auth_requirements.required = true;
        spec.auth_requirements.methods = vec![AuthMethod::BearerToken];
        let result = rt.validate_port(&spec);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_remote_irreversible_needs_verified_trust() {
        let rt = DefaultPortRuntime::new();
        let mut spec = test_port_spec("pg");
        spec.remote_exposure = true;
        spec.auth_requirements.required = true;
        spec.auth_requirements.methods = vec![AuthMethod::BearerToken];
        // Capability with irreversible side effects + remote_exposable.
        spec.capabilities = vec![{
            let mut cap = test_capability("danger");
            cap.effect_class = SideEffectClass::Irreversible;
            cap.remote_exposable = true;
            cap
        }];
        // Trust level below Verified should fail.
        spec.trust_level = TrustLevel::Restricted;
        assert!(rt.validate_port(&spec).is_err());

        // Trust level Verified should pass.
        spec.trust_level = TrustLevel::Verified;
        assert!(rt.validate_port(&spec).is_ok());
    }

    #[test]
    fn validate_remote_exposure_rejects_process_access() {
        let rt = DefaultPortRuntime::new();
        let mut spec = test_port_spec("proc");
        spec.remote_exposure = true;
        spec.auth_requirements.required = true;
        spec.auth_requirements.methods = vec![AuthMethod::BearerToken];
        spec.sandbox_requirements.process_access = true;
        let result = rt.validate_port(&spec);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("process_access"));
    }

    #[test]
    fn validate_remote_exposure_rejects_device_access() {
        let rt = DefaultPortRuntime::new();
        let mut spec = test_port_spec("dev");
        spec.remote_exposure = true;
        spec.auth_requirements.required = true;
        spec.auth_requirements.methods = vec![AuthMethod::BearerToken];
        spec.sandbox_requirements.device_access = true;
        let result = rt.validate_port(&spec);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("device_access"));
    }

    #[test]
    fn validate_remote_exposure_allows_network_and_filesystem() {
        let rt = DefaultPortRuntime::new();
        let mut spec = test_port_spec("net");
        spec.remote_exposure = true;
        spec.auth_requirements.required = true;
        spec.auth_requirements.methods = vec![AuthMethod::BearerToken];
        spec.sandbox_requirements.network_access = true;
        spec.sandbox_requirements.filesystem_access = true;
        spec.sandbox_requirements.process_access = false;
        spec.sandbox_requirements.device_access = false;
        assert!(rt.validate_port(&spec).is_ok());
    }

    #[test]
    fn invoke_rejects_input_that_violates_declared_schema() {
        let mut rt = DefaultPortRuntime::new();
        let mut spec = test_port_spec("schema");
        spec.input_schema = SchemaRef {
            schema: serde_json::json!({
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": { "type": "string", "minLength": 3 }
                },
                "additionalProperties": false
            }),
        };
        spec.capabilities[0].input_schema = spec.input_schema.clone();
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec)))
            .unwrap();
        rt.activate("schema").unwrap();

        let record = rt
            .invoke("schema", "read", serde_json::json!({"query": 42}), &InvocationContext::local())
            .unwrap();
        assert!(!record.success);
        assert_eq!(record.failure_class, Some(PortFailureClass::ValidationError));
    }

    #[test]
    fn invoke_rejects_output_that_violates_declared_schema() {
        struct BadOutputPort {
            spec: PortSpec,
        }

        impl Port for BadOutputPort {
            fn spec(&self) -> &PortSpec {
                &self.spec
            }

            fn invoke(
                &self,
                capability_id: &str,
                _input: serde_json::Value,
            ) -> Result<PortCallRecord> {
                Ok(PortCallRecord {
                    observation_id: Uuid::new_v4(),
                    port_id: self.spec.port_id.clone(),
                    capability_id: capability_id.to_string(),
                    invocation_id: Uuid::new_v4(),
                    success: true,
                    failure_class: None,
                    raw_result: serde_json::json!({"ok": true}),
                    structured_result: serde_json::json!({"rows": "wrong-type"}),
                    effect_patch: None,
                    side_effect_summary: Some("none".to_string()),
                    latency_ms: 1,
                    resource_cost: 0.01,
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
                })
            }

            fn validate_input(
                &self,
                _capability_id: &str,
                _input: &serde_json::Value,
            ) -> Result<()> {
                Ok(())
            }

            fn lifecycle_state(&self) -> PortLifecycleState {
                PortLifecycleState::Active
            }
        }

        let mut rt = DefaultPortRuntime::new();
        let mut spec = test_port_spec("bad-output");
        spec.output_schema = SchemaRef {
            schema: serde_json::json!({
                "type": "object",
                "required": ["rows"],
                "properties": {
                    "rows": { "type": "integer" }
                }
            }),
        };
        spec.capabilities[0].output_schema = spec.output_schema.clone();
        rt.register_port(spec.clone(), Box::new(BadOutputPort { spec }))
            .unwrap();
        rt.activate("bad-output").unwrap();

        let record = rt
            .invoke("bad-output", "read", serde_json::json!({}), &InvocationContext::local())
            .unwrap();
        assert!(!record.success);
        assert_eq!(record.failure_class, Some(PortFailureClass::ExternalError));
    }

    #[test]
    fn invoke_denies_when_runtime_cannot_satisfy_sandbox() {
        struct FlagPort {
            spec: PortSpec,
            invoked: AtomicBool,
        }

        impl Port for FlagPort {
            fn spec(&self) -> &PortSpec {
                &self.spec
            }

            fn invoke(
                &self,
                capability_id: &str,
                _input: serde_json::Value,
            ) -> Result<PortCallRecord> {
                self.invoked.store(true, Ordering::SeqCst);
                Ok(PortCallRecord {
                    observation_id: Uuid::new_v4(),
                    port_id: self.spec.port_id.clone(),
                    capability_id: capability_id.to_string(),
                    invocation_id: Uuid::new_v4(),
                    success: true,
                    failure_class: None,
                    raw_result: serde_json::json!({"ok": true}),
                    structured_result: serde_json::json!({"ok": true}),
                    effect_patch: None,
                    side_effect_summary: Some("none".to_string()),
                    latency_ms: 1,
                    resource_cost: 0.01,
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
                })
            }

            fn validate_input(
                &self,
                _capability_id: &str,
                _input: &serde_json::Value,
            ) -> Result<()> {
                Ok(())
            }

            fn lifecycle_state(&self) -> PortLifecycleState {
                PortLifecycleState::Active
            }
        }

        let mut spec = test_port_spec("sandbox");
        spec.sandbox_requirements.network_access = true;

        let mut rt = DefaultPortRuntime::with_sandbox_profile(RuntimeSandboxProfile {
            network_access: false,
            ..RuntimeSandboxProfile::default()
        });
        rt.register_port(
            spec.clone(),
            Box::new(FlagPort {
                spec,
                invoked: AtomicBool::new(false),
            }),
        )
        .unwrap();
        rt.activate("sandbox").unwrap();

        let record = rt
            .invoke("sandbox", "read", serde_json::json!({}), &InvocationContext::local())
            .unwrap();
        assert!(!record.success);
        assert_eq!(record.failure_class, Some(PortFailureClass::SandboxViolation));
        assert!(matches!(
            record.sandbox_result,
            Some(SandboxOutcome::Violated { .. })
        ));
    }

    #[test]
    fn default_runtime_fails_closed_for_declared_network_access() {
        let mut spec = test_port_spec("networked");
        spec.sandbox_requirements.network_access = true;

        let mut rt = DefaultPortRuntime::with_sandbox_profile(RuntimeSandboxProfile {
            network_access: false,
            ..RuntimeSandboxProfile::default()
        });
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec)))
            .unwrap();
        rt.activate("networked").unwrap();

        let record = rt
            .invoke("networked", "read", serde_json::json!({}), &InvocationContext::local())
            .unwrap();
        assert!(!record.success);
        assert_eq!(record.failure_class, Some(PortFailureClass::SandboxViolation));
        assert!(matches!(
            record.sandbox_result,
            Some(SandboxOutcome::Violated { dimension, .. }) if dimension == "network_access"
        ));
    }

    #[test]
    fn validate_observable_fields_must_exist_in_output_schema() {
        let rt = DefaultPortRuntime::new();
        let mut spec = test_port_spec("obs");
        spec.observable_fields = vec!["missing.path".to_string()];
        assert!(rt.validate_port(&spec).is_err());
    }

    #[test]
    fn invoke_missing_observable_field_returns_failure_record() {
        struct MissingObservablePort {
            spec: PortSpec,
        }

        impl Port for MissingObservablePort {
            fn spec(&self) -> &PortSpec {
                &self.spec
            }

            fn invoke(
                &self,
                capability_id: &str,
                _input: serde_json::Value,
            ) -> Result<PortCallRecord> {
                Ok(PortCallRecord {
                    observation_id: Uuid::new_v4(),
                    port_id: self.spec.port_id.clone(),
                    capability_id: capability_id.to_string(),
                    invocation_id: Uuid::new_v4(),
                    success: true,
                    failure_class: None,
                    raw_result: serde_json::json!({"ok": true}),
                    structured_result: serde_json::json!({"ok": true}),
                    effect_patch: None,
                    side_effect_summary: Some("none".to_string()),
                    latency_ms: 1,
                    resource_cost: 0.01,
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
                })
            }

            fn validate_input(
                &self,
                _capability_id: &str,
                _input: &serde_json::Value,
            ) -> Result<()> {
                Ok(())
            }

            fn lifecycle_state(&self) -> PortLifecycleState {
                PortLifecycleState::Active
            }
        }

        let mut rt = DefaultPortRuntime::new();
        let mut spec = test_port_spec("observable-runtime");
        spec.observable_fields = vec!["rows_affected".to_string()];
        spec.output_schema = SchemaRef {
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "rows_affected": { "type": "integer" }
                }
            }),
        };
        spec.capabilities[0].output_schema = spec.output_schema.clone();
        rt.register_port(spec.clone(), Box::new(MissingObservablePort { spec }))
            .unwrap();
        rt.activate("observable-runtime").unwrap();

        let record = rt
            .invoke(
                "observable-runtime",
                "read",
                serde_json::json!({}),
                &InvocationContext::local(),
            )
            .unwrap();
        assert!(!record.success);
        assert_eq!(record.failure_class, Some(PortFailureClass::ExternalError));
    }

    #[test]
    fn invoke_fills_missing_side_effect_summary() {
        struct NoSummaryPort {
            spec: PortSpec,
        }

        impl Port for NoSummaryPort {
            fn spec(&self) -> &PortSpec {
                &self.spec
            }

            fn invoke(
                &self,
                capability_id: &str,
                _input: serde_json::Value,
            ) -> Result<PortCallRecord> {
                Ok(PortCallRecord {
                    observation_id: Uuid::new_v4(),
                    port_id: self.spec.port_id.clone(),
                    capability_id: capability_id.to_string(),
                    invocation_id: Uuid::new_v4(),
                    success: true,
                    failure_class: None,
                    raw_result: serde_json::json!({"ok": true}),
                    structured_result: serde_json::json!({"rows_affected": 1}),
                    effect_patch: None,
                    side_effect_summary: None,
                    latency_ms: 1,
                    resource_cost: 0.01,
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
                })
            }

            fn validate_input(
                &self,
                _capability_id: &str,
                _input: &serde_json::Value,
            ) -> Result<()> {
                Ok(())
            }

            fn lifecycle_state(&self) -> PortLifecycleState {
                PortLifecycleState::Active
            }
        }

        let mut rt = DefaultPortRuntime::new();
        let mut spec = test_port_spec("summary");
        spec.output_schema = SchemaRef {
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "rows_affected": { "type": "integer" }
                }
            }),
        };
        spec.capabilities[0].output_schema = spec.output_schema.clone();
        rt.register_port(spec.clone(), Box::new(NoSummaryPort { spec }))
            .unwrap();
        rt.activate("summary").unwrap();

        let record = rt
            .invoke("summary", "read", serde_json::json!({}), &InvocationContext::local())
            .unwrap();
        assert!(record.success);
        assert_eq!(record.side_effect_summary.as_deref(), Some("read_only"));
    }

    // -- Non-callable lifecycle state observation --

    #[test]
    fn invoke_validated_but_not_active_returns_observation() {
        let mut rt = DefaultPortRuntime::new();
        let spec = test_port_spec("pg");
        rt.register_port(spec.clone(), Box::new(StubPort::new(spec)))
            .unwrap();
        // Port is Validated (not yet Active).
        let record = rt
            .invoke("pg", "read", serde_json::json!({}), &InvocationContext::local())
            .unwrap();
        assert!(!record.success);
        assert_eq!(record.failure_class, Some(PortFailureClass::PolicyDenied));
    }

    // -- Retry safety by failure class --

    #[test]
    fn retry_safe_for_timeout() {
        assert!(DefaultPortRuntime::retry_safe_for(PortFailureClass::Timeout));
    }

    #[test]
    fn retry_not_safe_for_authorization_denied() {
        assert!(!DefaultPortRuntime::retry_safe_for(
            PortFailureClass::AuthorizationDenied
        ));
    }

    #[test]
    fn retry_not_safe_for_sandbox_violation() {
        assert!(!DefaultPortRuntime::retry_safe_for(
            PortFailureClass::SandboxViolation
        ));
    }

    #[test]
    fn retry_not_safe_for_policy_denied() {
        assert!(!DefaultPortRuntime::retry_safe_for(
            PortFailureClass::PolicyDenied
        ));
    }

    #[test]
    fn retry_safe_for_transport_error() {
        assert!(DefaultPortRuntime::retry_safe_for(
            PortFailureClass::TransportError
        ));
    }

    #[test]
    fn retry_safe_for_dependency_unavailable() {
        assert!(DefaultPortRuntime::retry_safe_for(
            PortFailureClass::DependencyUnavailable
        ));
    }
}
