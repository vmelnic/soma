//! Execution history — recent intent executions with results
//! (Whitepaper Section 7.5).

use serde::Serialize;
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

/// A recorded execution — intent, program, result.
#[derive(Debug, Clone, Serialize)]
pub struct ExecutionRecord {
    pub id: String,
    pub intent: String,
    pub program_steps: usize,
    pub confidence: f32,
    pub success: bool,
    pub execution_time_ms: u64,
    pub timestamp: u64,
    pub trace_id: String,
    pub error: Option<String>,
}

/// Bounded ring buffer of recent execution records.
pub struct ExecutionHistory {
    records: VecDeque<ExecutionRecord>,
    max_size: usize,
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

    /// Record a new execution.
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

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn total_count(&self) -> u64 {
        self.total_count
    }

    /// Success rate over the buffer.
    pub fn success_rate(&self) -> f64 {
        if self.records.is_empty() {
            return 0.0;
        }
        let ok = self.records.iter().filter(|r| r.success).count();
        (ok as f64 / self.records.len() as f64) * 100.0
    }

    /// Average execution time over the buffer.
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
