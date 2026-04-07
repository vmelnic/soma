//! Checkpoint system — save/restore LoRA state + experience metadata.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::mind::lora::LoRALayerState;

pub const CHECKPOINT_MAGIC: &[u8; 4] = b"SOMA";
pub const CHECKPOINT_VERSION: u32 = 1;

/// A serializable checkpoint of the SOMA's learned state.
#[derive(Serialize, Deserialize)]
pub struct Checkpoint {
    pub version: u32,
    pub soma_id: String,
    pub timestamp: u64,
    pub lora_state: Vec<LoRALayerState>,
    pub experience_count: u64,
    pub adaptation_count: u64,
}

impl Checkpoint {
    /// Create a new checkpoint with current timestamp.
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
        }
    }

    /// Save checkpoint to a file. Format: SOMA magic + version (4 bytes) + JSON body.
    pub fn save(&self, path: &Path) -> Result<()> {
        // Ensure parent directory exists
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

    /// Load a checkpoint from file.
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read(path)
            .context("Failed to read checkpoint file")?;

        if data.len() < 8 {
            return Err(anyhow::anyhow!("Checkpoint file too small"));
        }

        // Verify magic
        if &data[0..4] != CHECKPOINT_MAGIC {
            return Err(anyhow::anyhow!(
                "Invalid checkpoint magic: expected SOMA, got {:?}",
                &data[0..4]
            ));
        }

        // Read version
        let version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        if version != CHECKPOINT_VERSION {
            return Err(anyhow::anyhow!(
                "Unsupported checkpoint version: {} (expected {})",
                version,
                CHECKPOINT_VERSION
            ));
        }

        let checkpoint: Checkpoint = serde_json::from_slice(&data[8..])
            .context("Failed to deserialize checkpoint")?;

        tracing::info!(
            path = %path.display(),
            lora_layers = checkpoint.lora_state.len(),
            experiences = checkpoint.experience_count,
            "Checkpoint loaded"
        );
        Ok(checkpoint)
    }

    /// Generate a filename for a new checkpoint based on timestamp.
    pub fn filename(soma_id: &str) -> String {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        format!("{}-{}.ckpt", soma_id, ts)
    }

    /// List checkpoint files in a directory, sorted by modification time (newest first).
    pub fn list_checkpoints(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries: Vec<(std::path::PathBuf, std::time::SystemTime)> = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "ckpt")
                    .unwrap_or(false)
            })
            .filter_map(|e| {
                let modified = e.metadata().ok()?.modified().ok()?;
                Some((e.path(), modified))
            })
            .collect();

        entries.sort_by(|a, b| b.1.cmp(&a.1)); // newest first
        Ok(entries.into_iter().map(|(p, _)| p).collect())
    }

    /// Prune old checkpoints, keeping only the newest `max_keep`.
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
