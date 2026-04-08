use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::errors::{Result, SomaError};
use crate::types::common::FactProvenance;
use crate::types::peer::ConflictState;

// --- SubscriptionId ---

/// Opaque identifier for a resource subscription.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubscriptionId(pub String);

impl SubscriptionId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for SubscriptionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// --- SyncOutcome ---

/// The outcome of a belief synchronization attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncOutcome {
    /// Beliefs were successfully merged.
    Merged,
    /// A conflict was detected that requires resolution.
    Conflict,
    /// The remote belief is stale relative to local state.
    Stale,
}

// --- ResourceSyncMode ---

/// Resource sync levels from distributed.md: snapshot, delta, event stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceSyncMode {
    Snapshot,
    Delta,
    EventStream,
}

// --- SyncResult ---

/// Full result of a belief sync operation, including outcome, details,
/// conflict state, freshness indicator, and fact provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResult {
    pub outcome: SyncOutcome,
    pub details: serde_json::Value,
    pub peer_id: String,
    pub local_version: u64,
    pub remote_version: u64,
    /// Conflict state for each synced item — 5 states from distributed.md.
    pub conflict_state: ConflictState,
    /// Freshness indicator — each remote result includes freshness.
    pub freshness_ms: u64,
    /// Timestamp of the sync.
    pub timestamp: DateTime<Utc>,
    /// Whether this result exceeds the acceptable staleness threshold.
    /// Callers should treat stale results as unreliable.
    #[serde(default)]
    pub stale: bool,
}

impl SyncResult {
    /// Check freshness against a threshold and mark the result stale if
    /// `freshness_ms` exceeds `max_staleness_ms`. A threshold of 0 means
    /// no staleness constraint (the result is never marked stale).
    pub fn enforce_staleness(&mut self, max_staleness_ms: u64) {
        if max_staleness_ms > 0 && self.freshness_ms > max_staleness_ms {
            self.stale = true;
            self.outcome = SyncOutcome::Stale;
        }
    }
}

// --- BeliefFactEntry ---

/// A belief fact entry for sync — distinguishes 5 fact types per distributed.md:
/// asserted, observed, inferred, stale, remote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeliefFactEntry {
    pub fact_id: String,
    pub subject: String,
    pub predicate: String,
    pub value: serde_json::Value,
    pub provenance: FactProvenance,
    pub confidence: f64,
    pub version: u64,
    pub timestamp: DateTime<Utc>,
}

// --- Subscription record ---

/// Record for an active resource subscription. Tracks the peer, resource type,
/// sync mode, and the last version seen so `check_subscriptions` can detect
/// changes that occurred since the previous poll.
#[derive(Debug, Clone)]
pub struct SubscriptionRecord {
    pub peer_id: String,
    pub resource_type: String,
    pub sync_mode: ResourceSyncMode,
    /// The version at which this subscription was created or last checked.
    pub last_seen_version: u64,
    /// Timestamp when the subscription was created.
    pub created_at: DateTime<Utc>,
}

/// Notification emitted by `check_subscriptions` when a resource change is
/// detected (or will be detected once transport polling is wired).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceChangeNotification {
    pub subscription_id: String,
    pub peer_id: String,
    pub resource_type: String,
    pub sync_mode: ResourceSyncMode,
    pub timestamp: DateTime<Utc>,
}

// --- BeliefSync trait ---

/// Belief and resource synchronization between peers.
/// Handles sync of belief summaries, resource subscriptions,
/// and conflict-aware merging per distributed.md.
pub trait BeliefSync: Send + Sync {
    /// Synchronize a belief summary with a remote peer.
    /// MUST preserve provenance and versioning.
    /// MUST distinguish 5 fact types: asserted, observed, inferred, stale, remote.
    /// Returns conflict state and freshness indicator.
    fn sync_belief(
        &mut self,
        peer_id: &str,
        facts: &[BeliefFactEntry],
    ) -> Result<SyncResult>;

    /// Subscribe to resource updates from a remote peer.
    /// The sync_mode MUST be declared by the peer (snapshot, delta, event stream).
    fn subscribe_resource(
        &mut self,
        peer_id: &str,
        resource_type: &str,
        sync_mode: ResourceSyncMode,
    ) -> Result<SubscriptionId>;

    /// Unsubscribe from a previously established resource subscription.
    fn unsubscribe(&mut self, subscription_id: &SubscriptionId) -> Result<()>;
}

// --- DefaultBeliefSync ---

/// Default implementation with local belief storage.
/// Maintains subscriptions and a local belief store keyed by belief key.
/// The `sync_belief` method merges incoming facts against local beliefs
/// and returns a SyncResult with freshness based on last update time.
pub struct DefaultBeliefSync {
    subscriptions: HashMap<String, SubscriptionRecord>,
    next_sub_id: u64,
    /// Local belief store: belief_key -> value.
    local_beliefs: HashMap<String, serde_json::Value>,
    /// Timestamps tracking when each belief was last updated.
    belief_timestamps: HashMap<String, DateTime<Utc>>,
    /// Version counter for local beliefs.
    local_version: u64,
}

impl DefaultBeliefSync {
    pub fn new() -> Self {
        Self {
            subscriptions: HashMap::new(),
            next_sub_id: 1,
            local_beliefs: HashMap::new(),
            belief_timestamps: HashMap::new(),
            local_version: 0,
        }
    }

    /// Set a local belief value, recording the current time for freshness.
    pub fn set_local_belief(&mut self, key: impl Into<String>, value: serde_json::Value) {
        let key = key.into();
        self.local_beliefs.insert(key.clone(), value);
        self.belief_timestamps.insert(key, Utc::now());
        self.local_version += 1;
    }

    /// Return active subscriptions as a reference to the internal map.
    pub fn subscriptions(&self) -> &HashMap<String, SubscriptionRecord> {
        &self.subscriptions
    }

    /// Return a list of all active subscription records with their IDs.
    pub fn list_subscriptions(&self) -> Vec<(&str, &SubscriptionRecord)> {
        self.subscriptions
            .iter()
            .map(|(id, rec)| (id.as_str(), rec))
            .collect()
    }

    /// Remove all subscriptions matching a given peer and resource type.
    /// Returns the number of subscriptions removed.
    pub fn unsubscribe_by_resource(&mut self, peer_id: &str, resource_type: &str) -> usize {
        let to_remove: Vec<String> = self
            .subscriptions
            .iter()
            .filter(|(_, rec)| rec.peer_id == peer_id && rec.resource_type == resource_type)
            .map(|(id, _)| id.clone())
            .collect();
        let count = to_remove.len();
        for id in to_remove {
            self.subscriptions.remove(&id);
        }
        count
    }

    /// Poll all active subscriptions for resource changes. Returns a
    /// notification for each subscription whose peer has produced changes
    /// since the last check. Currently returns empty — actual transport
    /// polling will fill this in once the transport layer is integrated.
    /// The subscription bookkeeping is live: records are created, stored,
    /// and removed through `subscribe_resource` / `unsubscribe`.
    pub fn check_subscriptions(&mut self) -> Vec<ResourceChangeNotification> {
        let mut notifications = Vec::new();

        // For each subscription, check if the local version has advanced
        // past the last_seen_version (which would indicate that new beliefs
        // arrived from syncs). This is a local-only heuristic; real change
        // detection will query the peer over transport.
        for (sub_id, record) in &mut self.subscriptions {
            if self.local_version > record.last_seen_version {
                notifications.push(ResourceChangeNotification {
                    subscription_id: sub_id.clone(),
                    peer_id: record.peer_id.clone(),
                    resource_type: record.resource_type.clone(),
                    sync_mode: record.sync_mode,
                    timestamp: Utc::now(),
                });
                record.last_seen_version = self.local_version;
            }
        }

        notifications
    }
}

impl Default for DefaultBeliefSync {
    fn default() -> Self {
        Self::new()
    }
}

impl BeliefSync for DefaultBeliefSync {
    fn sync_belief(
        &mut self,
        peer_id: &str,
        facts: &[BeliefFactEntry],
    ) -> Result<SyncResult> {
        let now = Utc::now();

        // Merge incoming facts into local beliefs. Each fact's fact_id is
        // used as the belief key. Newer versions overwrite older ones.
        let mut merged_count = 0u64;
        let mut remote_version = 0u64;
        for fact in facts {
            remote_version = remote_version.max(fact.version);
            let dominated = self.local_beliefs.contains_key(&fact.fact_id)
                && self.belief_timestamps.get(&fact.fact_id)
                    .map(|ts| *ts >= fact.timestamp)
                    .unwrap_or(false);
            if !dominated {
                self.local_beliefs.insert(fact.fact_id.clone(), fact.value.clone());
                self.belief_timestamps.insert(fact.fact_id.clone(), fact.timestamp);
                self.local_version += 1;
                merged_count += 1;
            }
        }

        // Freshness: time since the oldest belief timestamp among merged facts,
        // or 0 if nothing was merged.
        let freshness_ms = if merged_count > 0 {
            let oldest = facts.iter()
                .map(|f| f.timestamp)
                .min()
                .unwrap_or(now);
            let elapsed = now.signed_duration_since(oldest);
            elapsed.num_milliseconds().max(0) as u64
        } else {
            0
        };

        Ok(SyncResult {
            outcome: SyncOutcome::Merged,
            details: serde_json::json!({
                "merged_facts": merged_count,
                "total_incoming": facts.len(),
            }),
            peer_id: peer_id.to_string(),
            local_version: self.local_version,
            remote_version,
            conflict_state: ConflictState::Confirmed,
            freshness_ms,
            timestamp: now,
            stale: false,
        })
    }

    fn subscribe_resource(
        &mut self,
        peer_id: &str,
        resource_type: &str,
        sync_mode: ResourceSyncMode,
    ) -> Result<SubscriptionId> {
        let id = format!("sub-{}", self.next_sub_id);
        self.next_sub_id += 1;
        let sub_id = SubscriptionId::new(&id);
        self.subscriptions.insert(
            id,
            SubscriptionRecord {
                peer_id: peer_id.to_string(),
                resource_type: resource_type.to_string(),
                sync_mode,
                last_seen_version: self.local_version,
                created_at: Utc::now(),
            },
        );
        Ok(sub_id)
    }

    fn unsubscribe(&mut self, subscription_id: &SubscriptionId) -> Result<()> {
        if self.subscriptions.remove(&subscription_id.0).is_none() {
            return Err(SomaError::Resource(format!(
                "subscription not found: {}",
                subscription_id
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fact(id: &str, provenance: FactProvenance) -> BeliefFactEntry {
        BeliefFactEntry {
            fact_id: id.to_string(),
            subject: "test_subject".to_string(),
            predicate: "has_value".to_string(),
            value: serde_json::json!(42),
            provenance,
            confidence: 0.9,
            version: 1,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn sync_belief_merges_facts_into_local_store() {
        let mut sync = DefaultBeliefSync::new();
        let facts = vec![
            make_fact("f1", FactProvenance::Asserted),
            make_fact("f2", FactProvenance::Observed),
        ];
        let result = sync.sync_belief("peer-1", &facts).unwrap();
        assert_eq!(result.outcome, SyncOutcome::Merged);
        assert_eq!(result.peer_id, "peer-1");
        assert_eq!(result.details["merged_facts"], 2);
        assert_eq!(result.details["total_incoming"], 2);
        assert_eq!(result.conflict_state, ConflictState::Confirmed);
        assert!(!result.stale);
    }

    #[test]
    fn subscribe_resource_returns_id() {
        let mut sync = DefaultBeliefSync::new();
        let sub_id = sync
            .subscribe_resource("peer-1", "filesystem", ResourceSyncMode::Snapshot)
            .unwrap();
        assert!(sub_id.0.starts_with("sub-"));
    }

    #[test]
    fn subscribe_resource_increments_ids() {
        let mut sync = DefaultBeliefSync::new();
        let id1 = sync
            .subscribe_resource("peer-1", "filesystem", ResourceSyncMode::Snapshot)
            .unwrap();
        let id2 = sync
            .subscribe_resource("peer-2", "database", ResourceSyncMode::Delta)
            .unwrap();
        assert_ne!(id1, id2);
    }

    #[test]
    fn unsubscribe_succeeds() {
        let mut sync = DefaultBeliefSync::new();
        let sub_id = sync
            .subscribe_resource("peer-1", "filesystem", ResourceSyncMode::EventStream)
            .unwrap();
        sync.unsubscribe(&sub_id).unwrap();
    }

    #[test]
    fn unsubscribe_unknown_fails() {
        let mut sync = DefaultBeliefSync::new();
        let bogus = SubscriptionId::new("sub-999");
        assert!(sync.unsubscribe(&bogus).is_err());
    }

    #[test]
    fn unsubscribe_twice_fails() {
        let mut sync = DefaultBeliefSync::new();
        let sub_id = sync
            .subscribe_resource("peer-1", "filesystem", ResourceSyncMode::Snapshot)
            .unwrap();
        sync.unsubscribe(&sub_id).unwrap();
        assert!(sync.unsubscribe(&sub_id).is_err());
    }

    #[test]
    fn subscription_id_display() {
        let id = SubscriptionId::new("sub-42");
        assert_eq!(format!("{}", id), "sub-42");
    }

    #[test]
    fn sync_result_serialization() {
        let result = SyncResult {
            outcome: SyncOutcome::Merged,
            details: serde_json::json!({"merged_facts": 3}),
            peer_id: "peer-1".to_string(),
            local_version: 5,
            remote_version: 4,
            conflict_state: ConflictState::Confirmed,
            freshness_ms: 200,
            timestamp: Utc::now(),
            stale: false,
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["outcome"], "merged");
        assert_eq!(json["local_version"], 5);
        assert_eq!(json["remote_version"], 4);
        assert_eq!(json["conflict_state"], "confirmed");
        assert_eq!(json["freshness_ms"], 200);
        assert_eq!(json["stale"], false);
    }

    #[test]
    fn sync_outcome_variants_serialize() {
        let merged = serde_json::to_value(SyncOutcome::Merged).unwrap();
        let conflict = serde_json::to_value(SyncOutcome::Conflict).unwrap();
        let stale = serde_json::to_value(SyncOutcome::Stale).unwrap();
        assert_eq!(merged, "merged");
        assert_eq!(conflict, "conflict");
        assert_eq!(stale, "stale");
    }

    #[test]
    fn conflict_state_variants_serialize() {
        let confirmed = serde_json::to_value(ConflictState::Confirmed).unwrap();
        let tentative = serde_json::to_value(ConflictState::Tentative).unwrap();
        let conflicting = serde_json::to_value(ConflictState::Conflicting).unwrap();
        let stale = serde_json::to_value(ConflictState::Stale).unwrap();
        let unresolved = serde_json::to_value(ConflictState::Unresolved).unwrap();
        assert_eq!(confirmed, "confirmed");
        assert_eq!(tentative, "tentative");
        assert_eq!(conflicting, "conflicting");
        assert_eq!(stale, "stale");
        assert_eq!(unresolved, "unresolved");
    }

    #[test]
    fn resource_sync_mode_variants_serialize() {
        let snap = serde_json::to_value(ResourceSyncMode::Snapshot).unwrap();
        let delta = serde_json::to_value(ResourceSyncMode::Delta).unwrap();
        let stream = serde_json::to_value(ResourceSyncMode::EventStream).unwrap();
        assert_eq!(snap, "snapshot");
        assert_eq!(delta, "delta");
        assert_eq!(stream, "event_stream");
    }

    #[test]
    fn belief_fact_entry_five_provenance_types() {
        // Verify all 5 fact types from distributed.md can be created.
        let facts = vec![
            make_fact("f1", FactProvenance::Asserted),
            make_fact("f2", FactProvenance::Observed),
            make_fact("f3", FactProvenance::Inferred),
            make_fact("f4", FactProvenance::Stale),
            make_fact("f5", FactProvenance::Remote),
        ];
        assert_eq!(facts.len(), 5);
        for fact in &facts {
            let json = serde_json::to_value(fact).unwrap();
            assert!(json["provenance"].is_string());
        }
    }

    fn make_sync_result(freshness_ms: u64) -> SyncResult {
        SyncResult {
            outcome: SyncOutcome::Merged,
            details: serde_json::json!({}),
            peer_id: "peer-1".to_string(),
            local_version: 1,
            remote_version: 2,
            conflict_state: ConflictState::Confirmed,
            freshness_ms,
            timestamp: Utc::now(),
            stale: false,
        }
    }

    #[test]
    fn enforce_staleness_marks_stale_when_exceeded() {
        let mut result = make_sync_result(5000);
        result.enforce_staleness(3000);
        assert!(result.stale);
        assert_eq!(result.outcome, SyncOutcome::Stale);
    }

    #[test]
    fn enforce_staleness_leaves_fresh_result_unchanged() {
        let mut result = make_sync_result(1000);
        result.enforce_staleness(3000);
        assert!(!result.stale);
        assert_eq!(result.outcome, SyncOutcome::Merged);
    }

    #[test]
    fn enforce_staleness_zero_threshold_means_no_constraint() {
        let mut result = make_sync_result(999_999);
        result.enforce_staleness(0);
        assert!(!result.stale);
        assert_eq!(result.outcome, SyncOutcome::Merged);
    }

    #[test]
    fn enforce_staleness_exact_threshold_is_not_stale() {
        let mut result = make_sync_result(3000);
        result.enforce_staleness(3000);
        assert!(!result.stale);
        assert_eq!(result.outcome, SyncOutcome::Merged);
    }

    #[test]
    fn stale_defaults_false_on_deserialize() {
        let json = serde_json::json!({
            "outcome": "merged",
            "details": {},
            "peer_id": "peer-1",
            "local_version": 1,
            "remote_version": 2,
            "conflict_state": "confirmed",
            "freshness_ms": 200,
            "timestamp": Utc::now()
        });
        let result: SyncResult = serde_json::from_value(json).unwrap();
        assert!(!result.stale);
    }

    // --- sync_belief local store tests ---

    #[test]
    fn sync_belief_stores_beliefs_locally() {
        let mut sync = DefaultBeliefSync::new();
        let facts = vec![make_fact("f1", FactProvenance::Asserted)];
        sync.sync_belief("peer-1", &facts).unwrap();

        // The belief should now be in the local store.
        assert!(sync.local_beliefs.contains_key("f1"));
        assert_eq!(sync.local_beliefs["f1"], serde_json::json!(42));
    }

    #[test]
    fn sync_belief_increments_local_version() {
        let mut sync = DefaultBeliefSync::new();
        assert_eq!(sync.local_version, 0);

        let facts = vec![make_fact("f1", FactProvenance::Asserted)];
        let result = sync.sync_belief("peer-1", &facts).unwrap();
        // Version is incremented per merged fact.
        assert!(result.local_version > 0);
    }

    #[test]
    fn sync_belief_does_not_overwrite_newer_local() {
        let mut sync = DefaultBeliefSync::new();

        // Set a local belief first (which gets a current timestamp).
        sync.set_local_belief("f1", serde_json::json!(99));

        // Create a fact with a timestamp in the past.
        let mut old_fact = make_fact("f1", FactProvenance::Remote);
        old_fact.timestamp = Utc::now() - chrono::Duration::hours(1);
        old_fact.value = serde_json::json!(1);

        sync.sync_belief("peer-1", &[old_fact]).unwrap();

        // The local value should remain unchanged.
        assert_eq!(sync.local_beliefs["f1"], serde_json::json!(99));
    }

    #[test]
    fn set_local_belief_stores_and_timestamps() {
        let mut sync = DefaultBeliefSync::new();
        sync.set_local_belief("key1", serde_json::json!("hello"));

        assert_eq!(sync.local_beliefs["key1"], serde_json::json!("hello"));
        assert!(sync.belief_timestamps.contains_key("key1"));
        assert_eq!(sync.local_version, 1);

        sync.set_local_belief("key2", serde_json::json!(42));
        assert_eq!(sync.local_version, 2);
    }

    #[test]
    fn sync_belief_empty_facts_returns_merged_with_zero() {
        let mut sync = DefaultBeliefSync::new();
        let result = sync.sync_belief("peer-1", &[]).unwrap();
        assert_eq!(result.outcome, SyncOutcome::Merged);
        assert_eq!(result.details["merged_facts"], 0);
        assert_eq!(result.details["total_incoming"], 0);
        assert_eq!(result.freshness_ms, 0);
    }

    #[test]
    fn sync_belief_tracks_remote_version() {
        let mut sync = DefaultBeliefSync::new();
        let mut f1 = make_fact("f1", FactProvenance::Asserted);
        f1.version = 5;
        let mut f2 = make_fact("f2", FactProvenance::Observed);
        f2.version = 10;

        let result = sync.sync_belief("peer-1", &[f1, f2]).unwrap();
        assert_eq!(result.remote_version, 10); // max of incoming versions
    }

    #[test]
    fn sync_belief_multiple_rounds_accumulate() {
        let mut sync = DefaultBeliefSync::new();

        let facts1 = vec![make_fact("f1", FactProvenance::Asserted)];
        sync.sync_belief("peer-1", &facts1).unwrap();

        let facts2 = vec![make_fact("f2", FactProvenance::Observed)];
        let result = sync.sync_belief("peer-2", &facts2).unwrap();

        assert_eq!(result.peer_id, "peer-2");
        assert!(sync.local_beliefs.contains_key("f1"));
        assert!(sync.local_beliefs.contains_key("f2"));
    }

    // --- check_subscriptions tests ---

    #[test]
    fn check_subscriptions_returns_empty_when_no_changes() {
        let mut sync = DefaultBeliefSync::new();
        sync.subscribe_resource("peer-1", "filesystem", ResourceSyncMode::Snapshot)
            .unwrap();

        // No beliefs synced yet — no version advancement.
        let notifications = sync.check_subscriptions();
        assert!(notifications.is_empty());
    }

    #[test]
    fn check_subscriptions_detects_version_advancement() {
        let mut sync = DefaultBeliefSync::new();
        let sub_id = sync
            .subscribe_resource("peer-1", "filesystem", ResourceSyncMode::Delta)
            .unwrap();

        // Sync a belief to advance the local version.
        let facts = vec![make_fact("f1", FactProvenance::Asserted)];
        sync.sync_belief("peer-1", &facts).unwrap();

        let notifications = sync.check_subscriptions();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].subscription_id, sub_id.0);
        assert_eq!(notifications[0].peer_id, "peer-1");
        assert_eq!(notifications[0].resource_type, "filesystem");
        assert_eq!(notifications[0].sync_mode, ResourceSyncMode::Delta);
    }

    #[test]
    fn check_subscriptions_does_not_repeat_notification() {
        let mut sync = DefaultBeliefSync::new();
        sync.subscribe_resource("peer-1", "filesystem", ResourceSyncMode::Snapshot)
            .unwrap();

        let facts = vec![make_fact("f1", FactProvenance::Asserted)];
        sync.sync_belief("peer-1", &facts).unwrap();

        // First check fires the notification.
        let notifications = sync.check_subscriptions();
        assert_eq!(notifications.len(), 1);

        // Second check with no new changes returns empty.
        let notifications = sync.check_subscriptions();
        assert!(notifications.is_empty());
    }

    #[test]
    fn check_subscriptions_multiple_subscriptions() {
        let mut sync = DefaultBeliefSync::new();
        sync.subscribe_resource("peer-1", "filesystem", ResourceSyncMode::Snapshot)
            .unwrap();
        sync.subscribe_resource("peer-2", "database", ResourceSyncMode::EventStream)
            .unwrap();

        let facts = vec![make_fact("f1", FactProvenance::Asserted)];
        sync.sync_belief("peer-1", &facts).unwrap();

        let notifications = sync.check_subscriptions();
        // Both subscriptions see the version advance.
        assert_eq!(notifications.len(), 2);
    }

    #[test]
    fn subscription_record_fields_accessible() {
        let mut sync = DefaultBeliefSync::new();
        sync.subscribe_resource("peer-1", "filesystem", ResourceSyncMode::Snapshot)
            .unwrap();

        let subs = sync.subscriptions();
        assert_eq!(subs.len(), 1);
        let record = subs.values().next().unwrap();
        assert_eq!(record.peer_id, "peer-1");
        assert_eq!(record.resource_type, "filesystem");
        assert_eq!(record.sync_mode, ResourceSyncMode::Snapshot);
        assert_eq!(record.last_seen_version, 0);
    }

    #[test]
    fn resource_change_notification_serializes() {
        let notification = ResourceChangeNotification {
            subscription_id: "sub-1".to_string(),
            peer_id: "peer-1".to_string(),
            resource_type: "filesystem".to_string(),
            sync_mode: ResourceSyncMode::Delta,
            timestamp: Utc::now(),
        };
        let json = serde_json::to_value(&notification).unwrap();
        assert_eq!(json["subscription_id"], "sub-1");
        assert_eq!(json["peer_id"], "peer-1");
        assert_eq!(json["resource_type"], "filesystem");
        assert_eq!(json["sync_mode"], "delta");
    }

    // --- list_subscriptions tests ---

    #[test]
    fn list_subscriptions_returns_all_active() {
        let mut sync = DefaultBeliefSync::new();
        sync.subscribe_resource("peer-1", "filesystem", ResourceSyncMode::Snapshot)
            .unwrap();
        sync.subscribe_resource("peer-2", "database", ResourceSyncMode::Delta)
            .unwrap();

        let listed = sync.list_subscriptions();
        assert_eq!(listed.len(), 2);

        let peer_ids: Vec<&str> = listed.iter().map(|(_, r)| r.peer_id.as_str()).collect();
        assert!(peer_ids.contains(&"peer-1"));
        assert!(peer_ids.contains(&"peer-2"));
    }

    #[test]
    fn list_subscriptions_empty_when_none() {
        let sync = DefaultBeliefSync::new();
        assert!(sync.list_subscriptions().is_empty());
    }

    #[test]
    fn list_subscriptions_reflects_unsubscribe() {
        let mut sync = DefaultBeliefSync::new();
        let sub_id = sync
            .subscribe_resource("peer-1", "filesystem", ResourceSyncMode::Snapshot)
            .unwrap();
        assert_eq!(sync.list_subscriptions().len(), 1);

        sync.unsubscribe(&sub_id).unwrap();
        assert!(sync.list_subscriptions().is_empty());
    }

    // --- unsubscribe_by_resource tests ---

    #[test]
    fn unsubscribe_by_resource_removes_matching() {
        let mut sync = DefaultBeliefSync::new();
        sync.subscribe_resource("peer-1", "filesystem", ResourceSyncMode::Snapshot)
            .unwrap();
        sync.subscribe_resource("peer-1", "database", ResourceSyncMode::Delta)
            .unwrap();

        let removed = sync.unsubscribe_by_resource("peer-1", "filesystem");
        assert_eq!(removed, 1);
        assert_eq!(sync.subscriptions().len(), 1);

        // The remaining subscription should be the database one.
        let remaining = sync.subscriptions().values().next().unwrap();
        assert_eq!(remaining.resource_type, "database");
    }

    #[test]
    fn unsubscribe_by_resource_returns_zero_when_no_match() {
        let mut sync = DefaultBeliefSync::new();
        sync.subscribe_resource("peer-1", "filesystem", ResourceSyncMode::Snapshot)
            .unwrap();

        let removed = sync.unsubscribe_by_resource("peer-2", "filesystem");
        assert_eq!(removed, 0);
        assert_eq!(sync.subscriptions().len(), 1);
    }

    #[test]
    fn unsubscribe_by_resource_removes_multiple_matching() {
        let mut sync = DefaultBeliefSync::new();
        // Subscribe twice to the same peer+resource (different sync modes).
        sync.subscribe_resource("peer-1", "filesystem", ResourceSyncMode::Snapshot)
            .unwrap();
        sync.subscribe_resource("peer-1", "filesystem", ResourceSyncMode::Delta)
            .unwrap();

        let removed = sync.unsubscribe_by_resource("peer-1", "filesystem");
        assert_eq!(removed, 2);
        assert!(sync.subscriptions().is_empty());
    }
}
