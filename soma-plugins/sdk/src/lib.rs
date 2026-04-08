//! SOMA Plugin SDK -- shared interface types for building external plugins.
//!
//! This crate defines the contract between the SOMA runtime (`soma-core`) and
//! dynamically loaded plugins.  Every catalog plugin (`crypto`, `postgres`,
//! `redis`, `auth`, `geo`, `http-bridge`) depends on this crate and implements
//! the [`SomaPlugin`] trait.
//!
//! # Plugin loading
//!
//! Plugins are compiled as `cdylib` crates.  The runtime discovers them by
//! scanning a plugin directory for shared libraries, then calls the C ABI init
//! function each library must export:
//!
//! ```rust,ignore
//! #[no_mangle]
//! pub extern "C" fn soma_plugin_init() -> *mut dyn SomaPlugin {
//!     Box::into_raw(Box::new(MyPlugin::new()))
//! }
//! ```
//!
//! # Convention routing
//!
//! Each plugin is assigned a `plugin_idx` at registration time.  Convention IDs
//! are offset by `plugin_idx * 1000` to prevent routing conflicts across
//! plugins.  See `soma-core/src/plugin/manager.rs` for details.

use serde::Serialize;
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

/// Re-exports of every public type for convenient `use soma_plugin_sdk::prelude::*`.
pub mod prelude {
    pub use crate::{
        ArgSpec, ArgType, CleanupSpec, Convention, PluginConfig, PluginDependency, PluginError,
        PluginPermissions, ReturnSpec, SideEffect, SomaPlugin, TrustLevel, Value,
    };
}

// ---------------------------------------------------------------------------
// Value
// ---------------------------------------------------------------------------

/// The universal data type exchanged between the Mind, runtime, and plugins.
///
/// `Value` is intentionally a closed enum rather than a trait object so that
/// every plugin speaks exactly the same serialization vocabulary.  The ten
/// variants cover all data that flows through SOMA conventions:
///
/// | Variant   | Typical use |
/// |-----------|-------------|
/// | `Null`    | Optional return, absence of data |
/// | `Bool`    | Flags, conditions |
/// | `Int`     | Counts, IDs, timestamps (epoch millis) |
/// | `Float`   | Measurements, scores |
/// | `String`  | Text, SQL, paths, URLs |
/// | `Bytes`   | Binary blobs, encrypted payloads |
/// | `List`    | Ordered collections (rows, results) |
/// | `Map`     | Key-value structures (JSON-like objects) |
/// | `Handle`  | Opaque resource references -- see below |
/// | `Signal`  | Serialized Synaptic Protocol signals -- see below |
///
/// ## Why `Handle` is a bare `u64`
///
/// Handles represent file descriptors, database connections, transaction IDs,
/// and other opaque resources that only the owning plugin can interpret.  A
/// raw `u64` avoids any lifetime or ownership coupling between the runtime
/// and the plugin's internal resource table.
///
/// ## Why `Signal` is `Vec<u8>` instead of `Box<Signal>`
///
/// The Synaptic Protocol `Signal` type lives in `soma-core`, which this SDK
/// crate must not depend on (the dependency arrow points the other way).
/// Storing the wire bytes as an opaque `Vec<u8>` keeps the SDK free of
/// circular dependencies while still allowing plugins to forward signals.
#[derive(Debug, Clone, Serialize)]
pub enum Value {
    /// Absence of a value (analogous to JSON `null`).
    Null,
    /// Boolean flag.
    Bool(bool),
    /// Signed 64-bit integer.
    Int(i64),
    /// 64-bit floating-point number.
    Float(f64),
    /// UTF-8 text.
    String(String),
    /// Arbitrary binary data.
    Bytes(Vec<u8>),
    /// Ordered sequence of values (heterogeneous).
    List(Vec<Self>),
    /// String-keyed map of values (heterogeneous).
    Map(HashMap<String, Self>),
    /// Opaque resource handle managed by the owning plugin.
    Handle(u64),
    /// Serialized Synaptic Protocol signal (opaque to most plugins).
    Signal(Vec<u8>),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => write!(f, "(null)"),
            Self::Bool(b) => write!(f, "{b}"),
            Self::Int(n) => write!(f, "{n}"),
            Self::Float(n) => write!(f, "{n}"),
            Self::String(s) => write!(f, "{s}"),
            Self::Bytes(b) => write!(f, "[{} bytes]", b.len()),
            Self::List(v) => write!(f, "[{} items]", v.len()),
            Self::Map(m) => {
                let pairs: Vec<std::string::String> = m
                    .iter()
                    .take(3)
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect();
                write!(f, "{{{}}}", pairs.join(", "))
            }
            Self::Handle(h) => write!(f, "handle:{h}"),
            Self::Signal(s) => write!(f, "signal:[{} bytes]", s.len()),
        }
    }
}

impl Value {
    /// Extracts a `&str` reference, returning an error if the variant is not
    /// `Value::String`.
    pub fn as_str(&self) -> Result<&str, PluginError> {
        match self {
            Self::String(s) => Ok(s.as_str()),
            other => Err(PluginError::InvalidArg(format!(
                "expected String, got {other}"
            ))),
        }
    }

    /// Extracts an `i64`, returning an error if the variant is not
    /// `Value::Int`.
    pub fn as_int(&self) -> Result<i64, PluginError> {
        match self {
            Self::Int(n) => Ok(*n),
            other => Err(PluginError::InvalidArg(format!(
                "expected Int, got {other}"
            ))),
        }
    }

    /// Extracts an `f64`, coercing `Value::Int` to `f64` for convenience.
    ///
    /// The `i64`-to-`f64` cast can lose precision for integers wider than 52
    /// bits, but this matches the behavior callers expect when they ask for a
    /// numeric value without caring about the exact representation.
    #[allow(clippy::cast_precision_loss)]
    pub fn as_float(&self) -> Result<f64, PluginError> {
        match self {
            Self::Float(n) => Ok(*n),
            Self::Int(n) => Ok(*n as f64),
            other => Err(PluginError::InvalidArg(format!(
                "expected Float, got {other}"
            ))),
        }
    }

    /// Extracts a `bool`, returning an error if the variant is not
    /// `Value::Bool`.
    pub fn as_bool(&self) -> Result<bool, PluginError> {
        match self {
            Self::Bool(b) => Ok(*b),
            other => Err(PluginError::InvalidArg(format!(
                "expected Bool, got {other}"
            ))),
        }
    }

    /// Extracts a handle (`u64`), returning an error if the variant is not
    /// `Value::Handle`.
    pub fn as_handle(&self) -> Result<u64, PluginError> {
        match self {
            Self::Handle(h) => Ok(*h),
            other => Err(PluginError::InvalidArg(format!(
                "expected Handle, got {other}"
            ))),
        }
    }

    /// Extracts a byte-slice reference, returning an error if the variant is
    /// not `Value::Bytes`.
    pub fn as_bytes(&self) -> Result<&[u8], PluginError> {
        match self {
            Self::Bytes(b) => Ok(b.as_slice()),
            other => Err(PluginError::InvalidArg(format!(
                "expected Bytes, got {other}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// PluginError
// ---------------------------------------------------------------------------

/// Structured error returned by plugin convention execution.
///
/// Variants are split into *permanent* failures (the caller should not retry)
/// and *transient* failures (retrying may succeed).  Use [`is_retryable`]
/// to distinguish them programmatically.
///
/// [`is_retryable`]: PluginError::is_retryable
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    /// The requested resource (file, key, row) does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The caller lacks the required permissions.
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    /// An argument failed validation (wrong type, out of range, etc.).
    #[error("invalid argument: {0}")]
    InvalidArg(String),
    /// General execution failure (I/O error, query error, etc.).
    #[error("execution failed: {0}")]
    Failed(String),
    /// A network connection could not be established.
    #[error("connection refused: {0}")]
    ConnectionRefused(String),
}

impl PluginError {
    /// Returns `true` for transient errors (`Failed`, `ConnectionRefused`)
    /// where retrying the operation may succeed.  Returns `false` for
    /// permanent errors (`NotFound`, `PermissionDenied`, `InvalidArg`).
    pub const fn is_retryable(&self) -> bool {
        match self {
            Self::NotFound(_) | Self::PermissionDenied(_) | Self::InvalidArg(_) => false,
            Self::Failed(_) | Self::ConnectionRefused(_) => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Trust, Dependencies, Permissions, Config
// ---------------------------------------------------------------------------

/// Trust level assigned to a plugin, governing what the runtime allows it to
/// do (Whitepaper Section 12.2).
///
/// Higher trust levels unlock broader permissions.  `BuiltIn` plugins ship
/// with the runtime itself; `Untrusted` plugins run with maximal sandboxing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TrustLevel {
    /// Ships with `soma-core` (e.g., `PosixPlugin`).
    BuiltIn,
    /// Contributed by the community, signature verified.
    Community,
    /// Published by a known vendor.
    Vendor,
    /// Private / internal to an organization.
    Private,
    /// Unknown provenance -- maximum sandboxing.
    Untrusted,
}

/// Declares that a plugin depends on another plugin being loaded first
/// (Whitepaper Section 6.7).
///
/// The runtime performs a topological sort of dependencies at startup and
/// refuses to load a plugin whose *required* dependencies are missing.
#[derive(Debug, Clone, Serialize)]
pub struct PluginDependency {
    /// Name of the depended-upon plugin (e.g., `"postgres"`).
    pub name: String,
    /// If `true`, the runtime will refuse to load this plugin without the
    /// dependency.  If `false`, the dependency is advisory.
    pub required: bool,
}

/// Least-privilege permission declarations for a plugin (Whitepaper Section
/// 12.2).
///
/// Plugins declare the resources they need upfront.  The runtime enforces
/// these declarations, rejecting operations that exceed the declared scope.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PluginPermissions {
    /// Filesystem paths the plugin may access (e.g., `["/tmp", "/var/data"]`).
    pub filesystem: Vec<String>,
    /// Network hosts/ports the plugin may connect to (e.g.,
    /// `["localhost:5432"]`).
    pub network: Vec<String>,
    /// Environment variable names the plugin may read.
    pub env_vars: Vec<String>,
    /// Whether the plugin may spawn child processes.
    pub process_spawn: bool,
}

/// Per-plugin configuration passed to [`SomaPlugin::on_load`].
///
/// Settings are deserialized from the `[plugins.<name>]` section of
/// `soma.toml`.  Helper methods provide typed access with optional
/// environment-variable fallback.
#[derive(Debug, Clone, Default)]
pub struct PluginConfig {
    /// Raw key-value settings from `soma.toml`.
    pub settings: HashMap<String, serde_json::Value>,
}

impl PluginConfig {
    /// Returns a string setting, or `None` if the key is absent or not a
    /// string.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.settings.get(key).and_then(|v| v.as_str())
    }

    /// Returns a string setting, falling back to an environment variable.
    ///
    /// First checks `key` in settings.  If absent, looks up `env_key` in
    /// settings -- if that value names an environment variable, resolves it.
    /// This two-step lookup lets `soma.toml` use indirection like
    /// `password_env = "PGPASSWORD"`.
    pub fn get_str_or_env(&self, key: &str, env_key: &str) -> Option<String> {
        if let Some(v) = self.settings.get(key).and_then(|v| v.as_str()) {
            return Some(v.to_string());
        }
        if let Some(env_ref) = self.settings.get(env_key).and_then(|v| v.as_str()) {
            return std::env::var(env_ref).ok();
        }
        None
    }

    /// Returns an integer setting, or `None` if absent or wrong type.
    pub fn get_int(&self, key: &str) -> Option<i64> {
        self.settings.get(key).and_then(serde_json::Value::as_i64)
    }

    /// Returns a boolean setting, or `None` if absent or wrong type.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.settings.get(key).and_then(serde_json::Value::as_bool)
    }

    /// Validates settings against a JSON-Schema-like `schema` object.
    ///
    /// Currently checks the `"required"` array: any field name listed there
    /// but absent from `settings` produces an error string.  Returns an empty
    /// `Vec` if validation passes.
    pub fn validate(&self, schema: &serde_json::Value) -> Vec<String> {
        let mut errors = Vec::new();
        if let Some(required) = schema.get("required")
            && let Some(arr) = required.as_array()
        {
            for field in arr {
                if let Some(name) = field.as_str()
                    && !self.settings.contains_key(name)
                {
                    errors.push(format!("Missing required field: {name}"));
                }
            }
        }
        errors
    }
}

// ---------------------------------------------------------------------------
// Convention types
// ---------------------------------------------------------------------------

/// The data type of a convention argument (Whitepaper Section 3.1).
///
/// Maps directly to [`Value`] variants, except `Any` which accepts all of
/// them.  Used for argument validation before a convention is invoked.
#[derive(Debug, Clone, Serialize)]
pub enum ArgType {
    /// UTF-8 text.
    String,
    /// Signed 64-bit integer.
    Int,
    /// 64-bit floating-point number.
    Float,
    /// Boolean flag.
    Bool,
    /// Arbitrary binary data.
    Bytes,
    /// Opaque resource handle.
    Handle,
    /// Accepts any `Value` variant.
    Any,
}

impl ArgType {
    /// Returns the JSON Schema type string for MCP tool generation.
    ///
    /// `Bytes` and `Any` map to `"string"` because JSON has no binary type
    /// and `Any` must degrade gracefully.
    pub const fn json_type(&self) -> &'static str {
        match self {
            Self::String | Self::Bytes | Self::Any => "string",
            Self::Int | Self::Handle => "integer",
            Self::Float => "number",
            Self::Bool => "boolean",
        }
    }
}

/// What a convention returns (Whitepaper Section 3.2).
#[derive(Debug, Clone, Serialize)]
pub enum ReturnSpec {
    /// A single value of the described type (e.g., `"string"`, `"map"`).
    Value(String),
    /// A streaming sequence of values of the described element type.
    Stream(String),
    /// An opaque resource handle.
    Handle,
    /// No meaningful return value.
    Void,
}

/// Names the convention to call for cleanup on error (Whitepaper Section 3.3).
///
/// When a program step fails, the runtime invokes the cleanup convention,
/// passing the partial result in argument slot `pass_result_as`.
#[derive(Debug, Clone, Serialize)]
pub struct CleanupSpec {
    /// Convention ID of the cleanup handler.
    pub convention_id: u32,
    /// Argument index where the failed step's result is passed.
    pub pass_result_as: u8,
}

/// A declared side effect of a convention (Whitepaper Section 3.2).
///
/// Side effects are advisory strings (e.g., `"writes filesystem"`,
/// `"sends network"`) used by the Mind during program generation to reason
/// about ordering and rollback.
#[derive(Debug, Clone, Serialize)]
pub struct SideEffect(pub String);

/// Specification of a single argument to a convention (Whitepaper Section 6.1).
#[derive(Debug, Clone, Serialize)]
pub struct ArgSpec {
    /// Argument name used in training data and MCP tool schemas.
    pub name: String,
    /// Expected data type.
    pub arg_type: ArgType,
    /// Whether this argument must be supplied by the caller.
    pub required: bool,
    /// Human-readable description (also surfaced via MCP).
    pub description: String,
}

/// A named operation ("calling convention") that a plugin exposes to the Mind
/// (Whitepaper Section 6.1).
///
/// Conventions are the atomic building blocks that the Mind assembles into
/// execution programs.  Each convention declares its arguments, return type,
/// latency bounds, side effects, and optional cleanup handler.
#[derive(Debug, Clone, Serialize)]
pub struct Convention {
    /// Unique ID within the plugin (before `plugin_idx * 1000` offset).
    pub id: u32,
    /// Dot-separated name (e.g., `"fs.read_file"`, `"db.query"`).
    pub name: String,
    /// Human-readable description used for training data and MCP tools.
    pub description: String,
    /// Invocation pattern (e.g., `"sync"`, `"async"`, `"stream"`).
    pub call_pattern: String,
    /// Ordered list of argument specifications.
    pub args: Vec<ArgSpec>,
    /// What this convention returns.
    pub returns: ReturnSpec,
    /// If `true`, repeated calls with identical args produce identical results.
    pub is_deterministic: bool,
    /// Typical execution time in milliseconds (used by the Mind for planning).
    pub estimated_latency_ms: u32,
    /// Hard timeout in milliseconds; the runtime kills the call if exceeded.
    pub max_latency_ms: u32,
    /// Advisory side-effect declarations.
    pub side_effects: Vec<SideEffect>,
    /// Optional cleanup convention invoked on failure.
    pub cleanup: Option<CleanupSpec>,
}

// ---------------------------------------------------------------------------
// SomaPlugin trait
// ---------------------------------------------------------------------------

/// The trait every SOMA plugin must implement (Whitepaper Section 6.2).
///
/// The runtime loads plugins as trait objects (`Box<dyn SomaPlugin>`) and
/// interacts with them exclusively through this interface.  Only [`name`],
/// [`conventions`], and [`execute`] are required; all other methods have
/// sensible defaults.
///
/// [`name`]: SomaPlugin::name
/// [`conventions`]: SomaPlugin::conventions
/// [`execute`]: SomaPlugin::execute
///
/// # Lifecycle
///
/// 1. The runtime calls `soma_plugin_init()` to obtain a `*mut dyn SomaPlugin`.
/// 2. [`on_load`] is called with the plugin's `soma.toml` settings.
/// 3. [`conventions`] is called to register available operations.
/// 4. [`execute`] / [`execute_async`] / [`execute_stream`] are called as
///    the Mind generates programs.
/// 5. [`on_unload`] is called during graceful shutdown.
///
/// [`on_load`]: SomaPlugin::on_load
/// [`execute_async`]: SomaPlugin::execute_async
/// [`execute_stream`]: SomaPlugin::execute_stream
/// [`on_unload`]: SomaPlugin::on_unload
#[allow(clippy::unnecessary_literal_bound)]
pub trait SomaPlugin: Send + Sync {
    /// Unique plugin name used for routing and configuration (e.g., `"postgres"`).
    fn name(&self) -> &str;

    /// Semantic version string (default `"0.1.0"`).
    fn version(&self) -> &str {
        "0.1.0"
    }

    /// Short human-readable description of the plugin's purpose.
    fn description(&self) -> &str {
        ""
    }

    /// Trust level controlling the runtime's sandbox strictness.
    fn trust_level(&self) -> TrustLevel {
        TrustLevel::BuiltIn
    }

    /// Whether this plugin supports streaming conventions.
    fn supports_streaming(&self) -> bool {
        false
    }

    /// Returns the full list of conventions this plugin provides.
    ///
    /// Called once at registration time.  Convention IDs must be unique
    /// within the plugin (the runtime offsets them by `plugin_idx * 1000`).
    fn conventions(&self) -> Vec<Convention>;

    /// Synchronous convention execution -- the primary entry point.
    ///
    /// `convention_id` is the *local* ID (before the `plugin_idx * 1000`
    /// offset).  `args` are positional, matching the convention's `ArgSpec`
    /// order.
    fn execute(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError>;

    /// Asynchronous convention execution.
    ///
    /// The default implementation delegates to [`execute`](SomaPlugin::execute)
    /// wrapped in a ready future.  Override this for truly async operations
    /// (e.g., non-blocking network I/O).
    fn execute_async(
        &self,
        convention_id: u32,
        args: Vec<Value>,
    ) -> Pin<Box<dyn Future<Output = Result<Value, PluginError>> + Send + '_>> {
        let result = self.execute(convention_id, args);
        Box::pin(async move { result })
    }

    /// Returns optional pre-trained `LoRA` weight bytes for this plugin's
    /// domain.
    ///
    /// The runtime merges these into the Mind at load time so the model
    /// better understands the plugin's conventions without full retraining.
    fn lora_weights(&self) -> Option<Vec<u8>> {
        None
    }

    /// Returns optional training examples for the synthesizer.
    ///
    /// The JSON value should match the format expected by
    /// `soma-synthesizer`'s `ConventionCatalog`.
    fn training_data(&self) -> Option<serde_json::Value> {
        None
    }

    /// Streaming convention execution -- returns all chunks at once.
    ///
    /// Only callable when [`supports_streaming`](SomaPlugin::supports_streaming)
    /// returns `true`.  The default returns an error.
    fn execute_stream(
        &self,
        _convention_id: u32,
        _args: Vec<Value>,
    ) -> Result<Vec<Value>, PluginError> {
        Err(PluginError::Failed("streaming not supported".into()))
    }

    /// Serializes plugin-internal state for inclusion in a checkpoint.
    ///
    /// Return `None` if the plugin is stateless.
    fn checkpoint_state(&self) -> Option<serde_json::Value> {
        None
    }

    /// Restores plugin-internal state from a previous checkpoint.
    fn restore_state(&mut self, _state: &serde_json::Value) -> Result<(), PluginError> {
        Ok(())
    }

    /// Called after the runtime loads the plugin and before any conventions
    /// are invoked.  Use this to establish connections, validate config, etc.
    fn on_load(&mut self, _config: &PluginConfig) -> Result<(), PluginError> {
        Ok(())
    }

    /// Called during graceful shutdown.  Release resources, close connections.
    fn on_unload(&mut self) -> Result<(), PluginError> {
        Ok(())
    }

    /// Plugins this plugin depends on (for topological load ordering).
    fn dependencies(&self) -> Vec<PluginDependency> {
        Vec::new()
    }

    /// Declared permissions this plugin requires.
    fn permissions(&self) -> PluginPermissions {
        PluginPermissions::default()
    }

    /// Optional JSON Schema describing the plugin's expected `soma.toml`
    /// settings, used by [`PluginConfig::validate`].
    fn config_schema(&self) -> Option<serde_json::Value> {
        None
    }
}
