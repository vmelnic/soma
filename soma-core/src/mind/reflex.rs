//! Reflex layer: caches proven (intent -> program) pairs for zero-inference execution.
//!
//! After a successful Mind inference + execution, the (tokenized intent, program) pair
//! is stored. On subsequent intents, the reflex layer checks for exact or fuzzy matches
//! before invoking the Mind. This is analogous to muscle memory / spinal reflexes --
//! proven patterns execute without conscious thought.
//!
//! Critical for ESP32 where Mind inference takes 50-500ms but reflex lookup is <1ms.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use super::Program;

/// Minimum hit count to be considered for consolidation into permanent status.
const CONSOLIDATION_THRESHOLD: u64 = 50;

/// A single cached (intent -> program) mapping with usage statistics.
#[derive(Debug, Clone)]
pub struct ReflexEntry {
    pub tokens: Vec<u32>,
    pub program: Program,
    /// Confidence from the original Mind inference that produced this program.
    pub confidence: f32,
    /// Number of times this reflex has been matched and served.
    pub hit_count: u64,
    /// Monotonic counter value at last use, for LRU eviction.
    pub last_used: u64,
    /// Permanent reflexes are never evicted by LRU (promoted via consolidation).
    pub permanent: bool,
}

/// Usage statistics for the reflex layer.
#[derive(Debug, Clone, Default)]
pub struct ReflexStats {
    pub total_entries: usize,
    pub total_lookups: u64,
    pub exact_hits: u64,
    pub fuzzy_hits: u64,
    pub misses: u64,
}

/// Caches proven (intent -> program) pairs and serves them without Mind inference.
///
/// Lookup strategy:
/// 1. Hash the token sequence, check for exact match in O(1).
/// 2. If no exact match, scan entries for fuzzy match via Jaccard similarity on
///    token bigrams. Only matches above `fuzzy_threshold` are accepted.
/// 3. On match, update `hit_count` and `last_used` for LRU tracking.
///
/// Eviction: when at capacity, the least recently used entry is evicted.
pub struct ReflexLayer {
    /// Token hash -> index into `entries`.
    exact: HashMap<u64, usize>,
    /// All cached reflex entries.
    entries: Vec<ReflexEntry>,
    /// Monotonic counter for LRU tracking. Incremented on every lookup.
    counter: u64,
    /// Maximum number of cached entries before LRU eviction kicks in.
    max_entries: usize,
    /// Minimum Jaccard similarity (on token bigrams) for fuzzy matching.
    fuzzy_threshold: f32,
    /// Cumulative statistics.
    stats: ReflexStats,
}

impl ReflexLayer {
    /// Create a new reflex layer with the given capacity and fuzzy threshold.
    ///
    /// - `max_entries`: maximum cached reflexes (default recommendation: 10,000).
    /// - `fuzzy_threshold`: minimum Jaccard similarity on token bigrams for fuzzy
    ///   matches (0.0..1.0, default recommendation: 0.9).
    pub fn new(max_entries: usize, fuzzy_threshold: f32) -> Self {
        Self {
            exact: HashMap::with_capacity(max_entries.min(1024)),
            entries: Vec::with_capacity(max_entries.min(1024)),
            counter: 0,
            max_entries,
            fuzzy_threshold,
            stats: ReflexStats::default(),
        }
    }

    /// Try to find a cached program for the given tokens.
    ///
    /// Returns `(program, confidence)` if an exact or fuzzy match is found.
    /// Updates hit counters and LRU timestamp on match.
    pub fn try_match(&mut self, tokens: &[u32]) -> Option<(&Program, f32)> {
        self.counter += 1;
        self.stats.total_lookups += 1;

        // 1. Exact match via token hash.
        let hash = Self::hash_tokens(tokens);
        if let Some(&idx) = self.exact.get(&hash) {
            if idx < self.entries.len() && self.entries[idx].tokens == tokens {
                self.entries[idx].hit_count += 1;
                self.entries[idx].last_used = self.counter;
                self.stats.exact_hits += 1;
                let entry = &self.entries[idx];
                return Some((&entry.program, entry.confidence));
            }
        }

        // 2. Fuzzy match via Jaccard similarity on token bigrams.
        let mut best_idx: Option<usize> = None;
        let mut best_sim: f32 = 0.0;
        for (i, entry) in self.entries.iter().enumerate() {
            let sim = Self::jaccard_similarity(tokens, &entry.tokens);
            if sim >= self.fuzzy_threshold && sim > best_sim {
                best_sim = sim;
                best_idx = Some(i);
            }
        }

        if let Some(idx) = best_idx {
            self.entries[idx].hit_count += 1;
            self.entries[idx].last_used = self.counter;
            self.stats.fuzzy_hits += 1;
            let entry = &self.entries[idx];
            // Scale confidence by similarity for fuzzy matches.
            let adjusted_confidence = entry.confidence * best_sim;
            return Some((&entry.program, adjusted_confidence));
        }

        self.stats.misses += 1;
        None
    }

    /// Record a successful (intent, program) pair as a new reflex.
    ///
    /// If the exact token sequence already exists, updates the existing entry.
    /// If at capacity, evicts the least recently used entry first.
    pub fn record(&mut self, tokens: Vec<u32>, program: Program, confidence: f32) {
        let hash = Self::hash_tokens(&tokens);

        // Update existing entry if exact match exists.
        if let Some(&idx) = self.exact.get(&hash) {
            if idx < self.entries.len() && self.entries[idx].tokens == tokens {
                self.entries[idx].program = program;
                self.entries[idx].confidence = confidence;
                self.entries[idx].last_used = self.counter;
                return;
            }
        }

        // Evict LRU if at capacity.
        if self.entries.len() >= self.max_entries {
            self.evict_lru();
        }

        let idx = self.entries.len();
        self.entries.push(ReflexEntry {
            tokens,
            program,
            confidence,
            hit_count: 0,
            last_used: self.counter,
            permanent: false,
        });
        self.exact.insert(hash, idx);
        self.stats.total_entries = self.entries.len();
    }

    /// Number of stored reflexes.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the reflex layer has no entries.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Hit rate statistics.
    #[allow(dead_code)]
    pub fn stats(&self) -> ReflexStats {
        let mut s = self.stats.clone();
        s.total_entries = self.entries.len();
        s
    }

    /// Promote reflexes with `hit_count >= threshold` to permanent status.
    ///
    /// Permanent reflexes are never evicted by LRU, ensuring proven high-traffic
    /// patterns persist indefinitely. Returns the count of newly promoted entries.
    #[allow(dead_code)]
    pub fn consolidate(&mut self, threshold: u64) -> usize {
        let mut promoted = 0;
        for entry in &mut self.entries {
            if !entry.permanent && entry.hit_count >= threshold {
                entry.permanent = true;
                promoted += 1;
            }
        }
        promoted
    }

    /// Serialize for checkpoint persistence.
    ///
    /// Format: JSON with version, entries (tokens, steps, confidence, hit_count).
    #[allow(dead_code)]
    pub fn serialize(&self) -> Vec<u8> {
        let json = serde_json::json!({
            "version": 1u32,
            "max_entries": self.max_entries,
            "fuzzy_threshold": self.fuzzy_threshold,
            "entries": self.entries.iter().map(|e| {
                serde_json::json!({
                    "tokens": e.tokens,
                    "confidence": e.confidence,
                    "hit_count": e.hit_count,
                    "permanent": e.permanent,
                    "steps": e.program.steps.iter().map(|s| {
                        serde_json::json!({
                            "conv_id": s.conv_id,
                            "arg0_type": arg_type_to_u8(&s.arg0_type),
                            "arg0_value": arg_value_to_json(&s.arg0_value),
                            "arg1_type": arg_type_to_u8(&s.arg1_type),
                            "arg1_value": arg_value_to_json(&s.arg1_value),
                        })
                    }).collect::<Vec<_>>(),
                    "program_confidence": e.program.confidence,
                })
            }).collect::<Vec<_>>(),
        });
        serde_json::to_vec(&json).unwrap_or_default()
    }

    /// Deserialize from checkpoint data.
    #[allow(dead_code)]
    pub fn deserialize(data: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        let json: serde_json::Value = serde_json::from_slice(data)?;

        let version = json.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
        if version != 1 {
            return Err(format!("Unsupported reflex checkpoint version: {version}").into());
        }

        let max_entries = json.get("max_entries")
            .and_then(|v| v.as_u64())
            .unwrap_or(10_000) as usize;
        let fuzzy_threshold = json.get("fuzzy_threshold")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.9) as f32;

        let mut layer = Self::new(max_entries, fuzzy_threshold);

        if let Some(entries) = json.get("entries").and_then(|v| v.as_array()) {
            for entry_val in entries {
                let tokens: Vec<u32> = entry_val.get("tokens")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect())
                    .unwrap_or_default();

                let confidence = entry_val.get("confidence")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as f32;

                let hit_count = entry_val.get("hit_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                let permanent = entry_val.get("permanent")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let program_confidence = entry_val.get("program_confidence")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(f64::from(confidence)) as f32;

                let steps: Vec<super::ProgramStep> = entry_val.get("steps")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().map(|s| {
                        let conv_id = s.get("conv_id").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let arg0_type = u8_to_arg_type(s.get("arg0_type").and_then(|v| v.as_u64()).unwrap_or(0) as u8);
                        let arg0_value = json_to_arg_value(s.get("arg0_value"));
                        let arg1_type = u8_to_arg_type(s.get("arg1_type").and_then(|v| v.as_u64()).unwrap_or(0) as u8);
                        let arg1_value = json_to_arg_value(s.get("arg1_value"));
                        super::ProgramStep { conv_id, arg0_type, arg0_value, arg1_type, arg1_value }
                    }).collect())
                    .unwrap_or_default();

                let program = Program {
                    steps,
                    confidence: program_confidence,
                    cached_states: Vec::new(), // Not needed for cached programs.
                };

                let hash = Self::hash_tokens(&tokens);
                let idx = layer.entries.len();
                layer.entries.push(ReflexEntry {
                    tokens,
                    program,
                    confidence,
                    hit_count,
                    permanent,
                    last_used: 0,
                });
                layer.exact.insert(hash, idx);
            }
        }

        layer.stats.total_entries = layer.entries.len();
        Ok(layer)
    }

    /// Hash a token sequence for exact lookup.
    fn hash_tokens(tokens: &[u32]) -> u64 {
        let mut hasher = DefaultHasher::new();
        tokens.hash(&mut hasher);
        hasher.finish()
    }

    /// Jaccard similarity on token bigrams.
    ///
    /// Bigrams are pairs of consecutive tokens: `[a, b, c]` -> `{(a,b), (b,c)}`.
    /// Jaccard = |intersection| / |union|. For single-token sequences, falls back
    /// to exact token comparison (1.0 if equal, 0.0 otherwise).
    fn jaccard_similarity(a: &[u32], b: &[u32]) -> f32 {
        if a.is_empty() && b.is_empty() {
            return 1.0;
        }
        if a.is_empty() || b.is_empty() {
            return 0.0;
        }

        // For single tokens, compare directly.
        if a.len() == 1 && b.len() == 1 {
            return if a[0] == b[0] { 1.0 } else { 0.0 };
        }

        let bigrams_a: HashSet<(u32, u32)> = a.windows(2).map(|w| (w[0], w[1])).collect();
        let bigrams_b: HashSet<(u32, u32)> = b.windows(2).map(|w| (w[0], w[1])).collect();

        if bigrams_a.is_empty() && bigrams_b.is_empty() {
            // Both have len 1 but different tokens (handled above), or both empty.
            return 0.0;
        }

        let intersection = bigrams_a.intersection(&bigrams_b).count();
        let union = bigrams_a.union(&bigrams_b).count();

        if union == 0 {
            return 0.0;
        }

        intersection as f32 / union as f32
    }

    /// Evict the least recently used non-permanent entry.
    fn evict_lru(&mut self) {
        if self.entries.is_empty() {
            return;
        }

        // Find the non-permanent entry with the smallest `last_used`.
        let mut lru_idx: Option<usize> = None;
        let mut lru_val = u64::MAX;
        for (i, entry) in self.entries.iter().enumerate() {
            if !entry.permanent && entry.last_used < lru_val {
                lru_val = entry.last_used;
                lru_idx = Some(i);
            }
        }

        // If all entries are permanent, nothing to evict.
        let Some(lru_idx) = lru_idx else { return };

        // Remove from exact map.
        let lru_hash = Self::hash_tokens(&self.entries[lru_idx].tokens);
        self.exact.remove(&lru_hash);

        // Swap-remove for O(1) deletion.
        let last_idx = self.entries.len() - 1;
        if lru_idx != last_idx {
            // Update the exact map for the entry that will be moved.
            let moved_hash = Self::hash_tokens(&self.entries[last_idx].tokens);
            if let Some(map_idx) = self.exact.get_mut(&moved_hash) {
                *map_idx = lru_idx;
            }
        }
        self.entries.swap_remove(lru_idx);
        self.stats.total_entries = self.entries.len();
    }
}

/// Convert `ArgType` to a serializable u8.
#[allow(dead_code)]
fn arg_type_to_u8(at: &super::ArgType) -> u8 {
    match at {
        super::ArgType::None => 0,
        super::ArgType::Span => 1,
        super::ArgType::Ref => 2,
        super::ArgType::Literal => 3,
    }
}

/// Convert u8 back to `ArgType`.
#[allow(dead_code)]
fn u8_to_arg_type(v: u8) -> super::ArgType {
    match v {
        1 => super::ArgType::Span,
        2 => super::ArgType::Ref,
        3 => super::ArgType::Literal,
        _ => super::ArgType::None,
    }
}

/// Convert `ArgValue` to JSON for serialization.
#[allow(dead_code)]
fn arg_value_to_json(av: &super::ArgValue) -> serde_json::Value {
    match av {
        super::ArgValue::None => serde_json::json!({"type": "none"}),
        super::ArgValue::Span(s) => serde_json::json!({"type": "span", "value": s}),
        super::ArgValue::Ref(r) => serde_json::json!({"type": "ref", "value": r}),
        super::ArgValue::Literal(s) => serde_json::json!({"type": "literal", "value": s}),
    }
}

/// Convert JSON back to `ArgValue`.
#[allow(dead_code)]
fn json_to_arg_value(v: Option<&serde_json::Value>) -> super::ArgValue {
    let Some(v) = v else { return super::ArgValue::None };
    let t = v.get("type").and_then(|t| t.as_str()).unwrap_or("none");
    match t {
        "span" => super::ArgValue::Span(
            v.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        ),
        "ref" => super::ArgValue::Ref(
            v.get("value").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        ),
        "literal" => super::ArgValue::Literal(
            v.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        ),
        _ => super::ArgValue::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mind::{ArgType, ArgValue, Program, ProgramStep, STOP_ID};

    /// Helper: create a simple test program with the given number of steps.
    fn make_program(steps: usize, confidence: f32) -> Program {
        let mut program_steps = Vec::new();
        for i in 0..steps {
            program_steps.push(ProgramStep {
                conv_id: i as i32,
                arg0_type: ArgType::Span,
                arg0_value: ArgValue::Span(format!("/tmp/test{i}")),
                arg1_type: ArgType::None,
                arg1_value: ArgValue::None,
            });
        }
        program_steps.push(ProgramStep {
            conv_id: STOP_ID,
            arg0_type: ArgType::None,
            arg0_value: ArgValue::None,
            arg1_type: ArgType::None,
            arg1_value: ArgValue::None,
        });
        Program {
            steps: program_steps,
            confidence,
            cached_states: Vec::new(),
        }
    }

    #[test]
    fn test_exact_match() {
        let mut layer = ReflexLayer::new(100, 0.9);
        let tokens = vec![1, 2, 3, 4, 5];
        let program = make_program(2, 0.95);

        layer.record(tokens.clone(), program.clone(), 0.95);
        assert_eq!(layer.len(), 1);

        let result = layer.try_match(&tokens);
        assert!(result.is_some());
        let (matched_prog, conf) = result.unwrap();
        assert_eq!(matched_prog.steps.len(), program.steps.len());
        assert!((conf - 0.95).abs() < 0.001);
    }

    #[test]
    fn test_no_match() {
        let mut layer = ReflexLayer::new(100, 0.9);
        let tokens = vec![1, 2, 3, 4, 5];
        let program = make_program(2, 0.95);

        layer.record(tokens, program, 0.95);

        // Completely different tokens should not match.
        let different = vec![100, 200, 300, 400, 500];
        let result = layer.try_match(&different);
        assert!(result.is_none());
    }

    #[test]
    fn test_fuzzy_match() {
        let mut layer = ReflexLayer::new(100, 0.5); // Lower threshold for testing.
        let tokens = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let program = make_program(2, 0.90);

        layer.record(tokens, program, 0.90);

        // Swap one token: [1, 2, 99, 4, 5, 6, 7, 8] -- most bigrams preserved.
        let similar = vec![1, 2, 99, 4, 5, 6, 7, 8];
        let result = layer.try_match(&similar);
        assert!(result.is_some(), "Fuzzy match should find similar token sequence");

        let (_, conf) = result.unwrap();
        // Confidence should be scaled by similarity.
        assert!(conf > 0.0);
        assert!(conf <= 0.90);
    }

    #[test]
    fn test_lru_eviction() {
        let mut layer = ReflexLayer::new(3, 0.9);

        // Fill to capacity.
        layer.record(vec![1, 2], make_program(1, 0.9), 0.9);
        layer.record(vec![3, 4], make_program(1, 0.8), 0.8);
        layer.record(vec![5, 6], make_program(1, 0.7), 0.7);
        assert_eq!(layer.len(), 3);

        // Touch the first entry to make it recently used.
        let _ = layer.try_match(&[1, 2]);

        // Add a 4th entry -- should evict the LRU (which is [3, 4], untouched).
        layer.record(vec![7, 8], make_program(1, 0.6), 0.6);
        assert_eq!(layer.len(), 3);

        // [3, 4] should be evicted.
        let result = layer.try_match(&[3, 4]);
        assert!(result.is_none(), "Evicted entry should not be found");

        // [1, 2] should still be there (was recently used).
        let result = layer.try_match(&[1, 2]);
        assert!(result.is_some(), "Recently used entry should survive eviction");

        // [7, 8] should be there (just added).
        let result = layer.try_match(&[7, 8]);
        assert!(result.is_some(), "Newly added entry should be present");
    }

    #[test]
    fn test_hit_count() {
        let mut layer = ReflexLayer::new(100, 0.9);
        let tokens = vec![10, 20, 30];
        layer.record(tokens.clone(), make_program(1, 0.85), 0.85);

        // Look up 3 times.
        for _ in 0..3 {
            let _ = layer.try_match(&tokens);
        }

        let stats = layer.stats();
        assert_eq!(stats.exact_hits, 3);
        assert_eq!(stats.total_lookups, 3);
        assert_eq!(stats.misses, 0);

        // Verify hit_count on the entry itself.
        assert_eq!(layer.entries[0].hit_count, 3);
    }

    #[test]
    fn test_serialize_roundtrip() {
        let mut layer = ReflexLayer::new(100, 0.85);
        layer.record(vec![1, 2, 3], make_program(2, 0.95), 0.95);
        layer.record(vec![10, 20, 30], make_program(1, 0.80), 0.80);

        // Touch the first to set hit_count.
        let _ = layer.try_match(&[1, 2, 3]);

        let data = layer.serialize();
        assert!(!data.is_empty());

        let restored = ReflexLayer::deserialize(&data).expect("Deserialize should succeed");
        assert_eq!(restored.len(), 2);

        // Verify entries are preserved.
        assert_eq!(restored.entries[0].tokens, vec![1, 2, 3]);
        assert_eq!(restored.entries[0].confidence, 0.95);
        assert_eq!(restored.entries[0].hit_count, 1);
        assert_eq!(restored.entries[0].program.steps.len(), 3); // 2 real + STOP

        assert_eq!(restored.entries[1].tokens, vec![10, 20, 30]);
        assert_eq!(restored.entries[1].confidence, 0.80);
        assert_eq!(restored.entries[1].hit_count, 0);
    }

    #[test]
    fn test_jaccard_similarity_identical() {
        let a = vec![1, 2, 3, 4, 5];
        assert!((ReflexLayer::jaccard_similarity(&a, &a) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_jaccard_similarity_disjoint() {
        let a = vec![1, 2, 3];
        let b = vec![10, 20, 30];
        assert!((ReflexLayer::jaccard_similarity(&a, &b) - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_jaccard_similarity_partial() {
        // [1,2,3,4] -> bigrams {(1,2), (2,3), (3,4)}
        // [1,2,3,5] -> bigrams {(1,2), (2,3), (3,5)}
        // intersection = {(1,2), (2,3)} = 2
        // union = {(1,2), (2,3), (3,4), (3,5)} = 4
        // Jaccard = 2/4 = 0.5
        let a = vec![1, 2, 3, 4];
        let b = vec![1, 2, 3, 5];
        let sim = ReflexLayer::jaccard_similarity(&a, &b);
        assert!((sim - 0.5).abs() < 0.001, "Expected 0.5, got {sim}");
    }

    #[test]
    fn test_update_existing_entry() {
        let mut layer = ReflexLayer::new(100, 0.9);
        let tokens = vec![1, 2, 3];

        layer.record(tokens.clone(), make_program(1, 0.7), 0.7);
        assert_eq!(layer.len(), 1);

        // Record same tokens with a new program -- should update, not add.
        layer.record(tokens.clone(), make_program(2, 0.9), 0.9);
        assert_eq!(layer.len(), 1);

        let result = layer.try_match(&tokens);
        assert!(result.is_some());
        let (prog, conf) = result.unwrap();
        assert_eq!(prog.steps.len(), 3); // 2 real + STOP
        assert!((conf - 0.9).abs() < 0.001);
    }

    #[test]
    fn test_empty_tokens() {
        let mut layer = ReflexLayer::new(100, 0.9);
        let result = layer.try_match(&[]);
        assert!(result.is_none());
    }

    #[test]
    fn test_consolidation() {
        use super::CONSOLIDATION_THRESHOLD;

        // Use a small capacity so we can easily fill the cache.
        let mut layer = ReflexLayer::new(3, 0.9);

        // Record a reflex and hit it enough times to exceed the consolidation threshold.
        let perm_tokens = vec![42, 43, 44];
        layer.record(perm_tokens.clone(), make_program(1, 0.95), 0.95);

        for _ in 0..CONSOLIDATION_THRESHOLD {
            let _ = layer.try_match(&perm_tokens);
        }
        assert_eq!(layer.entries[0].hit_count, CONSOLIDATION_THRESHOLD);

        // Consolidate — should promote 1 entry.
        let promoted = layer.consolidate(CONSOLIDATION_THRESHOLD);
        assert_eq!(promoted, 1);
        assert!(layer.entries[0].permanent);

        // A second consolidate should promote 0 (already permanent).
        let promoted = layer.consolidate(CONSOLIDATION_THRESHOLD);
        assert_eq!(promoted, 0);

        // Fill cache to capacity with other entries.
        layer.record(vec![10, 11], make_program(1, 0.8), 0.8);
        layer.record(vec![20, 21], make_program(1, 0.7), 0.7);
        assert_eq!(layer.len(), 3);

        // Adding another entry should evict a non-permanent entry, NOT the permanent one.
        layer.record(vec![30, 31], make_program(1, 0.6), 0.6);
        assert_eq!(layer.len(), 3);

        // The permanent reflex must still be present.
        let result = layer.try_match(&perm_tokens);
        assert!(result.is_some(), "Permanent reflex should survive LRU eviction");
    }

    #[test]
    fn test_serialize_roundtrip_with_permanent() {
        let mut layer = ReflexLayer::new(100, 0.85);
        layer.record(vec![1, 2, 3], make_program(2, 0.95), 0.95);
        layer.record(vec![10, 20, 30], make_program(1, 0.80), 0.80);

        // Hit the first entry enough to consolidate it.
        for _ in 0..50 {
            let _ = layer.try_match(&[1, 2, 3]);
        }
        layer.consolidate(50);
        assert!(layer.entries[0].permanent);
        assert!(!layer.entries[1].permanent);

        let data = layer.serialize();
        let restored = ReflexLayer::deserialize(&data).expect("Deserialize should succeed");
        assert_eq!(restored.len(), 2);
        assert!(restored.entries[0].permanent, "Permanent flag should survive serialization roundtrip");
        assert!(!restored.entries[1].permanent);
    }
}
