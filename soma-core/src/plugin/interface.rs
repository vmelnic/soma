//! Plugin trait — the contract every SOMA plugin implements.

use std::fmt;

/// A value that can be passed between plugins and the mind.
#[derive(Debug, Clone)]
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
                let pairs: Vec<String> = m.iter().take(3)
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
}

/// The trait every SOMA plugin implements (Whitepaper Section 5.2).
pub trait SomaPlugin: Send + Sync {
    /// Plugin identity
    fn name(&self) -> &str;
    fn version(&self) -> &str { "0.1.0" }

    /// Calling conventions this plugin provides
    fn conventions(&self) -> Vec<Convention>;

    /// Execute a calling convention by ID
    fn execute(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError>;

    /// Lifecycle
    fn on_load(&mut self) -> Result<(), PluginError> { Ok(()) }
    fn on_unload(&mut self) -> Result<(), PluginError> { Ok(()) }
}

/// A calling convention provided by a plugin.
#[derive(Debug, Clone)]
pub struct Convention {
    pub id: u32,
    pub name: String,
    pub description: String,
    pub call_pattern: String,
}
