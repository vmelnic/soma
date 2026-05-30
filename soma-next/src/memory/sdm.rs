//! Sparse Distributed Memory (Kanerva 1988).
//!
//! Content-addressable memory: write and read by similarity, not by key.
//! No weights, no gradients, no learnable parameters — pure vector geometry.
//!
//! Each entry is an (address, data) vector pair stored in RAM. Reads activate
//! the top-k nearest addresses and return a similarity-weighted blend of their
//! data vectors. The blending IS generalization: querying with a never-stored
//! address returns a useful interpolation of nearby entries.

use serde::{Deserialize, Serialize};

use crate::errors::{Result, SomaError};
use crate::memory::embedder::cosine_similarity;

/// A single SDM entry: address vector + data vector + metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SdmEntry {
    pub address: Vec<f32>,
    pub data: Vec<f32>,
    pub label: String,
    pub write_count: u64,
    pub last_access_tick: u64,
}

/// Result of a top-k read: the entry, its similarity score, and its index.
#[derive(Clone, Debug)]
pub struct SdmMatch<'a> {
    pub entry: &'a SdmEntry,
    pub similarity: f64,
    pub index: usize,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

pub trait SparseDistributedMemoryStore: Send {
    /// Write an entry. If an entry with the same label exists, reinforce it
    /// (increment write_count, blend data via running average). Otherwise append.
    fn write(&mut self, address: Vec<f32>, data: Vec<f32>, label: String) -> Result<()>;

    /// Read: find the top-k entries most similar to the query address.
    /// Returns entries sorted by descending similarity.
    fn read(&self, query: &[f32], top_k: usize) -> Vec<SdmMatch<'_>>;

    /// Read and blend: return a single data vector that is the similarity-weighted
    /// average of the top-k nearest entries. This is the core SDM operation —
    /// the result is a generalization over similar past entries.
    fn read_blended(&self, query: &[f32], top_k: usize) -> Option<Vec<f32>>;

    /// How many entries are stored.
    fn count(&self) -> usize;

    /// Remove entries below a similarity threshold relative to a query.
    /// Returns the number removed. Useful for pruning stale/irrelevant memory.
    fn prune_below(&mut self, query: &[f32], threshold: f64) -> usize;

    /// Decay all entries by reducing write_count toward zero. Entries whose
    /// write_count reaches zero are removed. Returns number removed.
    fn decay(&mut self, factor: f64) -> usize;

    /// Remove all entries.
    fn clear(&mut self);

    /// Snapshot all entries for persistence.
    fn entries(&self) -> &[SdmEntry];
}

// ---------------------------------------------------------------------------
// Default in-memory implementation
// ---------------------------------------------------------------------------

pub struct DefaultSdm {
    entries: Vec<SdmEntry>,
    address_size: usize,
    data_size: usize,
    tick: u64,
    /// Maximum number of entries. When a write would exceed this cap, the
    /// entry with the lowest write_count is evicted. 0 means unlimited.
    max_entries: usize,
}

impl DefaultSdm {
    pub fn new(address_size: usize, data_size: usize) -> Self {
        Self {
            entries: Vec::new(),
            address_size,
            data_size,
            tick: 0,
            max_entries: 0,
        }
    }

    pub fn with_max_entries(address_size: usize, data_size: usize, max_entries: usize) -> Self {
        Self {
            entries: Vec::new(),
            address_size,
            data_size,
            tick: 0,
            max_entries,
        }
    }

    pub fn from_entries(address_size: usize, data_size: usize, entries: Vec<SdmEntry>) -> Self {
        Self {
            entries,
            address_size,
            data_size,
            tick: 0,
            max_entries: 0,
        }
    }

    pub fn from_entries_with_max(
        address_size: usize,
        data_size: usize,
        entries: Vec<SdmEntry>,
        max_entries: usize,
    ) -> Self {
        Self {
            entries,
            address_size,
            data_size,
            tick: 0,
            max_entries,
        }
    }

    pub fn advance_tick(&mut self) {
        self.tick += 1;
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Evict the entry with the lowest write_count to make room for a new one.
    fn evict_least_reinforced(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let min_idx = self
            .entries
            .iter()
            .enumerate()
            .min_by_key(|(_, e)| e.write_count)
            .map(|(i, _)| i)
            .unwrap();
        self.entries.swap_remove(min_idx);
    }
}

fn normalize(v: &mut [f32]) {
    let mag: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag > 0.0 {
        for x in v.iter_mut() {
            *x /= mag;
        }
    }
}

impl SparseDistributedMemoryStore for DefaultSdm {
    fn write(&mut self, mut address: Vec<f32>, data: Vec<f32>, label: String) -> Result<()> {
        if address.len() != self.address_size {
            return Err(SomaError::Memory(format!(
                "address size mismatch: expected {}, got {}",
                self.address_size,
                address.len()
            )));
        }
        if data.len() != self.data_size {
            return Err(SomaError::Memory(format!(
                "data size mismatch: expected {}, got {}",
                self.data_size,
                data.len()
            )));
        }

        normalize(&mut address);

        // Reinforce existing entry with same label (running average blend).
        if let Some(existing) = self.entries.iter_mut().find(|e| e.label == label) {
            let n = existing.write_count as f32;
            let new_n = n + 1.0;
            for (i, val) in data.iter().enumerate() {
                existing.data[i] = (existing.data[i] * n + val) / new_n;
            }
            for (i, val) in address.iter().enumerate() {
                existing.address[i] = (existing.address[i] * n + val) / new_n;
            }
            normalize(&mut existing.address);
            existing.write_count += 1;
            existing.last_access_tick = self.tick;
            return Ok(());
        }

        // Evict the least-reinforced entry if we're at the cap.
        if self.max_entries > 0 && self.entries.len() >= self.max_entries {
            self.evict_least_reinforced();
        }

        self.entries.push(SdmEntry {
            address,
            data,
            label,
            write_count: 1,
            last_access_tick: self.tick,
        });

        Ok(())
    }

    fn read(&self, query: &[f32], top_k: usize) -> Vec<SdmMatch<'_>> {
        if self.entries.is_empty() || query.len() != self.address_size {
            return Vec::new();
        }

        let mut scored: Vec<(usize, f64)> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| (i, cosine_similarity(query, &e.address)))
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);

        scored
            .into_iter()
            .map(|(i, sim)| SdmMatch {
                entry: &self.entries[i],
                similarity: sim,
                index: i,
            })
            .collect()
    }

    fn read_blended(&self, query: &[f32], top_k: usize) -> Option<Vec<f32>> {
        let matches = self.read(query, top_k);
        if matches.is_empty() {
            return None;
        }

        // Softmax over similarities (temperature=10 for sharpness).
        let max_sim = matches
            .iter()
            .map(|m| m.similarity)
            .fold(f64::NEG_INFINITY, f64::max);
        let exps: Vec<f64> = matches
            .iter()
            .map(|m| ((m.similarity - max_sim) * 10.0).exp())
            .collect();
        let sum_exp: f64 = exps.iter().sum();
        let weights: Vec<f64> = exps.iter().map(|e| e / sum_exp).collect();

        let mut blended = vec![0.0f32; self.data_size];
        for (m, &w) in matches.iter().zip(weights.iter()) {
            for (i, &val) in m.entry.data.iter().enumerate() {
                blended[i] += val * w as f32;
            }
        }

        Some(blended)
    }

    fn count(&self) -> usize {
        self.entries.len()
    }

    fn prune_below(&mut self, query: &[f32], threshold: f64) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| {
            cosine_similarity(query, &e.address) >= threshold
        });
        before - self.entries.len()
    }

    fn decay(&mut self, factor: f64) -> usize {
        for entry in &mut self.entries {
            entry.write_count = (entry.write_count as f64 * factor) as u64;
        }
        let before = self.entries.len();
        self.entries.retain(|e| e.write_count > 0);
        before - self.entries.len()
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    fn entries(&self) -> &[SdmEntry] {
        &self.entries
    }
}

// ---------------------------------------------------------------------------
// World state snapshot → SDM address encoder
// ---------------------------------------------------------------------------

use crate::memory::embedder::GoalEmbedder;

/// Encode a world state JSON snapshot into a fixed-size SDM address vector.
/// Flattens all `subject.predicate = value` pairs into a deterministic string
/// and hashes it via the weight-free HashEmbedder.
pub fn encode_snapshot(snapshot: &serde_json::Value, embedder: &dyn GoalEmbedder) -> Vec<f32> {
    let text = snapshot_to_text(snapshot);
    embedder.embed(&text)
}

/// Encode a skill outcome (skill_id + success/failure + key observations)
/// into a data vector for SDM storage.
pub fn encode_outcome(skill_id: &str, success: bool, observations: &serde_json::Value, embedder: &dyn GoalEmbedder) -> Vec<f32> {
    let mut text = format!("skill={skill_id} success={success}");
    if let Some(obj) = observations.as_object() {
        let mut keys: Vec<&String> = obj.keys().collect();
        keys.sort();
        for key in keys {
            let val = &obj[key];
            let val_str = if val.is_string() {
                val.as_str().unwrap_or("").to_string()
            } else {
                val.to_string()
            };
            text.push(' ');
            text.push_str(key);
            text.push('=');
            text.push_str(&val_str);
        }
    }
    embedder.embed(&text)
}

fn snapshot_to_text(snapshot: &serde_json::Value) -> String {
    let mut parts = Vec::new();
    if let Some(obj) = snapshot.as_object() {
        let mut keys: Vec<&String> = obj.keys().collect();
        keys.sort();
        for key in keys {
            let val = &obj[key];
            let val_str = if val.is_string() {
                val.as_str().unwrap_or("").to_string()
            } else {
                val.to_string()
            };
            parts.push(format!("{key}={val_str}"));
        }
    }
    parts.join(" ")
}

// ---------------------------------------------------------------------------
// SDM-based brain fallback — replaces LLM calls with memory retrieval
// ---------------------------------------------------------------------------

/// Selects skills by querying SDM with the current world state, retrieving
/// the most similar past situations, and picking the skill that succeeded
/// most often in those situations. No LLM, no weights.
pub struct SdmBrainFallback {
    sdm: std::sync::Arc<std::sync::Mutex<dyn SparseDistributedMemoryStore>>,
    embedder: std::sync::Arc<dyn GoalEmbedder + Send + Sync>,
    top_k: usize,
}

impl SdmBrainFallback {
    pub fn new(
        sdm: std::sync::Arc<std::sync::Mutex<dyn SparseDistributedMemoryStore>>,
        embedder: std::sync::Arc<dyn GoalEmbedder + Send + Sync>,
    ) -> Self {
        Self { sdm, embedder, top_k: 8 }
    }

    pub fn with_top_k(mut self, top_k: usize) -> Self {
        self.top_k = top_k;
        self
    }
}

impl crate::runtime::session::BrainFallback for SdmBrainFallback {
    fn select_skill(
        &self,
        goal: &str,
        candidates: &[String],
        belief_summary: &str,
    ) -> crate::errors::Result<String> {
        let query_text = format!("{goal} {belief_summary}");
        let query_vec = self.embedder.embed(&query_text);

        let sdm = self.sdm.lock().map_err(|_| {
            crate::errors::SomaError::Skill("SDM lock poisoned".into())
        })?;

        let matches = sdm.read(&query_vec, self.top_k);

        if matches.is_empty() {
            return Err(crate::errors::SomaError::Skill(
                "SDM has no similar episodes — need exploration".into(),
            ));
        }

        // Score each candidate by how often it appears in successful similar episodes.
        let mut scores: std::collections::HashMap<&str, f64> = std::collections::HashMap::new();
        for m in &matches {
            // Labels encode "skill_id:success" or just "skill_id".
            let label = &m.entry.label;
            for candidate in candidates {
                if label.contains(candidate.as_str()) {
                    *scores.entry(candidate.as_str()).or_default() += m.similarity;
                }
            }
        }

        // Pick the highest-scoring candidate.
        scores
            .into_iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(skill_id, _)| skill_id.to_string())
            .ok_or_else(|| {
                crate::errors::SomaError::Skill(
                    "SDM found similar episodes but none matched any candidate skill".into(),
                )
            })
    }
}

// ---------------------------------------------------------------------------
// Shared type alias
// ---------------------------------------------------------------------------

pub type SharedSdm = std::sync::Arc<std::sync::Mutex<dyn SparseDistributedMemoryStore>>;

// ---------------------------------------------------------------------------
// SDM-driven routine discovery
// ---------------------------------------------------------------------------

/// A candidate routine discovered from recurring skill patterns in SDM.
/// Not a real routine — the brain/user decides whether to promote it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SdmRoutineCandidate {
    /// Skills sorted by descending write_count (most reinforced first).
    pub skill_ids: Vec<String>,
    /// Average (write_count / max_write_count) across the skills.
    pub confidence: f64,
    /// Sum of write_counts across all skills in this candidate.
    pub total_reinforcement: u64,
}

/// Scan SDM entries for recurring successful skill patterns and return
/// candidates that could be promoted into compiled routines.
///
/// Labels are expected to follow the `"skill_id:ok"` / `"skill_id:fail"`
/// convention. Only entries whose label ends with `:ok` and whose
/// `write_count >= threshold` are considered.
pub fn discover_routines_from_sdm(
    sdm: &dyn SparseDistributedMemoryStore,
    threshold: u64,
) -> Vec<SdmRoutineCandidate> {
    let entries = sdm.entries();
    if entries.is_empty() {
        return Vec::new();
    }

    // Collect successful entries grouped by skill_id.
    let mut skill_counts: std::collections::HashMap<String, u64> =
        std::collections::HashMap::new();
    for entry in entries {
        if let Some(skill_id) = entry.label.strip_suffix(":ok")
            && entry.write_count >= threshold
        {
            skill_counts
                .entry(skill_id.to_string())
                .and_modify(|c| *c = (*c).max(entry.write_count))
                .or_insert(entry.write_count);
        }
    }

    if skill_counts.is_empty() {
        return Vec::new();
    }

    // Sort skills by descending write_count.
    let mut skills: Vec<(String, u64)> = skill_counts.into_iter().collect();
    skills.sort_by(|a, b| b.1.cmp(&a.1));

    let max_wc = skills.iter().map(|(_, wc)| *wc).max().unwrap_or(1);
    let total_reinforcement: u64 = skills.iter().map(|(_, wc)| *wc).sum();
    let confidence: f64 = skills
        .iter()
        .map(|(_, wc)| *wc as f64 / max_wc as f64)
        .sum::<f64>()
        / skills.len() as f64;

    let skill_ids: Vec<String> = skills.into_iter().map(|(id, _)| id).collect();

    vec![SdmRoutineCandidate {
        skill_ids,
        confidence,
        total_reinforcement,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sdm() -> DefaultSdm {
        DefaultSdm::new(4, 4)
    }

    #[test]
    fn write_and_read_back() {
        let mut sdm = make_sdm();
        sdm.write(
            vec![1.0, 0.0, 0.0, 0.0],
            vec![10.0, 20.0, 30.0, 40.0],
            "entry_a".into(),
        )
        .unwrap();

        let matches = sdm.read(&[1.0, 0.0, 0.0, 0.0], 1);
        assert_eq!(matches.len(), 1);
        assert!(matches[0].similarity > 0.99);
        assert_eq!(matches[0].entry.label, "entry_a");
    }

    #[test]
    fn similarity_ordering() {
        let mut sdm = make_sdm();
        sdm.write(vec![1.0, 0.0, 0.0, 0.0], vec![1.0, 0.0, 0.0, 0.0], "north".into()).unwrap();
        sdm.write(vec![0.0, 1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0, 0.0], "east".into()).unwrap();
        sdm.write(vec![0.7, 0.7, 0.0, 0.0], vec![0.5, 0.5, 0.0, 0.0], "northeast".into()).unwrap();

        let matches = sdm.read(&[0.8, 0.6, 0.0, 0.0], 3);
        assert_eq!(matches[0].entry.label, "northeast");
    }

    #[test]
    fn blended_read_interpolates() {
        let mut sdm = make_sdm();
        sdm.write(vec![1.0, 0.0, 0.0, 0.0], vec![100.0, 0.0, 0.0, 0.0], "a".into()).unwrap();
        sdm.write(vec![0.0, 1.0, 0.0, 0.0], vec![0.0, 100.0, 0.0, 0.0], "b".into()).unwrap();

        // Query equidistant from both — blend should mix.
        let blended = sdm.read_blended(&[0.707, 0.707, 0.0, 0.0], 2).unwrap();
        assert!(blended[0] > 10.0, "expected non-trivial blend in dim 0");
        assert!(blended[1] > 10.0, "expected non-trivial blend in dim 1");
    }

    #[test]
    fn reinforce_existing_label() {
        let mut sdm = make_sdm();
        sdm.write(vec![1.0, 0.0, 0.0, 0.0], vec![10.0, 0.0, 0.0, 0.0], "x".into()).unwrap();
        sdm.write(vec![1.0, 0.0, 0.0, 0.0], vec![20.0, 0.0, 0.0, 0.0], "x".into()).unwrap();

        assert_eq!(sdm.count(), 1);
        let matches = sdm.read(&[1.0, 0.0, 0.0, 0.0], 1);
        assert_eq!(matches[0].entry.write_count, 2);
        // Running average of 10 and 20 = 15
        assert!((matches[0].entry.data[0] - 15.0).abs() < 0.01);
    }

    #[test]
    fn decay_removes_weak_entries() {
        let mut sdm = make_sdm();
        sdm.write(vec![1.0, 0.0, 0.0, 0.0], vec![1.0, 0.0, 0.0, 0.0], "weak".into()).unwrap();
        sdm.write(vec![0.0, 1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0, 0.0], "strong".into()).unwrap();
        // Reinforce "strong" many times
        for _ in 0..10 {
            sdm.write(vec![0.0, 1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0, 0.0], "strong".into()).unwrap();
        }

        let removed = sdm.decay(0.5);
        // "weak" had write_count=1, 1*0.5=0 → removed
        assert_eq!(removed, 1);
        assert_eq!(sdm.count(), 1);
        assert_eq!(sdm.entries[0].label, "strong");
    }

    #[test]
    fn empty_read_returns_empty() {
        let sdm = make_sdm();
        assert!(sdm.read(&[1.0, 0.0, 0.0, 0.0], 5).is_empty());
        assert!(sdm.read_blended(&[1.0, 0.0, 0.0, 0.0], 5).is_none());
    }

    #[test]
    fn size_mismatch_errors() {
        let mut sdm = make_sdm();
        assert!(sdm.write(vec![1.0, 0.0], vec![1.0, 0.0, 0.0, 0.0], "x".into()).is_err());
        assert!(sdm.write(vec![1.0, 0.0, 0.0, 0.0], vec![1.0], "x".into()).is_err());
    }

    #[test]
    fn prune_removes_distant_entries() {
        let mut sdm = make_sdm();
        sdm.write(vec![1.0, 0.0, 0.0, 0.0], vec![1.0, 0.0, 0.0, 0.0], "close".into()).unwrap();
        sdm.write(vec![0.0, 0.0, 0.0, 1.0], vec![0.0, 0.0, 0.0, 1.0], "far".into()).unwrap();

        let removed = sdm.prune_below(&[1.0, 0.0, 0.0, 0.0], 0.5);
        assert_eq!(removed, 1);
        assert_eq!(sdm.count(), 1);
        assert_eq!(sdm.entries[0].label, "close");
    }

    #[test]
    fn max_entries_evicts_least_reinforced() {
        // Cap at 3 entries. Write 3, then a 4th should evict the weakest.
        let mut sdm = DefaultSdm::with_max_entries(4, 4, 3);

        // Write three entries. Reinforce "strong" so it survives eviction.
        sdm.write(vec![1.0, 0.0, 0.0, 0.0], vec![1.0, 0.0, 0.0, 0.0], "weak".into()).unwrap();
        sdm.write(vec![0.0, 1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0, 0.0], "strong".into()).unwrap();
        for _ in 0..5 {
            sdm.write(vec![0.0, 1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0, 0.0], "strong".into()).unwrap();
        }
        sdm.write(vec![0.0, 0.0, 1.0, 0.0], vec![0.0, 0.0, 1.0, 0.0], "medium".into()).unwrap();
        // Reinforce "medium" once so it has write_count=2.
        sdm.write(vec![0.0, 0.0, 1.0, 0.0], vec![0.0, 0.0, 1.0, 0.0], "medium".into()).unwrap();
        assert_eq!(sdm.count(), 3);

        // Now write a 4th distinct entry. "weak" (write_count=1) should be evicted.
        sdm.write(vec![0.0, 0.0, 0.0, 1.0], vec![0.0, 0.0, 0.0, 1.0], "newcomer".into()).unwrap();
        assert_eq!(sdm.count(), 3);

        let labels: Vec<&str> = sdm.entries.iter().map(|e| e.label.as_str()).collect();
        assert!(!labels.contains(&"weak"), "weak should have been evicted");
        assert!(labels.contains(&"strong"));
        assert!(labels.contains(&"medium"));
        assert!(labels.contains(&"newcomer"));
    }

    #[test]
    fn max_entries_zero_means_unlimited() {
        let mut sdm = DefaultSdm::with_max_entries(4, 4, 0);
        for i in 0..100 {
            sdm.write(
                vec![i as f32, 0.0, 0.0, 0.0],
                vec![0.0, 0.0, 0.0, 0.0],
                format!("entry_{i}"),
            )
            .unwrap();
        }
        assert_eq!(sdm.count(), 100);
    }

    #[test]
    fn reinforce_does_not_trigger_eviction() {
        // Cap at 2. Write 2 entries, then reinforce one. Count should stay at 2.
        let mut sdm = DefaultSdm::with_max_entries(4, 4, 2);
        sdm.write(vec![1.0, 0.0, 0.0, 0.0], vec![1.0, 0.0, 0.0, 0.0], "a".into()).unwrap();
        sdm.write(vec![0.0, 1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0, 0.0], "b".into()).unwrap();
        // Reinforce "a" — should NOT evict "b".
        sdm.write(vec![1.0, 0.0, 0.0, 0.0], vec![1.0, 0.0, 0.0, 0.0], "a".into()).unwrap();
        assert_eq!(sdm.count(), 2);
        let labels: Vec<&str> = sdm.entries.iter().map(|e| e.label.as_str()).collect();
        assert!(labels.contains(&"a"));
        assert!(labels.contains(&"b"));
    }

    #[test]
    fn world_state_to_sdm_roundtrip() {
        // Simulate: world state snapshot → HashEmbedder → SDM address
        use crate::memory::embedder::HashEmbedder;
        use crate::memory::embedder::GoalEmbedder;

        let embedder = HashEmbedder::new();
        let dim = 128;
        let mut sdm = DefaultSdm::new(dim, dim);

        // Store two world state snapshots with different outcomes.
        let snap1 = "target.detected=true iff.hostile=true range_m=500";
        let snap2 = "target.detected=true iff.hostile=false range_m=200";

        let addr1 = embedder.embed(snap1);
        let outcome1 = embedder.embed("intercept.succeeded skill=proportional_navigation");
        sdm.write(addr1, outcome1, "episode_1".into()).unwrap();

        let addr2 = embedder.embed(snap2);
        let outcome2 = embedder.embed("abort.succeeded skill=abort_engagement");
        sdm.write(addr2, outcome2, "episode_2".into()).unwrap();

        // Query with a new situation similar to snap1.
        let query = embedder.embed("target.detected=true iff.hostile=true range_m=450");
        let matches = sdm.read(&query, 2);

        // snap1 (hostile) should be more similar than snap2 (friendly).
        assert_eq!(matches[0].entry.label, "episode_1");
        assert!(matches[0].similarity > matches[1].similarity);
    }

    // --- SDM routine discovery tests ---

    #[test]
    fn discover_routines_empty_sdm_returns_none() {
        let sdm = make_sdm();
        let candidates = discover_routines_from_sdm(&sdm, 3);
        assert!(candidates.is_empty());
    }

    #[test]
    fn discover_routines_below_threshold_returns_none() {
        let mut sdm = make_sdm();
        // write_count=1, threshold=3 — not enough reinforcement.
        sdm.write(
            vec![1.0, 0.0, 0.0, 0.0],
            vec![1.0, 0.0, 0.0, 0.0],
            "navigate:ok".into(),
        )
        .unwrap();
        sdm.write(
            vec![0.0, 1.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0, 0.0],
            "scan:ok".into(),
        )
        .unwrap();

        let candidates = discover_routines_from_sdm(&sdm, 3);
        assert!(candidates.is_empty());
    }

    #[test]
    fn discover_routines_reinforced_entries_returns_candidates() {
        let mut sdm = make_sdm();

        // "navigate:ok" reinforced 5 times.
        for _ in 0..5 {
            sdm.write(
                vec![1.0, 0.0, 0.0, 0.0],
                vec![1.0, 0.0, 0.0, 0.0],
                "navigate:ok".into(),
            )
            .unwrap();
        }

        // "scan:ok" reinforced 3 times.
        for _ in 0..3 {
            sdm.write(
                vec![0.0, 1.0, 0.0, 0.0],
                vec![0.0, 1.0, 0.0, 0.0],
                "scan:ok".into(),
            )
            .unwrap();
        }

        // "abort:fail" should be ignored (not :ok).
        for _ in 0..10 {
            sdm.write(
                vec![0.0, 0.0, 1.0, 0.0],
                vec![0.0, 0.0, 1.0, 0.0],
                "abort:fail".into(),
            )
            .unwrap();
        }

        // "weak:ok" reinforced only 2 times (below threshold of 3).
        for _ in 0..2 {
            sdm.write(
                vec![0.0, 0.0, 0.0, 1.0],
                vec![0.0, 0.0, 0.0, 1.0],
                "weak:ok".into(),
            )
            .unwrap();
        }

        let candidates = discover_routines_from_sdm(&sdm, 3);
        assert_eq!(candidates.len(), 1);

        let c = &candidates[0];
        // Most reinforced first: navigate (5) then scan (3).
        assert_eq!(c.skill_ids, vec!["navigate", "scan"]);
        assert_eq!(c.total_reinforcement, 8); // 5 + 3
        // confidence = avg(5/5, 3/5) = avg(1.0, 0.6) = 0.8
        assert!((c.confidence - 0.8).abs() < 1e-9);
    }
}
