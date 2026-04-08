use std::collections::HashMap;

use crate::errors::{Result, SomaError};

/// WorldSummaryStore — compact summaries of durable state.
/// Used for bootstrapping belief state quickly at session start.
/// Organized by namespace (e.g., per-pack or per-domain).
pub trait WorldSummaryStore {
    /// Update or create a summary for the given namespace.
    fn update_summary(&mut self, namespace: &str, summary: serde_json::Value) -> Result<()>;

    /// Get the summary for a specific namespace.
    fn get_summary(&self, namespace: &str) -> Option<&serde_json::Value>;

    /// Get a merged view of all namespace summaries as a single JSON object.
    fn get_full_summary(&self) -> serde_json::Value;

    /// Serialize the entire world summary store to a binary checkpoint.
    fn checkpoint(&self) -> Result<Vec<u8>>;

    /// Restore the world summary store from a binary checkpoint.
    fn restore(&mut self, data: &[u8]) -> Result<()>;
}

/// Default in-memory world summary store backed by HashMap.
pub struct DefaultWorldSummaryStore {
    summaries: HashMap<String, serde_json::Value>,
}

impl DefaultWorldSummaryStore {
    pub fn new() -> Self {
        Self {
            summaries: HashMap::new(),
        }
    }
}

impl Default for DefaultWorldSummaryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl WorldSummaryStore for DefaultWorldSummaryStore {
    fn update_summary(&mut self, namespace: &str, summary: serde_json::Value) -> Result<()> {
        self.summaries.insert(namespace.to_string(), summary);
        Ok(())
    }

    fn get_summary(&self, namespace: &str) -> Option<&serde_json::Value> {
        self.summaries.get(namespace)
    }

    fn get_full_summary(&self) -> serde_json::Value {
        let map: serde_json::Map<String, serde_json::Value> = self
            .summaries
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        serde_json::Value::Object(map)
    }

    fn checkpoint(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(&self.summaries).map_err(SomaError::from)
    }

    fn restore(&mut self, data: &[u8]) -> Result<()> {
        let restored: HashMap<String, serde_json::Value> =
            serde_json::from_slice(data).map_err(SomaError::from)?;
        self.summaries = restored;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_and_get_summary() {
        let mut store = DefaultWorldSummaryStore::new();
        store
            .update_summary("fs", serde_json::json!({ "cwd": "/home" }))
            .unwrap();

        let s = store.get_summary("fs");
        assert!(s.is_some());
        assert_eq!(s.unwrap(), &serde_json::json!({ "cwd": "/home" }));
    }

    #[test]
    fn test_get_summary_missing() {
        let store = DefaultWorldSummaryStore::new();
        assert!(store.get_summary("nonexistent").is_none());
    }

    #[test]
    fn test_update_overwrites() {
        let mut store = DefaultWorldSummaryStore::new();
        store
            .update_summary("fs", serde_json::json!({ "cwd": "/home" }))
            .unwrap();
        store
            .update_summary("fs", serde_json::json!({ "cwd": "/tmp" }))
            .unwrap();

        assert_eq!(
            store.get_summary("fs").unwrap(),
            &serde_json::json!({ "cwd": "/tmp" })
        );
    }

    #[test]
    fn test_get_full_summary() {
        let mut store = DefaultWorldSummaryStore::new();
        store
            .update_summary("fs", serde_json::json!({ "cwd": "/home" }))
            .unwrap();
        store
            .update_summary("net", serde_json::json!({ "connected": true }))
            .unwrap();

        let full = store.get_full_summary();
        assert!(full.is_object());
        let obj = full.as_object().unwrap();
        assert_eq!(obj.len(), 2);
        assert_eq!(obj["fs"], serde_json::json!({ "cwd": "/home" }));
        assert_eq!(obj["net"], serde_json::json!({ "connected": true }));
    }

    #[test]
    fn test_get_full_summary_empty() {
        let store = DefaultWorldSummaryStore::new();
        let full = store.get_full_summary();
        assert!(full.is_object());
        assert!(full.as_object().unwrap().is_empty());
    }

    #[test]
    fn test_checkpoint_and_restore() {
        let mut store = DefaultWorldSummaryStore::new();
        store
            .update_summary("fs", serde_json::json!({ "cwd": "/home" }))
            .unwrap();
        store
            .update_summary("net", serde_json::json!({ "ip": "10.0.0.1" }))
            .unwrap();

        let data = store.checkpoint().unwrap();

        let mut restored = DefaultWorldSummaryStore::new();
        restored.restore(&data).unwrap();

        assert_eq!(
            restored.get_summary("fs").unwrap(),
            &serde_json::json!({ "cwd": "/home" })
        );
        assert_eq!(
            restored.get_summary("net").unwrap(),
            &serde_json::json!({ "ip": "10.0.0.1" })
        );
    }

    #[test]
    fn test_restore_replaces_existing() {
        let mut store = DefaultWorldSummaryStore::new();
        store
            .update_summary("old_ns", serde_json::json!("old"))
            .unwrap();

        // Create checkpoint from a different store state.
        let mut source = DefaultWorldSummaryStore::new();
        source
            .update_summary("new_ns", serde_json::json!("new"))
            .unwrap();
        let data = source.checkpoint().unwrap();

        store.restore(&data).unwrap();
        assert!(store.get_summary("old_ns").is_none());
        assert!(store.get_summary("new_ns").is_some());
    }

    #[test]
    fn test_restore_invalid_data() {
        let mut store = DefaultWorldSummaryStore::new();
        let result = store.restore(b"not valid json");
        assert!(result.is_err());
    }
}
