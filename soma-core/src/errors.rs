//! SOMA error types (Spec Section 13).

use thiserror::Error;

#[derive(Error, Debug)]
pub enum SomaError {
    #[error("inference error: {0}")]
    Inference(String),

    #[error("plugin error in {plugin}: {message}")]
    Plugin {
        plugin: String,
        message: String,
        retryable: bool,
        step_index: usize,
    },

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("resource exhausted: {0}")]
    Resource(String),

    #[error("convention not found: {0}")]
    Convention(String),

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}
