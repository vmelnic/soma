//! SOMA-next configuration system.
//!
//! Loads configuration from a TOML file with sensible defaults for all fields.
//! Override order: compiled defaults < `soma.toml` < environment variables (`SOMA_*`) < CLI flags.

use serde::Deserialize;
use std::path::Path;

use crate::errors::{Result, SomaError};

// ---------------------------------------------------------------------------
// Default value functions for serde
// ---------------------------------------------------------------------------

fn default_id() -> String {
    #[cfg(feature = "native-hostname")]
    {
        hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string())
    }
    #[cfg(not(feature = "native-hostname"))]
    {
        "unknown".to_string()
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_data_dir() -> String {
    "~/.soma/data".to_string()
}

const fn default_max_steps() -> u32 {
    100
}

const fn default_risk_budget() -> f64 {
    0.5
}

const fn default_latency_budget_ms() -> u64 {
    30_000
}

const fn default_resource_budget() -> f64 {
    100.0
}

const fn default_consolidation_interval_secs() -> u64 {
    300
}

const fn default_reactive_monitor_interval_secs() -> u64 {
    0
}

const fn default_port_health_interval_secs() -> u64 {
    0
}

const fn default_checkpoint_every_n_steps() -> u32 {
    0
}

const fn default_resume_on_boot() -> bool {
    false
}

fn default_mcp_transport() -> String {
    "stdio".to_string()
}

const fn default_true() -> bool {
    true
}

fn default_distributed_bind() -> String {
    "0.0.0.0:9100".to_string()
}

const fn default_rate_limit_rps() -> u32 {
    100
}

const fn default_burst_limit() -> u32 {
    20
}

const fn default_blacklist_threshold() -> u32 {
    50
}

const fn default_heartbeat_interval_ms() -> u64 {
    5000
}

const fn default_heartbeat_max_missed() -> u32 {
    3
}

const fn default_heartbeat_timeout_ms() -> u64 {
    2000
}

const fn default_clustering_threshold() -> f64 {
    0.8
}

const fn default_min_support() -> f64 {
    0.7
}

const fn default_min_episodes() -> usize {
    3
}

const fn default_embedder_dimensions() -> usize {
    128
}

const fn default_max_pattern_length() -> usize {
    20
}

const fn default_max_results() -> usize {
    1000
}

// ---------------------------------------------------------------------------
// Config sections
// ---------------------------------------------------------------------------

/// Top-level configuration, mapping 1:1 to `soma.toml` sections.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SomaConfig {
    #[serde(default)]
    pub soma: SomaSection,
    #[serde(default)]
    pub runtime: RuntimeSection,
    #[serde(default)]
    pub mcp: McpSection,
    #[serde(default)]
    pub ports: PortsSection,
    #[serde(default)]
    pub distributed: DistributedSection,
    #[serde(default)]
    pub learning: LearningSection,
    #[serde(default)]
    pub webhooks: WebhooksSection,
    #[serde(default)]
    pub scheduler: SchedulerSection,
}

/// Instance identity and logging (`[soma]` section).
#[derive(Debug, Clone, Deserialize)]
pub struct SomaSection {
    /// Instance identifier. Defaults to the machine hostname.
    #[serde(default = "default_id")]
    pub id: String,

    /// Log verbosity: trace, debug, info, warn, error.
    #[serde(default = "default_log_level")]
    pub log_level: String,

    /// Directory for persistent memory data (episodes, schemas, routines).
    /// Supports `~` expansion. Default: `~/.soma/data`.
    #[serde(default = "default_data_dir")]
    pub data_dir: String,
}

impl Default for SomaSection {
    fn default() -> Self {
        Self {
            id: default_id(),
            log_level: default_log_level(),
            data_dir: default_data_dir(),
        }
    }
}

/// Session controller defaults (`[runtime]` section).
#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeSection {
    /// Maximum control-loop steps before a session is force-terminated.
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,

    /// Default risk budget allocated to new sessions (0.0 to 1.0).
    #[serde(default = "default_risk_budget")]
    pub default_risk_budget: f64,

    /// Default latency budget in milliseconds.
    #[serde(default = "default_latency_budget_ms")]
    pub default_latency_budget_ms: u64,

    /// Default resource budget (abstract units consumed by port calls).
    #[serde(default = "default_resource_budget")]
    pub default_resource_budget: f64,

    /// Interval in seconds between background consolidation cycles.
    /// Set to 0 to disable background consolidation.
    #[serde(default = "default_consolidation_interval_secs")]
    pub consolidation_interval_secs: u64,

    /// Interval in seconds between reactive monitor ticks.
    /// Set to 0 to disable the reactive monitor. When enabled, the monitor
    /// scans the world state for changes and fires autonomous routines whose
    /// match conditions are satisfied.
    #[serde(default = "default_reactive_monitor_interval_secs")]
    pub reactive_monitor_interval_secs: u64,

    /// Interval in seconds between port health monitor ticks.
    /// Set to 0 to disable. When enabled, the monitor analyzes per-port
    /// latency samples and emits health facts into world state.
    #[serde(default = "default_port_health_interval_secs")]
    pub port_health_interval_secs: u64,

    /// Write a mid-run session checkpoint every N control-loop steps.
    /// Set to 0 to disable mid-run checkpointing (terminal checkpoints
    /// from the CLI path still fire). Mid-run checkpoints let an
    /// interrupted session resume on boot without losing progress.
    #[serde(default = "default_checkpoint_every_n_steps")]
    pub checkpoint_every_n_steps: u32,

    /// When true, on boot the runtime loads non-terminal session
    /// checkpoints from disk and marks them for resumption. Requires a
    /// non-zero `checkpoint_every_n_steps` to be useful in practice.
    #[serde(default = "default_resume_on_boot")]
    pub resume_sessions_on_boot: bool,
}

impl Default for RuntimeSection {
    fn default() -> Self {
        Self {
            max_steps: default_max_steps(),
            default_risk_budget: default_risk_budget(),
            default_latency_budget_ms: default_latency_budget_ms(),
            default_resource_budget: default_resource_budget(),
            consolidation_interval_secs: default_consolidation_interval_secs(),
            reactive_monitor_interval_secs: default_reactive_monitor_interval_secs(),
            port_health_interval_secs: default_port_health_interval_secs(),
            checkpoint_every_n_steps: default_checkpoint_every_n_steps(),
            resume_sessions_on_boot: default_resume_on_boot(),
        }
    }
}

/// MCP interface settings (`[mcp]` section).
#[derive(Debug, Clone, Deserialize)]
pub struct McpSection {
    /// Transport protocol: "stdio" or "http".
    #[serde(default = "default_mcp_transport")]
    pub transport: String,

    /// Whether the MCP server starts automatically.
    #[serde(default)]
    pub enabled: bool,
}

impl Default for McpSection {
    fn default() -> Self {
        Self {
            transport: default_mcp_transport(),
            enabled: false,
        }
    }
}

/// Distributed transport settings (`[distributed]` section).
#[derive(Debug, Clone, Deserialize)]
pub struct DistributedSection {
    /// Bind address for the TCP listener.
    #[serde(default = "default_distributed_bind")]
    pub bind: String,

    /// Path to a PEM-encoded TLS certificate for the server.
    /// When set (along with `tls_key`), the transport uses TLS.
    #[serde(default)]
    pub tls_cert: Option<String>,

    /// Path to a PEM-encoded TLS private key for the server.
    #[serde(default)]
    pub tls_key: Option<String>,

    /// Path to a PEM-encoded CA certificate for client-side verification.
    /// When set, the TLS client verifies the server against this CA.
    /// When absent, the system root CA store is used.
    #[serde(default)]
    pub tls_ca: Option<String>,

    /// Maximum sustained requests per second per peer (token bucket refill rate).
    #[serde(default = "default_rate_limit_rps")]
    pub rate_limit_rps: u32,

    /// Extra burst capacity above the steady-state rate (token bucket size = rps + burst).
    #[serde(default = "default_burst_limit")]
    pub burst_limit: u32,

    /// Number of consecutive rate-limit violations before a peer is blacklisted.
    #[serde(default = "default_blacklist_threshold")]
    pub blacklist_threshold: u32,

    /// Whether per-peer rate limiting is active. When false, all requests are allowed.
    #[serde(default = "default_true")]
    pub rate_limit_enabled: bool,

    /// Whether the blacklist mechanism is active. When false, peers are never banned.
    #[serde(default = "default_true")]
    pub blacklist_enabled: bool,

    /// Interval between heartbeat rounds, in milliseconds.
    #[serde(default = "default_heartbeat_interval_ms")]
    pub heartbeat_interval_ms: u64,

    /// Number of consecutive missed heartbeats before marking a peer unavailable.
    #[serde(default = "default_heartbeat_max_missed")]
    pub heartbeat_max_missed: u32,

    /// Per-peer ping timeout in milliseconds (connect + response).
    #[serde(default = "default_heartbeat_timeout_ms")]
    pub heartbeat_timeout_ms: u64,
}

impl Default for DistributedSection {
    fn default() -> Self {
        Self {
            bind: default_distributed_bind(),
            tls_cert: None,
            tls_key: None,
            tls_ca: None,
            rate_limit_rps: default_rate_limit_rps(),
            burst_limit: default_burst_limit(),
            blacklist_threshold: default_blacklist_threshold(),
            rate_limit_enabled: true,
            blacklist_enabled: true,
            heartbeat_interval_ms: default_heartbeat_interval_ms(),
            heartbeat_max_missed: default_heartbeat_max_missed(),
            heartbeat_timeout_ms: default_heartbeat_timeout_ms(),
        }
    }
}

impl DistributedSection {
    /// Build a `RateLimitConfig` from the distributed section settings.
    #[cfg(feature = "distributed")]
    pub fn rate_limit_config(&self) -> crate::distributed::rate_limit::RateLimitConfig {
        crate::distributed::rate_limit::RateLimitConfig {
            max_requests_per_second: self.rate_limit_rps,
            burst_limit: self.burst_limit,
            blacklist_threshold: self.blacklist_threshold,
            blacklist_duration: std::time::Duration::from_secs(300),
            rate_limit_enabled: self.rate_limit_enabled,
            blacklist_enabled: self.blacklist_enabled,
        }
    }

    /// Returns a `TlsConfig` if both cert and key paths are configured.
    pub fn tls_config(&self) -> Option<TlsConfig> {
        match (&self.tls_cert, &self.tls_key) {
            (Some(cert), Some(key)) => Some(TlsConfig {
                cert_path: cert.clone(),
                key_path: key.clone(),
                ca_path: self.tls_ca.clone(),
            }),
            _ => None,
        }
    }
}

/// TLS configuration extracted from the `[distributed]` section.
/// Present only when both `tls_cert` and `tls_key` are set.
#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub cert_path: String,
    pub key_path: String,
    pub ca_path: Option<String>,
}

/// Port configuration (`[ports]` section).
#[derive(Debug, Clone, Deserialize)]
pub struct PortsSection {
    /// Enable the filesystem port.
    #[serde(default = "default_true")]
    pub filesystem_enabled: bool,

    /// Enable the HTTP port.
    #[serde(default = "default_true")]
    pub http_enabled: bool,

    /// Directories to search for dynamic port libraries (.dylib/.so).
    #[serde(default)]
    pub plugin_path: Vec<String>,

    /// Allow pack ports to spawn child processes.
    #[serde(default)]
    pub process_access_enabled: bool,

    /// Require Ed25519 signatures for dynamic port libraries.
    /// When true, port libraries without valid `.sig`/`.pub` sidecar files are
    /// rejected. Default false for development; set true for production.
    #[serde(default)]
    pub require_signatures: bool,
}

impl Default for PortsSection {
    fn default() -> Self {
        Self {
            filesystem_enabled: true,
            http_enabled: true,
            plugin_path: Vec::new(),
            process_access_enabled: false,
            require_signatures: false,
        }
    }
}

/// Learning pipeline settings (`[learning]` section).
///
/// Controls how the memory system induces schemas from episodes and mines
/// frequent skill subsequences via PrefixSpan.
#[derive(Debug, Clone, Deserialize)]
pub struct LearningSection {
    /// Cosine similarity threshold for embedding-based episode clustering.
    #[serde(default = "default_clustering_threshold")]
    pub clustering_threshold: f64,

    /// Minimum support ratio for PrefixSpan pattern mining (0.0 to 1.0).
    #[serde(default = "default_min_support")]
    pub min_support: f64,

    /// Minimum number of successful episodes required to induce a schema.
    #[serde(default = "default_min_episodes")]
    pub min_episodes: usize,

    /// Dimensionality of the HashEmbedder output vectors.
    #[serde(default = "default_embedder_dimensions")]
    pub embedder_dimensions: usize,

    /// Maximum pattern length for PrefixSpan mining.
    #[serde(default = "default_max_pattern_length")]
    pub max_pattern_length: usize,

    /// Maximum number of frequent patterns returned by PrefixSpan.
    #[serde(default = "default_max_results")]
    pub max_results: usize,
}

impl Default for LearningSection {
    fn default() -> Self {
        Self {
            clustering_threshold: default_clustering_threshold(),
            min_support: default_min_support(),
            min_episodes: default_min_episodes(),
            embedder_dimensions: default_embedder_dimensions(),
            max_pattern_length: default_max_pattern_length(),
            max_results: default_max_results(),
        }
    }
}

/// Webhook action configuration (`[webhooks]` section).
///
/// Each entry under `[webhooks.trigger_goal.<name>]` turns an incoming
/// POST to `/<name>` into an async goal whose objective is rendered from
/// the request payload via `{{field.path}}` placeholders. Hooks that are
/// not listed here fall back to the default `deposit_fact` behavior.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WebhooksSection {
    #[serde(default)]
    pub trigger_goal: std::collections::HashMap<String, WebhookGoalTrigger>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebhookGoalTrigger {
    /// Goal objective template. Supports `{{path.to.field}}` substitution
    /// against the POST body (interpreted as JSON).
    pub objective_template: String,
    /// Optional per-hook override for the session step budget.
    #[serde(default)]
    pub max_steps: Option<u32>,
}

/// Scheduler configuration (`[scheduler]` section). Operators declare
/// cron-style or interval-style recurring goals here; on boot the runtime
/// materializes each entry into a `Schedule` with `goal_trigger` set.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SchedulerSection {
    #[serde(default)]
    pub goal: std::collections::HashMap<String, ScheduledGoal>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScheduledGoal {
    /// Goal objective to launch each time this schedule fires.
    pub objective: String,
    /// Cron expression (seconds-granularity, `cron` crate format).
    /// Mutually exclusive with `interval_ms`.
    #[serde(default)]
    pub cron_expr: Option<String>,
    /// Recurring interval in milliseconds. Mutually exclusive with
    /// `cron_expr`.
    #[serde(default)]
    pub interval_ms: Option<u64>,
    /// Optional per-goal override for the session step budget.
    #[serde(default)]
    pub max_steps: Option<u32>,
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

impl SomaConfig {
    /// Load configuration from a TOML file.
    ///
    /// If the file does not exist, returns the compiled defaults.
    /// After parsing, validates values and clamps any that are out of range.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            tracing::info!("No config file at {}, using defaults", path.display());
            let mut config = Self::default();
            config.apply_env_overrides();
            return Ok(config);
        }

        let content = std::fs::read_to_string(path).map_err(|e| {
            SomaError::Config(format!("failed to read {}: {e}", path.display()))
        })?;

        let mut config: Self = toml::from_str(&content).map_err(|e| {
            SomaError::Config(format!("failed to parse {}: {e}", path.display()))
        })?;

        config.validate_and_clamp();
        config.apply_env_overrides();

        tracing::info!("Loaded config from {}", path.display());
        Ok(config)
    }

    /// Validate configuration values and clamp invalid ones to sensible defaults.
    fn validate_and_clamp(&mut self) {
        if self.runtime.max_steps == 0 {
            tracing::warn!(
                "runtime.max_steps must be > 0 (was 0), clamping to {}",
                default_max_steps()
            );
            self.runtime.max_steps = default_max_steps();
        }

        if self.runtime.default_risk_budget < 0.0 || self.runtime.default_risk_budget > 1.0 {
            tracing::warn!(
                "runtime.default_risk_budget must be in [0.0, 1.0] (was {}), clamping to {}",
                self.runtime.default_risk_budget,
                default_risk_budget()
            );
            self.runtime.default_risk_budget = default_risk_budget();
        }

        if self.runtime.default_resource_budget < 0.0 {
            tracing::warn!(
                "runtime.default_resource_budget must be >= 0 (was {}), clamping to {}",
                self.runtime.default_resource_budget,
                default_resource_budget()
            );
            self.runtime.default_resource_budget = default_resource_budget();
        }

        if self.learning.clustering_threshold < 0.0 || self.learning.clustering_threshold > 1.0 {
            tracing::warn!(
                "learning.clustering_threshold must be in [0.0, 1.0] (was {}), clamping to {}",
                self.learning.clustering_threshold,
                default_clustering_threshold()
            );
            self.learning.clustering_threshold = default_clustering_threshold();
        }

        if self.learning.min_support < 0.0 || self.learning.min_support > 1.0 {
            tracing::warn!(
                "learning.min_support must be in [0.0, 1.0] (was {}), clamping to {}",
                self.learning.min_support,
                default_min_support()
            );
            self.learning.min_support = default_min_support();
        }

        if self.learning.min_episodes == 0 {
            tracing::warn!(
                "learning.min_episodes must be > 0 (was 0), clamping to {}",
                default_min_episodes()
            );
            self.learning.min_episodes = default_min_episodes();
        }

        if self.learning.embedder_dimensions == 0 {
            tracing::warn!(
                "learning.embedder_dimensions must be > 0 (was 0), clamping to {}",
                default_embedder_dimensions()
            );
            self.learning.embedder_dimensions = default_embedder_dimensions();
        }

        if self.learning.max_pattern_length == 0 {
            tracing::warn!(
                "learning.max_pattern_length must be > 0 (was 0), clamping to {}",
                default_max_pattern_length()
            );
            self.learning.max_pattern_length = default_max_pattern_length();
        }

        if self.learning.max_results == 0 {
            tracing::warn!(
                "learning.max_results must be > 0 (was 0), clamping to {}",
                default_max_results()
            );
            self.learning.max_results = default_max_results();
        }

        let valid_levels = ["trace", "debug", "info", "warn", "error"];
        if !valid_levels.contains(&self.soma.log_level.to_lowercase().as_str()) {
            tracing::warn!(
                "soma.log_level '{}' is not valid (trace/debug/info/warn/error), defaulting to 'info'",
                self.soma.log_level
            );
            self.soma.log_level = "info".to_string();
        }
    }

    /// Apply environment variable overrides.
    ///
    /// Format: `SOMA_SECTION_KEY` maps to `[section].key`.
    pub fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("SOMA_SOMA_ID") {
            self.soma.id = v;
        }
        if let Ok(v) = std::env::var("SOMA_SOMA_LOG_LEVEL") {
            self.soma.log_level = v;
        }
        if let Ok(v) = std::env::var("SOMA_SOMA_DATA_DIR") {
            self.soma.data_dir = v;
        }
        if let Ok(v) = std::env::var("SOMA_RUNTIME_MAX_STEPS")
            && let Ok(n) = v.parse::<u32>() {
                self.runtime.max_steps = n;
            }
        if let Ok(v) = std::env::var("SOMA_RUNTIME_RISK_BUDGET")
            && let Ok(n) = v.parse::<f64>() {
                self.runtime.default_risk_budget = n;
            }
        if let Ok(v) = std::env::var("SOMA_RUNTIME_LATENCY_BUDGET_MS")
            && let Ok(n) = v.parse::<u64>() {
                self.runtime.default_latency_budget_ms = n;
            }
        if let Ok(v) = std::env::var("SOMA_RUNTIME_CONSOLIDATION_INTERVAL_SECS")
            && let Ok(n) = v.parse::<u64>() {
                self.runtime.consolidation_interval_secs = n;
            }
        if let Ok(v) = std::env::var("SOMA_RUNTIME_REACTIVE_MONITOR_INTERVAL_SECS")
            && let Ok(n) = v.parse::<u64>() {
                self.runtime.reactive_monitor_interval_secs = n;
            }
        if let Ok(v) = std::env::var("SOMA_RUNTIME_PORT_HEALTH_INTERVAL_SECS")
            && let Ok(n) = v.parse::<u64>() {
                self.runtime.port_health_interval_secs = n;
            }
        if let Ok(v) = std::env::var("SOMA_RUNTIME_CHECKPOINT_EVERY_N_STEPS")
            && let Ok(n) = v.parse::<u32>() {
                self.runtime.checkpoint_every_n_steps = n;
            }
        if let Ok(v) = std::env::var("SOMA_RUNTIME_RESUME_SESSIONS_ON_BOOT")
            && let Ok(b) = v.parse::<bool>() {
                self.runtime.resume_sessions_on_boot = b;
            }
        if let Ok(v) = std::env::var("SOMA_MCP_TRANSPORT") {
            self.mcp.transport = v;
        }
        if let Ok(v) = std::env::var("SOMA_MCP_ENABLED")
            && let Ok(b) = v.parse::<bool>() {
                self.mcp.enabled = b;
            }
        if let Ok(v) = std::env::var("SOMA_DISTRIBUTED_BIND") {
            self.distributed.bind = v;
        }
        if let Ok(v) = std::env::var("SOMA_DISTRIBUTED_TLS_CERT") {
            self.distributed.tls_cert = Some(v);
        }
        if let Ok(v) = std::env::var("SOMA_DISTRIBUTED_TLS_KEY") {
            self.distributed.tls_key = Some(v);
        }
        if let Ok(v) = std::env::var("SOMA_DISTRIBUTED_TLS_CA") {
            self.distributed.tls_ca = Some(v);
        }
        if let Ok(v) = std::env::var("SOMA_DISTRIBUTED_RATE_LIMIT_ENABLED")
            && let Ok(b) = v.parse::<bool>() {
                self.distributed.rate_limit_enabled = b;
            }
        if let Ok(v) = std::env::var("SOMA_DISTRIBUTED_BLACKLIST_ENABLED")
            && let Ok(b) = v.parse::<bool>() {
                self.distributed.blacklist_enabled = b;
            }
        if let Ok(v) = std::env::var("SOMA_DISTRIBUTED_HEARTBEAT_INTERVAL_MS")
            && let Ok(n) = v.parse::<u64>() {
                self.distributed.heartbeat_interval_ms = n;
            }
        if let Ok(v) = std::env::var("SOMA_DISTRIBUTED_HEARTBEAT_MAX_MISSED")
            && let Ok(n) = v.parse::<u32>() {
                self.distributed.heartbeat_max_missed = n;
            }
        if let Ok(v) = std::env::var("SOMA_DISTRIBUTED_HEARTBEAT_TIMEOUT_MS")
            && let Ok(n) = v.parse::<u64>() {
                self.distributed.heartbeat_timeout_ms = n;
            }
        if let Ok(v) = std::env::var("SOMA_LEARNING_CLUSTERING_THRESHOLD")
            && let Ok(n) = v.parse::<f64>() {
                self.learning.clustering_threshold = n;
            }
        if let Ok(v) = std::env::var("SOMA_LEARNING_MIN_SUPPORT")
            && let Ok(n) = v.parse::<f64>() {
                self.learning.min_support = n;
            }
        if let Ok(v) = std::env::var("SOMA_LEARNING_MIN_EPISODES")
            && let Ok(n) = v.parse::<usize>() {
                self.learning.min_episodes = n;
            }
        if let Ok(v) = std::env::var("SOMA_LEARNING_EMBEDDER_DIMENSIONS")
            && let Ok(n) = v.parse::<usize>() {
                self.learning.embedder_dimensions = n;
            }
        if let Ok(v) = std::env::var("SOMA_LEARNING_MAX_PATTERN_LENGTH")
            && let Ok(n) = v.parse::<usize>() {
                self.learning.max_pattern_length = n;
            }
        if let Ok(v) = std::env::var("SOMA_LEARNING_MAX_RESULTS")
            && let Ok(n) = v.parse::<usize>() {
                self.learning.max_results = n;
            }
        if let Ok(v) = std::env::var("SOMA_PORTS_REQUIRE_SIGNATURES")
            && let Ok(b) = v.parse::<bool>() {
                self.ports.require_signatures = b;
            }
        if let Ok(v) = std::env::var("SOMA_PORTS_PLUGIN_PATH") {
            self.ports.plugin_path = v
                .split(':')
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn default_config_has_sensible_values() {
        let cfg = SomaConfig::default();
        assert_eq!(cfg.runtime.max_steps, 100);
        assert!((cfg.runtime.default_risk_budget - 0.5).abs() < f64::EPSILON);
        assert_eq!(cfg.runtime.default_latency_budget_ms, 30_000);
        assert!((cfg.runtime.default_resource_budget - 100.0).abs() < f64::EPSILON);
        assert_eq!(cfg.mcp.transport, "stdio");
        assert!(!cfg.mcp.enabled);
        assert!(cfg.ports.filesystem_enabled);
        assert!(cfg.ports.http_enabled);

        // Learning defaults
        assert!((cfg.learning.clustering_threshold - 0.8).abs() < f64::EPSILON);
        assert!((cfg.learning.min_support - 0.7).abs() < f64::EPSILON);
        assert_eq!(cfg.learning.min_episodes, 3);
        assert_eq!(cfg.learning.embedder_dimensions, 128);
        assert_eq!(cfg.learning.max_pattern_length, 20);
        assert_eq!(cfg.learning.max_results, 1000);
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let path = std::path::PathBuf::from("/tmp/soma_test_nonexistent.toml");
        let cfg = SomaConfig::load(&path).unwrap();
        assert_eq!(cfg.runtime.max_steps, 100);
    }

    #[test]
    fn load_valid_toml() {
        let dir = std::env::temp_dir().join("soma_config_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("soma.toml");

        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[soma]
id = "test-node"
log_level = "debug"

[runtime]
max_steps = 50
default_risk_budget = 0.3

[mcp]
transport = "http"
enabled = true

[ports]
filesystem_enabled = false
"#
        )
        .unwrap();

        let cfg = SomaConfig::load(&path).unwrap();
        assert_eq!(cfg.soma.id, "test-node");
        assert_eq!(cfg.soma.log_level, "debug");
        assert_eq!(cfg.runtime.max_steps, 50);
        assert!((cfg.runtime.default_risk_budget - 0.3).abs() < f64::EPSILON);
        assert_eq!(cfg.mcp.transport, "http");
        assert!(cfg.mcp.enabled);
        assert!(!cfg.ports.filesystem_enabled);
        assert!(cfg.ports.http_enabled); // default

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_risk_budget_gets_clamped() {
        let dir = std::env::temp_dir().join("soma_config_clamp_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("soma.toml");

        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[runtime]
default_risk_budget = 5.0
"#
        )
        .unwrap();

        let cfg = SomaConfig::load(&path).unwrap();
        assert!((cfg.runtime.default_risk_budget - 0.5).abs() < f64::EPSILON);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn zero_max_steps_gets_clamped() {
        let dir = std::env::temp_dir().join("soma_config_steps_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("soma.toml");

        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[runtime]
max_steps = 0
"#
        )
        .unwrap();

        let cfg = SomaConfig::load(&path).unwrap();
        assert_eq!(cfg.runtime.max_steps, 100);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_log_level_gets_clamped() {
        let dir = std::env::temp_dir().join("soma_config_log_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("soma.toml");

        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[soma]
log_level = "banana"
"#
        )
        .unwrap();

        let cfg = SomaConfig::load(&path).unwrap();
        assert_eq!(cfg.soma.log_level, "info");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn distributed_tls_config_parsed() {
        let dir = std::env::temp_dir().join("soma_config_tls_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("soma.toml");

        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[distributed]
bind = "0.0.0.0:9200"
tls_cert = "/path/to/cert.pem"
tls_key = "/path/to/key.pem"
tls_ca = "/path/to/ca.pem"
"#
        )
        .unwrap();

        let cfg = SomaConfig::load(&path).unwrap();
        assert_eq!(cfg.distributed.bind, "0.0.0.0:9200");
        assert_eq!(cfg.distributed.tls_cert.as_deref(), Some("/path/to/cert.pem"));
        assert_eq!(cfg.distributed.tls_key.as_deref(), Some("/path/to/key.pem"));
        assert_eq!(cfg.distributed.tls_ca.as_deref(), Some("/path/to/ca.pem"));

        let tls = cfg.distributed.tls_config();
        assert!(tls.is_some());
        let tls = tls.unwrap();
        assert_eq!(tls.cert_path, "/path/to/cert.pem");
        assert_eq!(tls.key_path, "/path/to/key.pem");
        assert_eq!(tls.ca_path.as_deref(), Some("/path/to/ca.pem"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn distributed_tls_config_none_when_incomplete() {
        let dir = std::env::temp_dir().join("soma_config_tls_none_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("soma.toml");

        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[distributed]
tls_cert = "/path/to/cert.pem"
"#
        )
        .unwrap();

        let cfg = SomaConfig::load(&path).unwrap();
        // Only cert is set, no key — tls_config() should return None.
        assert!(cfg.distributed.tls_config().is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn distributed_defaults_when_absent() {
        let cfg = SomaConfig::default();
        assert_eq!(cfg.distributed.bind, "0.0.0.0:9100");
        assert!(cfg.distributed.tls_cert.is_none());
        assert!(cfg.distributed.tls_key.is_none());
        assert!(cfg.distributed.tls_ca.is_none());
        assert!(cfg.distributed.tls_config().is_none());
    }

    #[test]
    fn parse_error_returns_config_error() {
        let dir = std::env::temp_dir().join("soma_config_parse_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("soma.toml");

        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "this is not valid toml {{{{").unwrap();

        let err = SomaConfig::load(&path).unwrap_err();
        assert!(matches!(err, SomaError::Config(_)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plugin_path_env_override() {
        let mut cfg = SomaConfig::default();
        assert!(cfg.ports.plugin_path.is_empty());

        unsafe { std::env::set_var("SOMA_PORTS_PLUGIN_PATH", "/tmp/ports:/opt/ports"); }
        cfg.apply_env_overrides();
        unsafe { std::env::remove_var("SOMA_PORTS_PLUGIN_PATH"); }

        assert_eq!(cfg.ports.plugin_path, vec!["/tmp/ports", "/opt/ports"]);
    }
}
