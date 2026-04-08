use uuid::Uuid;

use crate::errors::Result;
use crate::types::episode::Episode;

/// EpisodeStore — append-only storage for full session traces.
/// Stores both successes and failures. Supports retrieval by goal
/// similarity, tags, and direct ID lookup.
pub trait EpisodeStore {
    /// Append a completed episode to the store.
    fn store(&mut self, episode: Episode) -> Result<()>;

    /// Retrieve episodes whose goal_fingerprint is nearest to the query.
    /// Uses exact prefix matching in the default implementation;
    /// production implementations should use embedding similarity.
    fn retrieve_nearest(&self, goal_fingerprint: &str, limit: usize) -> Vec<&Episode>;

    /// Retrieve episodes matching any of the given tags.
    fn retrieve_by_tags(&self, tags: &[String], limit: usize) -> Vec<&Episode>;

    /// Get a single episode by ID.
    fn get(&self, episode_id: &Uuid) -> Option<&Episode>;

    /// List episodes in insertion order (newest first), with pagination.
    fn list(&self, limit: usize, offset: usize) -> Vec<&Episode>;

    /// Total number of stored episodes.
    fn count(&self) -> usize;
}

/// Default in-memory episode store backed by Vec<Episode>.
/// Goal similarity uses longest common prefix length as a proxy
/// for fingerprint similarity. Embedding-based retrieval can be
/// layered on top by a production implementation.
pub struct DefaultEpisodeStore {
    episodes: Vec<Episode>,
}

impl DefaultEpisodeStore {
    pub fn new() -> Self {
        Self {
            episodes: Vec::new(),
        }
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
    fn store(&mut self, episode: Episode) -> Result<()> {
        self.episodes.push(episode);
        Ok(())
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
        }
    }

    #[test]
    fn test_store_and_count() {
        let mut store = DefaultEpisodeStore::new();
        assert_eq!(store.count(), 0);

        store.store(make_episode("fp1", vec![], true)).unwrap();
        assert_eq!(store.count(), 1);

        store.store(make_episode("fp2", vec![], false)).unwrap();
        assert_eq!(store.count(), 2);
    }

    #[test]
    fn test_get_by_id() {
        let mut store = DefaultEpisodeStore::new();
        let ep = make_episode("fp1", vec![], true);
        let id = ep.episode_id;
        store.store(ep).unwrap();

        assert!(store.get(&id).is_some());
        assert_eq!(store.get(&id).unwrap().episode_id, id);
        assert!(store.get(&Uuid::new_v4()).is_none());
    }

    #[test]
    fn test_retrieve_nearest_by_prefix() {
        let mut store = DefaultEpisodeStore::new();
        store
            .store(make_episode("file_read_json", vec![], true))
            .unwrap();
        store
            .store(make_episode("file_write_csv", vec![], true))
            .unwrap();
        store
            .store(make_episode("network_http_get", vec![], true))
            .unwrap();
        store
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
        store
            .store(make_episode("file_read", vec![], true))
            .unwrap();

        let results = store.retrieve_nearest("zzz_nothing", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_retrieve_by_tags() {
        let mut store = DefaultEpisodeStore::new();
        store
            .store(make_episode("fp1", vec!["io", "file"], true))
            .unwrap();
        store
            .store(make_episode("fp2", vec!["network"], true))
            .unwrap();
        store
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
            store
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
}
