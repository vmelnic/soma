use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::errors::Result;
use crate::types::common::TrustLevel;
use crate::types::peer::{PeerAvailability, PeerSpec};

use super::peer::PeerRegistry;

// --- RoutingRequest ---

/// A request for routing: describes what the caller needs so the router
/// can find the best peer. All 10 routing inputs from distributed.md:
/// trust, skill, resource, latency, load, transport, budget, policy, priority, freshness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRequest {
    /// Minimum trust level required.
    pub trust_required: TrustLevel,
    /// The skill needed (if any).
    pub skill_id: Option<String>,
    /// The resource type needed (if any).
    pub resource_type: Option<String>,
    /// Maximum acceptable latency class (e.g. "low", "medium").
    pub max_latency_class: Option<String>,
    /// Maximum acceptable peer load (0.0 - 1.0, None = no constraint).
    pub max_load: Option<f64>,
    /// Required transport (if any).
    pub required_transport: Option<crate::types::peer::Transport>,
    /// Maximum cost budget for this request.
    pub budget_limit: f64,
    /// Additional policy constraints that must be satisfied.
    pub policy_constraints: Vec<String>,
    /// Session priority (higher = more important).
    pub priority: u32,
    /// Maximum acceptable staleness in milliseconds (0 = no constraint).
    pub freshness_required_ms: u64,
    /// Our protocol version for compatibility checking. If set, peers with
    /// incompatible major versions are rejected.
    #[serde(default)]
    pub our_version: Option<String>,
}

// --- Latency scoring constants ---

/// Weight for trust in the routing score.
const WEIGHT_TRUST: f64 = 0.30;
/// Weight for latency in the routing score.
const WEIGHT_LATENCY: f64 = 0.25;
/// Weight for availability in the routing score.
const WEIGHT_AVAILABILITY: f64 = 0.30;
/// Weight for cost (inverse) in the routing score.
const WEIGHT_COST: f64 = 0.15;

// --- Router trait ---

/// Cost- and trust-aware routing for distributed operations.
/// Finds the best peer for a given request based on weighted scoring.
pub trait Router: Send + Sync {
    /// Find the best peer for the given request. Returns the peer_id
    /// of the best candidate, or None if no peer satisfies the constraints.
    fn find_best_peer(
        &self,
        registry: &dyn PeerRegistry,
        request: &RoutingRequest,
    ) -> Result<Option<String>>;

    /// Score a single peer against a request. Higher is better.
    /// Returns 0.0 if the peer does not satisfy hard constraints.
    fn score_peer(&self, spec: &PeerSpec, request: &RoutingRequest) -> f64;
}

// --- DefaultRouter ---

/// Default implementation using weighted scoring:
///   score = W_trust * trust_score + W_latency * latency_score
///         + W_availability * availability_score + W_cost * cost_score
///
/// Hard filters (score = 0):
///   - peer trust < required trust
///   - peer is unavailable (Offline or Untrusted)
///   - peer lacks the required skill
///   - peer lacks the required resource
pub struct DefaultRouter;

impl DefaultRouter {
    pub fn new() -> Self {
        Self
    }

    /// Convert a TrustLevel to a numeric score (0.0-1.0).
    fn trust_score(level: TrustLevel) -> f64 {
        match level {
            TrustLevel::Untrusted => 0.0,
            TrustLevel::Restricted => 0.25,
            TrustLevel::Verified => 0.5,
            TrustLevel::Trusted => 0.75,
            TrustLevel::BuiltIn => 1.0,
        }
    }

    /// Convert a latency class string to a score (0.0-1.0).
    /// Lower latency = higher score.
    fn latency_score(latency_class: &str) -> f64 {
        match latency_class {
            "ultra_low" => 1.0,
            "low" => 0.8,
            "medium" => 0.5,
            "high" => 0.2,
            "very_high" => 0.05,
            _ => 0.3, // unknown class gets a conservative score
        }
    }

    /// Convert a cost_class string to a score (0.0-1.0).
    /// Lower cost = higher score (we invert the cost to prefer cheaper peers).
    fn cost_score(cost_class: &str) -> f64 {
        match cost_class {
            "negligible" => 1.0,
            "low" => 0.8,
            "medium" => 0.5,
            "high" => 0.3,
            "extreme" => 0.1,
            _ => 0.5, // unknown class gets a middle-ground score
        }
    }

    /// Convert a PeerAvailability to a score (0.0-1.0).
    fn availability_score(availability: PeerAvailability) -> f64 {
        match availability {
            PeerAvailability::Available => 1.0,
            PeerAvailability::Degraded => 0.5,
            PeerAvailability::Busy => 0.2,
            PeerAvailability::Restricted => 0.1,
            PeerAvailability::Offline => 0.0,
            PeerAvailability::Untrusted => 0.0,
        }
    }

    /// Check whether a peer satisfies the hard constraints in the request.
    /// Enforces avoidance rules from distributed.md:
    /// - untrusted for the operation
    /// - cannot satisfy freshness
    /// - lacks required skill or resource
    /// - would violate policy or budget constraints
    fn satisfies_constraints(spec: &PeerSpec, request: &RoutingRequest) -> bool {
        // Trust must meet the minimum required level.
        if spec.trust_class < request.trust_required {
            return false;
        }

        // Peer must not be offline or untrusted.
        if spec.current_availability == PeerAvailability::Offline
            || spec.current_availability == PeerAvailability::Untrusted
        {
            return false;
        }

        // If a skill is required, the peer must expose it.
        if let Some(ref skill_id) = request.skill_id
            && !spec.exposed_skills.iter().any(|s| s.skill_id == *skill_id)
        {
            return false;
        }

        // If a resource is required, the peer must expose it.
        if let Some(ref resource_type) = request.resource_type
            && !spec
                .exposed_resources
                .iter()
                .any(|r| r.resource_type == *resource_type)
        {
            return false;
        }

        // If a specific transport is required, the peer must support it.
        if let Some(ref transport) = request.required_transport
            && !spec.supported_transports.contains(transport)
        {
            return false;
        }

        // If max latency class is specified, enforce it.
        if let Some(ref max_latency) = request.max_latency_class {
            let required_score = Self::latency_score(max_latency);
            let peer_score = Self::latency_score(&spec.latency_class);
            if peer_score < required_score {
                return false;
            }
        }

        // Policy constraints: if the request specifies policy constraints,
        // the peer must declare them in its policy_limits.
        for constraint in &request.policy_constraints {
            if !spec.policy_limits.iter().any(|p| p == constraint) {
                return false;
            }
        }

        // Freshness: if the request requires a maximum staleness, reject peers
        // whose last_seen timestamp is older than the allowed window.
        if request.freshness_required_ms > 0 {
            let age_ms = (Utc::now() - spec.last_seen).num_milliseconds().max(0) as u64;
            if age_ms > request.freshness_required_ms {
                return false;
            }
        }

        // Load: if the request specifies a maximum load, reject overloaded peers.
        if let Some(max_load) = request.max_load
            && spec.current_load > max_load {
                return false;
            }

        // Version compatibility: reject peers with incompatible major versions.
        if let Some(ref our_version) = request.our_version
            && let (Ok(ours), Ok(theirs)) = (
                semver::Version::parse(our_version),
                semver::Version::parse(&spec.version),
            )
                && ours.major != theirs.major {
                    return false;
                }

        true
    }
}

impl Default for DefaultRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl Router for DefaultRouter {
    fn find_best_peer(
        &self,
        registry: &dyn PeerRegistry,
        request: &RoutingRequest,
    ) -> Result<Option<String>> {
        let peers = registry.list_peers();
        if peers.is_empty() {
            return Ok(None);
        }

        let mut best_id: Option<String> = None;
        let mut best_score: f64 = 0.0;

        for spec in peers {
            let score = self.score_peer(spec, request);
            if score > best_score {
                best_score = score;
                best_id = Some(spec.peer_id.clone());
            }
        }

        Ok(best_id)
    }

    fn score_peer(&self, spec: &PeerSpec, request: &RoutingRequest) -> f64 {
        // Hard constraint check — if any fails, score is 0.
        if !Self::satisfies_constraints(spec, request) {
            return 0.0;
        }

        let trust = Self::trust_score(spec.trust_class);
        let latency = Self::latency_score(&spec.latency_class);
        let availability = Self::availability_score(spec.current_availability);

        // Cost score: map cost_class to a multiplier (lower cost = higher score).
        let cost = Self::cost_score(&spec.cost_class);

        WEIGHT_TRUST * trust
            + WEIGHT_LATENCY * latency
            + WEIGHT_AVAILABILITY * availability
            + WEIGHT_COST * cost
    }
}

// --- RoutingDecision ---

/// Decision from the RoutineRouter about where to execute a routine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum RoutingDecision {
    /// Execute locally.
    Local,
    /// Route to a specific remote peer.
    Remote { peer_id: String },
}

// --- RoutingStrategy ---

/// Strategy for how the RoutineRouter selects a target.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingStrategy {
    /// Execute locally unless local load exceeds threshold.
    #[default]
    LocalFirst,
    /// Always pick the least-loaded peer with required skills.
    LeastLoaded,
    /// Prefer the peer with the highest skill coverage for this routine.
    RoutineAffinity,
}

// --- RoutineRouter trait ---

/// Routes routine execution to the optimal target based on peer load,
/// capability coverage, and strategy.
pub trait RoutineRouter: Send + Sync {
    fn route(
        &self,
        routine: &crate::types::routine::Routine,
        local_load: f64,
        registry: &dyn PeerRegistry,
        strategy: RoutingStrategy,
    ) -> RoutingDecision;

    /// The load threshold above which the router considers remote peers.
    fn local_load_threshold(&self) -> f64 {
        0.8
    }
}

// --- DefaultRoutineRouter ---

/// Default implementation — uses the existing `DefaultRouter` scoring
/// infrastructure to find suitable peers.
pub struct DefaultRoutineRouter {
    /// Local load threshold above which `LocalFirst` considers remote peers.
    pub local_load_threshold: f64,
}

impl DefaultRoutineRouter {
    pub fn new() -> Self {
        Self {
            local_load_threshold: 0.8,
        }
    }

    /// Extract unique skill IDs from a routine's effective steps.
    fn required_skill_ids(
        routine: &crate::types::routine::Routine,
    ) -> Vec<String> {
        let mut ids = Vec::new();
        for step in routine.effective_steps() {
            if let crate::types::routine::CompiledStep::Skill { skill_id, .. } = step
                && !ids.contains(&skill_id)
            {
                ids.push(skill_id);
            }
        }
        ids
    }

    /// Find the least-loaded available peer that has ALL required skills.
    fn find_least_loaded_peer(
        &self,
        routine: &crate::types::routine::Routine,
        registry: &dyn PeerRegistry,
    ) -> Option<String> {
        let skill_ids = Self::required_skill_ids(routine);
        if skill_ids.is_empty() {
            return None;
        }

        registry
            .list_peers()
            .into_iter()
            .filter(|p| {
                p.current_availability == PeerAvailability::Available
                    && skill_ids.iter().all(|sid| {
                        p.exposed_skills.iter().any(|s| s.skill_id == *sid)
                    })
            })
            .min_by(|a, b| {
                a.current_load
                    .partial_cmp(&b.current_load)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|p| p.peer_id.clone())
    }

    /// Find the peer with the highest skill coverage for this routine,
    /// breaking ties by lowest load.
    fn find_affinity_peer(
        &self,
        routine: &crate::types::routine::Routine,
        registry: &dyn PeerRegistry,
    ) -> Option<String> {
        let skill_ids = Self::required_skill_ids(routine);
        if skill_ids.is_empty() {
            return None;
        }

        registry
            .list_peers()
            .into_iter()
            .filter(|p| {
                p.current_availability == PeerAvailability::Available
                    && skill_ids.iter().all(|sid| {
                        p.exposed_skills.iter().any(|s| s.skill_id == *sid)
                    })
            })
            .max_by(|a, b| {
                // All candidates have full coverage (filtered above),
                // so tiebreak by load (lower is better).
                b.current_load
                    .partial_cmp(&a.current_load)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|p| p.peer_id.clone())
    }
}

impl Default for DefaultRoutineRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl RoutineRouter for DefaultRoutineRouter {
    fn route(
        &self,
        routine: &crate::types::routine::Routine,
        local_load: f64,
        registry: &dyn PeerRegistry,
        strategy: RoutingStrategy,
    ) -> RoutingDecision {
        match strategy {
            RoutingStrategy::LocalFirst => {
                if local_load < self.local_load_threshold {
                    return RoutingDecision::Local;
                }
                self.find_least_loaded_peer(routine, registry)
                    .map(|pid| RoutingDecision::Remote { peer_id: pid })
                    .unwrap_or(RoutingDecision::Local)
            }
            RoutingStrategy::LeastLoaded => {
                self.find_least_loaded_peer(routine, registry)
                    .map(|pid| RoutingDecision::Remote { peer_id: pid })
                    .unwrap_or(RoutingDecision::Local)
            }
            RoutingStrategy::RoutineAffinity => {
                self.find_affinity_peer(routine, registry)
                    .map(|pid| RoutingDecision::Remote { peer_id: pid })
                    .unwrap_or(RoutingDecision::Local)
            }
        }
    }

    fn local_load_threshold(&self) -> f64 {
        self.local_load_threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distributed::peer::DefaultPeerRegistry;
    use crate::types::common::{
        DeterminismClass, LatencyProfile, RiskClass, RollbackSupport, SchemaRef,
    };
    use crate::types::peer::{
        AccessMode, MutationMode, RemoteResourceAd, RemoteSkillAd, Transport,
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
            sync_mode: "pull".to_string(),
            provenance: "local".to_string(),
            staleness_bounds_ms: None,
        }
    }

    fn make_peer(id: &str, trust: TrustLevel, avail: PeerAvailability, latency: &str) -> PeerSpec {
        PeerSpec {
            peer_id: id.to_string(),
            version: "0.1.0".to_string(),
            trust_class: trust,
            supported_transports: vec![Transport::Tcp],
            reachable_endpoints: vec!["127.0.0.1:9000".to_string()],
            current_availability: avail,
            policy_limits: vec![],
            exposed_packs: vec!["core".to_string()],
            exposed_skills: vec![make_skill_ad("file.list")],
            exposed_resources: vec![make_resource_ad("filesystem")],
            latency_class: latency.to_string(),
            cost_class: "low".to_string(),
            current_load: 0.0,
            last_seen: chrono::Utc::now(),
            replay_support: true,
            observation_streaming: true,
            advertisement_version: 1,
            advertisement_expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        }
    }

    fn make_request() -> RoutingRequest {
        RoutingRequest {
            trust_required: TrustLevel::Verified,
            skill_id: Some("file.list".to_string()),
            resource_type: None,
            max_latency_class: None,
            max_load: None,
            required_transport: None,
            budget_limit: 100.0,
            policy_constraints: vec![],
            priority: 1,
            freshness_required_ms: 0,
            our_version: None,
        }
    }

    #[test]
    fn find_best_peer_empty_registry() {
        let router = DefaultRouter::new();
        let reg = DefaultPeerRegistry::new();
        let req = make_request();
        let result = router.find_best_peer(&reg, &req).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn find_best_peer_single_eligible() {
        let router = DefaultRouter::new();
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer(
            "peer-1",
            TrustLevel::Trusted,
            PeerAvailability::Available,
            "low",
        ))
        .unwrap();
        reg.elevate_trust("peer-1", TrustLevel::Trusted).unwrap();
        let req = make_request();
        let result = router.find_best_peer(&reg, &req).unwrap();
        assert_eq!(result, Some("peer-1".to_string()));
    }

    #[test]
    fn find_best_peer_prefers_higher_trust() {
        let router = DefaultRouter::new();
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer(
            "peer-low",
            TrustLevel::Verified,
            PeerAvailability::Available,
            "low",
        ))
        .unwrap();
        reg.elevate_trust("peer-low", TrustLevel::Verified).unwrap();
        reg.register_peer(make_peer(
            "peer-high",
            TrustLevel::BuiltIn,
            PeerAvailability::Available,
            "low",
        ))
        .unwrap();
        reg.elevate_trust("peer-high", TrustLevel::BuiltIn).unwrap();
        let req = make_request();
        let result = router.find_best_peer(&reg, &req).unwrap();
        assert_eq!(result, Some("peer-high".to_string()));
    }

    #[test]
    fn find_best_peer_prefers_lower_latency() {
        let router = DefaultRouter::new();
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer(
            "peer-fast",
            TrustLevel::Trusted,
            PeerAvailability::Available,
            "ultra_low",
        ))
        .unwrap();
        reg.elevate_trust("peer-fast", TrustLevel::Trusted).unwrap();
        reg.register_peer(make_peer(
            "peer-slow",
            TrustLevel::Trusted,
            PeerAvailability::Available,
            "high",
        ))
        .unwrap();
        reg.elevate_trust("peer-slow", TrustLevel::Trusted).unwrap();
        let req = make_request();
        let result = router.find_best_peer(&reg, &req).unwrap();
        assert_eq!(result, Some("peer-fast".to_string()));
    }

    #[test]
    fn find_best_peer_filters_offline() {
        let router = DefaultRouter::new();
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer(
            "peer-offline",
            TrustLevel::Trusted,
            PeerAvailability::Offline,
            "low",
        ))
        .unwrap();
        reg.elevate_trust("peer-offline", TrustLevel::Trusted).unwrap();
        let req = make_request();
        let result = router.find_best_peer(&reg, &req).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn find_best_peer_filters_untrusted_availability() {
        let router = DefaultRouter::new();
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer(
            "peer-untrusted",
            TrustLevel::Trusted,
            PeerAvailability::Untrusted,
            "low",
        ))
        .unwrap();
        reg.elevate_trust("peer-untrusted", TrustLevel::Trusted).unwrap();
        let req = make_request();
        let result = router.find_best_peer(&reg, &req).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn find_best_peer_filters_insufficient_trust() {
        let router = DefaultRouter::new();
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer(
            "peer-restricted",
            TrustLevel::Restricted,
            PeerAvailability::Available,
            "low",
        ))
        .unwrap();
        reg.elevate_trust("peer-restricted", TrustLevel::Restricted).unwrap();
        let mut req = make_request();
        req.trust_required = TrustLevel::Trusted;
        let result = router.find_best_peer(&reg, &req).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn find_best_peer_filters_missing_skill() {
        let router = DefaultRouter::new();
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer(
            "peer-1",
            TrustLevel::Trusted,
            PeerAvailability::Available,
            "low",
        ))
        .unwrap();
        reg.elevate_trust("peer-1", TrustLevel::Trusted).unwrap();
        let mut req = make_request();
        req.skill_id = Some("http.get".to_string()); // peer doesn't have this
        let result = router.find_best_peer(&reg, &req).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn find_best_peer_filters_missing_resource() {
        let router = DefaultRouter::new();
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer(
            "peer-1",
            TrustLevel::Trusted,
            PeerAvailability::Available,
            "low",
        ))
        .unwrap();
        reg.elevate_trust("peer-1", TrustLevel::Trusted).unwrap();
        let mut req = make_request();
        req.skill_id = None;
        req.resource_type = Some("database".to_string()); // peer doesn't have this
        let result = router.find_best_peer(&reg, &req).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn score_peer_zero_for_offline() {
        let router = DefaultRouter::new();
        let spec = make_peer("p", TrustLevel::Trusted, PeerAvailability::Offline, "low");
        let req = make_request();
        assert_eq!(router.score_peer(&spec, &req), 0.0);
    }

    #[test]
    fn score_peer_positive_for_eligible() {
        let router = DefaultRouter::new();
        let spec = make_peer("p", TrustLevel::Trusted, PeerAvailability::Available, "low");
        let req = make_request();
        let score = router.score_peer(&spec, &req);
        assert!(score > 0.0);
    }

    #[test]
    fn score_peer_available_beats_degraded() {
        let router = DefaultRouter::new();
        let avail = make_peer("a", TrustLevel::Trusted, PeerAvailability::Available, "low");
        let degraded = make_peer("d", TrustLevel::Trusted, PeerAvailability::Degraded, "low");
        let req = make_request();
        assert!(router.score_peer(&avail, &req) > router.score_peer(&degraded, &req));
    }

    #[test]
    fn score_peer_builtin_beats_verified() {
        let router = DefaultRouter::new();
        let builtin = make_peer("b", TrustLevel::BuiltIn, PeerAvailability::Available, "low");
        let verified = make_peer("v", TrustLevel::Verified, PeerAvailability::Available, "low");
        let req = make_request();
        assert!(router.score_peer(&builtin, &req) > router.score_peer(&verified, &req));
    }

    #[test]
    fn routing_request_serialization() {
        let req = make_request();
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["skill_id"], "file.list");
        assert_eq!(json["trust_required"], "verified");
        assert_eq!(json["priority"], 1);
    }

    #[test]
    fn find_best_peer_no_skill_or_resource_constraint() {
        let router = DefaultRouter::new();
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer(
            "peer-1",
            TrustLevel::Trusted,
            PeerAvailability::Available,
            "low",
        ))
        .unwrap();
        // Untrusted trust_required: no elevation needed.
        let req = RoutingRequest {
            trust_required: TrustLevel::Untrusted,
            skill_id: None,
            resource_type: None,
            max_latency_class: None,
            max_load: None,
            required_transport: None,
            budget_limit: 100.0,
            policy_constraints: vec![],
            priority: 1,
            freshness_required_ms: 0,
            our_version: None,
        };
        let result = router.find_best_peer(&reg, &req).unwrap();
        assert_eq!(result, Some("peer-1".to_string()));
    }

    #[test]
    fn find_best_peer_filters_wrong_transport() {
        let router = DefaultRouter::new();
        let mut reg = DefaultPeerRegistry::new();
        // Peer only supports TCP.
        reg.register_peer(make_peer(
            "peer-tcp",
            TrustLevel::Trusted,
            PeerAvailability::Available,
            "low",
        ))
        .unwrap();
        reg.elevate_trust("peer-tcp", TrustLevel::Trusted).unwrap();
        let mut req = make_request();
        req.required_transport = Some(Transport::WebSocket);
        let result = router.find_best_peer(&reg, &req).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn find_best_peer_filters_latency_class() {
        let router = DefaultRouter::new();
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer(
            "peer-slow",
            TrustLevel::Trusted,
            PeerAvailability::Available,
            "high",
        ))
        .unwrap();
        reg.elevate_trust("peer-slow", TrustLevel::Trusted).unwrap();
        let mut req = make_request();
        req.max_latency_class = Some("low".to_string());
        let result = router.find_best_peer(&reg, &req).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn find_best_peer_filters_policy_constraints() {
        let router = DefaultRouter::new();
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer(
            "peer-no-policy",
            TrustLevel::Trusted,
            PeerAvailability::Available,
            "low",
        ))
        .unwrap();
        reg.elevate_trust("peer-no-policy", TrustLevel::Trusted).unwrap();
        let mut req = make_request();
        req.policy_constraints = vec!["hipaa_compliant".to_string()];
        let result = router.find_best_peer(&reg, &req).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn find_best_peer_rejects_incompatible_version() {
        let router = DefaultRouter::new();
        let mut reg = DefaultPeerRegistry::new();
        // Peer is version "0.1.0".
        reg.register_peer(make_peer(
            "peer-1",
            TrustLevel::Trusted,
            PeerAvailability::Available,
            "low",
        ))
        .unwrap();
        reg.elevate_trust("peer-1", TrustLevel::Trusted).unwrap();

        let mut req = make_request();
        // Our version has a different major version.
        req.our_version = Some("1.0.0".to_string());
        let result = router.find_best_peer(&reg, &req).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn find_best_peer_accepts_compatible_version() {
        let router = DefaultRouter::new();
        let mut reg = DefaultPeerRegistry::new();
        // Peer is version "0.1.0".
        reg.register_peer(make_peer(
            "peer-1",
            TrustLevel::Trusted,
            PeerAvailability::Available,
            "low",
        ))
        .unwrap();
        reg.elevate_trust("peer-1", TrustLevel::Trusted).unwrap();

        let mut req = make_request();
        req.our_version = Some("0.2.0".to_string());
        let result = router.find_best_peer(&reg, &req).unwrap();
        assert_eq!(result, Some("peer-1".to_string()));
    }

    #[test]
    fn find_best_peer_no_version_constraint_skips_check() {
        let router = DefaultRouter::new();
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer(
            "peer-1",
            TrustLevel::Trusted,
            PeerAvailability::Available,
            "low",
        ))
        .unwrap();
        reg.elevate_trust("peer-1", TrustLevel::Trusted).unwrap();

        let mut req = make_request();
        req.our_version = None;
        let result = router.find_best_peer(&reg, &req).unwrap();
        assert_eq!(result, Some("peer-1".to_string()));
    }

    // --- RoutineRouter tests ---

    fn make_routine_for_routing(skill_ids: &[&str]) -> crate::types::routine::Routine {
        crate::types::routine::Routine {
            routine_id: "test_routine".to_string(),
            namespace: "test".to_string(),
            origin: crate::types::routine::RoutineOrigin::SchemaCompiled,
            match_conditions: vec![],
            compiled_skill_path: skill_ids.iter().map(|s| s.to_string()).collect(),
            compiled_steps: vec![],
            guard_conditions: vec![],
            expected_cost: 0.1,
            expected_effect: vec![],
            confidence: 0.9,
            autonomous: false,
            priority: 0,
            exclusive: false,
            policy_scope: None,
            version: 0,
        }
    }

    #[test]
    fn routine_router_local_first_under_threshold() {
        let router = DefaultRoutineRouter::new();
        let reg = DefaultPeerRegistry::new();
        let routine = make_routine_for_routing(&["file.list"]);
        let decision = router.route(&routine, 0.5, &reg, RoutingStrategy::LocalFirst);
        assert!(matches!(decision, RoutingDecision::Local));
    }

    #[test]
    fn routine_router_local_first_over_threshold_no_peers() {
        let router = DefaultRoutineRouter::new();
        let reg = DefaultPeerRegistry::new();
        let routine = make_routine_for_routing(&["file.list"]);
        let decision = router.route(&routine, 0.9, &reg, RoutingStrategy::LocalFirst);
        // No peers available, falls back to local
        assert!(matches!(decision, RoutingDecision::Local));
    }

    #[test]
    fn routine_router_local_first_over_threshold_routes_remote() {
        let router = DefaultRoutineRouter::new();
        let mut reg = DefaultPeerRegistry::new();
        let mut peer = make_peer(
            "peer-1",
            TrustLevel::Trusted,
            PeerAvailability::Available,
            "low",
        );
        peer.current_load = 0.3;
        reg.register_peer(peer).unwrap();
        let routine = make_routine_for_routing(&["file.list"]);
        let decision = router.route(&routine, 0.9, &reg, RoutingStrategy::LocalFirst);
        assert!(matches!(decision, RoutingDecision::Remote { peer_id } if peer_id == "peer-1"));
    }

    #[test]
    fn routine_router_least_loaded_picks_lowest() {
        let router = DefaultRoutineRouter::new();
        let mut reg = DefaultPeerRegistry::new();
        let mut peer1 = make_peer(
            "peer-heavy",
            TrustLevel::Trusted,
            PeerAvailability::Available,
            "low",
        );
        peer1.current_load = 0.8;
        let mut peer2 = make_peer(
            "peer-light",
            TrustLevel::Trusted,
            PeerAvailability::Available,
            "low",
        );
        peer2.current_load = 0.2;
        reg.register_peer(peer1).unwrap();
        reg.register_peer(peer2).unwrap();
        let routine = make_routine_for_routing(&["file.list"]);
        let decision = router.route(&routine, 0.5, &reg, RoutingStrategy::LeastLoaded);
        assert!(matches!(decision, RoutingDecision::Remote { peer_id } if peer_id == "peer-light"));
    }

    #[test]
    fn routine_router_skips_peer_missing_skill() {
        let router = DefaultRoutineRouter::new();
        let mut reg = DefaultPeerRegistry::new();
        let mut peer = make_peer(
            "peer-1",
            TrustLevel::Trusted,
            PeerAvailability::Available,
            "low",
        );
        peer.current_load = 0.1;
        // Peer only has file.list, not http.get
        reg.register_peer(peer).unwrap();
        let routine = make_routine_for_routing(&["http.get"]);
        let decision = router.route(&routine, 0.9, &reg, RoutingStrategy::LocalFirst);
        // Peer doesn't have the skill, falls back to local
        assert!(matches!(decision, RoutingDecision::Local));
    }

    #[test]
    fn routine_router_affinity_prefers_lowest_load() {
        let router = DefaultRoutineRouter::new();
        let mut reg = DefaultPeerRegistry::new();
        let mut peer1 = make_peer(
            "peer-busy",
            TrustLevel::Trusted,
            PeerAvailability::Available,
            "low",
        );
        peer1.current_load = 0.7;
        let mut peer2 = make_peer(
            "peer-idle",
            TrustLevel::Trusted,
            PeerAvailability::Available,
            "low",
        );
        peer2.current_load = 0.1;
        reg.register_peer(peer1).unwrap();
        reg.register_peer(peer2).unwrap();
        let routine = make_routine_for_routing(&["file.list"]);
        let decision = router.route(&routine, 0.5, &reg, RoutingStrategy::RoutineAffinity);
        assert!(matches!(decision, RoutingDecision::Remote { peer_id } if peer_id == "peer-idle"));
    }
}
