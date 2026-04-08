use std::collections::{HashMap, VecDeque};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::errors::{Result, SomaError};

// --- QueueItem ---

/// An item in the offline delivery queue.
/// Carries all fields required by the distributed spec: sequence, origin,
/// destination, timestamps, priority, payload, and replay eligibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueItem {
    pub sequence: u64,
    pub origin: String,
    pub destination: String,
    pub created_at: DateTime<Utc>,
    pub expiry_ms: u64,
    pub priority: u32,
    pub payload: serde_json::Value,
    pub replay_eligible: bool,
    /// Whether the item's replay policy is still valid.
    /// Items with policy_valid == false are skipped on dequeue.
    #[serde(default = "default_policy_valid")]
    pub policy_valid: bool,
}

fn default_policy_valid() -> bool {
    true
}

impl QueueItem {
    /// Check whether this item has expired given a reference time.
    pub fn is_expired_at(&self, now: DateTime<Utc>) -> bool {
        let age_ms = now
            .signed_duration_since(self.created_at)
            .num_milliseconds();
        if age_ms < 0 {
            return false;
        }
        age_ms as u64 >= self.expiry_ms
    }
}

// --- OfflineQueue trait ---

/// Offline delivery queue for distributed messages.
/// Items are queued per destination peer and ordered by sequence.
pub trait OfflineQueue: Send + Sync {
    /// Enqueue an item for later delivery. Validates destination is non-empty.
    fn enqueue(&mut self, item: QueueItem) -> Result<()>;

    /// Dequeue the next item for a given peer (FIFO within that peer's queue).
    fn dequeue(&mut self, peer_id: &str) -> Option<QueueItem>;

    /// Peek at the next item for a given peer without removing it.
    fn peek(&self, peer_id: &str) -> Option<&QueueItem>;

    /// Remove all items older than max_age_ms. Returns the count of removed items.
    fn expire_old(&mut self, max_age_ms: u64) -> usize;

    /// Return the number of items. If peer_id is Some, count only that peer's items.
    fn len(&self, peer_id: Option<&str>) -> usize;
}

// --- DefaultOfflineQueue ---

/// Default implementation backed by a HashMap of VecDeque per peer.
/// Supports duplicate detection via seen sequence numbers.
pub struct DefaultOfflineQueue {
    queues: HashMap<String, VecDeque<QueueItem>>,
    /// Track seen (origin, sequence) pairs for duplicate detection.
    seen_sequences: std::collections::HashSet<(String, u64)>,
}

impl DefaultOfflineQueue {
    pub fn new() -> Self {
        Self {
            queues: HashMap::new(),
            seen_sequences: std::collections::HashSet::new(),
        }
    }

    /// Check if the queue is completely empty.
    pub fn is_empty(&self) -> bool {
        self.queues.values().all(|q| q.is_empty())
    }
}

impl Default for DefaultOfflineQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl OfflineQueue for DefaultOfflineQueue {
    fn enqueue(&mut self, item: QueueItem) -> Result<()> {
        if item.destination.is_empty() {
            return Err(SomaError::Distributed {
                failure: crate::types::peer::DistributedFailure::PolicyViolation,
                details: "queue item destination must not be empty".to_string(),
            });
        }
        // Duplicate detection: reject items with already-seen (origin, sequence).
        let key = (item.origin.clone(), item.sequence);
        if !self.seen_sequences.insert(key) {
            return Err(SomaError::Distributed {
                failure: crate::types::peer::DistributedFailure::ReplayRejection,
                details: format!(
                    "duplicate queue item: origin={}, sequence={}",
                    item.origin, item.sequence
                ),
            });
        }
        self.queues
            .entry(item.destination.clone())
            .or_default()
            .push_back(item);
        Ok(())
    }

    fn dequeue(&mut self, peer_id: &str) -> Option<QueueItem> {
        let queue = self.queues.get_mut(peer_id)?;
        let now = Utc::now();
        // Remove expired and policy-invalid items before selecting.
        queue.retain(|item| !item.is_expired_at(now) && item.policy_valid);

        if queue.is_empty() {
            return None;
        }

        // Priority-aware dequeue: find the item with the highest priority value.
        // Among items with equal priority, the earliest (front-most) one wins
        // to maintain FIFO order within the same priority level.
        let mut best_idx = 0;
        let mut best_priority = queue[0].priority;
        for (i, item) in queue.iter().enumerate().skip(1) {
            if item.priority > best_priority {
                best_priority = item.priority;
                best_idx = i;
            }
        }
        queue.remove(best_idx)
    }

    fn peek(&self, peer_id: &str) -> Option<&QueueItem> {
        self.queues.get(peer_id)?.front()
    }

    fn expire_old(&mut self, max_age_ms: u64) -> usize {
        let now = Utc::now();
        let mut removed = 0;
        for queue in self.queues.values_mut() {
            let before = queue.len();
            queue.retain(|item| {
                let age_ms = now
                    .signed_duration_since(item.created_at)
                    .num_milliseconds();
                if age_ms < 0 {
                    return true; // future item, keep it
                }
                (age_ms as u64) < max_age_ms
            });
            removed += before - queue.len();
        }
        // Clean up empty queues.
        self.queues.retain(|_, q| !q.is_empty());
        removed
    }

    fn len(&self, peer_id: Option<&str>) -> usize {
        match peer_id {
            Some(id) => self.queues.get(id).map_or(0, |q| q.len()),
            None => self.queues.values().map(|q| q.len()).sum(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn make_item(seq: u64, dest: &str, expiry_ms: u64) -> QueueItem {
        QueueItem {
            sequence: seq,
            origin: "local".to_string(),
            destination: dest.to_string(),
            created_at: Utc::now(),
            expiry_ms,
            priority: 1,
            payload: serde_json::json!({"seq": seq}),
            replay_eligible: true,
            policy_valid: true,
        }
    }

    #[test]
    fn enqueue_and_dequeue() {
        let mut q = DefaultOfflineQueue::new();
        q.enqueue(make_item(1, "peer-1", 60_000)).unwrap();
        let item = q.dequeue("peer-1").unwrap();
        assert_eq!(item.sequence, 1);
        assert_eq!(item.destination, "peer-1");
    }

    #[test]
    fn dequeue_empty_returns_none() {
        let mut q = DefaultOfflineQueue::new();
        assert!(q.dequeue("peer-1").is_none());
    }

    #[test]
    fn dequeue_fifo_order() {
        let mut q = DefaultOfflineQueue::new();
        q.enqueue(make_item(1, "peer-1", 60_000)).unwrap();
        q.enqueue(make_item(2, "peer-1", 60_000)).unwrap();
        q.enqueue(make_item(3, "peer-1", 60_000)).unwrap();
        assert_eq!(q.dequeue("peer-1").unwrap().sequence, 1);
        assert_eq!(q.dequeue("peer-1").unwrap().sequence, 2);
        assert_eq!(q.dequeue("peer-1").unwrap().sequence, 3);
        assert!(q.dequeue("peer-1").is_none());
    }

    #[test]
    fn dequeue_per_peer_isolation() {
        let mut q = DefaultOfflineQueue::new();
        q.enqueue(make_item(1, "peer-1", 60_000)).unwrap();
        q.enqueue(make_item(2, "peer-2", 60_000)).unwrap();
        assert!(q.dequeue("peer-3").is_none());
        assert_eq!(q.dequeue("peer-1").unwrap().sequence, 1);
        assert_eq!(q.dequeue("peer-2").unwrap().sequence, 2);
    }

    #[test]
    fn peek_returns_front_without_removing() {
        let mut q = DefaultOfflineQueue::new();
        q.enqueue(make_item(1, "peer-1", 60_000)).unwrap();
        q.enqueue(make_item(2, "peer-1", 60_000)).unwrap();
        let peeked = q.peek("peer-1").unwrap();
        assert_eq!(peeked.sequence, 1);
        // Still there after peek.
        assert_eq!(q.len(Some("peer-1")), 2);
    }

    #[test]
    fn peek_empty_returns_none() {
        let q = DefaultOfflineQueue::new();
        assert!(q.peek("peer-1").is_none());
    }

    #[test]
    fn len_total_and_per_peer() {
        let mut q = DefaultOfflineQueue::new();
        q.enqueue(make_item(1, "peer-1", 60_000)).unwrap();
        q.enqueue(make_item(2, "peer-1", 60_000)).unwrap();
        q.enqueue(make_item(3, "peer-2", 60_000)).unwrap();
        assert_eq!(q.len(None), 3);
        assert_eq!(q.len(Some("peer-1")), 2);
        assert_eq!(q.len(Some("peer-2")), 1);
        assert_eq!(q.len(Some("peer-3")), 0);
    }

    #[test]
    fn expire_old_removes_expired_items() {
        let mut q = DefaultOfflineQueue::new();
        // Create an item that's already old.
        let mut old_item = make_item(1, "peer-1", 1_000);
        old_item.created_at = Utc::now() - Duration::seconds(10);
        q.enqueue(old_item).unwrap();
        // Create a fresh item.
        q.enqueue(make_item(2, "peer-1", 60_000)).unwrap();
        let removed = q.expire_old(5_000); // expire items older than 5s
        assert_eq!(removed, 1);
        assert_eq!(q.len(None), 1);
        assert_eq!(q.peek("peer-1").unwrap().sequence, 2);
    }

    #[test]
    fn expire_old_returns_zero_when_nothing_expired() {
        let mut q = DefaultOfflineQueue::new();
        q.enqueue(make_item(1, "peer-1", 60_000)).unwrap();
        let removed = q.expire_old(60_000);
        assert_eq!(removed, 0);
        assert_eq!(q.len(None), 1);
    }

    #[test]
    fn expire_old_cleans_up_empty_queues() {
        let mut q = DefaultOfflineQueue::new();
        let mut old_item = make_item(1, "peer-1", 1_000);
        old_item.created_at = Utc::now() - Duration::seconds(10);
        q.enqueue(old_item).unwrap();
        q.expire_old(5_000);
        assert!(q.is_empty());
    }

    #[test]
    fn enqueue_empty_destination_fails() {
        let mut q = DefaultOfflineQueue::new();
        let result = q.enqueue(make_item(1, "", 60_000));
        assert!(result.is_err());
    }

    #[test]
    fn is_expired_at_checks_correctly() {
        let item = QueueItem {
            sequence: 1,
            origin: "local".to_string(),
            destination: "peer-1".to_string(),
            created_at: Utc::now() - Duration::seconds(10),
            expiry_ms: 5_000,
            priority: 1,
            payload: serde_json::json!({}),
            replay_eligible: true,
            policy_valid: true,
        };
        assert!(item.is_expired_at(Utc::now()));

        let fresh = QueueItem {
            sequence: 2,
            origin: "local".to_string(),
            destination: "peer-1".to_string(),
            created_at: Utc::now(),
            expiry_ms: 60_000,
            priority: 1,
            payload: serde_json::json!({}),
            replay_eligible: true,
            policy_valid: true,
        };
        assert!(!fresh.is_expired_at(Utc::now()));
    }

    #[test]
    fn queue_item_serialization() {
        let item = make_item(42, "peer-1", 30_000);
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["sequence"], 42);
        assert_eq!(json["destination"], "peer-1");
        assert_eq!(json["expiry_ms"], 30_000);
        assert_eq!(json["replay_eligible"], true);
    }

    #[test]
    fn multiple_enqueue_dequeue_cycles() {
        let mut q = DefaultOfflineQueue::new();
        for i in 0..10 {
            q.enqueue(make_item(i, "peer-1", 60_000)).unwrap();
        }
        for i in 0..10 {
            assert_eq!(q.dequeue("peer-1").unwrap().sequence, i);
        }
        assert!(q.dequeue("peer-1").is_none());
        assert_eq!(q.len(None), 0);
    }

    #[test]
    fn enqueue_duplicate_sequence_rejected() {
        let mut q = DefaultOfflineQueue::new();
        q.enqueue(make_item(1, "peer-1", 60_000)).unwrap();
        // Same origin ("local") and sequence (1) — must be rejected.
        let result = q.enqueue(make_item(1, "peer-1", 60_000));
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, crate::types::peer::DistributedFailure::ReplayRejection);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn dequeue_skips_expired_items() {
        let mut q = DefaultOfflineQueue::new();
        // Create an already-expired item.
        let mut expired = make_item(1, "peer-1", 1);
        expired.created_at = Utc::now() - Duration::seconds(10);
        q.enqueue(expired).unwrap();
        // Create a fresh item.
        q.enqueue(make_item(2, "peer-1", 60_000)).unwrap();
        // Dequeue should skip the expired item and return the fresh one.
        let item = q.dequeue("peer-1").unwrap();
        assert_eq!(item.sequence, 2);
    }

    #[test]
    fn dequeue_returns_none_when_all_expired() {
        let mut q = DefaultOfflineQueue::new();
        let mut expired = make_item(1, "peer-1", 1);
        expired.created_at = Utc::now() - Duration::seconds(10);
        q.enqueue(expired).unwrap();
        // All items expired — dequeue returns None.
        assert!(q.dequeue("peer-1").is_none());
    }

    #[test]
    fn dequeue_skips_policy_invalid_items() {
        let mut q = DefaultOfflineQueue::new();
        let mut invalid = make_item(1, "peer-1", 60_000);
        invalid.policy_valid = false;
        q.enqueue(invalid).unwrap();
        q.enqueue(make_item(2, "peer-1", 60_000)).unwrap();
        // Dequeue should skip the policy-invalid item and return the valid one.
        let item = q.dequeue("peer-1").unwrap();
        assert_eq!(item.sequence, 2);
    }

    #[test]
    fn dequeue_returns_none_when_all_policy_invalid() {
        let mut q = DefaultOfflineQueue::new();
        let mut invalid = make_item(1, "peer-1", 60_000);
        invalid.policy_valid = false;
        q.enqueue(invalid).unwrap();
        assert!(q.dequeue("peer-1").is_none());
    }

    fn make_priority_item(seq: u64, dest: &str, priority: u32) -> QueueItem {
        QueueItem {
            sequence: seq,
            origin: "local".to_string(),
            destination: dest.to_string(),
            created_at: Utc::now(),
            expiry_ms: 60_000,
            priority,
            payload: serde_json::json!({"seq": seq}),
            replay_eligible: true,
            policy_valid: true,
        }
    }

    #[test]
    fn dequeue_priority_ordering() {
        let mut q = DefaultOfflineQueue::new();
        q.enqueue(make_priority_item(1, "peer-1", 1)).unwrap();
        q.enqueue(make_priority_item(2, "peer-1", 10)).unwrap();
        q.enqueue(make_priority_item(3, "peer-1", 5)).unwrap();
        // Highest priority (10) should come first.
        assert_eq!(q.dequeue("peer-1").unwrap().sequence, 2);
        // Next highest (5).
        assert_eq!(q.dequeue("peer-1").unwrap().sequence, 3);
        // Lowest priority (1) last.
        assert_eq!(q.dequeue("peer-1").unwrap().sequence, 1);
        assert!(q.dequeue("peer-1").is_none());
    }

    #[test]
    fn dequeue_equal_priority_maintains_fifo() {
        let mut q = DefaultOfflineQueue::new();
        q.enqueue(make_priority_item(1, "peer-1", 5)).unwrap();
        q.enqueue(make_priority_item(2, "peer-1", 5)).unwrap();
        q.enqueue(make_priority_item(3, "peer-1", 5)).unwrap();
        // Equal priority: FIFO order preserved.
        assert_eq!(q.dequeue("peer-1").unwrap().sequence, 1);
        assert_eq!(q.dequeue("peer-1").unwrap().sequence, 2);
        assert_eq!(q.dequeue("peer-1").unwrap().sequence, 3);
    }

    #[test]
    fn dequeue_mixed_priority_and_fifo() {
        let mut q = DefaultOfflineQueue::new();
        q.enqueue(make_priority_item(1, "peer-1", 3)).unwrap();
        q.enqueue(make_priority_item(2, "peer-1", 3)).unwrap();
        q.enqueue(make_priority_item(3, "peer-1", 7)).unwrap();
        q.enqueue(make_priority_item(4, "peer-1", 7)).unwrap();
        q.enqueue(make_priority_item(5, "peer-1", 1)).unwrap();
        // Priority 7 first (FIFO within: seq 3, then seq 4).
        assert_eq!(q.dequeue("peer-1").unwrap().sequence, 3);
        assert_eq!(q.dequeue("peer-1").unwrap().sequence, 4);
        // Then priority 3 (FIFO within: seq 1, then seq 2).
        assert_eq!(q.dequeue("peer-1").unwrap().sequence, 1);
        assert_eq!(q.dequeue("peer-1").unwrap().sequence, 2);
        // Then priority 1.
        assert_eq!(q.dequeue("peer-1").unwrap().sequence, 5);
    }

    #[test]
    fn policy_valid_defaults_true_on_deserialize() {
        // Simulate JSON without policy_valid field — should default to true.
        let json = serde_json::json!({
            "sequence": 1,
            "origin": "local",
            "destination": "peer-1",
            "created_at": Utc::now(),
            "expiry_ms": 60000,
            "priority": 1,
            "payload": {},
            "replay_eligible": true
        });
        let item: QueueItem = serde_json::from_value(json).unwrap();
        assert!(item.policy_valid);
    }
}
