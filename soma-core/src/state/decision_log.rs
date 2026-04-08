//! Decision log — records what was built, why, when, and by which LLM session
//! (Whitepaper Section 7.5).

use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

/// A recorded decision — what was done, why, and by whom.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Decision {
    pub id: String,
    pub what: String,
    pub why: String,
    pub timestamp: u64,
    pub session_id: String,
}

/// Persistent decision log. Every architectural choice, schema change,
/// or configuration decision is recorded here.
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

    /// Record a new decision.
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

    /// Get all decisions.
    pub fn list(&self) -> &[Decision] {
        &self.decisions
    }

    /// Get the N most recent decisions.
    pub fn recent(&self, n: usize) -> &[Decision] {
        let start = self.decisions.len().saturating_sub(n);
        &self.decisions[start..]
    }

    /// Search decisions by keyword in 'what' or 'why'.
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
