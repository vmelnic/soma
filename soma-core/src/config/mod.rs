//! SOMA Configuration system (Spec Section 15).
//! TOML-based configuration with sensible defaults.

use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

fn default_id() -> String {
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    format!("soma-{}", hostname)
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

fn default_max_steps() -> usize {
    32
}

fn default_rank() -> usize {
    8
}

fn default_alpha() -> f32 {
    16.0
}

fn default_true() -> bool {
    true
}

fn default_adapt_every() -> usize {
    10
}

fn default_batch() -> usize {
    8
}

fn default_lr() -> f32 {
    0.001
}

fn default_ckpt_dir() -> String {
    "checkpoints".to_string()
}

fn default_max_ckpt() -> usize {
    5
}

fn default_max_exp() -> usize {
    1000
}

fn default_bind() -> String {
    "127.0.0.1:9999".to_string()
}

fn default_max_conn() -> usize {
    16
}

fn default_max_infer() -> usize {
    4
}

fn default_max_plugin() -> usize {
    8
}

#[derive(Debug, Clone, Deserialize)]
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
}

#[derive(Debug, Clone, Deserialize)]
pub struct SomaSection {
    #[serde(default = "default_id")]
    pub id: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MindSection {
    #[serde(default = "default_backend")]
    pub backend: String,
    #[serde(default = "default_model_dir")]
    pub model_dir: String,
    #[serde(default = "default_max_steps")]
    pub max_program_steps: usize,
    #[serde(default)]
    pub lora: LoraConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
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
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemorySection {
    #[serde(default = "default_ckpt_dir")]
    pub checkpoint_dir: String,
    #[serde(default = "default_true")]
    pub auto_checkpoint: bool,
    #[serde(default = "default_max_ckpt")]
    pub max_checkpoints: usize,
    #[serde(default = "default_max_exp")]
    pub max_experience_buffer: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProtocolSection {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_max_conn")]
    pub max_connections: usize,
    #[serde(default)]
    pub peers: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResourceSection {
    #[serde(default = "default_max_infer")]
    pub max_concurrent_inferences: usize,
    #[serde(default = "default_max_plugin")]
    pub max_concurrent_plugin_calls: usize,
}

impl Default for SomaConfig {
    fn default() -> Self {
        Self {
            soma: SomaSection::default(),
            mind: MindSection::default(),
            memory: MemorySection::default(),
            protocol: ProtocolSection::default(),
            resources: ResourceSection::default(),
        }
    }
}

impl Default for SomaSection {
    fn default() -> Self {
        Self {
            id: default_id(),
            log_level: default_log_level(),
        }
    }
}

impl Default for MindSection {
    fn default() -> Self {
        Self {
            backend: default_backend(),
            model_dir: default_model_dir(),
            max_program_steps: default_max_steps(),
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
        }
    }
}

impl Default for ProtocolSection {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            max_connections: default_max_conn(),
            peers: HashMap::new(),
        }
    }
}

impl Default for ResourceSection {
    fn default() -> Self {
        Self {
            max_concurrent_inferences: default_max_infer(),
            max_concurrent_plugin_calls: default_max_plugin(),
        }
    }
}

impl SomaConfig {
    /// Load configuration from a TOML file. If the file does not exist,
    /// returns the default configuration.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            tracing::info!("No config file at {}, using defaults", path.display());
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)?;
        let config: SomaConfig = toml::from_str(&content)?;
        tracing::info!("Loaded config from {}", path.display());
        Ok(config)
    }
}

/// Helper to get hostname, kept simple for cross-platform.
mod hostname {
    use std::ffi::OsString;

    pub fn get() -> Result<OsString, std::io::Error> {
        let mut buf = vec![0u8; 256];
        let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut i8, buf.len()) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        buf.truncate(len);
        Ok(OsString::from(String::from_utf8_lossy(&buf).to_string()))
    }
}
