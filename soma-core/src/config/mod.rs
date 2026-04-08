//! SOMA configuration system (Spec Section 15).
//!
//! Loads configuration from a TOML file with sensible defaults for all fields.
//! Override order: compiled defaults < `soma.toml` < environment variables (`SOMA_*`) < CLI flags.
//! Invalid values are clamped and logged rather than causing startup failure.

use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Default instance ID derived from the machine hostname.
fn default_id() -> String {
    let hostname = hostname::get().map_or_else(|_| "unknown".to_string(), |h| h.to_string_lossy().to_string());
    format!("soma-{hostname}")
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_backend() -> String {
    "onnx".to_string()
}

fn default_model_dir() -> String {
    "models".to_string()
}

const fn default_max_steps() -> usize {
    16
}

const fn default_max_inference_time_secs() -> u64 {
    5
}

const fn default_rank() -> usize {
    8
}

const fn default_alpha() -> f32 {
    16.0
}

const fn default_true() -> bool {
    true
}

const fn default_adapt_every() -> usize {
    10
}

const fn default_batch() -> usize {
    8
}

const fn default_lr() -> f32 {
    0.001
}

fn default_ckpt_dir() -> String {
    "checkpoints".to_string()
}

const fn default_max_ckpt() -> usize {
    5
}

const fn default_max_exp() -> usize {
    1000
}

const fn default_checkpoint_interval_secs() -> u64 {
    3600
}

fn default_consolidation_trigger() -> String {
    "experience_count".to_string()
}

const fn default_consolidation_threshold() -> u64 {
    500
}

const fn default_min_lora_magnitude() -> f32 {
    0.01
}

const fn default_connection_timeout_secs() -> u64 {
    60
}

fn default_plugins_directory() -> String {
    "plugins".to_string()
}

fn default_bind() -> String {
    "127.0.0.1:9999".to_string()
}

const fn default_max_conn() -> usize {
    100
}

const fn default_max_infer() -> usize {
    10
}

const fn default_max_plugin() -> usize {
    50
}

const fn default_max_plugins_loaded() -> usize {
    50
}

const fn default_max_signal_size() -> usize {
    10_485_760 // 10 MB
}

const fn default_keepalive_interval_secs() -> u64 {
    30
}

const fn default_max_memory_bytes() -> usize {
    536_870_912 // 512 MB
}

const fn default_mcp_max_connections() -> usize {
    10
}

const fn default_confirmation_timeout_secs() -> u64 {
    60
}

fn default_confirmation_patterns() -> Vec<String> {
    vec![
        "DROP".to_string(),
        "DELETE".to_string(),
        "TRUNCATE".to_string(),
    ]
}

const fn default_max_lora_layers() -> usize {
    64
}

fn default_mcp_transport() -> String {
    "stdio".to_string()
}

fn default_mcp_http_bind() -> String {
    "127.0.0.1:3000".to_string()
}

const fn default_max_executions() -> usize {
    500
}

/// Top-level configuration for a SOMA instance, mapping 1:1 to `soma.toml` sections.
#[derive(Debug, Clone, Deserialize)]
#[derive(Default)]
pub struct SomaConfig {
    #[serde(default)]
    pub soma: SomaSection,
    #[serde(default)]
    pub mind: MindSection,
    #[serde(default)]
    pub memory: MemorySection,
    #[serde(default)]
    pub protocol: ProtocolSection,
    #[serde(default)]
    pub resources: ResourceSection,
    #[serde(default)]
    pub mcp: McpSection,
    #[serde(default)]
    pub security: SecuritySection,
    /// Per-plugin configuration passed through as opaque TOML: `[plugins.postgres]`, `[plugins.redis]`, etc.
    #[serde(default)]
    pub plugins: HashMap<String, toml::Value>,
}

fn default_trace_verbosity() -> String {
    "normal".to_string()
}

/// General instance identity and logging settings (`[soma]` section).
#[derive(Debug, Clone, Deserialize)]
pub struct SomaSection {
    #[serde(default = "default_id")]
    pub id: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    /// Program trace verbosity: "terse", "normal", "verbose" (Section 11.5)
    #[serde(default = "default_trace_verbosity")]
    pub trace_verbosity: String,
    /// Directory to search for plugin libraries (Section 15.1)
    #[serde(default = "default_plugins_directory")]
    pub plugins_directory: String,
}

const fn default_temperature() -> f32 {
    1.0
}

/// Neural inference engine settings (`[mind]` section).
#[derive(Debug, Clone, Deserialize)]
pub struct MindSection {
    #[serde(default = "default_backend")]
    pub backend: String,
    #[serde(default = "default_model_dir")]
    pub model_dir: String,
    #[serde(default = "default_max_steps")]
    pub max_program_steps: usize,
    /// Softmax temperature for inference (Section 2.3). Lower = more deterministic.
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// Maximum wall-clock time allowed for a single inference call, in seconds.
    /// Currently advisory: the sync decoder loop in `onnx_engine.rs` is bounded by
    /// `max_steps` which provides an implicit time cap. True `tokio::time::timeout`
    /// enforcement requires async `infer()`, tracked as a future change.
    #[serde(default = "default_max_inference_time_secs")]
    pub max_inference_time_secs: u64,
    #[serde(default)]
    pub lora: LoraConfig,
}

/// `LoRA` (Low-Rank Adaptation) parameters for runtime model adaptation (`[mind.lora]`).
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // Spec config fields: Section 15.1
pub struct LoraConfig {
    #[serde(default = "default_rank")]
    pub default_rank: usize,
    #[serde(default = "default_alpha")]
    pub default_alpha: f32,
    #[serde(default = "default_true")]
    pub adaptation_enabled: bool,
    #[serde(default = "default_adapt_every")]
    pub adapt_every_n_successes: usize,
    #[serde(default = "default_batch")]
    pub adapt_batch_size: usize,
    #[serde(default = "default_lr")]
    pub adapt_learning_rate: f32,
    /// Maximum number of `LoRA` adapter layers (Section 20.1).
    /// Checked at configuration/init time.
    #[serde(default = "default_max_lora_layers")]
    pub max_lora_layers: usize,
}

impl Default for LoraConfig {
    fn default() -> Self {
        Self {
            default_rank: default_rank(),
            default_alpha: default_alpha(),
            adaptation_enabled: default_true(),
            adapt_every_n_successes: default_adapt_every(),
            adapt_batch_size: default_batch(),
            adapt_learning_rate: default_lr(),
            max_lora_layers: default_max_lora_layers(),
        }
    }
}

/// Consolidation configuration (Spec Section 15.1).
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // Spec config fields: Section 15.1
pub struct ConsolidationConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_consolidation_trigger")]
    pub trigger: String,
    #[serde(default = "default_consolidation_threshold")]
    pub threshold: u64,
    #[serde(default = "default_min_lora_magnitude")]
    pub min_lora_magnitude: f32,
}

impl Default for ConsolidationConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            trigger: default_consolidation_trigger(),
            threshold: default_consolidation_threshold(),
            min_lora_magnitude: default_min_lora_magnitude(),
        }
    }
}

/// Encryption configuration (Spec Section 15.1).
#[derive(Debug, Clone, Deserialize)]
#[derive(Default)]
#[allow(dead_code)] // Spec config fields: Section 15.1
pub struct EncryptionConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub key_file: String,
}


/// Checkpoint and experience buffer settings (`[memory]` section).
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // Spec config fields: Section 15.1
pub struct MemorySection {
    #[serde(default = "default_ckpt_dir")]
    pub checkpoint_dir: String,
    #[serde(default = "default_true")]
    pub auto_checkpoint: bool,
    #[serde(default = "default_max_ckpt")]
    pub max_checkpoints: usize,
    #[serde(default = "default_max_exp")]
    pub max_experience_buffer: usize,
    #[serde(default = "default_checkpoint_interval_secs")]
    pub checkpoint_interval_secs: u64,
    #[serde(default)]
    pub consolidation: ConsolidationConfig,
}

/// Synaptic protocol (SOMA-to-SOMA networking) settings (`[protocol]` section).
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // Spec config fields: Section 15.1
pub struct ProtocolSection {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_max_conn")]
    pub max_connections: usize,
    #[serde(default)]
    pub peers: HashMap<String, String>,
    #[serde(default = "default_connection_timeout_secs")]
    pub connection_timeout_secs: u64,
    #[serde(default)]
    pub encryption: EncryptionConfig,
    /// Maximum signal payload size in bytes (Section 20.1). Default 10 MB.
    #[serde(default = "default_max_signal_size")]
    pub max_signal_size: usize,
    /// Keepalive interval in seconds (Section 20.1). Default 30s.
    #[serde(default = "default_keepalive_interval_secs")]
    pub keepalive_interval_secs: u64,
}

/// Concurrency and memory limits (`[resources]` section).
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code, clippy::struct_field_names)] // Spec config fields: Section 15.1; all fields share "max" prefix by design
pub struct ResourceSection {
    #[serde(default = "default_max_infer")]
    pub max_concurrent_inferences: usize,
    #[serde(default = "default_max_plugin")]
    pub max_concurrent_plugin_calls: usize,
    /// Maximum number of plugins that can be loaded simultaneously (Section 20.1).
    /// Checked at plugin registration time.
    #[serde(default = "default_max_plugins_loaded")]
    pub max_plugins_loaded: usize,
    /// Maximum memory usage in bytes (Section 20.1). Default 512 MB.
    #[serde(default = "default_max_memory_bytes")]
    pub max_memory_bytes: usize,
}

/// MCP Server configuration (Whitepaper Section 8).
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // Spec config fields: Section 8
pub struct McpSection {
    /// Transport: "stdio" or "http"
    #[serde(default = "default_mcp_transport")]
    pub transport: String,
    /// HTTP bind address (only used when transport = "http")
    #[serde(default = "default_mcp_http_bind")]
    pub http_bind: String,
    /// Enable MCP server on startup
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Maximum execution history entries
    #[serde(default = "default_max_executions")]
    pub max_execution_history: usize,
    /// Maximum concurrent MCP client connections. Default 10.
    #[serde(default = "default_mcp_max_connections")]
    pub max_connections: usize,
}

impl Default for McpSection {
    fn default() -> Self {
        Self {
            transport: default_mcp_transport(),
            http_bind: default_mcp_http_bind(),
            enabled: default_true(),
            max_execution_history: default_max_executions(),
            max_connections: default_mcp_max_connections(),
        }
    }
}

fn default_admin_token_env() -> String {
    "SOMA_MCP_ADMIN_TOKEN".to_string()
}

fn default_builder_token_env() -> String {
    "SOMA_MCP_BUILDER_TOKEN".to_string()
}

fn default_viewer_token_env() -> String {
    "SOMA_MCP_VIEWER_TOKEN".to_string()
}

/// Security configuration (Whitepaper Sections 8.3, 12.2).
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // Spec config fields: Sections 8.3, 12.2
pub struct SecuritySection {
    /// Require auth tokens for MCP connections
    #[serde(default)]
    pub require_auth: bool,
    /// Environment variable holding the admin token
    #[serde(default = "default_admin_token_env")]
    pub admin_token_env: String,
    /// Environment variable holding the builder token
    #[serde(default = "default_builder_token_env")]
    pub builder_token_env: String,
    /// Environment variable holding the viewer token
    #[serde(default = "default_viewer_token_env")]
    pub viewer_token_env: String,
    /// Destructive actions require two-step confirmation
    #[serde(default = "default_true")]
    pub require_confirmation: bool,
    /// Timeout in seconds for confirmation prompts. Default 60.
    #[serde(default = "default_confirmation_timeout_secs")]
    pub confirmation_timeout_secs: u64,
    /// Patterns that trigger confirmation when `require_confirmation` is true.
    /// Default: `["DROP", "DELETE", "TRUNCATE"]`.
    #[serde(default = "default_confirmation_patterns")]
    pub confirmation_patterns: Vec<String>,
    /// Plugins whose execution is denied (Section 12.2 permission enforcement).
    /// Plugin names in this list will be refused at `execute_step` time.
    #[serde(default)]
    pub denied_plugins: Vec<String>,
}

impl Default for SecuritySection {
    fn default() -> Self {
        Self {
            require_auth: false,
            admin_token_env: default_admin_token_env(),
            builder_token_env: default_builder_token_env(),
            viewer_token_env: default_viewer_token_env(),
            require_confirmation: true,
            confirmation_timeout_secs: default_confirmation_timeout_secs(),
            confirmation_patterns: default_confirmation_patterns(),
            denied_plugins: Vec::new(),
        }
    }
}


impl Default for SomaSection {
    fn default() -> Self {
        Self {
            id: default_id(),
            log_level: default_log_level(),
            trace_verbosity: default_trace_verbosity(),
            plugins_directory: default_plugins_directory(),
        }
    }
}

impl Default for MindSection {
    fn default() -> Self {
        Self {
            backend: default_backend(),
            model_dir: default_model_dir(),
            max_program_steps: default_max_steps(),
            temperature: default_temperature(),
            max_inference_time_secs: default_max_inference_time_secs(),
            lora: LoraConfig::default(),
        }
    }
}

impl Default for MemorySection {
    fn default() -> Self {
        Self {
            checkpoint_dir: default_ckpt_dir(),
            auto_checkpoint: default_true(),
            max_checkpoints: default_max_ckpt(),
            max_experience_buffer: default_max_exp(),
            checkpoint_interval_secs: default_checkpoint_interval_secs(),
            consolidation: ConsolidationConfig::default(),
        }
    }
}

impl Default for ProtocolSection {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            max_connections: default_max_conn(),
            peers: HashMap::new(),
            connection_timeout_secs: default_connection_timeout_secs(),
            encryption: EncryptionConfig::default(),
            max_signal_size: default_max_signal_size(),
            keepalive_interval_secs: default_keepalive_interval_secs(),
        }
    }
}

impl Default for ResourceSection {
    fn default() -> Self {
        Self {
            max_concurrent_inferences: default_max_infer(),
            max_concurrent_plugin_calls: default_max_plugin(),
            max_plugins_loaded: default_max_plugins_loaded(),
            max_memory_bytes: default_max_memory_bytes(),
        }
    }
}

impl SomaConfig {
    /// Load configuration from a TOML file. If the file does not exist,
    /// returns the default configuration.
    /// After parsing, validates values and clamps invalid ones to sensible defaults.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            tracing::info!("No config file at {}, using defaults", path.display());
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)?;
        let mut config: Self = toml::from_str(&content)?;
        config.validate_and_clamp();
        tracing::info!("Loaded config from {}", path.display());
        Ok(config)
    }

    /// Validate configuration values and clamp invalid ones to sensible defaults.
    /// Logs warnings for any values that were out of range.
    fn validate_and_clamp(&mut self) {
        // Temperature must be positive
        if self.mind.temperature <= 0.0 {
            tracing::warn!(
                "mind.temperature must be > 0 (was {}), clamping to 1.0",
                self.mind.temperature
            );
            self.mind.temperature = 1.0;
        }

        // max_program_steps must be positive
        if self.mind.max_program_steps == 0 {
            tracing::warn!(
                "mind.max_program_steps must be > 0 (was 0), clamping to {}",
                default_max_steps()
            );
            self.mind.max_program_steps = default_max_steps();
        }

        // Validate log level
        let valid_levels = ["trace", "debug", "info", "warn", "error"];
        if !valid_levels.contains(&self.soma.log_level.to_lowercase().as_str()) {
            tracing::warn!(
                "soma.log_level '{}' is not a valid level (trace/debug/info/warn/error), defaulting to 'info'",
                self.soma.log_level
            );
            self.soma.log_level = "info".to_string();
        }
    }

    /// Apply environment variable overrides (Section 15.3).
    /// Format: `SOMA_SECTION_KEY` maps to [section].key.
    /// Examples: `SOMA_MIND_TEMPERATURE=0.5`, `SOMA_PROTOCOL_BIND=0.0.0.0:9001`
    pub fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("SOMA_MIND_TEMPERATURE")
            && let Ok(t) = v.parse::<f32>() {
                self.mind.temperature = t;
            }
        if let Ok(v) = std::env::var("SOMA_MIND_MAX_PROGRAM_STEPS")
            && let Ok(s) = v.parse::<usize>() {
                self.mind.max_program_steps = s;
            }
        if let Ok(v) = std::env::var("SOMA_PROTOCOL_BIND") {
            self.protocol.bind = v;
        }
        if let Ok(v) = std::env::var("SOMA_SOMA_ID") {
            self.soma.id = v;
        }
        if let Ok(v) = std::env::var("SOMA_SOMA_LOG_LEVEL") {
            self.soma.log_level = v;
        }
        if let Ok(v) = std::env::var("SOMA_MEMORY_CHECKPOINT_DIR") {
            self.memory.checkpoint_dir = v;
        }
    }
}

/// Hostname retrieval via libc. Used only for the default instance ID.
mod hostname {
    use std::ffi::OsString;

    /// Calls `gethostname(2)` and returns the result as an `OsString`.
    /// Buffer is 256 bytes, which covers `HOST_NAME_MAX` on all supported platforms.
    pub fn get() -> Result<OsString, std::io::Error> {
        let mut buf = vec![0u8; 256];
        // SAFETY: buf is a valid, mutable, properly-sized buffer for gethostname.
        let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast::<i8>(), buf.len()) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        buf.truncate(len);
        Ok(OsString::from(String::from_utf8_lossy(&buf).to_string()))
    }
}
