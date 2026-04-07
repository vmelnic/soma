//! Plugin trait — the contract every SOMA plugin implements (Whitepaper Section 6).

use serde::Serialize;
use std::fmt;

/// A value that can be passed between plugins and the mind.
#[derive(Debug, Clone, Serialize)]
pub enum Value {
    None,
    Int(i64),
    Bool(bool),
    String(String),
    Bytes(Vec<u8>),
    List(Vec<String>),
    Map(Vec<(String, String)>),
    Handle(u64), // opaque handle (fd, pointer, etc.)
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::None => write!(f, "(none)"),
            Value::Int(n) => write!(f, "{}", n),
            Value::Bool(b) => write!(f, "{}", b),
            Value::String(s) => write!(f, "{}", s),
            Value::Bytes(b) => write!(f, "[{} bytes]", b.len()),
            Value::List(v) => write!(f, "[{} items]", v.len()),
            Value::Map(m) => {
                let pairs: Vec<std::string::String> = m.iter().take(3)
                    .map(|(k, v)| format!("{}={}", k, v)).collect();
                write!(f, "{{{}}}", pairs.join(", "))
            }
            Value::Handle(h) => write!(f, "handle:{}", h),
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
#[derive(Debug, Clone, Default, Serialize)]
pub struct PluginPermissions {
    /// Filesystem paths this plugin may access (e.g. ["/tmp", "/var/data"])
    pub filesystem: Vec<String>,
    /// Network hosts/ports this plugin may contact (e.g. ["localhost:5432"])
    pub network: Vec<String>,
    /// Environment variables this plugin may read (e.g. ["DATABASE_URL"])
    pub env_vars: Vec<String>,
}

/// The trait every SOMA plugin implements (Whitepaper Section 6.2).
pub trait SomaPlugin: Send + Sync {
    /// Plugin identity
    fn name(&self) -> &str;
    fn version(&self) -> &str { "0.1.0" }
    fn trust_level(&self) -> TrustLevel { TrustLevel::BuiltIn }

    /// Calling conventions this plugin provides
    fn conventions(&self) -> Vec<Convention>;

    /// Execute a calling convention by ID
    fn execute(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError>;

    /// Plugin dependencies (Section 6.7)
    fn dependencies(&self) -> Vec<PluginDependency> { Vec::new() }

    /// Least-privilege permission declarations (Section 12.2)
    fn permissions(&self) -> PluginPermissions { PluginPermissions::default() }

    /// Lifecycle
    fn on_load(&mut self) -> Result<(), PluginError> { Ok(()) }
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
}

/// Argument specification for a convention (Whitepaper Section 6.1).
#[derive(Debug, Clone, Serialize)]
pub struct ArgSpec {
    pub name: String,
    pub arg_type: String, // "string", "int", "bytes", "handle", "any"
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
    /// Return type description
    pub return_type: String,
    /// Expected latency in milliseconds (for scheduling)
    pub estimated_latency_ms: u32,
    /// Cleanup convention to call on error (Section 6.7)
    pub cleanup_convention: Option<u32>,
}
