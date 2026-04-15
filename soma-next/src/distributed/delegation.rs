use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::{Result, SomaError};
use crate::types::common::TrustLevel;
use crate::types::peer::{DelegationUnit, DistributedFailure, SessionMigrationData};

use super::auth::require_trust;
use super::peer::PeerRegistry;
use super::remote::RemoteExecutor;

// --- DelegationContext ---

/// Context carried with every delegation request.
/// Preserves all 5 delegation rules from distributed.md:
/// trace continuity, session identity, policy boundaries,
/// budget accounting, and attribution of actions/observations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationContext {
    pub session_id: Uuid,
    pub budget_remaining: f64,
    pub trust_required: TrustLevel,
    pub trace_required: bool,
    /// Policy boundaries carried with delegation.
    pub policy_context: serde_json::Value,
    /// Trace cursor for continuity.
    pub trace_cursor: u64,
    /// Attribution — who initiated this delegation.
    pub attribution: String,
    /// What unit is being delegated.
    pub delegation_unit: DelegationUnit,
}

// --- DelegationStatus ---

/// Status of a delegation handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationStatus {
    Pending,
    Accepted,
    Running,
    Completed,
    Failed,
    Refused,
}

// --- DelegationHandle ---

/// Handle returned when a delegation is created.
/// Tracks what was delegated, to whom, under what policy, and with what budget.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationHandle {
    pub delegation_id: Uuid,
    pub peer_id: String,
    pub delegation_unit: DelegationUnit,
    pub status: DelegationStatus,
    pub budget_allocated: f64,
}

// --- MigrationOutcome ---

/// Whether a migration succeeded or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationOutcome {
    Success,
    Failure,
}

// --- MigrationResult ---

/// Result of a session migration attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationResult {
    pub outcome: MigrationOutcome,
    pub reason: Option<String>,
    pub new_session_id: Option<Uuid>,
}

// --- DelegationManager trait ---

/// Manages all 5 delegation units from distributed.md:
/// skill, subgoal, session, resource operation, schema/routine lookup.
/// Preserves trace continuity, budget accounting, and policy boundaries.
pub trait DelegationManager: Send + Sync {
    /// Delegate a single skill invocation to a remote peer.
    fn delegate_skill(
        &self,
        peer_id: &str,
        skill_id: &str,
        input: serde_json::Value,
        context: DelegationContext,
    ) -> Result<DelegationHandle>;

    /// Delegate a subgoal to a remote peer while retaining session ownership.
    fn delegate_subgoal(
        &self,
        peer_id: &str,
        subgoal: serde_json::Value,
        context: DelegationContext,
    ) -> Result<DelegationHandle>;

    /// Delegate a resource read or write operation to a remote peer.
    fn delegate_resource_op(
        &self,
        peer_id: &str,
        resource_type: &str,
        resource_id: &str,
        operation: serde_json::Value,
        context: DelegationContext,
    ) -> Result<DelegationHandle>;

    /// Delegate a schema or routine lookup task to a remote peer.
    fn delegate_schema_routine_lookup(
        &self,
        peer_id: &str,
        query: serde_json::Value,
        context: DelegationContext,
    ) -> Result<DelegationHandle>;

    /// Migrate ownership of an active session to a remote peer.
    /// Migration MUST include all 8 required fields from distributed.md.
    /// Migration MUST fail closed if any required data cannot be transferred.
    fn migrate_session(
        &self,
        peer_id: &str,
        session_data: &SessionMigrationData,
    ) -> Result<MigrationResult>;

    /// Mirror an active session to a remote peer for redundancy.
    /// Mirroring MUST NOT transfer authority unless migration is explicitly accepted.
    fn mirror_session(&self, peer_id: &str, session_data: &SessionMigrationData) -> Result<()>;

    /// Decide whether to accept an incoming delegation from a remote peer.
    /// Checks that the peer's trust is sufficient for the requested action
    /// and that the action falls within our capability set.
    /// Returns Ok(true) to accept, Ok(false) to reject.
    fn accept_delegation(
        &self,
        from_peer: &str,
        context: &DelegationContext,
    ) -> Result<bool>;
}

// --- DefaultDelegationManager ---

/// Default implementation with basic acceptance logic.
/// Holds a set of capabilities (skill IDs) this node can handle and
/// a minimum trust level required from peers requesting delegation.
/// When constructed with `with_remote()`, outbound delegation operations
/// are forwarded to the configured `RemoteExecutor`. When no executor is
/// configured, outbound operations return structured errors explaining that
/// no remote executor is available.
pub struct DefaultDelegationManager {
    /// Skill IDs this node can accept delegations for.
    pub capabilities: HashSet<String>,
    /// Minimum trust level required from the delegating peer.
    pub min_accept_trust: TrustLevel,
    /// Optional remote executor for performing actual remote operations.
    remote_executor: Option<Box<dyn RemoteExecutor>>,
}

impl DefaultDelegationManager {
    pub fn new() -> Self {
        Self {
            capabilities: HashSet::new(),
            min_accept_trust: TrustLevel::Verified,
            remote_executor: None,
        }
    }

    /// Create a delegation manager with specific capabilities and trust threshold.
    pub fn with_capabilities(
        capabilities: HashSet<String>,
        min_accept_trust: TrustLevel,
    ) -> Self {
        Self {
            capabilities,
            min_accept_trust,
            remote_executor: None,
        }
    }

    /// Create a delegation manager with a remote executor for performing
    /// actual remote operations (e.g., `TcpRemoteExecutor`).
    pub fn with_remote(executor: Box<dyn RemoteExecutor>) -> Self {
        Self {
            capabilities: HashSet::new(),
            min_accept_trust: TrustLevel::Verified,
            remote_executor: Some(executor),
        }
    }

    /// Build a `DelegationHandle` for a successful delegation dispatch.
    fn make_handle(
        peer_id: &str,
        unit: DelegationUnit,
        budget: f64,
    ) -> DelegationHandle {
        DelegationHandle {
            delegation_id: Uuid::new_v4(),
            peer_id: peer_id.to_string(),
            delegation_unit: unit,
            status: DelegationStatus::Accepted,
            budget_allocated: budget,
        }
    }
}

impl Default for DefaultDelegationManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DelegationManager for DefaultDelegationManager {
    fn delegate_skill(
        &self,
        peer_id: &str,
        skill_id: &str,
        input: serde_json::Value,
        context: DelegationContext,
    ) -> Result<DelegationHandle> {
        match &self.remote_executor {
            Some(executor) => {
                executor.invoke_skill(peer_id, skill_id, input)?;
                Ok(Self::make_handle(peer_id, DelegationUnit::Skill, context.budget_remaining))
            }
            None => Err(SomaError::Distributed {
                failure: DistributedFailure::DelegationRefusal,
                details: format!(
                    "cannot delegate skill {} to peer {}: no remote executor configured",
                    skill_id, peer_id
                ),
            }),
        }
    }

    fn delegate_subgoal(
        &self,
        peer_id: &str,
        subgoal: serde_json::Value,
        context: DelegationContext,
    ) -> Result<DelegationHandle> {
        match &self.remote_executor {
            Some(executor) => {
                let request = crate::types::peer::RemoteGoalRequest {
                    goal: subgoal,
                    constraints: vec![],
                    budgets: crate::types::peer::RemoteBudget {
                        risk_limit: 0.5,
                        latency_limit_ms: 30_000,
                        resource_limit: context.budget_remaining,
                        step_limit: 100,
                    },
                    trust_required: context.trust_required,
                    request_result: true,
                    request_trace: context.trace_required,
                };
                executor.submit_goal(peer_id, &request)?;
                Ok(Self::make_handle(peer_id, DelegationUnit::Subgoal, context.budget_remaining))
            }
            None => Err(SomaError::Distributed {
                failure: DistributedFailure::DelegationRefusal,
                details: format!(
                    "cannot delegate subgoal to peer {}: no remote executor configured",
                    peer_id
                ),
            }),
        }
    }

    fn delegate_resource_op(
        &self,
        peer_id: &str,
        resource_type: &str,
        resource_id: &str,
        _operation: serde_json::Value,
        context: DelegationContext,
    ) -> Result<DelegationHandle> {
        match &self.remote_executor {
            Some(executor) => {
                executor.query_resource(peer_id, resource_type, resource_id)?;
                Ok(Self::make_handle(peer_id, DelegationUnit::ResourceOperation, context.budget_remaining))
            }
            None => Err(SomaError::Distributed {
                failure: DistributedFailure::DelegationRefusal,
                details: format!(
                    "cannot delegate resource op {}/{} to peer {}: no remote executor configured",
                    resource_type, resource_id, peer_id
                ),
            }),
        }
    }

    fn delegate_schema_routine_lookup(
        &self,
        peer_id: &str,
        query: serde_json::Value,
        context: DelegationContext,
    ) -> Result<DelegationHandle> {
        match &self.remote_executor {
            Some(executor) => {
                // Use transfer_schema for schema lookups, transfer_routine for routine lookups.
                // We determine which one based on whether the query contains a "routine_id" field.
                if query.get("routine_id").is_some() {
                    let routine = crate::types::peer::RoutineTransfer {
                        routine_id: query["routine_id"].as_str().unwrap_or("unknown").to_string(),
                        match_conditions: vec![],
                        compiled_skill_path: vec![],
                        compiled_steps: vec![],
                        guard_conditions: vec![],
                        expected_cost: 0.0,
                        expected_effect: vec![],
                        confidence: 0.0,
                        autonomous: false,
                        priority: 0,
                        exclusive: false,
                        policy_scope: None,
                    };
                    executor.transfer_routine(peer_id, &routine)?;
                } else {
                    let schema = crate::types::peer::SchemaTransfer {
                        schema_id: query["schema_id"].as_str().unwrap_or("unknown").to_string(),
                        version: query["version"].as_str().unwrap_or("1.0").to_string(),
                        trigger_conditions: vec![],
                        subgoal_structure: vec![],
                        candidate_skill_ordering: vec![],
                        stop_conditions: vec![],
                        confidence: 0.0,
                    };
                    executor.transfer_schema(peer_id, &schema)?;
                }
                Ok(Self::make_handle(peer_id, DelegationUnit::SchemaRoutineLookup, context.budget_remaining))
            }
            None => Err(SomaError::Distributed {
                failure: DistributedFailure::DelegationRefusal,
                details: format!(
                    "cannot delegate schema/routine lookup to peer {}: no remote executor configured",
                    peer_id
                ),
            }),
        }
    }

    fn migrate_session(
        &self,
        peer_id: &str,
        _session_data: &SessionMigrationData,
    ) -> Result<MigrationResult> {
        // Session migration requires a full state transfer protocol that serializes
        // all 8 session fields (goal, working memory, belief summary, pending
        // observations, budget, trace cursor, policy context) atomically. This
        // goes beyond individual skill/goal RPC calls and needs dedicated
        // session transfer support in the transport layer.
        Err(SomaError::Distributed {
            failure: DistributedFailure::MigrationFailure,
            details: format!(
                "cannot migrate session to peer {}: session migration requires a state transfer protocol that is not yet available",
                peer_id
            ),
        })
    }

    fn mirror_session(&self, peer_id: &str, _session_data: &SessionMigrationData) -> Result<()> {
        // Session mirroring requires the same atomic state transfer as migration,
        // plus ongoing synchronization to keep the mirror consistent.
        Err(SomaError::Distributed {
            failure: DistributedFailure::MigrationFailure,
            details: format!(
                "cannot mirror session to peer {}: session mirroring requires a state transfer protocol that is not yet available",
                peer_id
            ),
        })
    }

    fn accept_delegation(
        &self,
        from_peer: &str,
        context: &DelegationContext,
    ) -> Result<bool> {
        // Check that the peer's trust meets our minimum threshold.
        if context.trust_required < self.min_accept_trust {
            return Ok(false);
        }

        // For skill delegations, check that the requested skill is in our
        // capability set. For other delegation units, accept if trust is OK.
        match &context.delegation_unit {
            DelegationUnit::Skill => {
                // If we have no capabilities registered, reject.
                if self.capabilities.is_empty() {
                    return Ok(false);
                }
                // The attribution field carries the skill ID for skill delegations.
                Ok(self.capabilities.contains(&context.attribution))
            }
            _ => {
                // For subgoal, session, resource, and schema/routine lookups,
                // trust check alone is sufficient at this level.
                let _ = from_peer;
                Ok(true)
            }
        }
    }
}

// --- TrustAwareDelegationManager ---

/// Wrapper that enforces trust level checks on every delegation operation.
/// Looks up the peer's trust_class in the registry and compares it against
/// the `trust_required` field in the delegation context. If the peer's trust
/// is insufficient, returns `TrustValidationFailure` without forwarding.
pub struct TrustAwareDelegationManager<'a> {
    inner: Box<dyn DelegationManager>,
    registry: &'a dyn PeerRegistry,
}

impl<'a> TrustAwareDelegationManager<'a> {
    pub fn new(inner: Box<dyn DelegationManager>, registry: &'a dyn PeerRegistry) -> Self {
        Self { inner, registry }
    }

    /// Look up peer trust and validate against the context's requirement.
    fn check_trust(&self, peer_id: &str, context: &DelegationContext) -> Result<()> {
        let spec = self
            .registry
            .get_peer(peer_id)
            .ok_or_else(|| SomaError::PeerNotFound(peer_id.to_string()))?;
        require_trust(spec.trust_class, context.trust_required, peer_id)
    }
}

impl<'a> DelegationManager for TrustAwareDelegationManager<'a> {
    fn delegate_skill(
        &self,
        peer_id: &str,
        skill_id: &str,
        input: serde_json::Value,
        context: DelegationContext,
    ) -> Result<DelegationHandle> {
        self.check_trust(peer_id, &context)?;
        self.inner.delegate_skill(peer_id, skill_id, input, context)
    }

    fn delegate_subgoal(
        &self,
        peer_id: &str,
        subgoal: serde_json::Value,
        context: DelegationContext,
    ) -> Result<DelegationHandle> {
        self.check_trust(peer_id, &context)?;
        self.inner.delegate_subgoal(peer_id, subgoal, context)
    }

    fn delegate_resource_op(
        &self,
        peer_id: &str,
        resource_type: &str,
        resource_id: &str,
        operation: serde_json::Value,
        context: DelegationContext,
    ) -> Result<DelegationHandle> {
        self.check_trust(peer_id, &context)?;
        self.inner
            .delegate_resource_op(peer_id, resource_type, resource_id, operation, context)
    }

    fn delegate_schema_routine_lookup(
        &self,
        peer_id: &str,
        query: serde_json::Value,
        context: DelegationContext,
    ) -> Result<DelegationHandle> {
        self.check_trust(peer_id, &context)?;
        self.inner
            .delegate_schema_routine_lookup(peer_id, query, context)
    }

    fn migrate_session(
        &self,
        peer_id: &str,
        session_data: &SessionMigrationData,
    ) -> Result<MigrationResult> {
        let spec = self
            .registry
            .get_peer(peer_id)
            .ok_or_else(|| SomaError::PeerNotFound(peer_id.to_string()))?;
        require_trust(spec.trust_class, TrustLevel::Restricted, peer_id)?;
        self.inner.migrate_session(peer_id, session_data)
    }

    fn mirror_session(&self, peer_id: &str, session_data: &SessionMigrationData) -> Result<()> {
        let spec = self
            .registry
            .get_peer(peer_id)
            .ok_or_else(|| SomaError::PeerNotFound(peer_id.to_string()))?;
        require_trust(spec.trust_class, TrustLevel::Restricted, peer_id)?;
        self.inner.mirror_session(peer_id, session_data)
    }

    fn accept_delegation(
        &self,
        from_peer: &str,
        context: &DelegationContext,
    ) -> Result<bool> {
        // Verify the peer exists and meets the trust threshold before
        // forwarding to the inner manager's acceptance logic.
        self.check_trust(from_peer, context)?;
        self.inner.accept_delegation(from_peer, context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_context() -> DelegationContext {
        DelegationContext {
            session_id: Uuid::new_v4(),
            budget_remaining: 50.0,
            trust_required: TrustLevel::Verified,
            trace_required: true,
            policy_context: serde_json::json!({}),
            trace_cursor: 0,
            attribution: "test".to_string(),
            delegation_unit: DelegationUnit::Skill,
        }
    }

    #[test]
    fn delegate_skill_returns_delegation_refusal() {
        let mgr = DefaultDelegationManager::new();
        let result =
            mgr.delegate_skill("peer-1", "file.list", serde_json::json!({}), make_context());
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, details } => {
                assert_eq!(failure, DistributedFailure::DelegationRefusal);
                assert!(details.contains("peer-1"));
                assert!(details.contains("file.list"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn delegate_subgoal_returns_delegation_refusal() {
        let mgr = DefaultDelegationManager::new();
        let result = mgr.delegate_subgoal(
            "peer-2",
            serde_json::json!({"objective": "scan ports"}),
            make_context(),
        );
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, details } => {
                assert_eq!(failure, DistributedFailure::DelegationRefusal);
                assert!(details.contains("peer-2"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    fn make_migration_data() -> SessionMigrationData {
        use crate::types::peer::RemoteBudget;
        SessionMigrationData {
            session_id: Uuid::new_v4(),
            goal: serde_json::json!({"objective": "test"}),
            working_memory: serde_json::json!({}),
            belief_summary: serde_json::json!({}),
            pending_observations: vec![],
            current_budget: RemoteBudget {
                risk_limit: 0.5,
                latency_limit_ms: 30_000,
                resource_limit: 100.0,
                step_limit: 20,
            },
            trace_cursor: 0,
            policy_context: serde_json::json!({}),
        }
    }

    #[test]
    fn migrate_session_returns_migration_failure() {
        let mgr = DefaultDelegationManager::new();
        let data = make_migration_data();
        let result = mgr.migrate_session("peer-3", &data);
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, details } => {
                assert_eq!(failure, DistributedFailure::MigrationFailure);
                assert!(details.contains("peer-3"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn mirror_session_returns_migration_failure() {
        let mgr = DefaultDelegationManager::new();
        let data = make_migration_data();
        let result = mgr.mirror_session("peer-4", &data);
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, details } => {
                assert_eq!(failure, DistributedFailure::MigrationFailure);
                assert!(details.contains("peer-4"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn delegation_context_serialization() {
        let ctx = make_context();
        let json = serde_json::to_value(&ctx).unwrap();
        assert!(json["session_id"].is_string());
        assert_eq!(json["budget_remaining"], 50.0);
        assert_eq!(json["trust_required"], "verified");
        assert_eq!(json["trace_required"], true);
    }

    #[test]
    fn delegation_handle_serialization() {
        let handle = DelegationHandle {
            delegation_id: Uuid::new_v4(),
            peer_id: "peer-1".to_string(),
            delegation_unit: DelegationUnit::Skill,
            status: DelegationStatus::Pending,
            budget_allocated: 25.0,
        };
        let json = serde_json::to_value(&handle).unwrap();
        assert_eq!(json["peer_id"], "peer-1");
        assert_eq!(json["status"], "pending");
    }

    #[test]
    fn migration_result_serialization() {
        let success = MigrationResult {
            outcome: MigrationOutcome::Success,
            reason: None,
            new_session_id: Some(Uuid::new_v4()),
        };
        let json = serde_json::to_value(&success).unwrap();
        assert_eq!(json["outcome"], "success");
        assert!(json["new_session_id"].is_string());

        let failure = MigrationResult {
            outcome: MigrationOutcome::Failure,
            reason: Some("policy denied".to_string()),
            new_session_id: None,
        };
        let json = serde_json::to_value(&failure).unwrap();
        assert_eq!(json["outcome"], "failure");
        assert_eq!(json["reason"], "policy denied");
    }

    #[test]
    fn delegation_status_variants_serialize() {
        let variants = vec![
            (DelegationStatus::Pending, "pending"),
            (DelegationStatus::Accepted, "accepted"),
            (DelegationStatus::Running, "running"),
            (DelegationStatus::Completed, "completed"),
            (DelegationStatus::Failed, "failed"),
            (DelegationStatus::Refused, "refused"),
        ];
        for (status, expected) in variants {
            let json = serde_json::to_value(status).unwrap();
            assert_eq!(json, expected);
        }
    }

    #[test]
    fn delegate_resource_op_returns_delegation_refusal() {
        let mgr = DefaultDelegationManager::new();
        let result = mgr.delegate_resource_op(
            "peer-3",
            "filesystem",
            "root",
            serde_json::json!({"op": "read"}),
            make_context(),
        );
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, details } => {
                assert_eq!(failure, DistributedFailure::DelegationRefusal);
                assert!(details.contains("peer-3"));
                assert!(details.contains("filesystem"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn delegate_schema_routine_lookup_returns_delegation_refusal() {
        let mgr = DefaultDelegationManager::new();
        let result = mgr.delegate_schema_routine_lookup(
            "peer-4",
            serde_json::json!({"query": "file_management"}),
            make_context(),
        );
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, details } => {
                assert_eq!(failure, DistributedFailure::DelegationRefusal);
                assert!(details.contains("peer-4"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn delegation_unit_variants_serialize() {
        let variants = vec![
            (DelegationUnit::Skill, "skill"),
            (DelegationUnit::Subgoal, "subgoal"),
            (DelegationUnit::Session, "session"),
            (DelegationUnit::ResourceOperation, "resource_operation"),
            (DelegationUnit::SchemaRoutineLookup, "schema_routine_lookup"),
        ];
        for (unit, expected) in variants {
            let json = serde_json::to_value(unit).unwrap();
            assert_eq!(json, expected);
        }
    }

    #[test]
    fn delegation_context_full_serialization() {
        let ctx = make_context();
        let json = serde_json::to_value(&ctx).unwrap();
        // All delegation preservation fields from distributed.md.
        assert!(json["session_id"].is_string());
        assert_eq!(json["budget_remaining"], 50.0);
        assert_eq!(json["trust_required"], "verified");
        assert_eq!(json["trace_required"], true);
        assert!(json["policy_context"].is_object());
        assert!(json["trace_cursor"].is_number());
        assert!(json["attribution"].is_string());
        assert_eq!(json["delegation_unit"], "skill");
    }

    #[test]
    fn session_migration_data_has_all_eight_fields() {
        let data = make_migration_data();
        let json = serde_json::to_value(&data).unwrap();
        // 8 migration data fields from distributed.md.
        assert!(json["session_id"].is_string());
        assert!(json["goal"].is_object());
        assert!(json["working_memory"].is_object());
        assert!(json["belief_summary"].is_object());
        assert!(json["pending_observations"].is_array());
        assert!(json["current_budget"].is_object());
        assert!(json["trace_cursor"].is_number());
        assert!(json["policy_context"].is_object());
    }

    // --- TrustAwareDelegationManager tests ---

    use crate::distributed::peer::{DefaultPeerRegistry, PeerRegistry};
    use crate::types::common::{
        DeterminismClass, LatencyProfile, RiskClass, RollbackSupport, SchemaRef,
    };
    use crate::types::peer::{
        AccessMode, MutationMode, PeerAvailability, PeerSpec, RemoteResourceAd, RemoteSkillAd,
        Transport,
    };

    fn make_skill_ad(skill_id: &str) -> RemoteSkillAd {
        RemoteSkillAd {
            skill_id: skill_id.to_string(),
            name: skill_id.to_string(),
            kind: "action".to_string(),
            inputs: SchemaRef {
                schema: serde_json::json!({}),
            },
            outputs: SchemaRef {
                schema: serde_json::json!({}),
            },
            preconditions: vec![],
            expected_effects: vec![],
            observables: vec![],
            termination_conditions: vec![],
            rollback_or_compensation: RollbackSupport::Irreversible,
            cost_prior: LatencyProfile {
                expected_latency_ms: 10,
                p95_latency_ms: 50,
                max_latency_ms: 100,
            },
            risk_class: RiskClass::Low,
            determinism: DeterminismClass::Deterministic,
        }
    }

    fn make_resource_ad(resource_type: &str) -> RemoteResourceAd {
        RemoteResourceAd {
            resource_type: resource_type.to_string(),
            resource_id: resource_type.to_string(),
            version: 1,
            visibility: "public".to_string(),
            access_mode: AccessMode::ReadOnly,
            mutation_mode: MutationMode::Immutable,
            sync_mode: "sync".to_string(),
            provenance: "local".to_string(),
            staleness_bounds_ms: None,
        }
    }

    fn make_test_peer(id: &str) -> PeerSpec {
        PeerSpec {
            peer_id: id.to_string(),
            version: "0.1.0".to_string(),
            trust_class: TrustLevel::Verified,
            supported_transports: vec![Transport::Tcp],
            reachable_endpoints: vec!["127.0.0.1:9000".to_string()],
            current_availability: PeerAvailability::Available,
            policy_limits: vec![],
            exposed_packs: vec!["core".to_string()],
            exposed_skills: vec![make_skill_ad("file.list")],
            exposed_resources: vec![make_resource_ad("filesystem")],
            latency_class: "low".to_string(),
            cost_class: "low".to_string(),
            current_load: 0.0,
            last_seen: chrono::Utc::now(),
            replay_support: true,
            observation_streaming: true,
            advertisement_version: 1,
            advertisement_expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        }
    }

    #[test]
    fn trust_aware_delegate_skill_rejected_when_trust_insufficient() {
        let mut registry = DefaultPeerRegistry::new();
        registry.register_peer(make_test_peer("peer-1")).unwrap();
        // Peer starts Untrusted (registration default). Context requires Verified.

        let mgr =
            TrustAwareDelegationManager::new(Box::new(DefaultDelegationManager::new()), &registry);

        let result = mgr.delegate_skill(
            "peer-1",
            "file.list",
            serde_json::json!({}),
            make_context(), // trust_required: Verified
        );
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::TrustValidationFailure);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn trust_aware_delegate_skill_forwarded_when_trust_sufficient() {
        let mut registry = DefaultPeerRegistry::new();
        registry.register_peer(make_test_peer("peer-1")).unwrap();
        registry
            .elevate_trust("peer-1", TrustLevel::Trusted)
            .unwrap();

        let mgr =
            TrustAwareDelegationManager::new(Box::new(DefaultDelegationManager::new()), &registry);

        let result = mgr.delegate_skill(
            "peer-1",
            "file.list",
            serde_json::json!({}),
            make_context(), // trust_required: Verified, peer has Trusted
        );
        // Forwarded to inner, which returns DelegationRefusal (transport not wired).
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::DelegationRefusal);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn trust_aware_delegate_subgoal_rejected_when_trust_insufficient() {
        let mut registry = DefaultPeerRegistry::new();
        registry.register_peer(make_test_peer("peer-1")).unwrap();

        let mgr =
            TrustAwareDelegationManager::new(Box::new(DefaultDelegationManager::new()), &registry);

        let result = mgr.delegate_subgoal(
            "peer-1",
            serde_json::json!({"objective": "scan"}),
            make_context(),
        );
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::TrustValidationFailure);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn trust_aware_delegate_resource_op_rejected_when_trust_insufficient() {
        let mut registry = DefaultPeerRegistry::new();
        registry.register_peer(make_test_peer("peer-1")).unwrap();

        let mgr =
            TrustAwareDelegationManager::new(Box::new(DefaultDelegationManager::new()), &registry);

        let result = mgr.delegate_resource_op(
            "peer-1",
            "filesystem",
            "root",
            serde_json::json!({"op": "read"}),
            make_context(),
        );
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::TrustValidationFailure);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn trust_aware_delegate_schema_lookup_rejected_when_trust_insufficient() {
        let mut registry = DefaultPeerRegistry::new();
        registry.register_peer(make_test_peer("peer-1")).unwrap();

        let mgr =
            TrustAwareDelegationManager::new(Box::new(DefaultDelegationManager::new()), &registry);

        let result = mgr.delegate_schema_routine_lookup(
            "peer-1",
            serde_json::json!({"query": "file_mgmt"}),
            make_context(),
        );
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::TrustValidationFailure);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn trust_aware_delegate_to_unknown_peer_fails() {
        let registry = DefaultPeerRegistry::new();

        let mgr =
            TrustAwareDelegationManager::new(Box::new(DefaultDelegationManager::new()), &registry);

        let result =
            mgr.delegate_skill("ghost", "file.list", serde_json::json!({}), make_context());
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::PeerNotFound(id) => {
                assert_eq!(id, "ghost");
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn trust_aware_allows_untrusted_context_requirement() {
        let mut registry = DefaultPeerRegistry::new();
        registry.register_peer(make_test_peer("peer-1")).unwrap();
        // Peer is Untrusted. Context also requires Untrusted — should pass trust check.

        let mgr =
            TrustAwareDelegationManager::new(Box::new(DefaultDelegationManager::new()), &registry);

        let mut ctx = make_context();
        ctx.trust_required = TrustLevel::Untrusted;

        let result = mgr.delegate_skill("peer-1", "file.list", serde_json::json!({}), ctx);
        // Trust check passes, forwarded to inner which returns DelegationRefusal.
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::DelegationRefusal);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn trust_aware_migrate_session_rejected_when_trust_insufficient() {
        let mut registry = DefaultPeerRegistry::new();
        registry.register_peer(make_test_peer("peer-1")).unwrap();
        // Peer starts Untrusted after registration — below Restricted threshold.

        let mgr =
            TrustAwareDelegationManager::new(Box::new(DefaultDelegationManager::new()), &registry);

        let data = make_migration_data();
        let result = mgr.migrate_session("peer-1", &data);
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::TrustValidationFailure);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn trust_aware_migrate_session_forwarded_when_trust_sufficient() {
        let mut registry = DefaultPeerRegistry::new();
        registry.register_peer(make_test_peer("peer-1")).unwrap();
        registry
            .elevate_trust("peer-1", TrustLevel::Restricted)
            .unwrap();

        let mgr =
            TrustAwareDelegationManager::new(Box::new(DefaultDelegationManager::new()), &registry);

        let data = make_migration_data();
        let result = mgr.migrate_session("peer-1", &data);
        // Forwarded to inner, which returns MigrationFailure (transport not wired).
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::MigrationFailure);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn trust_aware_mirror_session_rejected_when_trust_insufficient() {
        let mut registry = DefaultPeerRegistry::new();
        registry.register_peer(make_test_peer("peer-1")).unwrap();
        // Peer starts Untrusted — below Restricted threshold.

        let mgr =
            TrustAwareDelegationManager::new(Box::new(DefaultDelegationManager::new()), &registry);

        let data = make_migration_data();
        let result = mgr.mirror_session("peer-1", &data);
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::TrustValidationFailure);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn trust_aware_mirror_session_forwarded_when_trust_sufficient() {
        let mut registry = DefaultPeerRegistry::new();
        registry.register_peer(make_test_peer("peer-1")).unwrap();
        registry
            .elevate_trust("peer-1", TrustLevel::Verified)
            .unwrap();

        let mgr =
            TrustAwareDelegationManager::new(Box::new(DefaultDelegationManager::new()), &registry);

        let data = make_migration_data();
        let result = mgr.mirror_session("peer-1", &data);
        // Forwarded to inner, which returns MigrationFailure (transport not wired).
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::MigrationFailure);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn trust_aware_migrate_session_unknown_peer_fails() {
        let registry = DefaultPeerRegistry::new();

        let mgr =
            TrustAwareDelegationManager::new(Box::new(DefaultDelegationManager::new()), &registry);

        let data = make_migration_data();
        let result = mgr.migrate_session("ghost", &data);
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::PeerNotFound(id) => {
                assert_eq!(id, "ghost");
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn trust_aware_mirror_session_unknown_peer_fails() {
        let registry = DefaultPeerRegistry::new();

        let mgr =
            TrustAwareDelegationManager::new(Box::new(DefaultDelegationManager::new()), &registry);

        let data = make_migration_data();
        let result = mgr.mirror_session("ghost", &data);
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::PeerNotFound(id) => {
                assert_eq!(id, "ghost");
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    // --- accept_delegation tests ---

    #[test]
    fn accept_delegation_rejects_when_no_capabilities() {
        let mgr = DefaultDelegationManager::new();
        let ctx = make_context();
        let result = mgr.accept_delegation("peer-1", &ctx).unwrap();
        // Skill delegation with empty capabilities set is rejected.
        assert!(!result);
    }

    #[test]
    fn accept_delegation_accepts_known_skill() {
        let mut caps = std::collections::HashSet::new();
        caps.insert("test".to_string());
        let mgr = DefaultDelegationManager::with_capabilities(caps, TrustLevel::Verified);
        let ctx = make_context(); // attribution is "test", trust_required is Verified
        let result = mgr.accept_delegation("peer-1", &ctx).unwrap();
        assert!(result);
    }

    #[test]
    fn accept_delegation_rejects_unknown_skill() {
        let mut caps = std::collections::HashSet::new();
        caps.insert("file.list".to_string());
        let mgr = DefaultDelegationManager::with_capabilities(caps, TrustLevel::Verified);
        let ctx = make_context(); // attribution is "test", not "file.list"
        let result = mgr.accept_delegation("peer-1", &ctx).unwrap();
        assert!(!result);
    }

    #[test]
    fn accept_delegation_rejects_insufficient_trust() {
        let mut caps = std::collections::HashSet::new();
        caps.insert("test".to_string());
        // Require Trusted, but context only requires Verified.
        let mgr = DefaultDelegationManager::with_capabilities(caps, TrustLevel::Trusted);
        let ctx = make_context(); // trust_required is Verified
        let result = mgr.accept_delegation("peer-1", &ctx).unwrap();
        assert!(!result);
    }

    #[test]
    fn accept_delegation_accepts_subgoal_with_trust() {
        let mgr = DefaultDelegationManager::with_capabilities(
            std::collections::HashSet::new(),
            TrustLevel::Verified,
        );
        let mut ctx = make_context();
        ctx.delegation_unit = DelegationUnit::Subgoal;
        let result = mgr.accept_delegation("peer-1", &ctx).unwrap();
        // Subgoal delegation only requires trust check, no capability match.
        assert!(result);
    }

    #[test]
    fn accept_delegation_accepts_resource_op_with_trust() {
        let mgr = DefaultDelegationManager::with_capabilities(
            std::collections::HashSet::new(),
            TrustLevel::Untrusted,
        );
        let mut ctx = make_context();
        ctx.delegation_unit = DelegationUnit::ResourceOperation;
        let result = mgr.accept_delegation("peer-1", &ctx).unwrap();
        assert!(result);
    }

    #[test]
    fn trust_aware_accept_delegation_rejects_untrusted_peer() {
        let mut registry = DefaultPeerRegistry::new();
        registry.register_peer(make_test_peer("peer-1")).unwrap();
        // Peer starts Untrusted. Context requires Verified.

        let mut caps = std::collections::HashSet::new();
        caps.insert("test".to_string());
        let inner = DefaultDelegationManager::with_capabilities(caps, TrustLevel::Verified);

        let mgr = TrustAwareDelegationManager::new(Box::new(inner), &registry);

        let ctx = make_context(); // trust_required: Verified
        let result = mgr.accept_delegation("peer-1", &ctx);
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::TrustValidationFailure);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn trust_aware_accept_delegation_forwards_when_trust_sufficient() {
        let mut registry = DefaultPeerRegistry::new();
        registry.register_peer(make_test_peer("peer-1")).unwrap();
        registry
            .elevate_trust("peer-1", TrustLevel::Trusted)
            .unwrap();

        let mut caps = std::collections::HashSet::new();
        caps.insert("test".to_string());
        let inner = DefaultDelegationManager::with_capabilities(caps, TrustLevel::Verified);

        let mgr = TrustAwareDelegationManager::new(Box::new(inner), &registry);

        let ctx = make_context();
        let result = mgr.accept_delegation("peer-1", &ctx).unwrap();
        assert!(result);
    }

}
