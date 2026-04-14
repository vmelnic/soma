use std::collections::VecDeque;

use uuid::Uuid;

use crate::errors::Result;
use crate::types::episode::Episode;

/// Default ring buffer capacity. At ~1.8KB per episode (typical JSON-serialized
/// size), 1024 episodes is roughly 1.8MB — small enough for any host.
const DEFAULT_CAPACITY: usize = 1024;

/// EpisodeStore — bounded storage for full session traces.
/// Stores both successes and failures. Supports retrieval by goal
/// similarity, tags, and direct ID lookup. When the store is full,
/// the oldest episode is evicted and returned so the caller can
/// consolidate it into a schema before the raw trace is lost.
pub trait EpisodeStore {
    /// Append a completed episode to the store. If the store is at capacity,
    /// the oldest episode is evicted and returned so the caller can
    /// consolidate it before it is lost.
    fn store(&mut self, episode: Episode) -> Result<Option<Episode>>;

    /// Retrieve episodes whose goal_fingerprint is nearest to the query.
    /// Uses exact prefix matching in the default implementation;
    /// production implementations should use embedding similarity.
    fn retrieve_nearest(&self, goal_fingerprint: &str, limit: usize) -> Vec<&Episode>;

    /// Retrieve episodes matching any of the given tags.
    fn retrieve_by_tags(&self, tags: &[String], limit: usize) -> Vec<&Episode>;

    /// Retrieve episodes whose embedding is similar to the query embedding.
    /// Returns episodes with cosine similarity >= threshold, sorted by
    /// similarity descending, limited to at most `limit` results.
    fn retrieve_by_embedding(&self, query_embedding: &[f32], similarity_threshold: f64, limit: usize) -> Vec<&Episode> {
        let _ = (query_embedding, similarity_threshold, limit);
        Vec::new()
    }

    /// Get a single episode by ID.
    fn get(&self, episode_id: &Uuid) -> Option<&Episode>;

    /// List episodes in insertion order (newest first), with pagination.
    fn list(&self, limit: usize, offset: usize) -> Vec<&Episode>;

    /// Total number of stored episodes.
    fn count(&self) -> usize;

    /// Returns true when the store is at 75%+ capacity, signaling that
    /// the runtime should run schema induction before episodes are evicted.
    fn needs_consolidation(&self) -> bool { false }

    /// Remove specific episodes that have already been consolidated into schemas.
    /// Returns the number of episodes actually removed.
    fn evict_consolidated(&mut self, _episode_ids: &[Uuid]) -> usize { 0 }
}

/// Default in-memory episode store backed by a VecDeque ring buffer.
/// Goal similarity uses longest common prefix length as a proxy
/// for fingerprint similarity. Embedding-based retrieval can be
/// layered on top by a production implementation.
pub struct DefaultEpisodeStore {
    episodes: VecDeque<Episode>,
    capacity: usize,
}

impl DefaultEpisodeStore {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            episodes: VecDeque::with_capacity(capacity.min(1024)),
            capacity,
        }
    }

    /// Maximum number of episodes this store will hold before evicting.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

impl Default for DefaultEpisodeStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute the length of the longest common prefix between two strings.
fn common_prefix_len(a: &str, b: &str) -> usize {
    a.chars()
        .zip(b.chars())
        .take_while(|(ca, cb)| ca == cb)
        .count()
}

impl EpisodeStore for DefaultEpisodeStore {
    fn retrieve_by_embedding(&self, query_embedding: &[f32], similarity_threshold: f64, limit: usize) -> Vec<&Episode> {
        if self.episodes.is_empty() || limit == 0 || query_embedding.is_empty() {
            return Vec::new();
        }

        let mut scored: Vec<(usize, f64)> = self.episodes
            .iter()
            .enumerate()
            .filter_map(|(idx, ep)| {
                ep.embedding.as_ref().and_then(|emb| {
                    if emb.len() == query_embedding.len() {
                        let sim = crate::memory::embedder::cosine_similarity(query_embedding, emb);
                        if sim >= similarity_threshold {
                            Some((idx, sim))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);

        scored.iter().map(|(idx, _)| &self.episodes[*idx]).collect()
    }

    fn needs_consolidation(&self) -> bool {
        if self.capacity == 0 {
            return false;
        }
        self.episodes.len() * 4 >= self.capacity * 3
    }

    fn evict_consolidated(&mut self, episode_ids: &[Uuid]) -> usize {
        let before = self.episodes.len();
        self.episodes.retain(|ep| !episode_ids.contains(&ep.episode_id));
        before - self.episodes.len()
    }

    fn store(&mut self, episode: Episode) -> Result<Option<Episode>> {
        let evicted = if self.capacity == 0 {
            // Zero-capacity store: every episode is immediately evicted.
            Some(episode)
        } else if self.episodes.len() >= self.capacity {
            let old = self.episodes.pop_front();
            self.episodes.push_back(episode);
            old
        } else {
            self.episodes.push_back(episode);
            None
        };
        Ok(evicted)
    }

    fn retrieve_nearest(&self, goal_fingerprint: &str, limit: usize) -> Vec<&Episode> {
        if self.episodes.is_empty() || limit == 0 {
            return Vec::new();
        }

        // Score each episode by common prefix length with the query fingerprint.
        // Ties broken by recency (later index = more recent).
        let mut scored: Vec<(usize, usize)> = self
            .episodes
            .iter()
            .enumerate()
            .map(|(idx, ep)| {
                let score = common_prefix_len(goal_fingerprint, &ep.goal_fingerprint);
                (idx, score)
            })
            .filter(|(_, score)| *score > 0)
            .collect();

        // Sort by score descending, then by index descending (recency).
        scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
        scored.truncate(limit);

        scored.iter().map(|(idx, _)| &self.episodes[*idx]).collect()
    }

    fn retrieve_by_tags(&self, tags: &[String], limit: usize) -> Vec<&Episode> {
        if tags.is_empty() || limit == 0 {
            return Vec::new();
        }

        self.episodes
            .iter()
            .rev() // newest first
            .filter(|ep| ep.tags.iter().any(|t| tags.contains(t)))
            .take(limit)
            .collect()
    }

    fn get(&self, episode_id: &Uuid) -> Option<&Episode> {
        self.episodes
            .iter()
            .find(|ep| &ep.episode_id == episode_id)
    }

    fn list(&self, limit: usize, offset: usize) -> Vec<&Episode> {
        self.episodes
            .iter()
            .rev() // newest first
            .skip(offset)
            .take(limit)
            .collect()
    }

    fn count(&self) -> usize {
        self.episodes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::episode::EpisodeOutcome;
    use chrono::Utc;

    fn make_episode(fingerprint: &str, tags: Vec<&str>, success: bool) -> Episode {
        Episode {
            episode_id: Uuid::new_v4(),
            goal_fingerprint: fingerprint.to_string(),
            initial_belief_summary: serde_json::json!({}),
            steps: Vec::new(),
            observations: Vec::new(),
            outcome: if success {
                EpisodeOutcome::Success
            } else {
                EpisodeOutcome::Failure
            },
            total_cost: 0.0,
            success,
            tags: tags.into_iter().map(String::from).collect(),
            embedding: None,
            created_at: Utc::now(),
            salience: 1.0,
            world_state_context: serde_json::json!({}),
        }
    }

    #[test]
    fn test_store_and_count() {
        let mut store = DefaultEpisodeStore::new();
        assert_eq!(store.count(), 0);

        let _ = store.store(make_episode("fp1", vec![], true)).unwrap();
        assert_eq!(store.count(), 1);

        let _ = store.store(make_episode("fp2", vec![], false)).unwrap();
        assert_eq!(store.count(), 2);
    }

    #[test]
    fn test_get_by_id() {
        let mut store = DefaultEpisodeStore::new();
        let ep = make_episode("fp1", vec![], true);
        let id = ep.episode_id;
        let _ = store.store(ep).unwrap();

        assert!(store.get(&id).is_some());
        assert_eq!(store.get(&id).unwrap().episode_id, id);
        assert!(store.get(&Uuid::new_v4()).is_none());
    }

    #[test]
    fn test_retrieve_nearest_by_prefix() {
        let mut store = DefaultEpisodeStore::new();
        let _ = store
            .store(make_episode("file_read_json", vec![], true))
            .unwrap();
        let _ = store
            .store(make_episode("file_write_csv", vec![], true))
            .unwrap();
        let _ = store
            .store(make_episode("network_http_get", vec![], true))
            .unwrap();
        let _ = store
            .store(make_episode("file_read_csv", vec![], true))
            .unwrap();

        let results = store.retrieve_nearest("file_read", 2);
        assert_eq!(results.len(), 2);
        // Both should be file_read_* episodes
        assert!(results[0].goal_fingerprint.starts_with("file_read"));
        assert!(results[1].goal_fingerprint.starts_with("file_read"));
    }

    #[test]
    fn test_retrieve_nearest_no_match() {
        let mut store = DefaultEpisodeStore::new();
        let _ = store
            .store(make_episode("file_read", vec![], true))
            .unwrap();

        let results = store.retrieve_nearest("zzz_nothing", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_retrieve_by_tags() {
        let mut store = DefaultEpisodeStore::new();
        let _ = store
            .store(make_episode("fp1", vec!["io", "file"], true))
            .unwrap();
        let _ = store
            .store(make_episode("fp2", vec!["network"], true))
            .unwrap();
        let _ = store
            .store(make_episode("fp3", vec!["io", "db"], false))
            .unwrap();

        let results = store.retrieve_by_tags(&["io".to_string()], 10);
        assert_eq!(results.len(), 2);

        let results = store.retrieve_by_tags(&["network".to_string()], 10);
        assert_eq!(results.len(), 1);

        let results = store.retrieve_by_tags(&["nonexistent".to_string()], 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_list_pagination() {
        let mut store = DefaultEpisodeStore::new();
        for i in 0..5 {
            let _ = store
                .store(make_episode(&format!("fp{i}"), vec![], true))
                .unwrap();
        }

        let page1 = store.list(2, 0);
        assert_eq!(page1.len(), 2);
        // Newest first: fp4, fp3
        assert_eq!(page1[0].goal_fingerprint, "fp4");
        assert_eq!(page1[1].goal_fingerprint, "fp3");

        let page2 = store.list(2, 2);
        assert_eq!(page2.len(), 2);
        assert_eq!(page2[0].goal_fingerprint, "fp2");

        let page3 = store.list(2, 4);
        assert_eq!(page3.len(), 1);
        assert_eq!(page3[0].goal_fingerprint, "fp0");
    }

    #[test]
    fn test_empty_queries() {
        let store = DefaultEpisodeStore::new();
        assert!(store.retrieve_nearest("fp", 5).is_empty());
        assert!(store.retrieve_by_tags(&["tag".to_string()], 5).is_empty());
        assert!(store.list(10, 0).is_empty());
        assert_eq!(store.count(), 0);
    }

    // --- Ring buffer tests ---

    #[test]
    fn test_with_capacity_and_defaults() {
        let store = DefaultEpisodeStore::new();
        assert_eq!(store.capacity(), 1024);
        assert_eq!(store.count(), 0);

        let store = DefaultEpisodeStore::with_capacity(8);
        assert_eq!(store.capacity(), 8);
    }

    #[test]
    fn test_ring_buffer_evicts_oldest() {
        let mut store = DefaultEpisodeStore::with_capacity(3);

        let ep1 = make_episode("fp1", vec![], true);
        let ep1_id = ep1.episode_id;
        let ep2 = make_episode("fp2", vec![], true);
        let ep3 = make_episode("fp3", vec![], true);

        // Fill to capacity — no evictions.
        assert!(store.store(ep1).unwrap().is_none());
        assert!(store.store(ep2).unwrap().is_none());
        assert!(store.store(ep3).unwrap().is_none());
        assert_eq!(store.count(), 3);

        // Fourth store evicts the oldest (ep1).
        let ep4 = make_episode("fp4", vec![], true);
        let evicted = store.store(ep4).unwrap();
        assert!(evicted.is_some());
        assert_eq!(evicted.unwrap().episode_id, ep1_id);
        assert_eq!(store.count(), 3);

        // ep1 is gone, ep2/ep3/ep4 remain.
        assert!(store.get(&ep1_id).is_none());
        assert_eq!(store.list(10, 0).len(), 3);
        assert_eq!(store.list(10, 0)[0].goal_fingerprint, "fp4");
    }

    #[test]
    fn test_store_returns_evicted_episode() {
        let mut store = DefaultEpisodeStore::with_capacity(1);

        let ep1 = make_episode("fp1", vec![], true);
        let ep1_id = ep1.episode_id;
        assert!(store.store(ep1).unwrap().is_none());

        let ep2 = make_episode("fp2", vec![], true);
        let evicted = store.store(ep2).unwrap();
        assert_eq!(evicted.unwrap().episode_id, ep1_id);
        assert_eq!(store.count(), 1);
        assert_eq!(store.list(10, 0)[0].goal_fingerprint, "fp2");
    }

    #[test]
    fn test_needs_consolidation_threshold() {
        let mut store = DefaultEpisodeStore::with_capacity(4);

        // 0/4 = 0% — no consolidation needed.
        assert!(!store.needs_consolidation());

        store.store(make_episode("fp1", vec![], true)).unwrap();
        // 1/4 = 25% — no.
        assert!(!store.needs_consolidation());

        store.store(make_episode("fp2", vec![], true)).unwrap();
        // 2/4 = 50% — no.
        assert!(!store.needs_consolidation());

        store.store(make_episode("fp3", vec![], true)).unwrap();
        // 3/4 = 75% — yes.
        assert!(store.needs_consolidation());

        store.store(make_episode("fp4", vec![], true)).unwrap();
        // 4/4 = 100% — yes.
        assert!(store.needs_consolidation());
    }

    #[test]
    fn test_evict_consolidated_removes_specific_episodes() {
        let mut store = DefaultEpisodeStore::with_capacity(5);

        let ep1 = make_episode("fp1", vec![], true);
        let id1 = ep1.episode_id;
        let ep2 = make_episode("fp2", vec![], true);
        let id2 = ep2.episode_id;
        let ep3 = make_episode("fp3", vec![], true);

        store.store(ep1).unwrap();
        store.store(ep2).unwrap();
        store.store(ep3).unwrap();
        assert_eq!(store.count(), 3);

        // Evict ep1 and ep2 (as if they were consolidated into a schema).
        let removed = store.evict_consolidated(&[id1, id2]);
        assert_eq!(removed, 2);
        assert_eq!(store.count(), 1);
        assert_eq!(store.list(10, 0)[0].goal_fingerprint, "fp3");

        // Evicting non-existent IDs removes nothing.
        let removed = store.evict_consolidated(&[Uuid::new_v4()]);
        assert_eq!(removed, 0);
        assert_eq!(store.count(), 1);
    }

    #[test]
    fn test_capacity_zero_always_evicts() {
        let mut store = DefaultEpisodeStore::with_capacity(0);
        assert_eq!(store.capacity(), 0);
        assert!(!store.needs_consolidation());

        let ep = make_episode("fp1", vec![], true);
        let ep_id = ep.episode_id;
        let evicted = store.store(ep).unwrap();
        // Episode is returned immediately — never stored.
        assert_eq!(evicted.unwrap().episode_id, ep_id);
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn test_capacity_one() {
        let mut store = DefaultEpisodeStore::with_capacity(1);

        let ep1 = make_episode("fp1", vec![], true);
        assert!(store.store(ep1).unwrap().is_none());
        assert_eq!(store.count(), 1);
        // 1/1 = 100% — needs consolidation.
        assert!(store.needs_consolidation());

        let ep2 = make_episode("fp2", vec![], true);
        let evicted = store.store(ep2).unwrap();
        assert!(evicted.is_some());
        assert_eq!(store.count(), 1);
    }

    #[test]
    fn test_prefix_matching_works_with_ring_buffer() {
        // Ensure the prefix-matching retrieval logic still works correctly
        // after switching from Vec to VecDeque.
        let mut store = DefaultEpisodeStore::with_capacity(4);
        store.store(make_episode("file_read_json", vec![], true)).unwrap();
        store.store(make_episode("file_write_csv", vec![], true)).unwrap();
        store.store(make_episode("network_get", vec![], true)).unwrap();
        store.store(make_episode("file_read_csv", vec![], true)).unwrap();

        let results = store.retrieve_nearest("file_read", 2);
        assert_eq!(results.len(), 2);
        assert!(results[0].goal_fingerprint.starts_with("file_read"));
        assert!(results[1].goal_fingerprint.starts_with("file_read"));
    }

    // --- retrieve_by_embedding tests ---

    fn make_episode_with_embedding(fingerprint: &str, embedding: Vec<f32>) -> Episode {
        let mut ep = make_episode(fingerprint, vec![], true);
        ep.embedding = Some(embedding);
        ep
    }

    #[test]
    fn test_retrieve_by_embedding_basic() {
        let mut store = DefaultEpisodeStore::new();

        // Store episodes with known embeddings (3-dimensional for simplicity).
        let _ = store.store(make_episode_with_embedding("fp1", vec![1.0, 0.0, 0.0])).unwrap();
        let _ = store.store(make_episode_with_embedding("fp2", vec![0.9, 0.1, 0.0])).unwrap();
        let _ = store.store(make_episode_with_embedding("fp3", vec![0.0, 0.0, 1.0])).unwrap();

        // Query with a vector close to fp1 and fp2.
        let results = store.retrieve_by_embedding(&[1.0, 0.0, 0.0], 0.8, 5);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].goal_fingerprint, "fp1");
        assert_eq!(results[1].goal_fingerprint, "fp2");
    }

    #[test]
    fn test_retrieve_by_embedding_respects_threshold() {
        let mut store = DefaultEpisodeStore::new();
        let _ = store.store(make_episode_with_embedding("fp1", vec![1.0, 0.0, 0.0])).unwrap();
        let _ = store.store(make_episode_with_embedding("fp2", vec![0.0, 1.0, 0.0])).unwrap();

        // High threshold: only exact match.
        let results = store.retrieve_by_embedding(&[1.0, 0.0, 0.0], 0.99, 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].goal_fingerprint, "fp1");
    }

    #[test]
    fn test_retrieve_by_embedding_respects_limit() {
        let mut store = DefaultEpisodeStore::new();
        for i in 0..5 {
            let _ = store.store(make_episode_with_embedding(&format!("fp{i}"), vec![1.0, 0.0, 0.0])).unwrap();
        }

        let results = store.retrieve_by_embedding(&[1.0, 0.0, 0.0], 0.5, 2);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_retrieve_by_embedding_skips_episodes_without_embedding() {
        let mut store = DefaultEpisodeStore::new();
        let _ = store.store(make_episode("no_emb", vec![], true)).unwrap();
        let _ = store.store(make_episode_with_embedding("has_emb", vec![1.0, 0.0, 0.0])).unwrap();

        let results = store.retrieve_by_embedding(&[1.0, 0.0, 0.0], 0.5, 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].goal_fingerprint, "has_emb");
    }

    #[test]
    fn test_retrieve_by_embedding_empty_store() {
        let store = DefaultEpisodeStore::new();
        let results = store.retrieve_by_embedding(&[1.0, 0.0], 0.5, 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_retrieve_by_embedding_empty_query() {
        let mut store = DefaultEpisodeStore::new();
        let _ = store.store(make_episode_with_embedding("fp1", vec![1.0, 0.0])).unwrap();

        let results = store.retrieve_by_embedding(&[], 0.5, 5);
        assert!(results.is_empty());
    }
}
