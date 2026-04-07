//! Prometheus-compatible metrics (Whitepaper Section 11.5).
//!
//! 16 metrics covering inference, plugin execution, protocol, and memory.
//! All counters use atomic operations for lock-free concurrent updates.

use std::sync::atomic::{AtomicU64, Ordering};

/// SOMA runtime metrics — Prometheus-compatible counters and gauges.
pub struct SomaMetrics {
    // Inference metrics
    pub inferences_total: AtomicU64,
    pub inferences_success: AtomicU64,
    pub inferences_failed: AtomicU64,
    pub inference_duration_sum_ms: AtomicU64,

    // Program execution metrics
    pub programs_executed: AtomicU64,
    pub program_steps_executed: AtomicU64,

    // Plugin metrics
    pub plugin_calls_total: AtomicU64,
    pub plugin_calls_failed: AtomicU64,
    pub plugin_retries: AtomicU64,

    // Memory metrics
    pub experience_buffer_size: AtomicU64,
    pub adaptations_total: AtomicU64,
    pub checkpoints_saved: AtomicU64,

    // Protocol metrics
    pub protocol_connections_active: AtomicU64,
    pub protocol_signals_sent: AtomicU64,
    pub protocol_signals_received: AtomicU64,
    pub protocol_bytes_transferred: AtomicU64,
}

impl SomaMetrics {
    pub fn new() -> Self {
        Self {
            inferences_total: AtomicU64::new(0),
            inferences_success: AtomicU64::new(0),
            inferences_failed: AtomicU64::new(0),
            inference_duration_sum_ms: AtomicU64::new(0),
            programs_executed: AtomicU64::new(0),
            program_steps_executed: AtomicU64::new(0),
            plugin_calls_total: AtomicU64::new(0),
            plugin_calls_failed: AtomicU64::new(0),
            plugin_retries: AtomicU64::new(0),
            experience_buffer_size: AtomicU64::new(0),
            adaptations_total: AtomicU64::new(0),
            checkpoints_saved: AtomicU64::new(0),
            protocol_connections_active: AtomicU64::new(0),
            protocol_signals_sent: AtomicU64::new(0),
            protocol_signals_received: AtomicU64::new(0),
            protocol_bytes_transferred: AtomicU64::new(0),
        }
    }

    /// Record inference start.
    pub fn record_inference(&self, success: bool, duration_ms: u64) {
        self.inferences_total.fetch_add(1, Ordering::Relaxed);
        self.inference_duration_sum_ms.fetch_add(duration_ms, Ordering::Relaxed);
        if success {
            self.inferences_success.fetch_add(1, Ordering::Relaxed);
        } else {
            self.inferences_failed.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record program execution.
    pub fn record_program(&self, steps: u64) {
        self.programs_executed.fetch_add(1, Ordering::Relaxed);
        self.program_steps_executed.fetch_add(steps, Ordering::Relaxed);
    }

    /// Record a plugin call.
    pub fn record_plugin_call(&self, success: bool) {
        self.plugin_calls_total.fetch_add(1, Ordering::Relaxed);
        if !success {
            self.plugin_calls_failed.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a plugin retry.
    pub fn record_plugin_retry(&self) {
        self.plugin_retries.fetch_add(1, Ordering::Relaxed);
    }

    /// Average inference duration in ms.
    pub fn avg_inference_ms(&self) -> f64 {
        let total = self.inferences_total.load(Ordering::Relaxed);
        if total == 0 { return 0.0; }
        self.inference_duration_sum_ms.load(Ordering::Relaxed) as f64 / total as f64
    }

    /// Serialize to Prometheus text exposition format.
    pub fn to_prometheus(&self) -> String {
        let mut out = String::with_capacity(2048);

        prom(&mut out, "soma_inferences_total", "counter",
            "Total inference requests", self.inferences_total.load(Ordering::Relaxed));
        prom(&mut out, "soma_inferences_success", "counter",
            "Successful inferences", self.inferences_success.load(Ordering::Relaxed));
        prom(&mut out, "soma_inferences_failed", "counter",
            "Failed inferences", self.inferences_failed.load(Ordering::Relaxed));
        prom(&mut out, "soma_inference_duration_sum_ms", "counter",
            "Sum of inference durations in ms", self.inference_duration_sum_ms.load(Ordering::Relaxed));
        prom(&mut out, "soma_programs_executed", "counter",
            "Total programs executed", self.programs_executed.load(Ordering::Relaxed));
        prom(&mut out, "soma_program_steps_executed", "counter",
            "Total program steps executed", self.program_steps_executed.load(Ordering::Relaxed));
        prom(&mut out, "soma_plugin_calls_total", "counter",
            "Total plugin convention calls", self.plugin_calls_total.load(Ordering::Relaxed));
        prom(&mut out, "soma_plugin_calls_failed", "counter",
            "Failed plugin calls", self.plugin_calls_failed.load(Ordering::Relaxed));
        prom(&mut out, "soma_plugin_retries", "counter",
            "Plugin call retries", self.plugin_retries.load(Ordering::Relaxed));
        prom(&mut out, "soma_experience_buffer_size", "gauge",
            "Current experience buffer size", self.experience_buffer_size.load(Ordering::Relaxed));
        prom(&mut out, "soma_adaptations_total", "counter",
            "Total LoRA adaptations", self.adaptations_total.load(Ordering::Relaxed));
        prom(&mut out, "soma_checkpoints_saved", "counter",
            "Total checkpoints saved", self.checkpoints_saved.load(Ordering::Relaxed));
        prom(&mut out, "soma_protocol_connections_active", "gauge",
            "Active protocol connections", self.protocol_connections_active.load(Ordering::Relaxed));
        prom(&mut out, "soma_protocol_signals_sent", "counter",
            "Total signals sent", self.protocol_signals_sent.load(Ordering::Relaxed));
        prom(&mut out, "soma_protocol_signals_received", "counter",
            "Total signals received", self.protocol_signals_received.load(Ordering::Relaxed));
        prom(&mut out, "soma_protocol_bytes_transferred", "counter",
            "Total bytes transferred", self.protocol_bytes_transferred.load(Ordering::Relaxed));

        out
    }

    /// Set the active connection count gauge.
    pub fn set_active_connections(&self, count: u64) {
        self.protocol_connections_active.store(count, Ordering::Relaxed);
    }

    /// Record protocol signal.
    pub fn record_signal_sent(&self, bytes: u64) {
        self.protocol_signals_sent.fetch_add(1, Ordering::Relaxed);
        self.protocol_bytes_transferred.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Record protocol signal received.
    pub fn record_signal_received(&self, bytes: u64) {
        self.protocol_signals_received.fetch_add(1, Ordering::Relaxed);
        self.protocol_bytes_transferred.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Serialize to JSON for MCP health/metrics endpoint.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "inference": {
                "total": self.inferences_total.load(Ordering::Relaxed),
                "success": self.inferences_success.load(Ordering::Relaxed),
                "failed": self.inferences_failed.load(Ordering::Relaxed),
                "avg_duration_ms": self.avg_inference_ms(),
            },
            "programs": {
                "executed": self.programs_executed.load(Ordering::Relaxed),
                "steps_executed": self.program_steps_executed.load(Ordering::Relaxed),
            },
            "plugins": {
                "calls_total": self.plugin_calls_total.load(Ordering::Relaxed),
                "calls_failed": self.plugin_calls_failed.load(Ordering::Relaxed),
                "retries": self.plugin_retries.load(Ordering::Relaxed),
            },
            "memory": {
                "experience_buffer_size": self.experience_buffer_size.load(Ordering::Relaxed),
                "adaptations_total": self.adaptations_total.load(Ordering::Relaxed),
                "checkpoints_saved": self.checkpoints_saved.load(Ordering::Relaxed),
            },
            "protocol": {
                "connections_active": self.protocol_connections_active.load(Ordering::Relaxed),
                "signals_sent": self.protocol_signals_sent.load(Ordering::Relaxed),
                "signals_received": self.protocol_signals_received.load(Ordering::Relaxed),
                "bytes_transferred": self.protocol_bytes_transferred.load(Ordering::Relaxed),
            },
        })
    }
}

fn prom(out: &mut String, name: &str, ptype: &str, help: &str, value: u64) {
    out.push_str(&format!("# HELP {} {}\n", name, help));
    out.push_str(&format!("# TYPE {} {}\n", name, ptype));
    out.push_str(&format!("{} {}\n", name, value));
}
