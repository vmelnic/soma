use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::common::FactProvenance;
use super::resource::ResourceRef;

/// BeliefState — the runtime's current model of the world.
/// Queryable, serializable, checkpointable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeliefState {
    pub belief_id: Uuid,
    pub session_id: Uuid,
    pub resources: Vec<ResourceEntry>,
    pub facts: Vec<Fact>,
    pub uncertainties: Vec<Uncertainty>,
    pub provenance: Vec<ProvenanceRecord>,
    pub active_bindings: Vec<Binding>,
    pub world_hash: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceEntry {
    pub resource_ref: ResourceRef,
    pub data: serde_json::Value,
    pub confidence: f64,
    pub provenance: FactProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    pub fact_id: String,
    pub subject: String,
    pub predicate: String,
    pub value: serde_json::Value,
    pub confidence: f64,
    pub provenance: FactProvenance,
    pub timestamp: DateTime<Utc>,
    /// Optional time-to-live in milliseconds from `timestamp`. When set, the
    /// fact is evicted from world-state snapshots and reactive matching once
    /// `now - timestamp > ttl_ms`. `None` means the fact persists until
    /// explicitly removed.
    #[serde(default)]
    pub ttl_ms: Option<u64>,
}

impl Fact {
    /// True when the fact's TTL has elapsed relative to `now`. Facts with no
    /// TTL never expire.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        match self.ttl_ms {
            None => false,
            Some(ttl) => {
                let elapsed = now
                    .signed_duration_since(self.timestamp)
                    .num_milliseconds();
                elapsed >= 0 && (elapsed as u64) > ttl
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Uncertainty {
    pub subject: String,
    pub description: String,
    pub magnitude: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceRecord {
    pub source: String,
    pub provenance_type: FactProvenance,
    pub timestamp: DateTime<Utc>,
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Binding {
    pub name: String,
    pub value: serde_json::Value,
    pub source: String,
    pub confidence: f64,
}

/// Patch applied to belief after an observation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeliefPatch {
    pub added_resources: Vec<ResourceEntry>,
    pub updated_resources: Vec<ResourceEntry>,
    pub removed_resource_ids: Vec<String>,
    pub added_facts: Vec<Fact>,
    pub updated_facts: Vec<Fact>,
    pub removed_fact_ids: Vec<String>,
    pub binding_updates: Vec<Binding>,
}
