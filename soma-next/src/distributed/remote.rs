use serde_json;
use tracing::warn;

use crate::errors::{Result, SomaError};
use crate::types::common::{RiskClass, TrustLevel};
use crate::types::peer::{
    DistributedFailure, FailureRecoverability, RemoteGoalRequest, RoutineTransfer, SchemaTransfer,
    StructuredFailure,
};

use super::auth::{require_authenticated, require_trust, PeerAuthenticator};
use super::peer::PeerRegistry;

// The trait, the response envelope types, and `RemoteInvocationContext` are
// defined in `crate::runtime::remote` so they can always compile regardless
// of the `distributed` feature. Re-export them here so existing call sites
// using `crate::distributed::remote::*` keep working unchanged.
pub use crate::runtime::remote::{
    RemoteExecutor, RemoteGoalResponse, RemoteGoalStatus, RemoteInvocationContext,
    RemoteResourceResponse, RemoteSkillResponse, ResourceDataMode,
};

// --- Failure classification and recovery ---

/// Outcome of a recovery attempt, determining what the caller should do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryOutcome {
    /// Retry the same operation on a different peer.
    RetryOnAnotherPeer,
    /// Delegate the operation to an alternate peer.
    DelegateToAlternate,
    /// Fall back to local execution.
    FallbackToLocal,
    /// Abort the operation. `terminal_for_session` indicates whether the
    /// entire session is poisoned or only this single action.
    Abort { terminal_for_session: bool },
}

/// Map a `DistributedFailure` variant to its recoverability category.
pub fn classify_failure(failure: DistributedFailure) -> FailureRecoverability {
    match failure {
        DistributedFailure::PeerUnreachable
        | DistributedFailure::UnsupportedSkill
        | DistributedFailure::UnsupportedResource
        | DistributedFailure::DelegationRefusal => FailureRecoverability::DelegatableToAnotherPeer,

        DistributedFailure::TransportFailure
        | DistributedFailure::StaleData
        | DistributedFailure::Timeout
        | DistributedFailure::PartialObservationStream => FailureRecoverability::Retryable,

        DistributedFailure::AuthenticationFailure
        | DistributedFailure::BudgetExhaustion
        | DistributedFailure::MigrationFailure => FailureRecoverability::TerminalForSession,

        DistributedFailure::AuthorizationFailure
        | DistributedFailure::TrustValidationFailure
        | DistributedFailure::ConflictingData
        | DistributedFailure::ReplayRejection
        | DistributedFailure::PolicyViolation => FailureRecoverability::TerminalForActionOnly,
    }
}

/// Wrap a `SomaError::Distributed` into a `StructuredFailure` with the correct
/// recoverability classification. Non-distributed errors are mapped to a
/// transport failure with retryable recoverability.
pub fn build_structured_failure(error: &SomaError) -> StructuredFailure {
    match error {
        SomaError::Distributed { failure, details } => StructuredFailure {
            failure: *failure,
            recoverability: classify_failure(*failure),
            details: details.clone(),
        },
        other => StructuredFailure {
            failure: DistributedFailure::TransportFailure,
            recoverability: FailureRecoverability::Retryable,
            details: other.to_string(),
        },
    }
}

/// Coordinates recovery from distributed failures by tracking retry budgets
/// and selecting the appropriate recovery action based on failure recoverability.
pub struct RecoveryCoordinator {
    max_retries: u32,
}

impl RecoveryCoordinator {
    pub fn new(max_retries: u32) -> Self {
        Self { max_retries }
    }

    /// Decide what to do after a failure on `failed_peer`.
    ///
    /// `attempt` is the current attempt number (0-based). When the retry budget
    /// is exhausted, retryable failures fall back to local execution instead of
    /// retrying on another peer.
    ///
    /// `budget_remaining`, when `Some(x)` with `x <= 0.0`, forces an immediate
    /// abort regardless of retry count or recoverability category. This ensures
    /// recovery respects external budget limits (e.g., resource or cost budgets).
    pub fn attempt_recovery(
        &self,
        failure: &StructuredFailure,
        _failed_peer: &str,
        attempt: u32,
        budget_remaining: Option<f64>,
    ) -> RecoveryOutcome {
        if let Some(budget) = budget_remaining
            && budget <= 0.0 {
                return RecoveryOutcome::Abort {
                    terminal_for_session: true,
                };
            }

        match failure.recoverability {
            FailureRecoverability::Retryable => {
                if attempt < self.max_retries {
                    RecoveryOutcome::RetryOnAnotherPeer
                } else {
                    RecoveryOutcome::FallbackToLocal
                }
            }
            FailureRecoverability::DelegatableToAnotherPeer => RecoveryOutcome::DelegateToAlternate,
            FailureRecoverability::TerminalForSession => RecoveryOutcome::Abort {
                terminal_for_session: true,
            },
            FailureRecoverability::TerminalForActionOnly => RecoveryOutcome::Abort {
                terminal_for_session: false,
            },
        }
    }
}

// --- DefaultRemoteExecutor ---

/// Default implementation that delegates to an optional inner `RemoteExecutor`.
/// When constructed with `with_transport()`, all calls are forwarded to the
/// inner executor. When no transport is configured (`new()`), all calls return
/// `PeerUnreachable` with a message explaining that no remote executor is configured.
pub struct DefaultRemoteExecutor {
    inner: Option<Box<dyn RemoteExecutor>>,
}

impl DefaultRemoteExecutor {
    pub fn new() -> Self {
        Self { inner: None }
    }

    /// Create a `DefaultRemoteExecutor` that forwards all calls to the given
    /// transport implementation (e.g., `TcpRemoteExecutor`).
    pub fn with_transport(inner: Box<dyn RemoteExecutor>) -> Self {
        Self { inner: Some(inner) }
    }
}

impl Default for DefaultRemoteExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl RemoteExecutor for DefaultRemoteExecutor {
    fn submit_goal(
        &self,
        peer_id: &str,
        request: &RemoteGoalRequest,
    ) -> Result<RemoteGoalResponse> {
        match &self.inner {
            Some(executor) => executor.submit_goal(peer_id, request),
            None => Err(SomaError::Distributed {
                failure: DistributedFailure::PeerUnreachable,
                details: format!(
                    "no remote executor configured for peer {}",
                    peer_id
                ),
            }),
        }
    }

    fn invoke_skill(
        &self,
        peer_id: &str,
        skill_id: &str,
        input: serde_json::Value,
    ) -> Result<RemoteSkillResponse> {
        match &self.inner {
            Some(executor) => executor.invoke_skill(peer_id, skill_id, input),
            None => Err(SomaError::Distributed {
                failure: DistributedFailure::PeerUnreachable,
                details: format!(
                    "no remote executor configured for peer {} to invoke skill {}",
                    peer_id, skill_id
                ),
            }),
        }
    }

    fn query_resource(
        &self,
        peer_id: &str,
        resource_type: &str,
        resource_id: &str,
    ) -> Result<RemoteResourceResponse> {
        match &self.inner {
            Some(executor) => executor.query_resource(peer_id, resource_type, resource_id),
            None => Err(SomaError::Distributed {
                failure: DistributedFailure::PeerUnreachable,
                details: format!(
                    "no remote executor configured for peer {} to query {}/{}",
                    peer_id, resource_type, resource_id
                ),
            }),
        }
    }

    fn transfer_schema(&self, peer_id: &str, schema: &SchemaTransfer) -> Result<()> {
        match &self.inner {
            Some(executor) => executor.transfer_schema(peer_id, schema),
            None => Err(SomaError::Distributed {
                failure: DistributedFailure::PeerUnreachable,
                details: format!(
                    "no remote executor configured for peer {} to transfer schema",
                    peer_id
                ),
            }),
        }
    }

    fn transfer_routine(&self, peer_id: &str, routine: &RoutineTransfer) -> Result<()> {
        match &self.inner {
            Some(executor) => executor.transfer_routine(peer_id, routine),
            None => Err(SomaError::Distributed {
                failure: DistributedFailure::PeerUnreachable,
                details: format!(
                    "no remote executor configured for peer {} to transfer routine",
                    peer_id
                ),
            }),
        }
    }
}

// --- ValidatingRemoteExecutor ---

/// Wrapper around a `RemoteExecutor` that enforces authentication, trust,
/// and skill/resource availability checks before forwarding to the inner executor.
/// Also enforces data freshness: resource query responses older than
/// `max_staleness_ms` are rejected with a StaleData error.
pub struct ValidatingRemoteExecutor<'a> {
    inner: Box<dyn RemoteExecutor>,
    registry: &'a dyn PeerRegistry,
    authenticator: &'a dyn PeerAuthenticator,
    /// Maximum acceptable freshness for resource query responses (in milliseconds).
    /// Responses with freshness_ms exceeding this threshold are rejected.
    /// Default: 30000 (30 seconds).
    pub max_staleness_ms: u64,
}

impl<'a> ValidatingRemoteExecutor<'a> {
    pub fn new(
        inner: Box<dyn RemoteExecutor>,
        registry: &'a dyn PeerRegistry,
        authenticator: &'a dyn PeerAuthenticator,
    ) -> Self {
        Self {
            inner,
            registry,
            authenticator,
            max_staleness_ms: 30_000,
        }
    }

    /// Look up the peer in the registry and return its trust class.
    fn peer_trust(&self, peer_id: &str) -> Result<TrustLevel> {
        let spec = self
            .registry
            .get_peer(peer_id)
            .ok_or_else(|| SomaError::PeerNotFound(peer_id.to_string()))?;
        Ok(spec.trust_class)
    }

    /// Look up the skill advertisement for a peer's skill.
    fn peer_skill_ad(
        &self,
        peer_id: &str,
        skill_id: &str,
    ) -> Result<Option<crate::types::peer::RemoteSkillAd>> {
        let spec = self
            .registry
            .get_peer(peer_id)
            .ok_or_else(|| SomaError::PeerNotFound(peer_id.to_string()))?;
        Ok(spec
            .exposed_skills
            .iter()
            .find(|s| s.skill_id == skill_id)
            .cloned())
    }

    /// Validate that the input JSON is a well-formed object and contains
    /// the required fields declared in the skill's input schema (if any).
    fn validate_input_binding(
        &self,
        skill_id: &str,
        input: &serde_json::Value,
        input_schema: &serde_json::Value,
    ) -> Result<()> {
        // Input must be a JSON object, not null, array, or scalar.
        if !input.is_object() {
            return Err(SomaError::Distributed {
                failure: DistributedFailure::PolicyViolation,
                details: format!(
                    "skill {} input must be a JSON object, got {}",
                    skill_id,
                    match input {
                        serde_json::Value::Null => "null",
                        serde_json::Value::Bool(_) => "bool",
                        serde_json::Value::Number(_) => "number",
                        serde_json::Value::String(_) => "string",
                        serde_json::Value::Array(_) => "array",
                        serde_json::Value::Object(_) => unreachable!(),
                    }
                ),
            });
        }

        // If the schema declares required fields, verify they are present.
        if let Some(required) = input_schema.get("required").and_then(|v| v.as_array()) {
            let input_obj = input.as_object().unwrap();
            let mut missing: Vec<&str> = Vec::new();
            for field in required {
                if let Some(name) = field.as_str()
                    && !input_obj.contains_key(name) {
                        missing.push(name);
                    }
            }
            if !missing.is_empty() {
                return Err(SomaError::Distributed {
                    failure: DistributedFailure::PolicyViolation,
                    details: format!(
                        "skill {} input is missing required fields: {}",
                        skill_id,
                        missing.join(", ")
                    ),
                });
            }
        }

        Ok(())
    }

    /// Invoke a remote skill with additional session policy and budget checks.
    ///
    /// Performs all the same auth, trust, availability, and input binding
    /// validations as `invoke_skill`, but also checks:
    /// - If `context.session_budget_remaining` is Some and <= 0, rejects with
    ///   BudgetExhaustion.
    /// - If `context.policy_allows` is false, rejects with PolicyViolation.
    pub fn invoke_skill_with_context(
        &self,
        peer_id: &str,
        skill_id: &str,
        input: serde_json::Value,
        context: &RemoteInvocationContext,
    ) -> Result<RemoteSkillResponse> {
        if let Some(budget) = context.session_budget_remaining
            && budget <= 0.0 {
                return Err(SomaError::Distributed {
                    failure: DistributedFailure::BudgetExhaustion,
                    details: format!(
                        "session budget exhausted (remaining: {}) for skill {} on peer {}",
                        budget, skill_id, peer_id
                    ),
                });
            }

        if !context.policy_allows {
            return Err(SomaError::Distributed {
                failure: DistributedFailure::PolicyViolation,
                details: format!(
                    "session policy denies invocation of skill {} on peer {}",
                    skill_id, peer_id
                ),
            });
        }

        // Delegate to the standard invoke_skill path for auth, trust, skill
        // availability, and input binding checks.
        self.invoke_skill(peer_id, skill_id, input)
    }
}

impl<'a> RemoteExecutor for ValidatingRemoteExecutor<'a> {
    fn submit_goal(
        &self,
        peer_id: &str,
        request: &RemoteGoalRequest,
    ) -> Result<RemoteGoalResponse> {
        require_authenticated(self.authenticator, peer_id)?;
        let trust = self.peer_trust(peer_id)?;
        require_trust(trust, request.trust_required, peer_id)?;
        self.inner.submit_goal(peer_id, request)
    }

    fn invoke_skill(
        &self,
        peer_id: &str,
        skill_id: &str,
        input: serde_json::Value,
    ) -> Result<RemoteSkillResponse> {
        require_authenticated(self.authenticator, peer_id)?;
        let trust = self.peer_trust(peer_id)?;
        require_trust(trust, TrustLevel::Restricted, peer_id)?;

        let skill_ad = self.peer_skill_ad(peer_id, skill_id)?;
        let skill_ad = match skill_ad {
            Some(ad) => ad,
            None => {
                return Err(SomaError::Distributed {
                    failure: DistributedFailure::UnsupportedSkill,
                    details: format!("peer {} does not advertise skill {}", peer_id, skill_id),
                });
            }
        };

        // Validate the input against the skill's advertised input schema.
        self.validate_input_binding(skill_id, &input, &skill_ad.inputs.schema)?;

        // Destructive operations (High or Critical risk) require elevated trust.
        // Remote destructive/irreversible actions need at least Verified trust to
        // prevent accidental or unauthorized damage across peer boundaries.
        if matches!(skill_ad.risk_class, RiskClass::High | RiskClass::Critical) {
            warn!(
                peer_id = peer_id,
                skill_id = skill_id,
                risk_class = ?skill_ad.risk_class,
                "destructive remote operation requested"
            );
            require_trust(trust, TrustLevel::Verified, peer_id).map_err(|_| {
                SomaError::Distributed {
                    failure: DistributedFailure::PolicyViolation,
                    details: format!(
                        "skill {} on peer {} has risk class {:?} (destructive); \
                         requires at least Verified trust, but peer has {:?}",
                        skill_id, peer_id, skill_ad.risk_class, trust
                    ),
                }
            })?;
        }

        self.inner.invoke_skill(peer_id, skill_id, input)
    }

    fn query_resource(
        &self,
        peer_id: &str,
        resource_type: &str,
        resource_id: &str,
    ) -> Result<RemoteResourceResponse> {
        require_authenticated(self.authenticator, peer_id)?;
        let trust = self.peer_trust(peer_id)?;
        require_trust(trust, TrustLevel::Restricted, peer_id)?;
        let response = self
            .inner
            .query_resource(peer_id, resource_type, resource_id)?;
        if response.freshness_ms > self.max_staleness_ms {
            return Err(SomaError::Distributed {
                failure: DistributedFailure::StaleData,
                details: format!(
                    "resource {}/{} from peer {} has freshness {}ms, exceeding max staleness {}ms",
                    resource_type,
                    resource_id,
                    peer_id,
                    response.freshness_ms,
                    self.max_staleness_ms
                ),
            });
        }
        Ok(response)
    }

    fn transfer_schema(&self, peer_id: &str, schema: &SchemaTransfer) -> Result<()> {
        require_authenticated(self.authenticator, peer_id)?;
        let trust = self.peer_trust(peer_id)?;
        require_trust(trust, TrustLevel::Verified, peer_id)?;

        // Exposure policy check: verify the schema belongs to an exposed pack.
        // SchemaTransfer does not carry pack association, so we cannot fully
        // verify exposure. Log a warning so operators can audit.
        let spec = self.registry.get_peer(peer_id);
        let exposed_packs = spec.map(|s| &s.exposed_packs);
        warn!(
            peer_id = peer_id,
            schema_id = schema.schema_id.as_str(),
            exposed_packs = ?exposed_packs,
            "schema transfer has unverified exposure policy — \
             cannot confirm schema belongs to an exposed pack"
        );

        self.inner.transfer_schema(peer_id, schema)
    }

    fn transfer_routine(&self, peer_id: &str, routine: &RoutineTransfer) -> Result<()> {
        require_authenticated(self.authenticator, peer_id)?;
        let trust = self.peer_trust(peer_id)?;
        require_trust(trust, TrustLevel::Verified, peer_id)?;

        // Exposure policy check: verify the routine belongs to an exposed pack.
        // RoutineTransfer does not carry pack association, so we cannot fully
        // verify exposure. Log a warning so operators can audit.
        let spec = self.registry.get_peer(peer_id);
        let exposed_packs = spec.map(|s| &s.exposed_packs);
        warn!(
            peer_id = peer_id,
            routine_id = routine.routine_id.as_str(),
            exposed_packs = ?exposed_packs,
            "routine transfer has unverified exposure policy — \
             cannot confirm routine belongs to an exposed pack"
        );

        self.inner.transfer_routine(peer_id, routine)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;
    use crate::types::common::TrustLevel;
    use crate::types::peer::RemoteBudget;

    fn make_goal_request() -> RemoteGoalRequest {
        RemoteGoalRequest {
            goal: serde_json::json!({"objective": "test"}),
            constraints: vec!["no_destructive".to_string()],
            budgets: RemoteBudget {
                risk_limit: 0.5,
                latency_limit_ms: 5000,
                resource_limit: 100.0,
                step_limit: 10,
            },
            trust_required: TrustLevel::Verified,
            request_result: true,
            request_trace: false,
        }
    }

    #[test]
    fn submit_goal_returns_peer_unreachable() {
        let exec = DefaultRemoteExecutor::new();
        let result = exec.submit_goal("peer-1", &make_goal_request());
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, details } => {
                assert_eq!(failure, DistributedFailure::PeerUnreachable);
                assert!(details.contains("peer-1"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn invoke_skill_returns_peer_unreachable() {
        let exec = DefaultRemoteExecutor::new();
        let result = exec.invoke_skill("peer-1", "file.list", serde_json::json!({}));
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, details } => {
                assert_eq!(failure, DistributedFailure::PeerUnreachable);
                assert!(details.contains("peer-1"));
                assert!(details.contains("file.list"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn query_resource_returns_peer_unreachable() {
        let exec = DefaultRemoteExecutor::new();
        let result = exec.query_resource("peer-1", "filesystem", "root");
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, details } => {
                assert_eq!(failure, DistributedFailure::PeerUnreachable);
                assert!(details.contains("peer-1"));
                assert!(details.contains("filesystem"));
                assert!(details.contains("root"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn transfer_schema_returns_peer_unreachable() {
        let exec = DefaultRemoteExecutor::new();
        let schema = SchemaTransfer {
            schema_id: "schema-1".to_string(),
            version: "1.0".to_string(),
            trigger_conditions: vec![],
            subgoal_structure: vec![],
            candidate_skill_ordering: vec![],
            stop_conditions: vec![],
            confidence: 0.9,
        };
        let result = exec.transfer_schema("peer-1", &schema);
        assert!(result.is_err());
    }

    #[test]
    fn transfer_routine_returns_peer_unreachable() {
        let exec = DefaultRemoteExecutor::new();
        let routine = RoutineTransfer {
            routine_id: "routine-1".to_string(),
            match_conditions: vec![],
            compiled_skill_path: vec!["skill.a".to_string()],
            guard_conditions: vec![],
            expected_cost: 1.0,
            expected_effect: vec![],
            confidence: 0.95,
        };
        let result = exec.transfer_routine("peer-1", &routine);
        assert!(result.is_err());
    }

    #[test]
    fn remote_goal_response_serialization() {
        let resp = RemoteGoalResponse {
            status: RemoteGoalStatus::Accepted,
            session_id: Some("sess-123".to_string()),
            reason: None,
            required_adjustments: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["status"], "accepted");
        assert_eq!(json["session_id"], "sess-123");

        let rejected = RemoteGoalResponse {
            status: RemoteGoalStatus::Rejected,
            session_id: None,
            reason: Some("budget too low".to_string()),
            required_adjustments: None,
        };
        let json = serde_json::to_value(&rejected).unwrap();
        assert_eq!(json["status"], "rejected");
        assert_eq!(json["reason"], "budget too low");

        let stricter = RemoteGoalResponse {
            status: RemoteGoalStatus::RequestStricterPolicy,
            session_id: None,
            reason: Some("need higher budget".to_string()),
            required_adjustments: Some(serde_json::json!({"min_budget": 200})),
        };
        let json = serde_json::to_value(&stricter).unwrap();
        assert_eq!(json["status"], "request_stricter_policy");
        assert!(json["required_adjustments"].is_object());
    }

    #[test]
    fn remote_skill_response_serialization() {
        let resp = RemoteSkillResponse {
            skill_id: "file.list".to_string(),
            peer_id: "peer-1".to_string(),
            success: true,
            observation: serde_json::json!({"files": ["a.txt", "b.txt"]}),
            latency_ms: 42,
            timestamp: chrono::Utc::now(),
            trace_id: Uuid::new_v4(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["skill_id"], "file.list");
        assert_eq!(json["success"], true);
        assert!(json["trace_id"].is_string());
    }

    #[test]
    fn remote_resource_response_serialization() {
        let resp = RemoteResourceResponse {
            resource_type: "filesystem".to_string(),
            resource_id: "root".to_string(),
            data: serde_json::json!({"entries": 42}),
            data_mode: ResourceDataMode::Snapshot,
            version: 5,
            provenance: "local".to_string(),
            freshness_ms: 100,
            timestamp: chrono::Utc::now(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["resource_type"], "filesystem");
        assert_eq!(json["version"], 5);
        assert_eq!(json["freshness_ms"], 100);
        assert_eq!(json["data_mode"], "snapshot");
    }

    #[test]
    fn remote_goal_request_serialization() {
        let req = make_goal_request();
        let json = serde_json::to_value(&req).unwrap();
        assert!(json["goal"].is_object());
        assert_eq!(json["constraints"][0], "no_destructive");
        assert_eq!(json["budgets"]["risk_limit"], 0.5);
        assert_eq!(json["trust_required"], "verified");
        assert_eq!(json["request_result"], true);
        assert_eq!(json["request_trace"], false);
    }

    #[test]
    fn resource_data_mode_variants() {
        let snap = serde_json::to_value(ResourceDataMode::Snapshot).unwrap();
        let delta = serde_json::to_value(ResourceDataMode::Delta).unwrap();
        assert_eq!(snap, "snapshot");
        assert_eq!(delta, "delta");
    }

    // --- ValidatingRemoteExecutor tests ---

    use crate::distributed::auth::{DefaultPeerAuthenticator, PeerAuthenticator, PeerCredentials};
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

    fn make_skill_ad_with_risk(skill_id: &str, risk: RiskClass) -> RemoteSkillAd {
        let mut ad = make_skill_ad(skill_id);
        ad.risk_class = risk;
        ad
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
            exposed_skills: vec![make_skill_ad("file.list"), make_skill_ad("file.read")],
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

    fn setup_validating() -> (DefaultPeerRegistry, DefaultPeerAuthenticator) {
        let mut registry = DefaultPeerRegistry::new();
        registry.register_peer(make_test_peer("peer-1")).unwrap();
        // Elevate trust so it passes trust checks.
        registry
            .elevate_trust("peer-1", TrustLevel::Verified)
            .unwrap();

        let mut auth = DefaultPeerAuthenticator::new();
        auth.authenticate(
            "peer-1",
            &PeerCredentials {
                method: "token".to_string(),
                token: Some("valid-token".to_string()),
            },
        )
        .unwrap();

        (registry, auth)
    }

    #[test]
    fn validating_invoke_skill_rejects_unauthenticated() {
        let (registry, auth) = setup_validating();
        // Revoke authentication.
        let mut auth = auth;
        auth.revoke("peer-1");

        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        let result = exec.invoke_skill("peer-1", "file.list", serde_json::json!({}));
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::AuthenticationFailure);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn validating_invoke_skill_rejects_unadvertised_skill() {
        let (registry, auth) = setup_validating();

        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        let result = exec.invoke_skill("peer-1", "http.post", serde_json::json!({}));
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::UnsupportedSkill);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn validating_invoke_skill_forwards_when_all_checks_pass() {
        let (registry, auth) = setup_validating();

        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        // Inner executor returns PeerUnreachable (transport not wired),
        // which proves the call was forwarded past all validation checks.
        let result = exec.invoke_skill("peer-1", "file.list", serde_json::json!({}));
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::PeerUnreachable);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn validating_invoke_skill_rejects_destructive_with_insufficient_trust() {
        // Register a peer with a High-risk skill and only Restricted trust.
        let mut registry = DefaultPeerRegistry::new();
        let mut peer = make_test_peer("peer-2");
        peer.exposed_skills = vec![
            make_skill_ad("file.list"),
            make_skill_ad_with_risk("disk.format", RiskClass::High),
        ];
        registry.register_peer(peer).unwrap();
        // Only elevate to Restricted — below what destructive operations require.
        registry
            .elevate_trust("peer-2", TrustLevel::Restricted)
            .unwrap();

        let mut auth = DefaultPeerAuthenticator::new();
        auth.authenticate(
            "peer-2",
            &PeerCredentials {
                method: "token".to_string(),
                token: Some("valid".to_string()),
            },
        )
        .unwrap();

        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        let result = exec.invoke_skill("peer-2", "disk.format", serde_json::json!({}));
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, details } => {
                assert_eq!(failure, DistributedFailure::PolicyViolation);
                assert!(details.contains("destructive"));
                assert!(details.contains("Verified"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn validating_invoke_skill_rejects_critical_risk_with_insufficient_trust() {
        let mut registry = DefaultPeerRegistry::new();
        let mut peer = make_test_peer("peer-3");
        peer.exposed_skills = vec![make_skill_ad_with_risk("nuke.all", RiskClass::Critical)];
        registry.register_peer(peer).unwrap();
        registry
            .elevate_trust("peer-3", TrustLevel::Restricted)
            .unwrap();

        let mut auth = DefaultPeerAuthenticator::new();
        auth.authenticate(
            "peer-3",
            &PeerCredentials {
                method: "token".to_string(),
                token: Some("valid".to_string()),
            },
        )
        .unwrap();

        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        let result = exec.invoke_skill("peer-3", "nuke.all", serde_json::json!({}));
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, details } => {
                assert_eq!(failure, DistributedFailure::PolicyViolation);
                assert!(details.contains("Critical"));
                assert!(details.contains("destructive"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn validating_invoke_skill_allows_destructive_with_verified_trust() {
        // A peer with Verified trust should be allowed to invoke a High-risk skill.
        let mut registry = DefaultPeerRegistry::new();
        let mut peer = make_test_peer("peer-4");
        peer.exposed_skills = vec![make_skill_ad_with_risk("disk.format", RiskClass::High)];
        registry.register_peer(peer).unwrap();
        registry
            .elevate_trust("peer-4", TrustLevel::Verified)
            .unwrap();

        let mut auth = DefaultPeerAuthenticator::new();
        auth.authenticate(
            "peer-4",
            &PeerCredentials {
                method: "token".to_string(),
                token: Some("valid".to_string()),
            },
        )
        .unwrap();

        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        // Should pass destructive check and reach inner executor (PeerUnreachable).
        let result = exec.invoke_skill("peer-4", "disk.format", serde_json::json!({}));
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::PeerUnreachable);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn validating_invoke_skill_allows_low_risk_with_restricted_trust() {
        // Low-risk skills should pass with Restricted trust (no destructive check).
        let (registry, auth) = setup_validating();

        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        // file.list is Low risk — no PolicyViolation from the destructive check.
        let result = exec.invoke_skill("peer-1", "file.list", serde_json::json!({}));
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::PeerUnreachable);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn validating_transfer_schema_requires_verified_trust() {
        let mut registry = DefaultPeerRegistry::new();
        registry.register_peer(make_test_peer("peer-1")).unwrap();
        // Leave trust at Untrusted (registration default).

        let mut auth = DefaultPeerAuthenticator::new();
        auth.authenticate(
            "peer-1",
            &PeerCredentials {
                method: "token".to_string(),
                token: Some("valid".to_string()),
            },
        )
        .unwrap();

        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        let schema = SchemaTransfer {
            schema_id: "schema-1".to_string(),
            version: "1.0".to_string(),
            trigger_conditions: vec![],
            subgoal_structure: vec![],
            candidate_skill_ordering: vec![],
            stop_conditions: vec![],
            confidence: 0.9,
        };

        let result = exec.transfer_schema("peer-1", &schema);
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::TrustValidationFailure);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn validating_transfer_routine_requires_verified_trust() {
        let mut registry = DefaultPeerRegistry::new();
        registry.register_peer(make_test_peer("peer-1")).unwrap();
        // Leave at Untrusted.

        let mut auth = DefaultPeerAuthenticator::new();
        auth.authenticate(
            "peer-1",
            &PeerCredentials {
                method: "token".to_string(),
                token: Some("valid".to_string()),
            },
        )
        .unwrap();

        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        let routine = RoutineTransfer {
            routine_id: "routine-1".to_string(),
            match_conditions: vec![],
            compiled_skill_path: vec!["skill.a".to_string()],
            guard_conditions: vec![],
            expected_cost: 1.0,
            expected_effect: vec![],
            confidence: 0.95,
        };

        let result = exec.transfer_routine("peer-1", &routine);
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::TrustValidationFailure);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn validating_query_resource_rejects_unauthenticated() {
        let (registry, _) = setup_validating();
        let empty_auth = DefaultPeerAuthenticator::new();

        let exec = ValidatingRemoteExecutor::new(
            Box::new(DefaultRemoteExecutor::new()),
            &registry,
            &empty_auth,
        );

        let result = exec.query_resource("peer-1", "filesystem", "root");
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::AuthenticationFailure);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn validating_submit_goal_checks_trust() {
        let (registry, auth) = setup_validating();

        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        // Request requires Trusted but peer is only Verified.
        let mut req = make_goal_request();
        req.trust_required = TrustLevel::Trusted;

        let result = exec.submit_goal("peer-1", &req);
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::TrustValidationFailure);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn validating_submit_goal_forwards_when_trust_sufficient() {
        let (registry, auth) = setup_validating();

        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        let req = make_goal_request(); // requires Verified, peer is Verified
        let result = exec.submit_goal("peer-1", &req);
        // Forwarded to inner — gets PeerUnreachable since transport not wired.
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::PeerUnreachable);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn validating_rejects_unknown_peer() {
        let (registry, auth) = setup_validating();

        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        let result = exec.invoke_skill("ghost", "file.list", serde_json::json!({}));
        assert!(result.is_err());
        // Unknown peer is not authenticated.
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::AuthenticationFailure);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    // --- classify_failure tests ---

    use crate::types::peer::{FailureRecoverability, StructuredFailure};

    #[test]
    fn classify_delegatable_failures() {
        let delegatable = [
            DistributedFailure::PeerUnreachable,
            DistributedFailure::UnsupportedSkill,
            DistributedFailure::UnsupportedResource,
            DistributedFailure::DelegationRefusal,
        ];
        for f in &delegatable {
            assert_eq!(
                classify_failure(*f),
                FailureRecoverability::DelegatableToAnotherPeer,
                "expected DelegatableToAnotherPeer for {:?}",
                f
            );
        }
    }

    #[test]
    fn classify_retryable_failures() {
        let retryable = [
            DistributedFailure::TransportFailure,
            DistributedFailure::StaleData,
            DistributedFailure::Timeout,
            DistributedFailure::PartialObservationStream,
        ];
        for f in &retryable {
            assert_eq!(
                classify_failure(*f),
                FailureRecoverability::Retryable,
                "expected Retryable for {:?}",
                f
            );
        }
    }

    #[test]
    fn classify_terminal_for_session_failures() {
        let terminal_session = [
            DistributedFailure::AuthenticationFailure,
            DistributedFailure::BudgetExhaustion,
            DistributedFailure::MigrationFailure,
        ];
        for f in &terminal_session {
            assert_eq!(
                classify_failure(*f),
                FailureRecoverability::TerminalForSession,
                "expected TerminalForSession for {:?}",
                f
            );
        }
    }

    #[test]
    fn classify_terminal_for_action_failures() {
        let terminal_action = [
            DistributedFailure::AuthorizationFailure,
            DistributedFailure::TrustValidationFailure,
            DistributedFailure::ConflictingData,
            DistributedFailure::ReplayRejection,
            DistributedFailure::PolicyViolation,
        ];
        for f in &terminal_action {
            assert_eq!(
                classify_failure(*f),
                FailureRecoverability::TerminalForActionOnly,
                "expected TerminalForActionOnly for {:?}",
                f
            );
        }
    }

    #[test]
    fn classify_all_sixteen_variants_covered() {
        // Ensures every DistributedFailure variant produces a valid classification.
        let all = [
            DistributedFailure::PeerUnreachable,
            DistributedFailure::TransportFailure,
            DistributedFailure::AuthenticationFailure,
            DistributedFailure::AuthorizationFailure,
            DistributedFailure::TrustValidationFailure,
            DistributedFailure::UnsupportedSkill,
            DistributedFailure::UnsupportedResource,
            DistributedFailure::StaleData,
            DistributedFailure::ConflictingData,
            DistributedFailure::ReplayRejection,
            DistributedFailure::BudgetExhaustion,
            DistributedFailure::Timeout,
            DistributedFailure::PartialObservationStream,
            DistributedFailure::MigrationFailure,
            DistributedFailure::DelegationRefusal,
            DistributedFailure::PolicyViolation,
        ];
        for f in &all {
            let _ = classify_failure(*f); // must not panic
        }
        assert_eq!(all.len(), 16);
    }

    // --- build_structured_failure tests ---

    #[test]
    fn build_structured_failure_from_distributed_error() {
        let err = SomaError::Distributed {
            failure: DistributedFailure::Timeout,
            details: "connection timed out".to_string(),
        };
        let sf = build_structured_failure(&err);
        assert_eq!(sf.failure, DistributedFailure::Timeout);
        assert_eq!(sf.recoverability, FailureRecoverability::Retryable);
        assert_eq!(sf.details, "connection timed out");
    }

    #[test]
    fn build_structured_failure_from_non_distributed_error() {
        let err = SomaError::Skill("something broke".to_string());
        let sf = build_structured_failure(&err);
        assert_eq!(sf.failure, DistributedFailure::TransportFailure);
        assert_eq!(sf.recoverability, FailureRecoverability::Retryable);
        assert!(sf.details.contains("something broke"));
    }

    // --- RecoveryCoordinator tests ---

    fn make_structured(failure: DistributedFailure, details: &str) -> StructuredFailure {
        StructuredFailure {
            failure,
            recoverability: classify_failure(failure),
            details: details.to_string(),
        }
    }

    #[test]
    fn recovery_retryable_within_budget() {
        let coord = RecoveryCoordinator::new(3);
        let sf = make_structured(DistributedFailure::Timeout, "timed out");

        assert_eq!(
            coord.attempt_recovery(&sf, "peer-1", 0, None),
            RecoveryOutcome::RetryOnAnotherPeer
        );
        assert_eq!(
            coord.attempt_recovery(&sf, "peer-1", 2, None),
            RecoveryOutcome::RetryOnAnotherPeer
        );
    }

    #[test]
    fn recovery_retryable_budget_exhausted() {
        let coord = RecoveryCoordinator::new(3);
        let sf = make_structured(DistributedFailure::TransportFailure, "conn reset");

        assert_eq!(
            coord.attempt_recovery(&sf, "peer-1", 3, None),
            RecoveryOutcome::FallbackToLocal
        );
        assert_eq!(
            coord.attempt_recovery(&sf, "peer-1", 10, None),
            RecoveryOutcome::FallbackToLocal
        );
    }

    #[test]
    fn recovery_delegatable_always_delegates() {
        let coord = RecoveryCoordinator::new(3);
        let sf = make_structured(DistributedFailure::PeerUnreachable, "down");

        assert_eq!(
            coord.attempt_recovery(&sf, "peer-1", 0, None),
            RecoveryOutcome::DelegateToAlternate
        );
        // Delegation does not consume retry budget.
        assert_eq!(
            coord.attempt_recovery(&sf, "peer-1", 100, None),
            RecoveryOutcome::DelegateToAlternate
        );
    }

    #[test]
    fn recovery_terminal_for_session_aborts() {
        let coord = RecoveryCoordinator::new(3);
        let sf = make_structured(DistributedFailure::AuthenticationFailure, "bad creds");

        assert_eq!(
            coord.attempt_recovery(&sf, "peer-1", 0, None),
            RecoveryOutcome::Abort {
                terminal_for_session: true
            }
        );
    }

    #[test]
    fn recovery_terminal_for_action_aborts() {
        let coord = RecoveryCoordinator::new(3);
        let sf = make_structured(DistributedFailure::PolicyViolation, "forbidden");

        assert_eq!(
            coord.attempt_recovery(&sf, "peer-1", 0, None),
            RecoveryOutcome::Abort {
                terminal_for_session: false
            }
        );
    }

    #[test]
    fn recovery_zero_retry_budget_falls_back_immediately() {
        let coord = RecoveryCoordinator::new(0);
        let sf = make_structured(DistributedFailure::StaleData, "stale");

        assert_eq!(
            coord.attempt_recovery(&sf, "peer-1", 0, None),
            RecoveryOutcome::FallbackToLocal
        );
    }

    #[test]
    fn recovery_aborts_when_budget_exhausted() {
        let coord = RecoveryCoordinator::new(3);
        let sf = make_structured(DistributedFailure::Timeout, "timed out");

        // Budget at zero forces abort even though retries remain.
        assert_eq!(
            coord.attempt_recovery(&sf, "peer-1", 0, Some(0.0)),
            RecoveryOutcome::Abort {
                terminal_for_session: true
            }
        );
        // Negative budget also forces abort.
        assert_eq!(
            coord.attempt_recovery(&sf, "peer-1", 0, Some(-5.0)),
            RecoveryOutcome::Abort {
                terminal_for_session: true
            }
        );
        // Delegatable failure is also overridden by exhausted budget.
        let sf_del = make_structured(DistributedFailure::PeerUnreachable, "down");
        assert_eq!(
            coord.attempt_recovery(&sf_del, "peer-1", 0, Some(0.0)),
            RecoveryOutcome::Abort {
                terminal_for_session: true
            }
        );
        // Positive budget does not interfere with normal recovery logic.
        assert_eq!(
            coord.attempt_recovery(&sf, "peer-1", 0, Some(10.0)),
            RecoveryOutcome::RetryOnAnotherPeer
        );
    }

    // --- Staleness enforcement tests ---

    /// A mock executor that returns a resource response with a configurable freshness.
    struct MockFreshnessExecutor {
        freshness_ms: u64,
    }

    impl RemoteExecutor for MockFreshnessExecutor {
        fn submit_goal(
            &self,
            _peer_id: &str,
            _req: &RemoteGoalRequest,
        ) -> Result<RemoteGoalResponse> {
            Ok(RemoteGoalResponse {
                status: RemoteGoalStatus::Rejected,
                session_id: None,
                reason: Some("mock: submit_goal not supported by freshness executor".to_string()),
                required_adjustments: None,
            })
        }
        fn invoke_skill(
            &self,
            _peer_id: &str,
            skill_id: &str,
            _input: serde_json::Value,
        ) -> Result<RemoteSkillResponse> {
            Ok(RemoteSkillResponse {
                skill_id: skill_id.to_string(),
                peer_id: "mock".to_string(),
                success: false,
                observation: serde_json::json!({"error": "mock: invoke_skill not supported by freshness executor"}),
                latency_ms: 0,
                timestamp: chrono::Utc::now(),
                trace_id: uuid::Uuid::new_v4(),
            })
        }
        fn query_resource(
            &self,
            _peer_id: &str,
            resource_type: &str,
            resource_id: &str,
        ) -> Result<RemoteResourceResponse> {
            Ok(RemoteResourceResponse {
                resource_type: resource_type.to_string(),
                resource_id: resource_id.to_string(),
                data: serde_json::json!({"value": 42}),
                data_mode: ResourceDataMode::Snapshot,
                version: 1,
                provenance: "remote".to_string(),
                freshness_ms: self.freshness_ms,
                timestamp: chrono::Utc::now(),
            })
        }
        fn transfer_schema(&self, _peer_id: &str, _schema: &SchemaTransfer) -> Result<()> {
            Ok(())
        }
        fn transfer_routine(&self, _peer_id: &str, _routine: &RoutineTransfer) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn validating_query_resource_rejects_stale_data() {
        let (registry, auth) = setup_validating();

        let mut exec = ValidatingRemoteExecutor::new(
            Box::new(MockFreshnessExecutor {
                freshness_ms: 60_000,
            }),
            &registry,
            &auth,
        );
        exec.max_staleness_ms = 30_000;

        let result = exec.query_resource("peer-1", "filesystem", "root");
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, details } => {
                assert_eq!(failure, DistributedFailure::StaleData);
                assert!(details.contains("60000"));
                assert!(details.contains("30000"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn validating_query_resource_accepts_fresh_data() {
        let (registry, auth) = setup_validating();

        let mut exec = ValidatingRemoteExecutor::new(
            Box::new(MockFreshnessExecutor {
                freshness_ms: 5_000,
            }),
            &registry,
            &auth,
        );
        exec.max_staleness_ms = 30_000;

        let result = exec.query_resource("peer-1", "filesystem", "root");
        assert!(result.is_ok());
        let resp = result.unwrap();
        assert_eq!(resp.freshness_ms, 5_000);
    }

    #[test]
    fn validating_query_resource_accepts_at_boundary() {
        let (registry, auth) = setup_validating();

        let mut exec = ValidatingRemoteExecutor::new(
            Box::new(MockFreshnessExecutor {
                freshness_ms: 30_000,
            }),
            &registry,
            &auth,
        );
        exec.max_staleness_ms = 30_000;

        // Exactly at the boundary should pass (not strictly greater).
        let result = exec.query_resource("peer-1", "filesystem", "root");
        assert!(result.is_ok());
    }

    // --- Input binding validation tests ---

    fn make_skill_ad_with_required(skill_id: &str, required: Vec<&str>) -> RemoteSkillAd {
        RemoteSkillAd {
            skill_id: skill_id.to_string(),
            name: skill_id.to_string(),
            kind: "action".to_string(),
            inputs: SchemaRef {
                schema: serde_json::json!({
                    "type": "object",
                    "required": required,
                    "properties": {}
                }),
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

    fn make_test_peer_with_skills(id: &str, skills: Vec<RemoteSkillAd>) -> PeerSpec {
        PeerSpec {
            peer_id: id.to_string(),
            version: "0.1.0".to_string(),
            trust_class: TrustLevel::Verified,
            supported_transports: vec![Transport::Tcp],
            reachable_endpoints: vec!["127.0.0.1:9000".to_string()],
            current_availability: PeerAvailability::Available,
            policy_limits: vec![],
            exposed_packs: vec!["core".to_string()],
            exposed_skills: skills,
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
    fn validating_invoke_skill_rejects_null_input() {
        let mut registry = DefaultPeerRegistry::new();
        registry
            .register_peer(make_test_peer_with_skills(
                "peer-1",
                vec![make_skill_ad("file.list")],
            ))
            .unwrap();
        registry
            .elevate_trust("peer-1", TrustLevel::Verified)
            .unwrap();

        let mut auth = DefaultPeerAuthenticator::new();
        auth.authenticate(
            "peer-1",
            &PeerCredentials {
                method: "token".to_string(),
                token: Some("valid".to_string()),
            },
        )
        .unwrap();

        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        let result = exec.invoke_skill("peer-1", "file.list", serde_json::Value::Null);
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, details } => {
                assert_eq!(failure, DistributedFailure::PolicyViolation);
                assert!(details.contains("null"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn validating_invoke_skill_rejects_array_input() {
        let mut registry = DefaultPeerRegistry::new();
        registry
            .register_peer(make_test_peer_with_skills(
                "peer-1",
                vec![make_skill_ad("file.list")],
            ))
            .unwrap();
        registry
            .elevate_trust("peer-1", TrustLevel::Verified)
            .unwrap();

        let mut auth = DefaultPeerAuthenticator::new();
        auth.authenticate(
            "peer-1",
            &PeerCredentials {
                method: "token".to_string(),
                token: Some("valid".to_string()),
            },
        )
        .unwrap();

        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        let result = exec.invoke_skill("peer-1", "file.list", serde_json::json!(["a", "b"]));
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, details } => {
                assert_eq!(failure, DistributedFailure::PolicyViolation);
                assert!(details.contains("array"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn validating_invoke_skill_rejects_missing_required_fields() {
        let mut registry = DefaultPeerRegistry::new();
        registry
            .register_peer(make_test_peer_with_skills(
                "peer-1",
                vec![make_skill_ad_with_required(
                    "file.list",
                    vec!["path", "recursive"],
                )],
            ))
            .unwrap();
        registry
            .elevate_trust("peer-1", TrustLevel::Verified)
            .unwrap();

        let mut auth = DefaultPeerAuthenticator::new();
        auth.authenticate(
            "peer-1",
            &PeerCredentials {
                method: "token".to_string(),
                token: Some("valid".to_string()),
            },
        )
        .unwrap();

        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        // Provide only "path", missing "recursive".
        let result = exec.invoke_skill("peer-1", "file.list", serde_json::json!({"path": "/tmp"}));
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, details } => {
                assert_eq!(failure, DistributedFailure::PolicyViolation);
                assert!(details.contains("recursive"));
                assert!(!details.contains("path"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn validating_invoke_skill_accepts_valid_input_with_required_fields() {
        let mut registry = DefaultPeerRegistry::new();
        registry
            .register_peer(make_test_peer_with_skills(
                "peer-1",
                vec![make_skill_ad_with_required("file.list", vec!["path"])],
            ))
            .unwrap();
        registry
            .elevate_trust("peer-1", TrustLevel::Verified)
            .unwrap();

        let mut auth = DefaultPeerAuthenticator::new();
        auth.authenticate(
            "peer-1",
            &PeerCredentials {
                method: "token".to_string(),
                token: Some("valid".to_string()),
            },
        )
        .unwrap();

        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        // Input has the required "path" field — should pass validation.
        // Will get PeerUnreachable from the inner executor (transport not wired).
        let result = exec.invoke_skill("peer-1", "file.list", serde_json::json!({"path": "/tmp"}));
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::PeerUnreachable);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn validating_invoke_skill_accepts_empty_object_when_no_required_fields() {
        let (registry, auth) = setup_validating();

        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        // file.list has no required fields in test setup — empty object should pass.
        let result = exec.invoke_skill("peer-1", "file.list", serde_json::json!({}));
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::PeerUnreachable);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    // --- RemoteInvocationContext tests ---

    #[test]
    fn invoke_with_context_rejects_zero_budget() {
        let (registry, auth) = setup_validating();
        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        let ctx = RemoteInvocationContext {
            session_budget_remaining: Some(0.0),
            policy_allows: true,
        };
        let result =
            exec.invoke_skill_with_context("peer-1", "file.list", serde_json::json!({}), &ctx);
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, details } => {
                assert_eq!(failure, DistributedFailure::BudgetExhaustion);
                assert!(details.contains("file.list"));
                assert!(details.contains("peer-1"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn invoke_with_context_rejects_negative_budget() {
        let (registry, auth) = setup_validating();
        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        let ctx = RemoteInvocationContext {
            session_budget_remaining: Some(-5.0),
            policy_allows: true,
        };
        let result =
            exec.invoke_skill_with_context("peer-1", "file.list", serde_json::json!({}), &ctx);
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::BudgetExhaustion);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn invoke_with_context_rejects_policy_denied() {
        let (registry, auth) = setup_validating();
        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        let ctx = RemoteInvocationContext {
            session_budget_remaining: Some(100.0),
            policy_allows: false,
        };
        let result =
            exec.invoke_skill_with_context("peer-1", "file.list", serde_json::json!({}), &ctx);
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, details } => {
                assert_eq!(failure, DistributedFailure::PolicyViolation);
                assert!(details.contains("policy denies"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn invoke_with_context_forwards_when_budget_and_policy_ok() {
        let (registry, auth) = setup_validating();
        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        let ctx = RemoteInvocationContext {
            session_budget_remaining: Some(50.0),
            policy_allows: true,
        };
        let result =
            exec.invoke_skill_with_context("peer-1", "file.list", serde_json::json!({}), &ctx);
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::PeerUnreachable);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn invoke_with_context_none_budget_skips_budget_check() {
        let (registry, auth) = setup_validating();
        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        let ctx = RemoteInvocationContext {
            session_budget_remaining: None,
            policy_allows: true,
        };
        let result =
            exec.invoke_skill_with_context("peer-1", "file.list", serde_json::json!({}), &ctx);
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::PeerUnreachable);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn invoke_with_context_budget_checked_before_policy() {
        let (registry, auth) = setup_validating();
        let exec =
            ValidatingRemoteExecutor::new(Box::new(DefaultRemoteExecutor::new()), &registry, &auth);

        let ctx = RemoteInvocationContext {
            session_budget_remaining: Some(0.0),
            policy_allows: false,
        };
        let result =
            exec.invoke_skill_with_context("peer-1", "file.list", serde_json::json!({}), &ctx);
        assert!(result.is_err());
        match result.unwrap_err() {
            SomaError::Distributed { failure, .. } => {
                assert_eq!(failure, DistributedFailure::BudgetExhaustion);
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn remote_invocation_context_serialization() {
        let ctx = RemoteInvocationContext {
            session_budget_remaining: Some(42.5),
            policy_allows: true,
        };
        let json = serde_json::to_value(&ctx).unwrap();
        assert_eq!(json["session_budget_remaining"], 42.5);
        assert_eq!(json["policy_allows"], true);

        let ctx_none = RemoteInvocationContext {
            session_budget_remaining: None,
            policy_allows: false,
        };
        let json = serde_json::to_value(&ctx_none).unwrap();
        assert!(json["session_budget_remaining"].is_null());
        assert_eq!(json["policy_allows"], false);
    }
}
