use thiserror::Error;

use crate::types::common::{PortFailureClass, SkillFailureClass};
use crate::types::peer::DistributedFailure;

#[derive(Debug, Error)]
pub enum SomaError {
    // Goal
    #[error("goal error: {0}")]
    Goal(String),

    #[error("goal validation failed: {0}")]
    GoalValidation(String),

    // Belief
    #[error("belief error: {0}")]
    Belief(String),

    #[error("belief merge conflict: {0}")]
    BeliefConflict(String),

    // Resource
    #[error("resource error: {0}")]
    Resource(String),

    #[error("resource not found: {resource_type}/{resource_id}")]
    ResourceNotFound {
        resource_type: String,
        resource_id: String,
    },

    #[error("resource version conflict: expected {expected}, found {found}")]
    ResourceVersionConflict { expected: u64, found: u64 },

    // Skill
    #[error("skill error: {0}")]
    Skill(String),

    #[error("skill validation failed: {skill_id}: {reason}")]
    SkillValidation { skill_id: String, reason: String },

    #[error("skill execution failed: {skill_id}: {failure_class:?}: {details}")]
    SkillExecution {
        skill_id: String,
        failure_class: SkillFailureClass,
        details: String,
    },

    #[error("skill not found: {0}")]
    SkillNotFound(String),

    // Port
    #[error("port error: {0}")]
    Port(String),

    #[error("port invocation failed: {port_id}/{capability_id}: {failure_class:?}")]
    PortInvocation {
        port_id: String,
        capability_id: String,
        failure_class: PortFailureClass,
    },

    #[error("port not found: {0}")]
    PortNotFound(String),

    // Session
    #[error("session error: {0}")]
    Session(String),

    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("session budget exhausted: {0}")]
    BudgetExhausted(String),

    // Memory
    #[error("memory error: {0}")]
    Memory(String),

    // Policy
    #[error("policy denied: {action}: {reason}")]
    PolicyDenied { action: String, reason: String },

    #[error("policy error: {0}")]
    Policy(String),

    // Trace
    #[error("trace error: {0}")]
    Trace(String),

    // Pack
    #[error("pack error: {0}")]
    Pack(String),

    #[error("pack validation failed: {pack_id}: {reason}")]
    PackValidation { pack_id: String, reason: String },

    #[error("pack dependency unsatisfied: {pack_id} requires {dependency}")]
    PackDependency {
        pack_id: String,
        dependency: String,
    },

    #[error("namespace collision: {0}")]
    NamespaceCollision(String),

    // Distributed
    #[error("distributed error: {failure:?}: {details}")]
    Distributed {
        failure: DistributedFailure,
        details: String,
    },

    #[error("peer not found: {0}")]
    PeerNotFound(String),

    // Interface
    #[error("interface error: {0}")]
    Interface(String),

    // Selector
    #[error("no viable candidates for goal")]
    NoCandidates,

    // Config
    #[error("configuration error: {0}")]
    Config(String),

    // IO
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    // Serialization
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, SomaError>;
