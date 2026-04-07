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
    /// Names of currently loaded plugins.
    pub loaded_plugins: Vec<String>,
    /// Current LoRA adapter magnitude.
    pub lora_magnitude: f32,
    /// Current CPU usage percentage.
    /// CPU tracking requires platform-specific implementation.
    pub cpu_usage_percent: f32,
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
            loaded_plugins: Vec::new(),
            lora_magnitude: 0.0,
            cpu_usage_percent: 0.0,
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
        let plugins_str = if self.loaded_plugins.is_empty() {
            "none".to_string()
        } else {
            self.loaded_plugins.join(", ")
        };
        format!(
            "Uptime: {}\n\
             Inferences: {} total ({} ok, {} err, {:.1}% success)\n\
             Adaptations: {}\n\
             Experiences: {}\n\
             Checkpoints: {}\n\
             Decisions: {}\n\
             Plugins: {}\n\
             LoRA magnitude: {:.6}\n\
             CPU usage: {:.1}%",
            self.format_uptime(),
            self.total_inferences,
            self.successful_inferences,
            self.failed_inferences,
            self.success_rate(),
            self.total_adaptations,
            self.experience_count,
            self.checkpoints_saved,
            self.total_decisions_recorded,
            plugins_str,
            self.lora_magnitude,
            self.cpu_usage_percent,
        )
    }

    /// Get peak RSS in bytes via `getrusage(2)`.
    ///
    /// Note: `ru_maxrss` reports **peak** (high-water-mark) RSS, not the
    /// *current* resident set size.  Retrieving current RSS on macOS would
    /// require `mach_task_basic_info` via the Mach kernel API, which adds
    /// significant platform-specific complexity.  On Linux, `/proc/self/statm`
    /// could be used instead.  For now this function is named `peak_rss_bytes`
    /// to accurately reflect what it measures.
    pub fn peak_rss_bytes() -> u64 {
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        if rc == 0 {
            // macOS: ru_maxrss is in bytes. Linux: in KB.
            #[cfg(target_os = "macos")]
            { usage.ru_maxrss as u64 }
            #[cfg(not(target_os = "macos"))]
            { (usage.ru_maxrss as u64) * 1024 }
        } else {
            0
        }
    }

    /// Set the list of currently loaded plugin names.
    pub fn set_plugins(&mut self, names: Vec<String>) {
        self.loaded_plugins = names;
    }

    /// Set the current LoRA adapter magnitude.
    pub fn set_lora_magnitude(&mut self, mag: f32) {
        self.lora_magnitude = mag;
    }

    /// Update CPU usage estimate.
    /// CPU tracking requires platform-specific implementation.
    pub fn update_cpu(&mut self) {
        // CPU tracking requires platform-specific implementation.
        self.cpu_usage_percent = 0.0;
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
            "memory_peak_rss_bytes": Self::peak_rss_bytes(),
            "loaded_plugins": self.loaded_plugins,
            "lora_magnitude": self.lora_magnitude,
            "cpu_usage_percent": self.cpu_usage_percent,
        })
    }
}
