use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use web_time::Instant;

/// Centralized runtime metrics, shared across all subsystems via `Arc<RuntimeMetrics>`.
///
/// Atomic counters are used for high-frequency increments (session lifecycle,
/// step counting, port calls). A mutex-guarded map is used for per-skill
/// invocation counts and per-port latency histograms, since those are
/// updated less frequently and need key-based access.
pub struct RuntimeMetrics {
    /// Number of sessions currently in a non-terminal state.
    pub active_sessions: AtomicU64,
    /// Total sessions that reached Completed status.
    pub completed_sessions: AtomicU64,
    /// Total sessions that reached Failed status.
    pub failed_sessions: AtomicU64,
    /// Total sessions that reached Aborted status.
    pub aborted_sessions: AtomicU64,
    /// Total control-loop steps executed across all sessions.
    pub total_steps: AtomicU64,
    /// Total port invocations across all sessions.
    pub total_port_calls: AtomicU64,
    /// Total policy denials (blocked-by-policy events).
    pub total_policy_denials: AtomicU64,
    /// Number of episodes currently stored in the episode store.
    pub episodes_stored: AtomicU64,
    /// Number of schemas currently registered in the schema store.
    pub schemas_induced: AtomicU64,
    /// Number of routines currently registered in the routine store.
    pub routines_compiled: AtomicU64,
    /// Per-skill invocation counts keyed by skill FQN.
    pub skill_invocations: Mutex<HashMap<String, u64>>,
    /// Per-port latency samples (most recent N) keyed by port_id.
    pub port_latencies: Mutex<HashMap<String, Vec<u64>>>,
    /// Instant when the runtime was created, used to compute uptime.
    pub started_at: Instant,
}

/// Maximum number of latency samples retained per port.
const MAX_LATENCY_SAMPLES: usize = 1000;

impl RuntimeMetrics {
    pub fn new() -> Self {
        Self {
            active_sessions: AtomicU64::new(0),
            completed_sessions: AtomicU64::new(0),
            failed_sessions: AtomicU64::new(0),
            aborted_sessions: AtomicU64::new(0),
            total_steps: AtomicU64::new(0),
            total_port_calls: AtomicU64::new(0),
            total_policy_denials: AtomicU64::new(0),
            episodes_stored: AtomicU64::new(0),
            schemas_induced: AtomicU64::new(0),
            routines_compiled: AtomicU64::new(0),
            skill_invocations: Mutex::new(HashMap::new()),
            port_latencies: Mutex::new(HashMap::new()),
            started_at: Instant::now(),
        }
    }

    /// Record that a new session was created.
    pub fn session_created(&self) {
        self.active_sessions.fetch_add(1, Ordering::Relaxed);
    }

    /// Record that a session completed successfully.
    pub fn session_completed(&self) {
        self.active_sessions.fetch_sub(1, Ordering::Relaxed);
        self.completed_sessions.fetch_add(1, Ordering::Relaxed);
    }

    /// Record that a session failed.
    pub fn session_failed(&self) {
        self.active_sessions.fetch_sub(1, Ordering::Relaxed);
        self.failed_sessions.fetch_add(1, Ordering::Relaxed);
    }

    /// Record that a session was aborted.
    pub fn session_aborted(&self) {
        self.active_sessions.fetch_sub(1, Ordering::Relaxed);
        self.aborted_sessions.fetch_add(1, Ordering::Relaxed);
    }

    /// Record that one control-loop step was executed.
    pub fn step_executed(&self) {
        self.total_steps.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a port invocation with its latency.
    pub fn port_call(&self, port_id: &str, latency_ms: u64) {
        self.total_port_calls.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut map) = self.port_latencies.lock() {
            let samples = map.entry(port_id.to_string()).or_default();
            if samples.len() >= MAX_LATENCY_SAMPLES {
                samples.remove(0);
            }
            samples.push(latency_ms);
        }
    }

    /// Record that a policy denied an action.
    pub fn policy_denial(&self) {
        self.total_policy_denials.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a skill invocation by FQN.
    pub fn skill_invoked(&self, skill_id: &str) {
        if let Ok(mut map) = self.skill_invocations.lock() {
            *map.entry(skill_id.to_string()).or_insert(0) += 1;
        }
    }

    /// Return uptime in seconds.
    pub fn uptime_seconds(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    /// Build a snapshot of all metrics as a JSON-compatible structure.
    pub fn snapshot(&self) -> MetricsSnapshot {
        let skill_invocations = self
            .skill_invocations
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default();

        let port_latencies = self
            .port_latencies
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default();

        MetricsSnapshot {
            active_sessions: self.active_sessions.load(Ordering::Relaxed),
            completed_sessions: self.completed_sessions.load(Ordering::Relaxed),
            failed_sessions: self.failed_sessions.load(Ordering::Relaxed),
            aborted_sessions: self.aborted_sessions.load(Ordering::Relaxed),
            total_steps: self.total_steps.load(Ordering::Relaxed),
            total_port_calls: self.total_port_calls.load(Ordering::Relaxed),
            total_policy_denials: self.total_policy_denials.load(Ordering::Relaxed),
            episodes_stored: self.episodes_stored.load(Ordering::Relaxed),
            schemas_induced: self.schemas_induced.load(Ordering::Relaxed),
            routines_compiled: self.routines_compiled.load(Ordering::Relaxed),
            skill_invocations,
            port_latencies,
            uptime_seconds: self.uptime_seconds(),
        }
    }
}

impl Default for RuntimeMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Point-in-time snapshot of all metrics, suitable for formatting as text,
/// JSON, or Prometheus exposition.
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub active_sessions: u64,
    pub completed_sessions: u64,
    pub failed_sessions: u64,
    pub aborted_sessions: u64,
    pub total_steps: u64,
    pub total_port_calls: u64,
    pub total_policy_denials: u64,
    pub episodes_stored: u64,
    pub schemas_induced: u64,
    pub routines_compiled: u64,
    pub skill_invocations: HashMap<String, u64>,
    pub port_latencies: HashMap<String, Vec<u64>>,
    pub uptime_seconds: u64,
}

impl MetricsSnapshot {
    /// Compute the session success rate as a fraction (0.0 to 1.0).
    pub fn session_success_rate(&self) -> f64 {
        let total = self.completed_sessions + self.failed_sessions + self.aborted_sessions;
        if total == 0 {
            0.0
        } else {
            self.completed_sessions as f64 / total as f64
        }
    }

    /// Average latency in ms for a given port, or None if no samples.
    pub fn avg_port_latency(&self, port_id: &str) -> Option<f64> {
        self.port_latencies.get(port_id).and_then(|samples| {
            if samples.is_empty() {
                None
            } else {
                Some(samples.iter().sum::<u64>() as f64 / samples.len() as f64)
            }
        })
    }

    /// Format as human-readable text for the CLI.
    pub fn format_text(&self) -> String {
        let mut lines = Vec::new();
        lines.push("Metrics:".to_string());
        lines.push(format!("  active_sessions:    {}", self.active_sessions));
        lines.push(format!("  completed_sessions: {}", self.completed_sessions));
        lines.push(format!("  failed_sessions:    {}", self.failed_sessions));
        lines.push(format!("  aborted_sessions:   {}", self.aborted_sessions));
        lines.push(format!(
            "  success_rate:       {:.1}%",
            self.session_success_rate() * 100.0
        ));
        lines.push(format!("  total_steps:        {}", self.total_steps));
        lines.push(format!("  total_port_calls:   {}", self.total_port_calls));
        lines.push(format!("  policy_denials:     {}", self.total_policy_denials));
        lines.push(format!("  episodes_stored:    {}", self.episodes_stored));
        lines.push(format!("  schemas_induced:    {}", self.schemas_induced));
        lines.push(format!("  routines_compiled:  {}", self.routines_compiled));
        lines.push(format!("  uptime_seconds:     {}", self.uptime_seconds));

        if !self.skill_invocations.is_empty() {
            lines.push("  skill_invocations:".to_string());
            let mut sorted: Vec<_> = self.skill_invocations.iter().collect();
            sorted.sort_by_key(|(k, _)| (*k).clone());
            for (skill, count) in sorted {
                lines.push(format!("    {}: {}", skill, count));
            }
        }

        if !self.port_latencies.is_empty() {
            lines.push("  port_latency_avg_ms:".to_string());
            let mut sorted: Vec<_> = self.port_latencies.keys().collect();
            sorted.sort();
            for port_id in sorted {
                if let Some(avg) = self.avg_port_latency(port_id) {
                    lines.push(format!("    {}: {:.1}", port_id, avg));
                }
            }
        }

        lines.join("\n")
    }

    /// Format as JSON for MCP responses.
    pub fn format_json(&self) -> serde_json::Value {
        let mut port_latency_avg: HashMap<String, f64> = HashMap::new();
        for port_id in self.port_latencies.keys() {
            if let Some(avg) = self.avg_port_latency(port_id) {
                port_latency_avg.insert(port_id.clone(), avg);
            }
        }

        serde_json::json!({
            "active_sessions": self.active_sessions,
            "completed_sessions": self.completed_sessions,
            "failed_sessions": self.failed_sessions,
            "aborted_sessions": self.aborted_sessions,
            "session_success_rate": self.session_success_rate(),
            "total_steps": self.total_steps,
            "total_port_calls": self.total_port_calls,
            "policy_denials": self.total_policy_denials,
            "episodes_stored": self.episodes_stored,
            "schemas_induced": self.schemas_induced,
            "routines_compiled": self.routines_compiled,
            "uptime_seconds": self.uptime_seconds,
            "skill_invocations": self.skill_invocations,
            "port_latency_avg_ms": port_latency_avg,
        })
    }

    /// Format as Prometheus text exposition.
    pub fn format_prometheus(&self) -> String {
        let mut lines = Vec::new();

        lines.push("# HELP soma_active_sessions Number of currently active sessions".to_string());
        lines.push("# TYPE soma_active_sessions gauge".to_string());
        lines.push(format!("soma_active_sessions {}", self.active_sessions));

        lines.push("# HELP soma_completed_sessions Total sessions completed successfully".to_string());
        lines.push("# TYPE soma_completed_sessions counter".to_string());
        lines.push(format!("soma_completed_sessions {}", self.completed_sessions));

        lines.push("# HELP soma_failed_sessions Total sessions that failed".to_string());
        lines.push("# TYPE soma_failed_sessions counter".to_string());
        lines.push(format!("soma_failed_sessions {}", self.failed_sessions));

        lines.push("# HELP soma_aborted_sessions Total sessions that were aborted".to_string());
        lines.push("# TYPE soma_aborted_sessions counter".to_string());
        lines.push(format!("soma_aborted_sessions {}", self.aborted_sessions));

        lines.push("# HELP soma_total_steps Total control-loop steps executed".to_string());
        lines.push("# TYPE soma_total_steps counter".to_string());
        lines.push(format!("soma_total_steps {}", self.total_steps));

        lines.push("# HELP soma_total_port_calls Total port invocations".to_string());
        lines.push("# TYPE soma_total_port_calls counter".to_string());
        lines.push(format!("soma_total_port_calls {}", self.total_port_calls));

        lines.push("# HELP soma_policy_denials Total policy denials".to_string());
        lines.push("# TYPE soma_policy_denials counter".to_string());
        lines.push(format!("soma_policy_denials {}", self.total_policy_denials));

        lines.push("# HELP soma_episodes_stored Number of episodes in memory".to_string());
        lines.push("# TYPE soma_episodes_stored gauge".to_string());
        lines.push(format!("soma_episodes_stored {}", self.episodes_stored));

        lines.push("# HELP soma_schemas_induced Number of schemas registered".to_string());
        lines.push("# TYPE soma_schemas_induced gauge".to_string());
        lines.push(format!("soma_schemas_induced {}", self.schemas_induced));

        lines.push("# HELP soma_routines_compiled Number of routines registered".to_string());
        lines.push("# TYPE soma_routines_compiled gauge".to_string());
        lines.push(format!("soma_routines_compiled {}", self.routines_compiled));

        lines.push("# HELP soma_uptime_seconds Runtime uptime in seconds".to_string());
        lines.push("# TYPE soma_uptime_seconds gauge".to_string());
        lines.push(format!("soma_uptime_seconds {}", self.uptime_seconds));

        if !self.skill_invocations.is_empty() {
            lines.push("# HELP soma_skill_invocations_total Per-skill invocation count".to_string());
            lines.push("# TYPE soma_skill_invocations_total counter".to_string());
            let mut sorted: Vec<_> = self.skill_invocations.iter().collect();
            sorted.sort_by_key(|(k, _)| (*k).clone());
            for (skill, count) in sorted {
                lines.push(format!(
                    "soma_skill_invocations_total{{skill=\"{}\"}} {}",
                    skill, count
                ));
            }
        }

        if !self.port_latencies.is_empty() {
            lines.push("# HELP soma_port_latency_avg_ms Average port latency in milliseconds".to_string());
            lines.push("# TYPE soma_port_latency_avg_ms gauge".to_string());
            let mut sorted: Vec<_> = self.port_latencies.keys().collect();
            sorted.sort();
            for port_id in sorted {
                if let Some(avg) = self.avg_port_latency(port_id) {
                    lines.push(format!(
                        "soma_port_latency_avg_ms{{port=\"{}\"}} {:.1}",
                        port_id, avg
                    ));
                }
            }
        }

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_new_metrics_all_zero() {
        let m = RuntimeMetrics::new();
        assert_eq!(m.active_sessions.load(Ordering::Relaxed), 0);
        assert_eq!(m.completed_sessions.load(Ordering::Relaxed), 0);
        assert_eq!(m.failed_sessions.load(Ordering::Relaxed), 0);
        assert_eq!(m.aborted_sessions.load(Ordering::Relaxed), 0);
        assert_eq!(m.total_steps.load(Ordering::Relaxed), 0);
        assert_eq!(m.total_port_calls.load(Ordering::Relaxed), 0);
        assert_eq!(m.total_policy_denials.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_session_lifecycle_counters() {
        let m = RuntimeMetrics::new();

        m.session_created();
        m.session_created();
        m.session_created();
        assert_eq!(m.active_sessions.load(Ordering::Relaxed), 3);

        m.session_completed();
        assert_eq!(m.active_sessions.load(Ordering::Relaxed), 2);
        assert_eq!(m.completed_sessions.load(Ordering::Relaxed), 1);

        m.session_failed();
        assert_eq!(m.active_sessions.load(Ordering::Relaxed), 1);
        assert_eq!(m.failed_sessions.load(Ordering::Relaxed), 1);

        m.session_aborted();
        assert_eq!(m.active_sessions.load(Ordering::Relaxed), 0);
        assert_eq!(m.aborted_sessions.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_step_counter() {
        let m = RuntimeMetrics::new();
        m.step_executed();
        m.step_executed();
        m.step_executed();
        assert_eq!(m.total_steps.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn test_port_call_tracking() {
        let m = RuntimeMetrics::new();
        m.port_call("filesystem", 10);
        m.port_call("filesystem", 20);
        m.port_call("http", 100);

        assert_eq!(m.total_port_calls.load(Ordering::Relaxed), 3);

        let snap = m.snapshot();
        assert_eq!(snap.port_latencies["filesystem"], vec![10, 20]);
        assert_eq!(snap.port_latencies["http"], vec![100]);
        assert!((snap.avg_port_latency("filesystem").unwrap() - 15.0).abs() < f64::EPSILON);
        assert!((snap.avg_port_latency("http").unwrap() - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_port_latency_cap() {
        let m = RuntimeMetrics::new();
        for i in 0..1500 {
            m.port_call("test_port", i);
        }
        let snap = m.snapshot();
        assert_eq!(snap.port_latencies["test_port"].len(), MAX_LATENCY_SAMPLES);
        // Should have kept the last 1000 samples (500..1499)
        assert_eq!(snap.port_latencies["test_port"][0], 500);
    }

    #[test]
    fn test_skill_invocation_tracking() {
        let m = RuntimeMetrics::new();
        m.skill_invoked("pack.read_file");
        m.skill_invoked("pack.read_file");
        m.skill_invoked("pack.write_file");

        let snap = m.snapshot();
        assert_eq!(snap.skill_invocations["pack.read_file"], 2);
        assert_eq!(snap.skill_invocations["pack.write_file"], 1);
    }

    #[test]
    fn test_policy_denial_counter() {
        let m = RuntimeMetrics::new();
        m.policy_denial();
        m.policy_denial();
        assert_eq!(m.total_policy_denials.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_snapshot_captures_all_fields() {
        let m = RuntimeMetrics::new();
        m.session_created();
        m.session_completed();
        m.step_executed();
        m.port_call("fs", 5);
        m.skill_invoked("s1");
        m.policy_denial();
        m.episodes_stored.store(3, Ordering::Relaxed);
        m.schemas_induced.store(1, Ordering::Relaxed);
        m.routines_compiled.store(2, Ordering::Relaxed);

        let snap = m.snapshot();
        assert_eq!(snap.active_sessions, 0);
        assert_eq!(snap.completed_sessions, 1);
        assert_eq!(snap.total_steps, 1);
        assert_eq!(snap.total_port_calls, 1);
        assert_eq!(snap.total_policy_denials, 1);
        assert_eq!(snap.episodes_stored, 3);
        assert_eq!(snap.schemas_induced, 1);
        assert_eq!(snap.routines_compiled, 2);
        assert!(snap.uptime_seconds < 5); // Should be nearly instant in test
    }

    #[test]
    fn test_session_success_rate() {
        let snap = MetricsSnapshot {
            active_sessions: 0,
            completed_sessions: 7,
            failed_sessions: 2,
            aborted_sessions: 1,
            total_steps: 0,
            total_port_calls: 0,
            total_policy_denials: 0,
            episodes_stored: 0,
            schemas_induced: 0,
            routines_compiled: 0,
            skill_invocations: HashMap::new(),
            port_latencies: HashMap::new(),
            uptime_seconds: 0,
        };
        assert!((snap.session_success_rate() - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn test_session_success_rate_zero_sessions() {
        let snap = MetricsSnapshot {
            active_sessions: 0,
            completed_sessions: 0,
            failed_sessions: 0,
            aborted_sessions: 0,
            total_steps: 0,
            total_port_calls: 0,
            total_policy_denials: 0,
            episodes_stored: 0,
            schemas_induced: 0,
            routines_compiled: 0,
            skill_invocations: HashMap::new(),
            port_latencies: HashMap::new(),
            uptime_seconds: 0,
        };
        assert!((snap.session_success_rate() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_format_text_contains_all_fields() {
        let m = RuntimeMetrics::new();
        m.session_created();
        m.step_executed();
        let snap = m.snapshot();
        let text = snap.format_text();
        assert!(text.contains("Metrics:"));
        assert!(text.contains("active_sessions:"));
        assert!(text.contains("completed_sessions:"));
        assert!(text.contains("failed_sessions:"));
        assert!(text.contains("aborted_sessions:"));
        assert!(text.contains("success_rate:"));
        assert!(text.contains("total_steps:"));
        assert!(text.contains("total_port_calls:"));
        assert!(text.contains("policy_denials:"));
        assert!(text.contains("episodes_stored:"));
        assert!(text.contains("schemas_induced:"));
        assert!(text.contains("routines_compiled:"));
        assert!(text.contains("uptime_seconds:"));
    }

    #[test]
    fn test_format_json_structure() {
        let m = RuntimeMetrics::new();
        m.session_created();
        m.session_completed();
        m.skill_invoked("test.skill");
        m.port_call("fs", 42);

        let snap = m.snapshot();
        let json = snap.format_json();

        assert_eq!(json["active_sessions"], 0);
        assert_eq!(json["completed_sessions"], 1);
        assert!(json["session_success_rate"].as_f64().unwrap() > 0.0);
        assert!(json["skill_invocations"].is_object());
        assert!(json["port_latency_avg_ms"].is_object());
    }

    #[test]
    fn test_format_prometheus() {
        let m = RuntimeMetrics::new();
        m.session_created();
        m.session_completed();
        m.skill_invoked("pack.read_file");
        m.port_call("filesystem", 15);

        let snap = m.snapshot();
        let prom = snap.format_prometheus();

        assert!(prom.contains("# TYPE soma_active_sessions gauge"));
        assert!(prom.contains("soma_active_sessions 0"));
        assert!(prom.contains("# TYPE soma_completed_sessions counter"));
        assert!(prom.contains("soma_completed_sessions 1"));
        assert!(prom.contains("# TYPE soma_total_steps counter"));
        assert!(prom.contains("soma_total_steps 0"));
        assert!(prom.contains("soma_skill_invocations_total{skill=\"pack.read_file\"} 1"));
        assert!(prom.contains("soma_port_latency_avg_ms{port=\"filesystem\"} 15.0"));
        assert!(prom.contains("# TYPE soma_uptime_seconds gauge"));
    }

    #[test]
    fn test_format_prometheus_empty() {
        let m = RuntimeMetrics::new();
        let snap = m.snapshot();
        let prom = snap.format_prometheus();
        assert!(prom.contains("soma_active_sessions 0"));
        assert!(!prom.contains("soma_skill_invocations_total"));
        assert!(!prom.contains("soma_port_latency_avg_ms{"));
    }

    #[test]
    fn test_concurrent_access() {
        let m = Arc::new(RuntimeMetrics::new());
        let mut handles = vec![];

        for _ in 0..10 {
            let m_clone = Arc::clone(&m);
            handles.push(std::thread::spawn(move || {
                for _ in 0..100 {
                    m_clone.session_created();
                    m_clone.step_executed();
                    m_clone.port_call("test", 1);
                    m_clone.skill_invoked("test.skill");
                    m_clone.session_completed();
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(m.active_sessions.load(Ordering::Relaxed), 0);
        assert_eq!(m.completed_sessions.load(Ordering::Relaxed), 1000);
        assert_eq!(m.total_steps.load(Ordering::Relaxed), 1000);
        assert_eq!(m.total_port_calls.load(Ordering::Relaxed), 1000);
    }
}
