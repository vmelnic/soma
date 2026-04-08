//! SOMA state system -- the permanent, queryable record of what happened
//! (Whitepaper Sections 1.2, 7.5).
//!
//! Contains two subsystems:
//! - **Decision log**: append-only record of architectural/configuration choices.
//! - **Execution history**: bounded ring buffer of recent intent executions.
//!
//! The full state is serialized to JSON for the MCP `get_state()` tool,
//! giving any LLM session complete context continuity.

pub mod decision_log;
pub mod execution_history;

use decision_log::DecisionLog;
use execution_history::ExecutionHistory;

/// Aggregate state exposed via MCP `get_state()` for cross-session LLM context.
pub struct SomaState {
    pub decisions: DecisionLog,
    pub executions: ExecutionHistory,
}

impl SomaState {
    pub fn new(max_executions: usize) -> Self {
        Self {
            decisions: DecisionLog::new(),
            executions: ExecutionHistory::new(max_executions),
        }
    }

    /// Serialize the complete state for the MCP `get_state()` response.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "decisions": self.decisions.to_json(),
            "recent_executions": self.executions.to_json(),
            "decision_count": self.decisions.len(),
            "execution_count": self.executions.len(),
        })
    }
}
