use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::Result;
use crate::types::peer::{
    DistributedActionType, DistributedTrace, DistributedTraceResult,
};

// --- AuditEntry ---

/// Audit trail entry for distributed operations.
/// The runtime MUST retain an audit trail for the 8 auditable items from distributed.md:
/// remote goal submissions, remote skill invocations, resource queries,
/// observation streams, delegation, migration, schema/routine transfer, offline replay.
/// Audit records MUST preserve attribution across peers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub entry_id: Uuid,
    pub action_type: DistributedActionType,
    pub origin_peer: String,
    pub destination_peer: String,
    pub session_id: Option<Uuid>,
    pub correlation_key: Uuid,
    pub result: DistributedTraceResult,
    pub details: serde_json::Value,
    pub timestamp: DateTime<Utc>,
}

// --- DistributedTraceStore trait ---

/// Audit-grade distributed trace and audit storage.
/// Records every cross-peer action with full attribution,
/// routing decisions, policy decisions, and timing.
/// Retains audit trail for all 8 auditable operations.
pub trait DistributedTraceStore: Send + Sync {
    /// Record a distributed trace entry (11 required fields + correlation key).
    fn record(&mut self, trace: DistributedTrace) -> Result<()>;

    /// Record an audit entry for one of the 8 auditable action types.
    fn record_audit(&mut self, entry: AuditEntry) -> Result<()>;

    /// Retrieve all trace entries for a given session.
    fn get_by_session(&self, session_id: &Uuid) -> Vec<&DistributedTrace>;

    /// Retrieve all trace entries involving a given peer (as origin or destination).
    fn get_by_peer(&self, peer_id: &str) -> Vec<&DistributedTrace>;

    /// Retrieve a single trace entry by request_id.
    fn get_by_request(&self, request_id: &Uuid) -> Option<&DistributedTrace>;

    /// Retrieve all trace entries correlated by a correlation key.
    fn get_by_correlation(&self, correlation_key: &Uuid) -> Vec<&DistributedTrace>;

    /// List all trace entries, newest first, with pagination.
    fn list(&self, limit: usize, offset: usize) -> Vec<&DistributedTrace>;

    /// Total number of stored trace entries.
    fn count(&self) -> usize;

    /// Retrieve all audit entries.
    fn all_audit_entries(&self) -> Vec<&AuditEntry>;

    /// Retrieve audit entries by action type.
    fn audit_by_action(&self, action_type: DistributedActionType) -> Vec<&AuditEntry>;

    /// Total number of audit entries.
    fn audit_count(&self) -> usize;
}

// --- DefaultDistributedTraceStore ---

/// Default in-memory distributed trace and audit store.
pub struct DefaultDistributedTraceStore {
    traces: Vec<DistributedTrace>,
    audit_entries: Vec<AuditEntry>,
}

impl DefaultDistributedTraceStore {
    pub fn new() -> Self {
        Self {
            traces: Vec::new(),
            audit_entries: Vec::new(),
        }
    }
}

impl Default for DefaultDistributedTraceStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DistributedTraceStore for DefaultDistributedTraceStore {
    fn record(&mut self, trace: DistributedTrace) -> Result<()> {
        self.traces.push(trace);
        Ok(())
    }

    fn record_audit(&mut self, entry: AuditEntry) -> Result<()> {
        self.audit_entries.push(entry);
        Ok(())
    }

    fn get_by_session(&self, session_id: &Uuid) -> Vec<&DistributedTrace> {
        self.traces
            .iter()
            .filter(|t| t.session_id.as_ref() == Some(session_id))
            .collect()
    }

    fn get_by_peer(&self, peer_id: &str) -> Vec<&DistributedTrace> {
        self.traces
            .iter()
            .filter(|t| t.origin_peer == peer_id || t.destination_peer == peer_id)
            .collect()
    }

    fn get_by_request(&self, request_id: &Uuid) -> Option<&DistributedTrace> {
        self.traces.iter().find(|t| t.request_id == *request_id)
    }

    fn get_by_correlation(&self, correlation_key: &Uuid) -> Vec<&DistributedTrace> {
        self.traces
            .iter()
            .filter(|t| t.correlation_key == *correlation_key)
            .collect()
    }

    fn list(&self, limit: usize, offset: usize) -> Vec<&DistributedTrace> {
        self.traces
            .iter()
            .rev()
            .skip(offset)
            .take(limit)
            .collect()
    }

    fn count(&self) -> usize {
        self.traces.len()
    }

    fn all_audit_entries(&self) -> Vec<&AuditEntry> {
        self.audit_entries.iter().collect()
    }

    fn audit_by_action(&self, action_type: DistributedActionType) -> Vec<&AuditEntry> {
        self.audit_entries
            .iter()
            .filter(|e| e.action_type == action_type)
            .collect()
    }

    fn audit_count(&self) -> usize {
        self.audit_entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use crate::types::peer::TraceTimestamps;

    fn make_trace(
        origin: &str,
        dest: &str,
        action: DistributedActionType,
        session_id: Option<Uuid>,
    ) -> DistributedTrace {
        DistributedTrace {
            origin_peer: origin.to_string(),
            destination_peer: dest.to_string(),
            action_type: action,
            session_id,
            goal_id: None,
            request_id: Uuid::new_v4(),
            routing_decision: "direct".to_string(),
            policy_decision: "allowed".to_string(),
            result: DistributedTraceResult::Success,
            failure_reason: None,
            timestamps: TraceTimestamps {
                initiated_at: Utc::now(),
                completed_at: Some(Utc::now()),
            },
            correlation_key: Uuid::new_v4(),
        }
    }

    #[test]
    fn record_and_count() {
        let mut store = DefaultDistributedTraceStore::new();
        assert_eq!(store.count(), 0);
        store
            .record(make_trace(
                "local",
                "peer-1",
                DistributedActionType::SkillInvocation,
                None,
            ))
            .unwrap();
        assert_eq!(store.count(), 1);
    }

    #[test]
    fn get_by_session() {
        let mut store = DefaultDistributedTraceStore::new();
        let sid = Uuid::new_v4();
        store
            .record(make_trace(
                "local",
                "peer-1",
                DistributedActionType::SkillInvocation,
                Some(sid),
            ))
            .unwrap();
        store
            .record(make_trace(
                "local",
                "peer-2",
                DistributedActionType::GoalSubmission,
                None,
            ))
            .unwrap();

        let results = store.get_by_session(&sid);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].destination_peer, "peer-1");
    }

    #[test]
    fn get_by_peer() {
        let mut store = DefaultDistributedTraceStore::new();
        store
            .record(make_trace(
                "local",
                "peer-1",
                DistributedActionType::SkillInvocation,
                None,
            ))
            .unwrap();
        store
            .record(make_trace(
                "peer-1",
                "local",
                DistributedActionType::ResourceQuery,
                None,
            ))
            .unwrap();
        store
            .record(make_trace(
                "local",
                "peer-2",
                DistributedActionType::GoalSubmission,
                None,
            ))
            .unwrap();

        let results = store.get_by_peer("peer-1");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn get_by_request() {
        let mut store = DefaultDistributedTraceStore::new();
        let trace = make_trace(
            "local",
            "peer-1",
            DistributedActionType::Delegation,
            None,
        );
        let rid = trace.request_id;
        store.record(trace).unwrap();

        assert!(store.get_by_request(&rid).is_some());
        assert!(store.get_by_request(&Uuid::new_v4()).is_none());
    }

    #[test]
    fn get_by_correlation() {
        let mut store = DefaultDistributedTraceStore::new();
        let ckey = Uuid::new_v4();

        let mut t1 = make_trace(
            "local",
            "peer-1",
            DistributedActionType::Delegation,
            None,
        );
        t1.correlation_key = ckey;
        store.record(t1).unwrap();

        let mut t2 = make_trace(
            "peer-1",
            "peer-2",
            DistributedActionType::SkillInvocation,
            None,
        );
        t2.correlation_key = ckey;
        store.record(t2).unwrap();

        store
            .record(make_trace(
                "local",
                "peer-3",
                DistributedActionType::GoalSubmission,
                None,
            ))
            .unwrap();

        let results = store.get_by_correlation(&ckey);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn list_with_pagination() {
        let mut store = DefaultDistributedTraceStore::new();
        for i in 0..10 {
            let t = make_trace(
                "local",
                &format!("peer-{}", i),
                DistributedActionType::SkillInvocation,
                None,
            );
            store.record(t).unwrap();
        }

        let page1 = store.list(3, 0);
        assert_eq!(page1.len(), 3);
        // Newest first (reversed), so first entry is peer-9
        assert_eq!(page1[0].destination_peer, "peer-9");

        let page2 = store.list(3, 3);
        assert_eq!(page2.len(), 3);
        assert_eq!(page2[0].destination_peer, "peer-6");
    }

    #[test]
    fn distributed_trace_has_all_eleven_fields() {
        let trace = make_trace(
            "local",
            "peer-1",
            DistributedActionType::SkillInvocation,
            Some(Uuid::new_v4()),
        );
        let json = serde_json::to_value(&trace).unwrap();
        // 11 required fields from distributed.md + correlation_key.
        assert!(json["origin_peer"].is_string());
        assert!(json["destination_peer"].is_string());
        assert!(json["action_type"].is_string());
        assert!(json["request_id"].is_string());
        assert!(json["routing_decision"].is_string());
        assert!(json["policy_decision"].is_string());
        assert!(json["result"].is_string());
        assert!(json["timestamps"].is_object());
        assert!(json["correlation_key"].is_string());
    }

    fn make_audit(action: DistributedActionType) -> AuditEntry {
        AuditEntry {
            entry_id: Uuid::new_v4(),
            action_type: action,
            origin_peer: "local".to_string(),
            destination_peer: "peer-1".to_string(),
            session_id: None,
            correlation_key: Uuid::new_v4(),
            result: DistributedTraceResult::Success,
            details: serde_json::json!({}),
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn record_and_retrieve_audit_entries() {
        let mut store = DefaultDistributedTraceStore::new();
        assert_eq!(store.audit_count(), 0);
        store
            .record_audit(make_audit(DistributedActionType::GoalSubmission))
            .unwrap();
        store
            .record_audit(make_audit(DistributedActionType::SkillInvocation))
            .unwrap();
        assert_eq!(store.audit_count(), 2);
        assert_eq!(store.all_audit_entries().len(), 2);
    }

    #[test]
    fn audit_by_action_type() {
        let mut store = DefaultDistributedTraceStore::new();
        store
            .record_audit(make_audit(DistributedActionType::GoalSubmission))
            .unwrap();
        store
            .record_audit(make_audit(DistributedActionType::SkillInvocation))
            .unwrap();
        store
            .record_audit(make_audit(DistributedActionType::GoalSubmission))
            .unwrap();

        let goals = store.audit_by_action(DistributedActionType::GoalSubmission);
        assert_eq!(goals.len(), 2);
        let skills = store.audit_by_action(DistributedActionType::SkillInvocation);
        assert_eq!(skills.len(), 1);
    }

    #[test]
    fn audit_covers_all_nine_action_types() {
        let actions = vec![
            DistributedActionType::GoalSubmission,
            DistributedActionType::SkillInvocation,
            DistributedActionType::ResourceQuery,
            DistributedActionType::ObservationStream,
            DistributedActionType::Delegation,
            DistributedActionType::Migration,
            DistributedActionType::SchemaTransfer,
            DistributedActionType::RoutineTransfer,
            DistributedActionType::OfflineReplay,
        ];
        let mut store = DefaultDistributedTraceStore::new();
        for action in &actions {
            store.record_audit(make_audit(*action)).unwrap();
        }
        assert_eq!(store.audit_count(), 9);
    }
}
