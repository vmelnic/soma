//! SOMA Plugin SDK — the contract every SOMA plugin implements.
//!
//! This crate provides the types and traits needed to build SOMA plugins.
//! Plugins are compiled as `cdylib` crates that export a C ABI init function:
//!
//! ```rust,ignore
//! #[no_mangle]
//! pub extern "C" fn soma_plugin_init() -> *mut dyn SomaPlugin {
//!     Box::into_raw(Box::new(MyPlugin::new()))
//! }
//! ```

use serde::Serialize;
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

/// Re-export everything for convenience.
pub mod prelude {
    pub use crate::{
        ArgSpec, ArgType, CleanupSpec, Convention, PluginConfig, PluginDependency, PluginError,
        PluginPermissions, ReturnSpec, SideEffect, SomaPlugin, TrustLevel, Value,
    };
}

// ---------------------------------------------------------------------------
// Value — the universal data type
// ---------------------------------------------------------------------------

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
    /// Opaque handle (fd, pointer, connection, transaction, etc.)
    Handle(u64),
    /// Serialized Synaptic Protocol signal (opaque bytes for most plugins).
    Signal(Vec<u8>),
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
                let pairs: Vec<std::string::String> = m
                    .iter()
                    .take(3)
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect();
                write!(f, "{{{}}}", pairs.join(", "))
            }
            Value::Handle(h) => write!(f, "handle:{}", h),
            Value::Signal(s) => write!(f, "signal:[{} bytes]", s.len()),
        }
    }
}

impl Value {
    /// Extract as string reference, or error.
    pub fn as_str(&self) -> Result<&str, PluginError> {
        match self {
            Value::String(s) => Ok(s.as_str()),
            other => Err(PluginError::InvalidArg(format!(
                "expected String, got {}",
                other
            ))),
        }
    }

    /// Extract as i64, or error.
    pub fn as_int(&self) -> Result<i64, PluginError> {
        match self {
            Value::Int(n) => Ok(*n),
            other => Err(PluginError::InvalidArg(format!(
                "expected Int, got {}",
                other
            ))),
        }
    }

    /// Extract as f64, or error.
    pub fn as_float(&self) -> Result<f64, PluginError> {
        match self {
            Value::Float(n) => Ok(*n),
            Value::Int(n) => Ok(*n as f64),
            other => Err(PluginError::InvalidArg(format!(
                "expected Float, got {}",
                other
            ))),
        }
    }

    /// Extract as bool, or error.
    pub fn as_bool(&self) -> Result<bool, PluginError> {
        match self {
            Value::Bool(b) => Ok(*b),
            other => Err(PluginError::InvalidArg(format!(
                "expected Bool, got {}",
                other
            ))),
        }
    }

    /// Extract as Handle (u64), or error.
    pub fn as_handle(&self) -> Result<u64, PluginError> {
        match self {
            Value::Handle(h) => Ok(*h),
            other => Err(PluginError::InvalidArg(format!(
                "expected Handle, got {}",
                other
            ))),
        }
    }

    /// Extract as bytes, or error.
    pub fn as_bytes(&self) -> Result<&[u8], PluginError> {
        match self {
            Value::Bytes(b) => Ok(b.as_slice()),
            other => Err(PluginError::InvalidArg(format!(
                "expected Bytes, got {}",
                other
            ))),
        }
    }

}

// ---------------------------------------------------------------------------
// Plugin Error
// ---------------------------------------------------------------------------

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
            PluginError::NotFound(_) | PluginError::PermissionDenied(_) | PluginError::InvalidArg(_) => false,
            PluginError::Failed(_) | PluginError::ConnectionRefused(_) => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Trust, Dependencies, Permissions, Config
// ---------------------------------------------------------------------------

/// Plugin trust levels (Whitepaper Section 12.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TrustLevel {
    BuiltIn,
    Community,
    Vendor,
    Private,
    Untrusted,
}

/// Plugin dependency declaration (Whitepaper Section 6.7).
#[derive(Debug, Clone, Serialize)]
pub struct PluginDependency {
    pub name: String,
    pub required: bool,
}

/// Least-privilege permission declarations (Whitepaper Section 12.2).
#[derive(Debug, Clone, Serialize)]
pub struct PluginPermissions {
    pub filesystem: Vec<String>,
    pub network: Vec<String>,
    pub env_vars: Vec<String>,
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
    pub settings: HashMap<String, serde_json::Value>,
}

impl PluginConfig {
    /// Get a string setting.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.settings.get(key).and_then(|v| v.as_str())
    }

    /// Get a string setting, falling back to an environment variable.
    pub fn get_str_or_env(&self, key: &str, env_key: &str) -> Option<String> {
        if let Some(v) = self.settings.get(key).and_then(|v| v.as_str()) {
            return Some(v.to_string());
        }
        // If the value references an env var (key ends with _env), resolve it
        if let Some(env_ref) = self.settings.get(env_key).and_then(|v| v.as_str()) {
            return std::env::var(env_ref).ok();
        }
        None
    }

    /// Get an integer setting.
    pub fn get_int(&self, key: &str) -> Option<i64> {
        self.settings.get(key).and_then(|v| v.as_i64())
    }

    /// Get a boolean setting.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.settings.get(key).and_then(|v| v.as_bool())
    }

    /// Validate config against a schema. Returns list of validation errors.
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

// ---------------------------------------------------------------------------
// Convention types
// ---------------------------------------------------------------------------

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
            ArgType::String | ArgType::Bytes | ArgType::Any => "string",
            ArgType::Int | ArgType::Handle => "integer",
            ArgType::Float => "number",
            ArgType::Bool => "boolean",
        }
    }
}

/// Return type specification for a convention (Whitepaper Section 3.2).
#[derive(Debug, Clone, Serialize)]
pub enum ReturnSpec {
    Value(String),
    Stream(String),
    Handle,
    Void,
}

/// Cleanup specification — which convention to call on error (Whitepaper Section 3.3).
#[derive(Debug, Clone, Serialize)]
pub struct CleanupSpec {
    pub convention_id: u32,
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
    pub args: Vec<ArgSpec>,
    pub returns: ReturnSpec,
    pub is_deterministic: bool,
    pub estimated_latency_ms: u32,
    pub max_latency_ms: u32,
    pub side_effects: Vec<SideEffect>,
    pub cleanup: Option<CleanupSpec>,
}

// ---------------------------------------------------------------------------
// SomaPlugin trait
// ---------------------------------------------------------------------------

/// The trait every SOMA plugin implements (Whitepaper Section 6.2).
pub trait SomaPlugin: Send + Sync {
    // === Identity ===
    fn name(&self) -> &str;
    fn version(&self) -> &str {
        "0.1.0"
    }
    fn description(&self) -> &str {
        ""
    }
    fn trust_level(&self) -> TrustLevel {
        TrustLevel::BuiltIn
    }

    // === Capabilities ===
    fn supports_streaming(&self) -> bool {
        false
    }
    fn conventions(&self) -> Vec<Convention>;
    fn execute(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError>;

    /// Async execution — default delegates to sync execute.
    fn execute_async(
        &self,
        convention_id: u32,
        args: Vec<Value>,
    ) -> Pin<Box<dyn Future<Output = Result<Value, PluginError>> + Send + '_>> {
        let result = self.execute(convention_id, args);
        Box::pin(async move { result })
    }

    // === Knowledge ===
    fn lora_weights(&self) -> Option<Vec<u8>> {
        None
    }
    fn training_data(&self) -> Option<serde_json::Value> {
        None
    }

    // === Streaming ===
    fn execute_stream(
        &self,
        _convention_id: u32,
        _args: Vec<Value>,
    ) -> Result<Vec<Value>, PluginError> {
        Err(PluginError::Failed("streaming not supported".into()))
    }

    // === State Persistence ===
    fn checkpoint_state(&self) -> Option<serde_json::Value> {
        None
    }
    fn restore_state(&mut self, _state: &serde_json::Value) -> Result<(), PluginError> {
        Ok(())
    }

    // === Lifecycle ===
    fn on_load(&mut self, _config: &PluginConfig) -> Result<(), PluginError> {
        Ok(())
    }
    fn on_unload(&mut self) -> Result<(), PluginError> {
        Ok(())
    }

    // === Meta ===
    fn dependencies(&self) -> Vec<PluginDependency> {
        Vec::new()
    }
    fn permissions(&self) -> PluginPermissions {
        PluginPermissions::default()
    }
    fn config_schema(&self) -> Option<serde_json::Value> {
        None
    }
}
