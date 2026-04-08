//! State system — SOMA as permanent memory (Whitepaper Sections 1.2, 7.5).
//!
//! State is the complete truth of what exists: decision log, execution history,
//! plugin configurations, business rules. Permanent, queryable, and transferable
//! across LLM sessions.

pub mod decision_log;
pub mod execution_history;

use decision_log::DecisionLog;
use execution_history::ExecutionHistory;

/// The complete SOMA state — exposed via MCP for LLM context continuity.
/// When any LLM calls `soma.get_state()`, it receives ALL of this.
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

    /// Serialize the complete state for MCP `get_state()` response.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "decisions": self.decisions.to_json(),
            "recent_executions": self.executions.to_json(),
            "decision_count": self.decisions.len(),
            "execution_count": self.executions.len(),
        })
    }
}
