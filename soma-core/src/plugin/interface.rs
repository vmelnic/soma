//! Plugin trait — the contract every SOMA plugin implements (Whitepaper Section 6).

use serde::Serialize;
use std::collections::HashMap;
use std::fmt;

/// A value that can be passed between plugins and the mind.
#[derive(Debug, Clone, Serialize)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Map(HashMap<String, Value>),
    Handle(u64), // opaque handle (fd, pointer, etc.)
    Signal(Box<crate::protocol::signal::Signal>),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => write!(f, "(null)"),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Int(n) => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{}", n),
            Value::String(s) => write!(f, "{}", s),
            Value::Bytes(b) => write!(f, "[{} bytes]", b.len()),
            Value::List(v) => write!(f, "[{} items]", v.len()),
            Value::Map(m) => {
                let pairs: Vec<std::string::String> = m.iter().take(3)
                    .map(|(k, v)| format!("{}={}", k, v)).collect();
                write!(f, "{{{}}}", pairs.join(", "))
            }
            Value::Handle(h) => write!(f, "handle:{}", h),
            Value::Signal(s) => write!(f, "signal:{:?}", s.signal_type),
        }
    }
}

/// Error from plugin execution.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("invalid argument: {0}")]
    InvalidArg(String),
    #[error("execution failed: {0}")]
    Failed(String),
    #[error("connection refused: {0}")]
    ConnectionRefused(String),
}

impl PluginError {
    /// Whether this error is transient and the operation may succeed on retry.
    pub fn is_retryable(&self) -> bool {
        match self {
            PluginError::NotFound(_) => false,
            PluginError::PermissionDenied(_) => false,
            PluginError::InvalidArg(_) => false,
            PluginError::Failed(_) => true,
            PluginError::ConnectionRefused(_) => true,
        }
    }
}

/// Plugin trust levels (Whitepaper Section 12.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TrustLevel {
    /// Full trust — built into the binary
    BuiltIn,
    /// Code-reviewed community plugin
    Community,
    /// Vendor-signed plugin (Ed25519)
    Vendor,
    /// Private in-house plugin
    Private,
    /// Untrusted — runs in WASM sandbox
    Untrusted,
}

/// Plugin dependency declaration (Whitepaper Section 6.7).
#[derive(Debug, Clone, Serialize)]
pub struct PluginDependency {
    pub name: String,
    pub required: bool,
}

/// Least-privilege permission declarations (Whitepaper Section 12.2).
/// Plugins declare what they need; the Plugin Manager enforces it.
#[derive(Debug, Clone, Serialize)]
pub struct PluginPermissions {
    /// Filesystem paths this plugin may access (e.g. ["/tmp", "/var/data"])
    pub filesystem: Vec<String>,
    /// Network hosts/ports this plugin may contact (e.g. ["localhost:5432"])
    pub network: Vec<String>,
    /// Environment variables this plugin may read (e.g. ["DATABASE_URL"])
    pub env_vars: Vec<String>,
    /// Whether this plugin can spawn child processes (e.g., MCP servers)
    pub process_spawn: bool,
}

impl Default for PluginPermissions {
    fn default() -> Self {
        Self {
            filesystem: Vec::new(),
            network: Vec::new(),
            env_vars: Vec::new(),
            process_spawn: false,
        }
    }
}

/// Per-plugin configuration passed during on_load (Section 5.2).
#[derive(Debug, Clone, Default)]
pub struct PluginConfig {
    pub settings: HashMap<String, String>,
}

impl PluginConfig {
    /// Validate config against a schema (Section 12.2).
    /// Returns list of validation errors (empty = valid).
    pub fn validate(&self, schema: &serde_json::Value) -> Vec<String> {
        let mut errors = Vec::new();
        if let Some(required) = schema.get("required") {
            if let Some(arr) = required.as_array() {
                for field in arr {
                    if let Some(name) = field.as_str() {
                        if !self.settings.contains_key(name) {
                            errors.push(format!("Missing required field: {}", name));
                        }
                    }
                }
            }
        }
        errors
    }
}

/// The trait every SOMA plugin implements (Whitepaper Section 6.2).
pub trait SomaPlugin: Send + Sync {
    /// Plugin identity
    fn name(&self) -> &str;
    fn version(&self) -> &str { "0.1.0" }
    fn description(&self) -> &str { "" }
    fn trust_level(&self) -> TrustLevel { TrustLevel::BuiltIn }

    /// Whether this plugin supports streaming execution.
    fn supports_streaming(&self) -> bool { false }

    /// Calling conventions this plugin provides
    fn conventions(&self) -> Vec<Convention>;

    /// Execute a calling convention by ID
    fn execute(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError>;

    /// Async execution with a default that delegates to sync execute.
    fn execute_async(&self, convention_id: u32, args: Vec<Value>)
        -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, PluginError>> + Send + '_>> {
        let result = self.execute(convention_id, args);
        Box::pin(async move { result })
    }

    /// Plugin dependencies (Section 6.7)
    fn dependencies(&self) -> Vec<PluginDependency> { Vec::new() }

    /// Least-privilege permission declarations (Section 12.2)
    fn permissions(&self) -> PluginPermissions { PluginPermissions::default() }

    /// Lifecycle
    fn on_load(&mut self, _config: &PluginConfig) -> Result<(), PluginError> { Ok(()) }
    fn on_unload(&mut self) -> Result<(), PluginError> { Ok(()) }

    /// Optional: pre-trained LoRA weights for this plugin (Section 6.1).
    fn lora_weights(&self) -> Option<Vec<u8>> { None }

    /// Optional: training data for the Synthesizer (Section 6.1).
    fn training_data(&self) -> Option<serde_json::Value> { None }

    /// Optional: streaming execution for convention calls (Section 6.2).
    /// Returns an iterator of intermediate values for long-running operations.
    fn execute_stream(&self, _convention_id: u32, _args: Vec<Value>) -> Result<Vec<Value>, PluginError> {
        Err(PluginError::Failed("streaming not supported".into()))
    }

    /// Optional: serialize plugin-specific state for checkpointing (Section 6.2).
    fn checkpoint_state(&self) -> Option<serde_json::Value> { None }

    /// Optional: restore plugin state from checkpoint (Section 6.2).
    fn restore_state(&mut self, _state: &serde_json::Value) -> Result<(), PluginError> { Ok(()) }

    /// Config schema for validation (Section 12.1). Returns JSON schema.
    fn config_schema(&self) -> Option<serde_json::Value> { None }
}

/// Argument type for convention args (Whitepaper Section 3.1).
#[derive(Debug, Clone, Serialize)]
pub enum ArgType {
    String,
    Int,
    Float,
    Bool,
    Bytes,
    Handle,
    Any,
}

impl ArgType {
    /// JSON Schema type string for MCP tool generation.
    pub fn json_type(&self) -> &'static str {
        match self {
            ArgType::String => "string",
            ArgType::Int | ArgType::Handle => "integer",
            ArgType::Float => "number",
            ArgType::Bool => "boolean",
            ArgType::Bytes => "string",
            ArgType::Any => "string",
        }
    }
}

/// Return type specification for a convention (Whitepaper Section 3.2).
#[derive(Debug, Clone, Serialize)]
pub enum ReturnSpec {
    /// Returns a single value of the described type.
    Value(String),
    /// Returns a stream of items of the described type.
    Stream(String),
    /// Returns an opaque handle.
    Handle,
    /// Returns nothing meaningful.
    Void,
}

/// Cleanup specification — which convention to call and how to pass the result
/// (Whitepaper Section 3.3).
#[derive(Debug, Clone, Serialize)]
pub struct CleanupSpec {
    /// Convention ID to invoke for cleanup.
    pub convention_id: u32,
    /// Which arg slot to pass the result into (0-based).
    pub pass_result_as: u8,
}

/// Declared side effect of a convention (Whitepaper Section 3.2).
#[derive(Debug, Clone, Serialize)]
pub struct SideEffect(pub String);

/// Argument specification for a convention (Whitepaper Section 6.1).
#[derive(Debug, Clone, Serialize)]
pub struct ArgSpec {
    pub name: String,
    pub arg_type: ArgType,
    pub required: bool,
    pub description: String,
}

/// A calling convention provided by a plugin (Whitepaper Section 6.1).
#[derive(Debug, Clone, Serialize)]
pub struct Convention {
    pub id: u32,
    pub name: String,
    pub description: String,
    pub call_pattern: String,
    /// Argument specifications
    pub args: Vec<ArgSpec>,
    /// Return type specification
    pub returns: ReturnSpec,
    /// Whether this convention is deterministic (same inputs → same outputs).
    pub is_deterministic: bool,
    /// Expected latency in milliseconds (for scheduling)
    pub estimated_latency_ms: u32,
    /// Maximum allowed latency in milliseconds (timeout).
    pub max_latency_ms: u32,
    /// Declared side effects of this convention.
    pub side_effects: Vec<SideEffect>,
    /// Cleanup convention to call on error (Section 6.7)
    pub cleanup: Option<CleanupSpec>,
}
