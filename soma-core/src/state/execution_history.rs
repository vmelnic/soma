//! Execution history -- bounded record of recent intent executions
//! (Whitepaper Section 7.5).
//!
//! Uses a `VecDeque` as a ring buffer: when `max_size` is reached, the oldest
//! record is evicted on each new insertion. `total_count` tracks the lifetime
//! count across evictions.

use serde::Serialize;
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

/// A single execution record: the intent, the generated program's metadata,
/// and whether execution succeeded.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ExecutionRecord {
    /// Monotonic identifier (`e-1`, `e-2`, ...).
    pub id: String,
    /// The natural-language intent that was inferred upon.
    pub intent: String,
    /// Number of steps in the generated program.
    pub program_steps: usize,
    /// Model confidence score for the generated program (0.0..1.0).
    pub confidence: f32,
    pub success: bool,
    /// Wall-clock execution time in milliseconds.
    pub execution_time_ms: u64,
    /// Unix timestamp in seconds.
    pub timestamp: u64,
    /// Opaque trace identifier for correlating logs and signals.
    pub trace_id: String,
    /// Error message if `success` is false.
    pub error: Option<String>,
}

/// Bounded ring buffer of recent execution records.
///
/// Oldest entries are evicted when `max_size` is reached. The MCP `get_state()`
/// tool serializes the 50 most recent records.
pub struct ExecutionHistory {
    records: VecDeque<ExecutionRecord>,
    max_size: usize,
    /// Lifetime execution count (never resets, even as old records are evicted).
    total_count: u64,
}

impl ExecutionHistory {
    pub fn new(max_size: usize) -> Self {
        Self {
            records: VecDeque::with_capacity(max_size),
            max_size,
            total_count: 0,
        }
    }

    /// Record a new execution, evicting the oldest entry if the buffer is full.
    #[allow(clippy::too_many_arguments)] // All fields are needed per spec Section 7.5
    pub fn record(
        &mut self,
        intent: String,
        program_steps: usize,
        confidence: f32,
        success: bool,
        execution_time_ms: u64,
        trace_id: String,
        error: Option<String>,
    ) {
        self.total_count += 1;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let id = format!("e-{}", self.total_count);

        if self.records.len() >= self.max_size {
            self.records.pop_front();
        }

        self.records.push_back(ExecutionRecord {
            id,
            intent,
            program_steps,
            confidence,
            success,
            execution_time_ms,
            timestamp,
            trace_id,
            error,
        });
    }

    /// Get the N most recent execution records.
    pub fn recent(&self, n: usize) -> Vec<&ExecutionRecord> {
        self.records.iter().rev().take(n).collect()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    #[allow(dead_code)] // Spec feature: Section 7.5
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    #[allow(dead_code)] // Spec feature: Section 7.5
    pub const fn total_count(&self) -> u64 {
        self.total_count
    }

    /// Success rate as a percentage (0.0..100.0) over the current buffer contents.
    #[allow(dead_code, clippy::cast_precision_loss)] // Spec feature; precision loss acceptable for percentages
    pub fn success_rate(&self) -> f64 {
        if self.records.is_empty() {
            return 0.0;
        }
        let ok = self.records.iter().filter(|r| r.success).count();
        (ok as f64 / self.records.len() as f64) * 100.0
    }

    /// Mean execution time in ms over the current buffer contents.
    #[allow(dead_code, clippy::cast_precision_loss)] // Spec feature; precision loss acceptable for averages
    pub fn avg_execution_time_ms(&self) -> f64 {
        if self.records.is_empty() {
            return 0.0;
        }
        let sum: u64 = self.records.iter().map(|r| r.execution_time_ms).sum();
        sum as f64 / self.records.len() as f64
    }

    /// Serialize for MCP response.
    pub fn to_json(&self) -> serde_json::Value {
        let recs: Vec<&ExecutionRecord> = self.records.iter().rev().take(50).collect();
        serde_json::json!(recs)
    }
}
