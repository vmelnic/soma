//! Decision log -- append-only record of what was built, why, and by which
//! LLM session (Whitepaper Section 7.5).
//!
//! Decisions are never deleted or modified. The log grows monotonically and
//! is serialized in full for MCP `get_state()` responses.

use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

/// A single recorded decision: what was done, the rationale, and provenance.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Decision {
    /// Monotonic identifier (`d-1`, `d-2`, ...).
    pub id: String,
    /// What was done (e.g., "added users table").
    pub what: String,
    /// Why it was done (e.g., "need user authentication for API access").
    pub why: String,
    /// Unix timestamp in seconds when this decision was recorded.
    pub timestamp: u64,
    /// Identifier of the LLM session that made this decision.
    pub session_id: String,
}

/// Append-only log of architectural and configuration decisions.
///
/// IDs are assigned via a monotonic counter (`next_id`), ensuring uniqueness
/// within a single runtime session. Persistence across restarts is handled
/// by the checkpoint system.
pub struct DecisionLog {
    decisions: Vec<Decision>,
    next_id: u64,
}

impl DecisionLog {
    pub const fn new() -> Self {
        Self {
            decisions: Vec::new(),
            next_id: 1,
        }
    }

    /// Record a new decision and return a reference to it.
    pub fn record(&mut self, what: String, why: String, session_id: String) -> &Decision {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let id = format!("d-{}", self.next_id);
        self.next_id += 1;

        self.decisions.push(Decision {
            id,
            what,
            why,
            timestamp,
            session_id,
        });
        self.decisions.last().unwrap()
    }

    /// Return all decisions in chronological order.
    pub fn list(&self) -> &[Decision] {
        &self.decisions
    }

    /// Get the N most recent decisions.
    pub fn recent(&self, n: usize) -> &[Decision] {
        let start = self.decisions.len().saturating_sub(n);
        &self.decisions[start..]
    }

    /// Case-insensitive keyword search across `what` and `why` fields.
    pub fn search(&self, query: &str) -> Vec<&Decision> {
        let q = query.to_lowercase();
        self.decisions.iter()
            .filter(|d| d.what.to_lowercase().contains(&q) || d.why.to_lowercase().contains(&q))
            .collect()
    }

    pub const fn len(&self) -> usize {
        self.decisions.len()
    }

    #[allow(dead_code)] // Spec feature: decision log API
    pub const fn is_empty(&self) -> bool {
        self.decisions.is_empty()
    }

    /// Serialize for MCP response.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!(self.decisions)
    }
}
