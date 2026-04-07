//! Proprioception — self-knowledge and runtime statistics (Spec Section 7).

use std::time::{Duration, Instant};

/// Runtime statistics and self-knowledge for the SOMA instance.
pub struct Proprioception {
    pub start_time: Instant,
    pub total_inferences: u64,
    pub successful_inferences: u64,
    pub failed_inferences: u64,
    pub total_adaptations: u64,
    pub experience_count: u64,
}

impl Proprioception {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            total_inferences: 0,
            successful_inferences: 0,
            failed_inferences: 0,
            total_adaptations: 0,
            experience_count: 0,
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
             Experiences: {}",
            self.format_uptime(),
            self.total_inferences,
            self.successful_inferences,
            self.failed_inferences,
            self.success_rate(),
            self.total_adaptations,
            self.experience_count,
        )
    }
}
