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

impl BeliefState {
    pub fn fact_free_energy(fact: &Fact) -> f64 {
        let q = fact.confidence.clamp(1e-10, 1.0 - 1e-10);
        let p = fact
            .prior_confidence
            .unwrap_or(fact.confidence)
            .clamp(1e-10, 1.0 - 1e-10);
        let pe = fact.prediction_error.unwrap_or(0.0);
        let kl = q * (q / p).ln() + (1.0 - q) * ((1.0 - q) / (1.0 - p)).ln();
        kl + pe * pe * q
    }

    pub fn total_free_energy(&self) -> f64 {
        self.facts.iter().map(Self::fact_free_energy).sum()
    }
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
    #[serde(default)]
    pub prior_confidence: Option<f64>,
    #[serde(default)]
    pub prediction_error: Option<f64>,
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
    #[serde(default)]
    pub free_energy_delta: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fact(confidence: f64, prior: Option<f64>, pe: Option<f64>) -> Fact {
        Fact {
            fact_id: "test".into(),
            subject: "s".into(),
            predicate: "p".into(),
            value: serde_json::Value::Null,
            confidence,
            provenance: FactProvenance::Observed,
            timestamp: Utc::now(),
            ttl_ms: None,
            prior_confidence: prior,
            prediction_error: pe,
        }
    }

    #[test]
    fn test_fact_free_energy_zero_when_no_change() {
        let f = make_fact(0.8, None, None);
        let fe = BeliefState::fact_free_energy(&f);
        assert!(fe.abs() < 1e-9, "expected ~0, got {fe}");
    }

    #[test]
    fn test_fact_free_energy_high_surprise() {
        let f = make_fact(0.5, Some(0.9), Some(0.8));
        let fe = BeliefState::fact_free_energy(&f);
        assert!(fe > 0.1, "expected positive free energy, got {fe}");
    }

    #[test]
    fn test_fact_free_energy_low_surprise() {
        let f = make_fact(0.85, Some(0.8), Some(0.05));
        let fe = BeliefState::fact_free_energy(&f);
        assert!(fe < 0.05, "expected near-zero free energy, got {fe}");
    }

    #[test]
    fn test_total_free_energy_empty() {
        let bs = BeliefState {
            belief_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            resources: vec![],
            facts: vec![],
            uncertainties: vec![],
            provenance: vec![],
            active_bindings: vec![],
            world_hash: String::new(),
            updated_at: Utc::now(),
        };
        assert!((bs.total_free_energy()).abs() < 1e-12);
    }

    #[test]
    fn test_total_free_energy_sums_facts() {
        let f1 = make_fact(0.5, Some(0.9), Some(0.8));
        let f2 = make_fact(0.85, Some(0.8), Some(0.05));
        let expected = BeliefState::fact_free_energy(&f1) + BeliefState::fact_free_energy(&f2);

        let bs = BeliefState {
            belief_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            resources: vec![],
            facts: vec![f1, f2],
            uncertainties: vec![],
            provenance: vec![],
            active_bindings: vec![],
            world_hash: String::new(),
            updated_at: Utc::now(),
        };
        assert!((bs.total_free_energy() - expected).abs() < 1e-12);
    }

    #[test]
    fn test_backward_compat_deserialize() {
        let json = r#"{
            "fact_id": "f1",
            "subject": "s",
            "predicate": "p",
            "value": null,
            "confidence": 0.9,
            "provenance": "observed",
            "timestamp": "2025-01-01T00:00:00Z"
        }"#;
        let fact: Fact = serde_json::from_str(json).unwrap();
        assert!(fact.prior_confidence.is_none());
        assert!(fact.prediction_error.is_none());
        assert!(fact.ttl_ms.is_none());
    }

    #[test]
    fn test_belief_patch_backward_compat_deserialize() {
        let json = r#"{
            "added_resources": [],
            "updated_resources": [],
            "removed_resource_ids": [],
            "added_facts": [],
            "updated_facts": [],
            "removed_fact_ids": [],
            "binding_updates": []
        }"#;
        let patch: BeliefPatch = serde_json::from_str(json).unwrap();
        assert!(patch.free_energy_delta.is_none());
    }
}
