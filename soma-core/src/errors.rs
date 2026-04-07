//! SOMA error types (Whitepaper Section 11.3).

use thiserror::Error;

/// Detailed plugin error information for rich error reporting.
/// Used by the `SomaError::PluginDetailed` variant, which carries full
/// context about what went wrong, where, and whether a retry might help.
#[derive(Debug, Clone)]
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
            write!(f, " (step {})", idx)?;
        }
        if let Some(ref conv) = self.convention {
            write!(f, " [convention: {}]", conv)?;
        }
        if self.retryable {
            write!(f, " [retryable]")?;
        }
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum SomaError {
    #[error("inference error: {0}")]
    Inference(String),

    #[error("plugin error in {plugin}: {message}")]
    Plugin {
        plugin: String,
        message: String,
        retryable: bool,
        step_index: Option<usize>,
        convention: Option<String>,
    },

    /// Rich plugin error with full context (structured alternative to Plugin).
    #[error("plugin error: {0}")]
    PluginDetailed(PluginErrorDetail),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("resource exhausted: {0}")]
    Resource(String),

    #[error("convention not found: {0}")]
    Convention(String),

    #[error("MCP error: {0}")]
    Mcp(String),

    #[error("state error: {0}")]
    State(String),

    #[error("auth error: {0}")]
    Auth(String),

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

impl SomaError {
    /// Convenience constructor for a simple plugin error.
    pub fn plugin(plugin: impl Into<String>, message: impl Into<String>) -> Self {
        SomaError::Plugin {
            plugin: plugin.into(),
            message: message.into(),
            retryable: false,
            step_index: None,
            convention: None,
        }
    }

    /// Convenience constructor for a detailed plugin error.
    pub fn plugin_detailed(detail: PluginErrorDetail) -> Self {
        SomaError::PluginDetailed(detail)
    }

    /// Whether this error is considered retryable.
    pub fn is_retryable(&self) -> bool {
        match self {
            SomaError::Plugin { retryable, .. } => *retryable,
            SomaError::PluginDetailed(d) => d.retryable,
            _ => false,
        }
    }
}
