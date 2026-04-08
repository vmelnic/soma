//! Disk-backed memory stores that persist episodes, schemas, and routines
//! across runtime restarts.
//!
//! Each store wraps the corresponding Default*Store, delegating all query
//! logic to the in-memory implementation. Mutations (store/register/invalidate)
//! write through to a JSON file on disk after updating the in-memory state.
//! On construction, the store loads any existing data from the file.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{de::DeserializeOwned, Serialize};
use uuid::Uuid;

use crate::errors::{Result, SomaError};
use crate::types::episode::Episode;
use crate::types::schema::Schema;
use crate::types::routine::Routine;

use super::episodes::{DefaultEpisodeStore, EpisodeStore};
use super::routines::{DefaultRoutineStore, InvalidationReason, RoutineStore};
use super::schemas::{DefaultSchemaStore, SchemaStore};

// ---------------------------------------------------------------------------
// Generic disk persistence helpers
// ---------------------------------------------------------------------------

/// Resolve a data_dir string, expanding a leading `~` to the user's home directory.
pub fn resolve_data_dir(raw: &str) -> PathBuf {
    if (raw.starts_with("~/") || raw == "~")
        && let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(&raw[2..]);
        }
    PathBuf::from(raw)
}

/// Load a Vec<T> from a JSON file. Returns an empty Vec if the file does not exist.
fn load_from_disk<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path).map_err(|e| {
        SomaError::Memory(format!("failed to read {}: {e}", path.display()))
    })?;
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    let items: Vec<T> = serde_json::from_str(&content).map_err(|e| {
        SomaError::Memory(format!("failed to parse {}: {e}", path.display()))
    })?;
    Ok(items)
}

/// Write a Vec<T> to a JSON file, creating parent directories if needed.
fn save_to_disk<T: Serialize>(path: &Path, items: &[T]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            SomaError::Memory(format!(
                "failed to create directory {}: {e}",
                parent.display()
            ))
        })?;
    }
    let content = serde_json::to_string_pretty(items)?;
    fs::write(path, content).map_err(|e| {
        SomaError::Memory(format!("failed to write {}: {e}", path.display()))
    })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// DiskEpisodeStore
// ---------------------------------------------------------------------------

/// Disk-backed episode store. Delegates queries to DefaultEpisodeStore, writes
/// the full episode list to disk after every store() call.
pub struct DiskEpisodeStore {
    inner: DefaultEpisodeStore,
    path: PathBuf,
}

impl DiskEpisodeStore {
    /// Create a new DiskEpisodeStore, loading any existing episodes from disk.
    pub fn new(data_dir: &Path) -> Result<Self> {
        let path = data_dir.join("episodes.json");
        let episodes: Vec<Episode> = load_from_disk(&path)?;

        let mut inner = DefaultEpisodeStore::new();
        for ep in episodes {
            inner.store(ep)?;
        }

        tracing::info!(
            path = %path.display(),
            count = inner.count(),
            "loaded episodes from disk"
        );

        Ok(Self { inner, path })
    }

    /// Flush current state to disk.
    fn flush(&self) -> Result<()> {
        let all: Vec<&Episode> = self.inner.list(self.inner.count(), 0);
        // list() returns newest-first; reverse for chronological order on disk.
        let chronological: Vec<&Episode> = all.into_iter().rev().collect();
        save_to_disk(&self.path, &chronological)
    }
}

impl EpisodeStore for DiskEpisodeStore {
    fn store(&mut self, episode: Episode) -> Result<()> {
        self.inner.store(episode)?;
        self.flush()
    }

    fn retrieve_nearest(&self, goal_fingerprint: &str, limit: usize) -> Vec<&Episode> {
        self.inner.retrieve_nearest(goal_fingerprint, limit)
    }

    fn retrieve_by_tags(&self, tags: &[String], limit: usize) -> Vec<&Episode> {
        self.inner.retrieve_by_tags(tags, limit)
    }

    fn get(&self, episode_id: &Uuid) -> Option<&Episode> {
        self.inner.get(episode_id)
    }

    fn list(&self, limit: usize, offset: usize) -> Vec<&Episode> {
        self.inner.list(limit, offset)
    }

    fn count(&self) -> usize {
        self.inner.count()
    }
}

// ---------------------------------------------------------------------------
// DiskSchemaStore
// ---------------------------------------------------------------------------

/// Disk-backed schema store. Delegates queries to DefaultSchemaStore, writes
/// the full schema list to disk after every register() call.
pub struct DiskSchemaStore {
    inner: DefaultSchemaStore,
    path: PathBuf,
    /// Snapshot of all schemas for serialization. Kept in sync with inner.
    schemas: Vec<Schema>,
}

impl DiskSchemaStore {
    /// Create a new DiskSchemaStore, loading any existing schemas from disk.
    pub fn new(data_dir: &Path) -> Result<Self> {
        let path = data_dir.join("schemas.json");
        let schemas: Vec<Schema> = load_from_disk(&path)?;

        let mut inner = DefaultSchemaStore::new();
        for schema in &schemas {
            inner.register(schema.clone())?;
        }

        let count = schemas.len();
        tracing::info!(
            path = %path.display(),
            count,
            "loaded schemas from disk"
        );

        Ok(Self {
            inner,
            path,
            schemas,
        })
    }

    fn flush(&self) -> Result<()> {
        save_to_disk(&self.path, &self.schemas)
    }
}

impl SchemaStore for DiskSchemaStore {
    fn register(&mut self, schema: Schema) -> Result<()> {
        self.inner.register(schema.clone())?;

        // Update local snapshot: replace if exists, otherwise append.
        if let Some(existing) = self
            .schemas
            .iter_mut()
            .find(|s| s.schema_id == schema.schema_id)
        {
            *existing = schema;
        } else {
            self.schemas.push(schema);
        }

        self.flush()
    }

    fn find_matching(&self, trigger_context: &serde_json::Value) -> Vec<&Schema> {
        self.inner.find_matching(trigger_context)
    }

    fn get(&self, schema_id: &str) -> Option<&Schema> {
        self.inner.get(schema_id)
    }

    fn induce_from_episodes(&self, episodes: &[&Episode]) -> Option<Schema> {
        self.inner.induce_from_episodes(episodes)
    }

    fn list_all(&self) -> Vec<&Schema> {
        self.inner.list_all()
    }
}

// ---------------------------------------------------------------------------
// DiskRoutineStore
// ---------------------------------------------------------------------------

/// Disk-backed routine store. Delegates queries to DefaultRoutineStore, writes
/// the full routine list to disk after every mutation.
pub struct DiskRoutineStore {
    inner: DefaultRoutineStore,
    path: PathBuf,
    /// Snapshot of all routines for serialization. Kept in sync with inner.
    routines: Vec<Routine>,
}

impl DiskRoutineStore {
    /// Create a new DiskRoutineStore, loading any existing routines from disk.
    pub fn new(data_dir: &Path) -> Result<Self> {
        let path = data_dir.join("routines.json");
        let routines: Vec<Routine> = load_from_disk(&path)?;

        let mut inner = DefaultRoutineStore::new();
        for routine in &routines {
            inner.register(routine.clone())?;
        }

        let count = routines.len();
        tracing::info!(
            path = %path.display(),
            count,
            "loaded routines from disk"
        );

        Ok(Self {
            inner,
            path,
            routines,
        })
    }

    fn flush(&self) -> Result<()> {
        save_to_disk(&self.path, &self.routines)
    }
}

impl RoutineStore for DiskRoutineStore {
    fn register(&mut self, routine: Routine) -> Result<()> {
        self.inner.register(routine.clone())?;

        // Update local snapshot: replace if exists, otherwise append.
        if let Some(existing) = self
            .routines
            .iter_mut()
            .find(|r| r.routine_id == routine.routine_id)
        {
            *existing = routine;
        } else {
            self.routines.push(routine);
        }

        self.flush()
    }

    fn find_matching(&self, context: &serde_json::Value) -> Vec<&Routine> {
        self.inner.find_matching(context)
    }

    fn get(&self, routine_id: &str) -> Option<&Routine> {
        self.inner.get(routine_id)
    }

    fn compile_from_schema(
        &self,
        schema: &Schema,
        episodes: &[&Episode],
    ) -> Option<Routine> {
        self.inner.compile_from_schema(schema, episodes)
    }

    fn invalidate(&mut self, routine_id: &str) -> Result<()> {
        self.inner.invalidate(routine_id)?;
        self.routines.retain(|r| r.routine_id != routine_id);
        self.flush()
    }

    fn invalidate_by_condition(&mut self, reason: &InvalidationReason) -> Vec<String> {
        let invalidated = self.inner.invalidate_by_condition(reason);
        // Remove the same IDs from our snapshot.
        self.routines
            .retain(|r| !invalidated.contains(&r.routine_id));
        if !invalidated.is_empty()
            && let Err(e) = self.flush() {
                tracing::error!(error = %e, "failed to flush routines after invalidation");
            }
        invalidated
    }

    fn list_all(&self) -> Vec<&Routine> {
        self.inner.list_all()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::common::Precondition;
    use crate::types::episode::EpisodeOutcome;
    use crate::types::routine::RoutineOrigin;
    use crate::types::schema::RollbackBias;
    use chrono::Utc;
    use uuid::Uuid;

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("soma_persistence_test")
            .join(name)
            .join(format!("{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    fn make_episode(fingerprint: &str, tags: Vec<&str>) -> Episode {
        Episode {
            episode_id: Uuid::new_v4(),
            goal_fingerprint: fingerprint.to_string(),
            initial_belief_summary: serde_json::json!({}),
            steps: Vec::new(),
            observations: Vec::new(),
            outcome: EpisodeOutcome::Success,
            total_cost: 0.1,
            success: true,
            tags: tags.into_iter().map(String::from).collect(),
            embedding: None,
            created_at: Utc::now(),
        }
    }

    fn make_schema(id: &str) -> Schema {
        Schema {
            schema_id: id.to_string(),
            namespace: "test".to_string(),
            pack: "test".to_string(),
            name: id.to_string(),
            version: semver::Version::new(1, 0, 0),
            trigger_conditions: vec![Precondition {
                condition_type: "key_match".to_string(),
                expression: serde_json::json!({ "domain": "file" }),
                description: "domain = file".to_string(),
            }],
            resource_requirements: Vec::new(),
            subgoal_structure: Vec::new(),
            candidate_skill_ordering: vec!["skill_a".into()],
            stop_conditions: Vec::new(),
            rollback_bias: RollbackBias::Cautious,
            confidence: 0.8,
        }
    }

    fn make_routine(id: &str) -> Routine {
        Routine {
            routine_id: id.to_string(),
            namespace: "test".to_string(),
            origin: RoutineOrigin::SchemaCompiled,
            match_conditions: vec![Precondition {
                condition_type: "key_match".to_string(),
                expression: serde_json::json!({ "domain": "file" }),
                description: "domain = file".to_string(),
            }],
            compiled_skill_path: vec!["a".into(), "b".into()],
            guard_conditions: Vec::new(),
            expected_cost: 0.1,
            expected_effect: Vec::new(),
            confidence: 0.9,
        }
    }

    // --- resolve_data_dir ---

    #[test]
    fn resolve_absolute_path() {
        let resolved = resolve_data_dir("/tmp/soma/data");
        assert_eq!(resolved, PathBuf::from("/tmp/soma/data"));
    }

    #[test]
    fn resolve_tilde_path() {
        // Only test if HOME is set (it always is on unix).
        if let Ok(home) = std::env::var("HOME") {
            let resolved = resolve_data_dir("~/.soma/data");
            assert_eq!(resolved, PathBuf::from(home).join(".soma/data"));
        }
    }

    // --- DiskEpisodeStore ---

    #[test]
    fn episode_store_persists_across_instances() {
        let dir = test_dir("ep_persist");

        let ep1 = make_episode("fp1", vec!["io"]);
        let ep1_id = ep1.episode_id;
        let ep2 = make_episode("fp2", vec!["net"]);
        let ep2_id = ep2.episode_id;

        // Store two episodes.
        {
            let mut store = DiskEpisodeStore::new(&dir).unwrap();
            store.store(ep1).unwrap();
            store.store(ep2).unwrap();
            assert_eq!(store.count(), 2);
        }

        // Create a new instance and verify data was loaded.
        {
            let store = DiskEpisodeStore::new(&dir).unwrap();
            assert_eq!(store.count(), 2);
            assert!(store.get(&ep1_id).is_some());
            assert!(store.get(&ep2_id).is_some());
        }

        cleanup(&dir);
    }

    #[test]
    fn episode_store_empty_dir() {
        let dir = test_dir("ep_empty");

        let store = DiskEpisodeStore::new(&dir).unwrap();
        assert_eq!(store.count(), 0);

        cleanup(&dir);
    }

    #[test]
    fn episode_store_queries_work() {
        let dir = test_dir("ep_queries");

        let mut store = DiskEpisodeStore::new(&dir).unwrap();
        store
            .store(make_episode("file_read", vec!["io"]))
            .unwrap();
        store
            .store(make_episode("file_write", vec!["io"]))
            .unwrap();
        store
            .store(make_episode("net_get", vec!["net"]))
            .unwrap();

        assert_eq!(store.retrieve_nearest("file_", 10).len(), 2);
        assert_eq!(
            store
                .retrieve_by_tags(&["io".to_string()], 10)
                .len(),
            2
        );
        assert_eq!(store.list(2, 0).len(), 2);

        cleanup(&dir);
    }

    // --- DiskSchemaStore ---

    #[test]
    fn schema_store_persists_across_instances() {
        let dir = test_dir("schema_persist");

        {
            let mut store = DiskSchemaStore::new(&dir).unwrap();
            store.register(make_schema("s1")).unwrap();
            store.register(make_schema("s2")).unwrap();
        }

        {
            let store = DiskSchemaStore::new(&dir).unwrap();
            assert!(store.get("s1").is_some());
            assert!(store.get("s2").is_some());
        }

        cleanup(&dir);
    }

    #[test]
    fn schema_store_overwrites_on_register() {
        let dir = test_dir("schema_overwrite");

        {
            let mut store = DiskSchemaStore::new(&dir).unwrap();
            store.register(make_schema("s1")).unwrap();
        }

        {
            let mut store = DiskSchemaStore::new(&dir).unwrap();
            let updated = Schema {
                confidence: 0.99,
                ..make_schema("s1")
            };
            store.register(updated).unwrap();
        }

        {
            let store = DiskSchemaStore::new(&dir).unwrap();
            assert!((store.get("s1").unwrap().confidence - 0.99).abs() < f64::EPSILON);
        }

        cleanup(&dir);
    }

    #[test]
    fn schema_store_find_matching() {
        let dir = test_dir("schema_match");

        let mut store = DiskSchemaStore::new(&dir).unwrap();
        store.register(make_schema("s1")).unwrap();

        let ctx = serde_json::json!({ "domain": "file" });
        assert_eq!(store.find_matching(&ctx).len(), 1);

        let ctx_miss = serde_json::json!({ "domain": "crypto" });
        assert!(store.find_matching(&ctx_miss).is_empty());

        cleanup(&dir);
    }

    // --- DiskRoutineStore ---

    #[test]
    fn routine_store_persists_across_instances() {
        let dir = test_dir("routine_persist");

        {
            let mut store = DiskRoutineStore::new(&dir).unwrap();
            store.register(make_routine("r1")).unwrap();
            store.register(make_routine("r2")).unwrap();
        }

        {
            let store = DiskRoutineStore::new(&dir).unwrap();
            assert!(store.get("r1").is_some());
            assert!(store.get("r2").is_some());
        }

        cleanup(&dir);
    }

    #[test]
    fn routine_store_invalidate_persists() {
        let dir = test_dir("routine_invalidate");

        {
            let mut store = DiskRoutineStore::new(&dir).unwrap();
            store.register(make_routine("r1")).unwrap();
            store.register(make_routine("r2")).unwrap();
            store.invalidate("r1").unwrap();
        }

        {
            let store = DiskRoutineStore::new(&dir).unwrap();
            assert!(store.get("r1").is_none());
            assert!(store.get("r2").is_some());
        }

        cleanup(&dir);
    }

    #[test]
    fn routine_store_invalidate_by_condition_persists() {
        let dir = test_dir("routine_inv_cond");

        {
            let mut store = DiskRoutineStore::new(&dir).unwrap();
            store.register(make_routine("r1")).unwrap();
            store.register(make_routine("r2")).unwrap();

            let invalidated = store.invalidate_by_condition(&InvalidationReason::PolicyChanged);
            assert_eq!(invalidated.len(), 2);
        }

        {
            let store = DiskRoutineStore::new(&dir).unwrap();
            assert!(store.get("r1").is_none());
            assert!(store.get("r2").is_none());
        }

        cleanup(&dir);
    }

    #[test]
    fn routine_store_find_matching() {
        let dir = test_dir("routine_match");

        let mut store = DiskRoutineStore::new(&dir).unwrap();
        store.register(make_routine("r1")).unwrap();

        let ctx = serde_json::json!({ "domain": "file" });
        assert_eq!(store.find_matching(&ctx).len(), 1);

        cleanup(&dir);
    }

    // --- load_from_disk edge cases ---

    #[test]
    fn load_from_nonexistent_file() {
        let items: Vec<Episode> = load_from_disk(Path::new("/tmp/soma_no_such_file.json")).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn load_from_empty_file() {
        let dir = test_dir("load_empty");
        let path = dir.join("empty.json");
        fs::write(&path, "").unwrap();

        let items: Vec<Episode> = load_from_disk(&path).unwrap();
        assert!(items.is_empty());

        cleanup(&dir);
    }

    #[test]
    fn load_from_corrupt_file() {
        let dir = test_dir("load_corrupt");
        let path = dir.join("corrupt.json");
        fs::write(&path, "not valid json at all").unwrap();

        let result: Result<Vec<Episode>> = load_from_disk(&path);
        assert!(result.is_err());

        cleanup(&dir);
    }
}
