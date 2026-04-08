//! SOMA error taxonomy.
//!
//! Every subsystem error folds into [`SomaError`], which provides structured
//! context for diagnostics, MCP error responses, and retry decisions.

use thiserror::Error;

/// Structured plugin error with enough context for diagnostics and retry logic.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PluginErrorDetail {
    /// Name of the plugin that produced the error.
    pub plugin: String,
    /// Human-readable error message.
    pub message: String,
    /// Whether the error is transient and the operation may succeed on retry.
    pub retryable: bool,
    /// Index of the program step that failed, if known.
    pub step_index: Option<usize>,
    /// Name of the convention that was being executed, if known.
    pub convention: Option<String>,
}

impl std::fmt::Display for PluginErrorDetail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "plugin '{}': {}", self.plugin, self.message)?;
        if let Some(idx) = self.step_index {
            write!(f, " (step {idx})")?;
        }
        if let Some(ref conv) = self.convention {
            write!(f, " [convention: {conv}]")?;
        }
        if self.retryable {
            write!(f, " [retryable]")?;
        }
        Ok(())
    }
}

/// Unified error type spanning all SOMA subsystems.
///
/// Only `Plugin` and `PluginDetailed` carry a `retryable` flag;
/// all other variants are treated as non-retryable by default.
#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum SomaError {
    /// Mind engine inference failure (model load, tokenization, decoding).
    #[error("inference error: {0}")]
    Inference(String),

    /// Plugin execution failure with inline context fields.
    #[error("plugin error in {plugin}: {message}")]
    Plugin {
        plugin: String,
        message: String,
        retryable: bool,
        step_index: Option<usize>,
        convention: Option<String>,
    },

    /// Plugin execution failure carrying a [`PluginErrorDetail`] struct.
    #[error("plugin error: {0}")]
    PluginDetailed(PluginErrorDetail),

    /// Synaptic protocol failure (connection, codec, routing).
    #[error("protocol error: {0}")]
    Protocol(String),

    /// Resource limit exceeded (concurrency, memory, plugin count).
    #[error("resource exhausted: {0}")]
    Resource(String),

    /// Referenced convention does not exist in any loaded plugin.
    #[error("convention not found: {0}")]
    Convention(String),

    /// MCP JSON-RPC server error (auth, tool dispatch, serialization).
    #[error("MCP error: {0}")]
    Mcp(String),

    /// State subsystem error (decision log, execution history persistence).
    #[error("state error: {0}")]
    State(String),

    /// Authentication or authorization failure.
    #[error("auth error: {0}")]
    Auth(String),

    /// Catch-all for errors from external crates via `anyhow`.
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

#[allow(dead_code)]
impl SomaError {
    /// Convenience constructor for a simple plugin error.
    pub fn plugin(plugin: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Plugin {
            plugin: plugin.into(),
            message: message.into(),
            retryable: false,
            step_index: None,
            convention: None,
        }
    }

    /// Convenience constructor for a detailed plugin error.
    pub const fn plugin_detailed(detail: PluginErrorDetail) -> Self {
        Self::PluginDetailed(detail)
    }

    /// Whether this error is considered retryable.
    pub const fn is_retryable(&self) -> bool {
        match self {
            Self::Plugin { retryable, .. } => *retryable,
            Self::PluginDetailed(d) => d.retryable,
            _ => false,
        }
    }
}
