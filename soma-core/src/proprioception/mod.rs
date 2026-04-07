//! Proprioception — self-knowledge and runtime statistics (Whitepaper Section 11).
//!
//! The SOMA's self-model: what it knows about itself, its capabilities,
//! its current state, and its recent performance.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Runtime statistics and self-knowledge for the SOMA instance.
pub struct Proprioception {
    pub start_time: Instant,
    pub start_timestamp: u64,
    pub total_inferences: u64,
    pub successful_inferences: u64,
    pub failed_inferences: u64,
    pub total_adaptations: u64,
    pub experience_count: u64,
    pub checkpoints_saved: u64,
    pub consolidations: u64,
    pub active_connections: u64,
    pub total_signals_processed: u64,
    pub total_decisions_recorded: u64,
}

impl Proprioception {
    pub fn new() -> Self {
        let start_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            start_time: Instant::now(),
            start_timestamp,
            total_inferences: 0,
            successful_inferences: 0,
            failed_inferences: 0,
            total_adaptations: 0,
            experience_count: 0,
            checkpoints_saved: 0,
            consolidations: 0,
            active_connections: 0,
            total_signals_processed: 0,
            total_decisions_recorded: 0,
        }
    }

    /// Record a successful inference+execution cycle.
    pub fn record_success(&mut self) {
        self.total_inferences += 1;
        self.successful_inferences += 1;
        self.experience_count += 1;
    }

    /// Record a failed inference or execution cycle.
    pub fn record_failure(&mut self) {
        self.total_inferences += 1;
        self.failed_inferences += 1;
        self.experience_count += 1;
    }

    /// Record that a LoRA adaptation was performed.
    pub fn record_adaptation(&mut self) {
        self.total_adaptations += 1;
    }

    pub fn record_checkpoint(&mut self) {
        self.checkpoints_saved += 1;
    }

    pub fn record_consolidation(&mut self) {
        self.consolidations += 1;
    }

    pub fn record_decision(&mut self) {
        self.total_decisions_recorded += 1;
    }

    /// How long this SOMA instance has been running.
    pub fn uptime(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Format uptime as a human-readable string.
    fn format_uptime(&self) -> String {
        let d = self.uptime();
        let secs = d.as_secs();
        if secs < 60 {
            format!("{}s", secs)
        } else if secs < 3600 {
            format!("{}m {}s", secs / 60, secs % 60)
        } else {
            format!("{}h {}m {}s", secs / 3600, (secs % 3600) / 60, secs % 60)
        }
    }

    /// Success rate as a percentage (0.0 - 100.0).
    pub fn success_rate(&self) -> f64 {
        if self.total_inferences == 0 {
            return 0.0;
        }
        (self.successful_inferences as f64 / self.total_inferences as f64) * 100.0
    }

    /// Generate a human-readable status report.
    pub fn report(&self) -> String {
        format!(
            "Uptime: {}\n\
             Inferences: {} total ({} ok, {} err, {:.1}% success)\n\
             Adaptations: {}\n\
             Experiences: {}\n\
             Checkpoints: {}\n\
             Decisions: {}",
            self.format_uptime(),
            self.total_inferences,
            self.successful_inferences,
            self.failed_inferences,
            self.success_rate(),
            self.total_adaptations,
            self.experience_count,
            self.checkpoints_saved,
            self.total_decisions_recorded,
        )
    }

    /// Serialize for MCP health endpoint.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "uptime_secs": self.uptime().as_secs(),
            "start_timestamp": self.start_timestamp,
            "inferences": {
                "total": self.total_inferences,
                "successful": self.successful_inferences,
                "failed": self.failed_inferences,
                "success_rate": self.success_rate(),
            },
            "adaptations": self.total_adaptations,
            "experience_count": self.experience_count,
            "checkpoints_saved": self.checkpoints_saved,
            "consolidations": self.consolidations,
            "decisions_recorded": self.total_decisions_recorded,
        })
    }
}
