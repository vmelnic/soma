use crate::active::ActiveWorldModel;
use crate::exact::Exact;
use crate::transducer::LearnedMacro;
use crate::universal::MacroDefinition;
use anyhow::{bail, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub fn digest_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn digest_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(digest_bytes(&serde_json::to_vec(value)?))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Episode {
    pub source_digest: String,
    pub sequence_id: String,
    pub ordinal: usize,
    pub experience: String,
    pub intervention: Option<String>,
    pub report_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SemanticMemory {
    pub transition_counts: BTreeMap<String, usize>,
    pub source_event_counts: BTreeMap<String, usize>,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutobiographicalEntry {
    pub sequence: u64,
    pub event: String,
    pub evidence_digest: String,
    pub prior_state_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdaptiveResourceState {
    pub evaluated_candidates: usize,
    pub retained_candidates: usize,
    pub rejected_by_resource: usize,
    pub current_batch: usize,
    pub stagnation_batches: usize,
    pub best_description_bits: Option<usize>,
    pub frontier_exhausted: bool,
    pub stop_reason: Option<String>,
    pub meta_policy_decisions: usize,
    pub last_meta_policy: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PersistentMemory {
    pub schema: String,
    pub identity: String,
    pub episodes: Vec<Episode>,
    pub semantic: SemanticMemory,
    pub procedures: BTreeMap<String, LearnedMacro>,
    #[serde(default)]
    pub generative_procedures: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub generative_posteriors: BTreeMap<String, Exact>,
    #[serde(default)]
    pub structural_macros: BTreeMap<String, MacroDefinition>,
    #[serde(default)]
    pub active_world_models: BTreeMap<String, ActiveWorldModel>,
    #[serde(default)]
    pub failed_structural_programs: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub policy_states: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub headset_states: BTreeMap<String, serde_json::Value>,
    pub autobiography: Vec<AutobiographicalEntry>,
    pub resource_state: AdaptiveResourceState,
}

impl PersistentMemory {
    pub fn empty(identity: impl Into<String>, initial_batch: usize) -> Result<Self> {
        if initial_batch == 0 {
            bail!("adaptive resource batch must start above zero");
        }
        Ok(Self {
            schema: "soma.rtl.memory.v1".to_string(),
            identity: identity.into(),
            episodes: Vec::new(),
            semantic: SemanticMemory {
                transition_counts: BTreeMap::new(),
                source_event_counts: BTreeMap::new(),
                provenance: Vec::new(),
            },
            procedures: BTreeMap::new(),
            generative_procedures: BTreeMap::new(),
            generative_posteriors: BTreeMap::new(),
            structural_macros: BTreeMap::new(),
            active_world_models: BTreeMap::new(),
            failed_structural_programs: BTreeMap::new(),
            policy_states: BTreeMap::new(),
            headset_states: BTreeMap::new(),
            autobiography: Vec::new(),
            resource_state: AdaptiveResourceState {
                evaluated_candidates: 0,
                retained_candidates: 0,
                rejected_by_resource: 0,
                current_batch: initial_batch,
                stagnation_batches: 0,
                best_description_bits: None,
                frontier_exhausted: false,
                stop_reason: None,
                meta_policy_decisions: 0,
                last_meta_policy: None,
            },
        })
    }

    /// Consolidate exact counts while retaining every episode's provenance.
    /// Raw external measurements remain in their immutable source files.
    pub fn consolidate(&mut self) {
        let mut transition_counts = BTreeMap::new();
        let mut source_event_counts = BTreeMap::new();
        let mut by_sequence: BTreeMap<&str, Vec<&Episode>> = BTreeMap::new();
        for episode in &self.episodes {
            *source_event_counts
                .entry(episode.source_digest.clone())
                .or_insert(0) += 1;
            by_sequence
                .entry(episode.sequence_id.as_str())
                .or_default()
                .push(episode);
        }
        for sequence in by_sequence.values_mut() {
            sequence.sort_by_key(|episode| episode.ordinal);
            for pair in sequence.windows(2) {
                let key = format!("{}\u{1f}{}", pair[0].experience, pair[1].experience);
                *transition_counts.entry(key).or_insert(0) += 1;
            }
        }
        self.semantic.transition_counts = transition_counts;
        self.semantic.source_event_counts = source_event_counts;
        self.semantic.provenance = self.semantic.source_event_counts.keys().cloned().collect();
    }

    /// Drop detailed episodes only after their sufficient statistics and
    /// source digests have been consolidated. The policy is explicit and
    /// deterministic; it never fabricates replacement memories.
    pub fn forget_before_ordinal(&mut self, ordinal: usize) {
        self.consolidate();
        self.episodes.retain(|episode| episode.ordinal >= ordinal);
    }

    pub fn record_transition(
        &mut self,
        event: impl Into<String>,
        evidence_digest: impl Into<String>,
    ) -> Result<()> {
        let prior_state_digest = digest_json(self)?;
        self.autobiography.push(AutobiographicalEntry {
            sequence: self.autobiography.len() as u64,
            event: event.into(),
            evidence_digest: evidence_digest.into(),
            prior_state_digest,
        });
        Ok(())
    }

    pub fn digest(&self) -> Result<String> {
        digest_json(self)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct MemoryEnvelope {
    schema: String,
    payload_sha256: String,
    payload: PersistentMemory,
}

pub fn save_atomic(path: &Path, memory: &PersistentMemory) -> Result<String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create memory directory {}", parent.display()))?;
    }
    let payload_sha256 = digest_json(memory)?;
    let envelope = MemoryEnvelope {
        schema: "soma.rtl.memory-envelope.v1".to_string(),
        payload_sha256: payload_sha256.clone(),
        payload: memory.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&envelope)?;
    let temporary = temporary_path(path);
    fs::write(&temporary, bytes)
        .with_context(|| format!("cannot write temporary memory {}", temporary.display()))?;
    fs::rename(&temporary, path)
        .with_context(|| format!("cannot commit memory {}", path.display()))?;
    Ok(payload_sha256)
}

pub fn load_verified(path: &Path) -> Result<PersistentMemory> {
    let bytes = fs::read(path)
        .with_context(|| format!("cannot read persistent memory {}", path.display()))?;
    let envelope: MemoryEnvelope = serde_json::from_slice(&bytes)?;
    if envelope.schema != "soma.rtl.memory-envelope.v1" {
        bail!("unsupported memory envelope schema");
    }
    let actual = digest_json(&envelope.payload)?;
    if actual != envelope.payload_sha256 {
        bail!("persistent memory checksum mismatch");
    }
    Ok(envelope.payload)
}

pub fn read_json_verified<T: DeserializeOwned>(path: &Path, expected_sha256: &str) -> Result<T> {
    let bytes = fs::read(path)?;
    let actual = digest_bytes(&bytes);
    if actual != expected_sha256 {
        bail!(
            "source checksum mismatch for {}: expected {}, found {}",
            path.display(),
            expected_sha256,
            actual
        );
    }
    Ok(serde_json::from_slice(&bytes)?)
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".next");
    PathBuf::from(name)
}
