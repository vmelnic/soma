//! Checkpoint persistence — save and restore the full learned state of a SOMA instance.
//!
//! File format: 4-byte magic (`SOMA`) + 4-byte little-endian version + JSON body.
//! Version 2 adds `base_model_hash`, `plugin_manifest`, and `merged_opcode_delta`.
//! Older v1 checkpoints are loaded via `#[serde(default)]` on new fields.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::mind::lora::LoRALayerState;

/// File magic bytes identifying a SOMA checkpoint.
pub const CHECKPOINT_MAGIC: &[u8; 4] = b"SOMA";
/// Current checkpoint format version. Loader accepts 1 through this value.
pub const CHECKPOINT_VERSION: u32 = 2;

/// Complete snapshot of a SOMA instance's learned state.
///
/// Captures everything needed to resume from where a SOMA left off:
/// `LoRA` weights, experience counts, plugin states, decision history,
/// and the accumulated consolidation delta.
#[derive(Serialize, Deserialize)]
pub struct Checkpoint {
    /// Format version (for forward/backward compatibility).
    pub version: u32,
    /// Unique identifier of the SOMA instance that created this checkpoint.
    pub soma_id: String,
    /// Unix timestamp (seconds) when the checkpoint was created.
    pub timestamp: u64,
    /// Serialized `LoRA` layer weights for each adapted layer.
    pub lora_state: Vec<LoRALayerState>,
    /// Total successful experiences recorded before this checkpoint.
    pub experience_count: u64,
    /// Total `LoRA` adaptation cycles completed before this checkpoint.
    pub adaptation_count: u64,
    /// Plugin-specific state snapshots (institutional memory).
    #[serde(default)]
    pub plugin_states: Vec<PluginStateEntry>,
    /// Decision log entries — what was built, why, and when.
    #[serde(default)]
    pub decisions: Vec<serde_json::Value>,
    /// Bounded recent execution history for auditing.
    #[serde(default)]
    pub recent_executions: Vec<serde_json::Value>,
    /// SHA-256 of the base ONNX model; detects model changes on restore
    /// so `LoRA` state incompatibility can be warned about.
    #[serde(default)]
    pub base_model_hash: String,
    /// Which plugins (name + version) were loaded when this checkpoint was taken.
    #[serde(default)]
    pub plugin_manifest: Vec<PluginManifestEntry>,
    /// Accumulated consolidation delta for the opcode head.
    /// Shape: `num_conventions * decoder_dim`, row-major. Applied during inference
    /// as `logits += hidden @ delta.T` since tract-onnx graphs are frozen.
    #[serde(default)]
    pub merged_opcode_delta: Vec<f32>,
}

/// A single plugin's serialized state within a checkpoint.
#[derive(Serialize, Deserialize)]
pub struct PluginStateEntry {
    pub plugin_name: String,
    pub state: serde_json::Value,
}

/// Records a plugin's identity at checkpoint time for compatibility verification on restore.
#[derive(Serialize, Deserialize)]
pub struct PluginManifestEntry {
    pub name: String,
    pub version: String,
}

impl Checkpoint {
    /// Create a new v2 checkpoint stamped with the current wall-clock time.
    pub fn new(soma_id: String, lora_state: Vec<LoRALayerState>, experience_count: u64, adaptation_count: u64) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            version: CHECKPOINT_VERSION,
            soma_id,
            timestamp,
            lora_state,
            experience_count,
            adaptation_count,
            plugin_states: Vec::new(),
            decisions: Vec::new(),
            recent_executions: Vec::new(),
            base_model_hash: String::new(),
            plugin_manifest: Vec::new(),
            merged_opcode_delta: Vec::new(),
        }
    }

    /// Serialize and write this checkpoint to disk.
    ///
    /// Wire format: `[SOMA][version_le32][JSON body]`. Creates parent directories if needed.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .context("Failed to create checkpoint directory")?;
        }

        let json = serde_json::to_vec(self)
            .context("Failed to serialize checkpoint")?;

        let mut data = Vec::with_capacity(8 + json.len());
        data.extend_from_slice(CHECKPOINT_MAGIC);
        data.extend_from_slice(&CHECKPOINT_VERSION.to_le_bytes());
        data.extend(json);

        std::fs::write(path, &data)
            .context("Failed to write checkpoint file")?;

        tracing::info!(
            path = %path.display(),
            lora_layers = self.lora_state.len(),
            experiences = self.experience_count,
            "Checkpoint saved"
        );
        Ok(())
    }

    /// Load a checkpoint from disk, verifying magic and version compatibility.
    ///
    /// Accepts versions 1 through `CHECKPOINT_VERSION`. Missing v2 fields
    /// default to empty via `#[serde(default)]`.
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read(path)
            .context("Failed to read checkpoint file")?;

        if data.len() < 8 {
            return Err(anyhow::anyhow!("Checkpoint file too small"));
        }

        if &data[0..4] != CHECKPOINT_MAGIC {
            return Err(anyhow::anyhow!(
                "Invalid checkpoint magic: expected SOMA, got {:?}",
                &data[0..4]
            ));
        }

        let version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        if version == 0 || version > CHECKPOINT_VERSION {
            return Err(anyhow::anyhow!(
                "Unsupported checkpoint version: {version} (supported: 1-{CHECKPOINT_VERSION})"
            ));
        }

        let checkpoint: Self = serde_json::from_slice(&data[8..])
            .context("Failed to deserialize checkpoint")?;

        tracing::info!(
            path = %path.display(),
            lora_layers = checkpoint.lora_state.len(),
            experiences = checkpoint.experience_count,
            "Checkpoint loaded"
        );
        Ok(checkpoint)
    }

    /// Generate a unique checkpoint filename using the current timestamp.
    pub fn filename(soma_id: &str) -> String {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        format!("{soma_id}-{ts}.ckpt")
    }

    /// List `.ckpt` files in `dir`, sorted newest-first by filesystem modification time.
    pub fn list_checkpoints(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries: Vec<(std::path::PathBuf, std::time::SystemTime)> = std::fs::read_dir(dir)?
            .filter_map(std::result::Result::ok)
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|ext| ext == "ckpt")
            })
            .filter_map(|e| {
                let modified = e.metadata().ok()?.modified().ok()?;
                Some((e.path(), modified))
            })
            .collect();

        entries.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(entries.into_iter().map(|(p, _)| p).collect())
    }

    /// Delete old checkpoints, keeping only the `max_keep` most recent. Returns count removed.
    pub fn prune_checkpoints(dir: &Path, max_keep: usize) -> Result<usize> {
        let checkpoints = Self::list_checkpoints(dir)?;
        let mut removed = 0;
        if checkpoints.len() > max_keep {
            for old in &checkpoints[max_keep..] {
                if std::fs::remove_file(old).is_ok() {
                    tracing::debug!(path = %old.display(), "Pruned old checkpoint");
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }
}
