use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::errors::{Result, SomaError};
use crate::types::common::TrustLevel;
use crate::types::peer::{PeerAvailability, PeerSpec, RemoteResourceAd, RemoteSkillAd, Transport};

// --- VersionCompatibility ---

/// Result of checking protocol version compatibility between two peers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VersionCompatibility {
    /// Same major version — fully compatible.
    Compatible,
    /// Same major version but different minor — backward compatible.
    BackwardCompatible,
    /// Different major versions — cannot interoperate.
    Incompatible { reason: String },
}

// --- PeerAdvertisement ---

/// Formatted capability advertisement for a peer — 11 required fields from distributed.md:
/// peer identity, trust class, supported transports, packs loaded, skills exposed,
/// resources exposed, policy constraints, cost/latency profile, current availability,
/// replay support, observation streaming support.
///
/// Advertisements MUST be versioned and cacheable with expiration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerAdvertisement {
    pub peer_id: String,
    pub version: String,
    pub trust_class: TrustLevel,
    pub supported_transports: Vec<Transport>,
    pub reachable_endpoints: Vec<String>,
    pub current_availability: PeerAvailability,
    pub packs_loaded: Vec<String>,
    pub skills_exposed: Vec<RemoteSkillAd>,
    pub resources_exposed: Vec<RemoteResourceAd>,
    pub policy_constraints: Vec<String>,
    pub cost_latency_profile: CostLatencyProfile,
    /// Current load on this peer (0.0 = idle, 1.0 = fully loaded).
    /// Populated from PeerSpec.current_load so remote peers can make
    /// load-aware routing decisions.
    #[serde(default)]
    pub current_load: f64,
    pub replay_support: bool,
    pub observation_streaming: bool,
    /// Versioned for cacheability.
    pub advertisement_version: u64,
    /// Expiry timestamp — advertisements MUST be cacheable with expiration.
    pub expires_at: DateTime<Utc>,
}

/// Cost/latency profile for the peer advertisement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostLatencyProfile {
    pub latency_class: String,
    pub cost_class: String,
}

impl PeerAdvertisement {
    /// Build an advertisement from a PeerSpec.
    pub fn from_spec(spec: &PeerSpec) -> Self {
        Self {
            peer_id: spec.peer_id.clone(),
            version: spec.version.clone(),
            trust_class: spec.trust_class,
            supported_transports: spec.supported_transports.clone(),
            reachable_endpoints: spec.reachable_endpoints.clone(),
            current_availability: spec.current_availability,
            packs_loaded: spec.exposed_packs.clone(),
            skills_exposed: spec.exposed_skills.clone(),
            resources_exposed: spec.exposed_resources.clone(),
            policy_constraints: spec.policy_limits.clone(),
            cost_latency_profile: CostLatencyProfile {
                latency_class: spec.latency_class.clone(),
                cost_class: spec.cost_class.clone(),
            },
            current_load: spec.current_load,
            replay_support: spec.replay_support,
            observation_streaming: spec.observation_streaming,
            advertisement_version: spec.advertisement_version,
            expires_at: spec.advertisement_expires_at,
        }
    }

    /// Check whether this advertisement has expired.
    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }
}

// --- PeerRegistry trait ---

/// The Peer Registry: manages known peers, their specs, and availability.
pub trait PeerRegistry: Send + Sync {
    /// Register a new peer. Returns error if peer_id already exists.
    fn register_peer(&mut self, spec: PeerSpec) -> Result<()>;

    /// Remove a peer by id. Returns error if not found.
    fn remove_peer(&mut self, peer_id: &str) -> Result<()>;

    /// Get a peer spec by id.
    fn get_peer(&self, peer_id: &str) -> Option<&PeerSpec>;

    /// List all known peers.
    fn list_peers(&self) -> Vec<&PeerSpec>;

    /// Update availability for a peer. Returns error if peer not found.
    fn update_availability(&mut self, peer_id: &str, availability: PeerAvailability) -> Result<()>;

    /// Get the formatted capability advertisement for a peer.
    fn get_advertisement(&self, peer_id: &str) -> Option<PeerAdvertisement>;

    /// Find all peers that expose a given skill.
    fn find_peers_with_skill(&self, skill_id: &str) -> Vec<&PeerSpec>;

    /// Find all peers that expose a given resource type.
    fn find_peers_with_resource(&self, resource_type: &str) -> Vec<&PeerSpec>;

    /// Elevate a peer's trust class. Only succeeds if the peer is registered
    /// and the caller has verified authentication. This is the only way to
    /// raise trust above `Untrusted` after initial registration.
    fn elevate_trust(&mut self, peer_id: &str, new_trust: TrustLevel) -> Result<()>;

    /// Check whether the given peer's version is compatible with ours.
    /// Uses semver: same major = compatible (different minor = backward compatible),
    /// different major = incompatible.
    fn check_version_compatibility(
        &self,
        peer_id: &str,
        our_version: &str,
    ) -> Result<VersionCompatibility>;

    /// Update the last_seen timestamp for a peer. Returns error if peer not found.
    fn update_last_seen(&mut self, peer_id: &str, timestamp: DateTime<Utc>) -> Result<()>;

    /// Update the current load for a peer. Returns error if peer not found.
    fn update_load(&mut self, peer_id: &str, load: f64) -> Result<()>;

    /// List all peer IDs. Convenience method for iteration without borrowing full specs.
    fn peer_ids(&self) -> Vec<String>;
}

// --- DefaultPeerRegistry ---

/// Default implementation backed by a HashMap.
pub struct DefaultPeerRegistry {
    peers: HashMap<String, PeerSpec>,
}

impl DefaultPeerRegistry {
    pub fn new() -> Self {
        Self {
            peers: HashMap::new(),
        }
    }
}

impl Default for DefaultPeerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PeerRegistry for DefaultPeerRegistry {
    fn register_peer(&mut self, mut spec: PeerSpec) -> Result<()> {
        if self.peers.contains_key(&spec.peer_id) {
            return Err(SomaError::Distributed {
                failure: crate::types::peer::DistributedFailure::PolicyViolation,
                details: format!("peer already registered: {}", spec.peer_id),
            });
        }
        // All peers start as Untrusted until authenticated and explicitly elevated.
        spec.trust_class = TrustLevel::Untrusted;
        self.peers.insert(spec.peer_id.clone(), spec);
        Ok(())
    }

    fn remove_peer(&mut self, peer_id: &str) -> Result<()> {
        if self.peers.remove(peer_id).is_none() {
            return Err(SomaError::PeerNotFound(peer_id.to_string()));
        }
        Ok(())
    }

    fn get_peer(&self, peer_id: &str) -> Option<&PeerSpec> {
        self.peers.get(peer_id)
    }

    fn list_peers(&self) -> Vec<&PeerSpec> {
        self.peers.values().collect()
    }

    fn update_availability(&mut self, peer_id: &str, availability: PeerAvailability) -> Result<()> {
        let spec = self
            .peers
            .get_mut(peer_id)
            .ok_or_else(|| SomaError::PeerNotFound(peer_id.to_string()))?;
        spec.current_availability = availability;
        Ok(())
    }

    fn get_advertisement(&self, peer_id: &str) -> Option<PeerAdvertisement> {
        self.peers.get(peer_id).map(PeerAdvertisement::from_spec)
    }

    fn find_peers_with_skill(&self, skill_id: &str) -> Vec<&PeerSpec> {
        self.peers
            .values()
            .filter(|spec| spec.exposed_skills.iter().any(|s| s.skill_id == skill_id))
            .collect()
    }

    fn find_peers_with_resource(&self, resource_type: &str) -> Vec<&PeerSpec> {
        self.peers
            .values()
            .filter(|spec| {
                spec.exposed_resources
                    .iter()
                    .any(|r| r.resource_type == resource_type)
            })
            .collect()
    }

    fn elevate_trust(&mut self, peer_id: &str, new_trust: TrustLevel) -> Result<()> {
        let spec = self
            .peers
            .get_mut(peer_id)
            .ok_or_else(|| SomaError::PeerNotFound(peer_id.to_string()))?;
        spec.trust_class = new_trust;
        Ok(())
    }

    fn check_version_compatibility(
        &self,
        peer_id: &str,
        our_version: &str,
    ) -> Result<VersionCompatibility> {
        let spec = self
            .peers
            .get(peer_id)
            .ok_or_else(|| SomaError::PeerNotFound(peer_id.to_string()))?;

        let our_ver = semver::Version::parse(our_version).map_err(|e| SomaError::Distributed {
            failure: crate::types::peer::DistributedFailure::PolicyViolation,
            details: format!("invalid our_version '{}': {}", our_version, e),
        })?;
        let peer_ver =
            semver::Version::parse(&spec.version).map_err(|e| SomaError::Distributed {
                failure: crate::types::peer::DistributedFailure::PolicyViolation,
                details: format!("invalid peer version '{}': {}", spec.version, e),
            })?;

        if our_ver.major != peer_ver.major {
            Ok(VersionCompatibility::Incompatible {
                reason: format!(
                    "major version mismatch: ours={}, peer={}",
                    our_ver.major, peer_ver.major
                ),
            })
        } else if our_ver.minor != peer_ver.minor || our_ver.patch != peer_ver.patch {
            Ok(VersionCompatibility::BackwardCompatible)
        } else {
            Ok(VersionCompatibility::Compatible)
        }
    }

    fn update_last_seen(&mut self, peer_id: &str, timestamp: DateTime<Utc>) -> Result<()> {
        let spec = self
            .peers
            .get_mut(peer_id)
            .ok_or_else(|| SomaError::PeerNotFound(peer_id.to_string()))?;
        spec.last_seen = timestamp;
        Ok(())
    }

    fn update_load(&mut self, peer_id: &str, load: f64) -> Result<()> {
        let spec = self
            .peers
            .get_mut(peer_id)
            .ok_or_else(|| SomaError::PeerNotFound(peer_id.to_string()))?;
        spec.current_load = load;
        Ok(())
    }

    fn peer_ids(&self) -> Vec<String> {
        self.peers.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::common::{
        DeterminismClass, LatencyProfile, RiskClass, RollbackSupport, SchemaRef,
    };
    use crate::types::peer::{AccessMode, MutationMode, RemoteResourceAd, RemoteSkillAd};

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

    fn make_peer(id: &str) -> PeerSpec {
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

    #[test]
    fn register_and_get_peer() {
        let mut reg = DefaultPeerRegistry::new();
        let peer = make_peer("peer-1");
        reg.register_peer(peer).unwrap();
        let got = reg.get_peer("peer-1").unwrap();
        assert_eq!(got.peer_id, "peer-1");
    }

    #[test]
    fn register_duplicate_fails() {
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer("peer-1")).unwrap();
        assert!(reg.register_peer(make_peer("peer-1")).is_err());
    }

    #[test]
    fn remove_peer_succeeds() {
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer("peer-1")).unwrap();
        reg.remove_peer("peer-1").unwrap();
        assert!(reg.get_peer("peer-1").is_none());
    }

    #[test]
    fn remove_unknown_peer_fails() {
        let mut reg = DefaultPeerRegistry::new();
        assert!(reg.remove_peer("ghost").is_err());
    }

    #[test]
    fn list_peers_returns_all() {
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer("peer-1")).unwrap();
        reg.register_peer(make_peer("peer-2")).unwrap();
        assert_eq!(reg.list_peers().len(), 2);
    }

    #[test]
    fn update_availability() {
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer("peer-1")).unwrap();
        reg.update_availability("peer-1", PeerAvailability::Degraded)
            .unwrap();
        let got = reg.get_peer("peer-1").unwrap();
        assert_eq!(got.current_availability, PeerAvailability::Degraded);
    }

    #[test]
    fn update_availability_unknown_peer_fails() {
        let mut reg = DefaultPeerRegistry::new();
        assert!(reg
            .update_availability("ghost", PeerAvailability::Offline)
            .is_err());
    }

    #[test]
    fn get_advertisement() {
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer("peer-1")).unwrap();
        let ad = reg.get_advertisement("peer-1").unwrap();
        assert_eq!(ad.peer_id, "peer-1");
        // Registered peers default to Untrusted until elevated.
        assert_eq!(ad.trust_class, TrustLevel::Untrusted);
        assert_eq!(ad.skills_exposed.len(), 2);
        assert_eq!(ad.skills_exposed[0].skill_id, "file.list");
        assert_eq!(ad.skills_exposed[1].skill_id, "file.read");
        assert_eq!(ad.resources_exposed.len(), 1);
        assert_eq!(ad.resources_exposed[0].resource_type, "filesystem");
        assert!(ad.replay_support);
    }

    #[test]
    fn get_advertisement_unknown_returns_none() {
        let reg = DefaultPeerRegistry::new();
        assert!(reg.get_advertisement("ghost").is_none());
    }

    #[test]
    fn find_peers_with_skill() {
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer("peer-1")).unwrap();
        let mut peer2 = make_peer("peer-2");
        peer2.exposed_skills = vec![make_skill_ad("http.get")];
        reg.register_peer(peer2).unwrap();

        let found = reg.find_peers_with_skill("file.list");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].peer_id, "peer-1");

        let found = reg.find_peers_with_skill("http.get");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].peer_id, "peer-2");

        let found = reg.find_peers_with_skill("nonexistent");
        assert!(found.is_empty());
    }

    #[test]
    fn find_peers_with_resource() {
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer("peer-1")).unwrap();
        let mut peer2 = make_peer("peer-2");
        peer2.exposed_resources = vec![make_resource_ad("database")];
        reg.register_peer(peer2).unwrap();

        let found = reg.find_peers_with_resource("filesystem");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].peer_id, "peer-1");

        let found = reg.find_peers_with_resource("database");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].peer_id, "peer-2");
    }

    #[test]
    fn advertisement_from_spec_all_fields() {
        let spec = make_peer("peer-x");
        let ad = PeerAdvertisement::from_spec(&spec);
        assert_eq!(ad.peer_id, spec.peer_id);
        assert_eq!(ad.version, spec.version);
        assert_eq!(ad.trust_class, spec.trust_class);
        assert_eq!(ad.supported_transports, spec.supported_transports);
        assert_eq!(ad.reachable_endpoints, spec.reachable_endpoints);
        assert_eq!(ad.current_availability, spec.current_availability);
        assert_eq!(ad.packs_loaded, spec.exposed_packs);
        assert_eq!(ad.skills_exposed, spec.exposed_skills);
        assert_eq!(ad.resources_exposed, spec.exposed_resources);
        assert_eq!(ad.policy_constraints, spec.policy_limits);
        assert_eq!(ad.cost_latency_profile.latency_class, spec.latency_class);
        assert_eq!(ad.cost_latency_profile.cost_class, spec.cost_class);
        assert!((ad.current_load - spec.current_load).abs() < f64::EPSILON);
        assert_eq!(ad.replay_support, spec.replay_support);
        assert_eq!(ad.observation_streaming, spec.observation_streaming);
        assert_eq!(ad.advertisement_version, spec.advertisement_version);
        assert_eq!(ad.expires_at, spec.advertisement_expires_at);
    }

    #[test]
    fn advertisement_expiry_check() {
        let mut spec = make_peer("peer-exp");
        // Set expiry to the past.
        spec.advertisement_expires_at = chrono::Utc::now() - chrono::Duration::hours(1);
        let ad = PeerAdvertisement::from_spec(&spec);
        assert!(ad.is_expired());

        // Set expiry to the future.
        spec.advertisement_expires_at = chrono::Utc::now() + chrono::Duration::hours(1);
        let ad = PeerAdvertisement::from_spec(&spec);
        assert!(!ad.is_expired());
    }

    #[test]
    fn register_peer_forces_untrusted() {
        let mut reg = DefaultPeerRegistry::new();
        let mut peer = make_peer("peer-1");
        peer.trust_class = TrustLevel::Trusted;
        reg.register_peer(peer).unwrap();
        let got = reg.get_peer("peer-1").unwrap();
        assert_eq!(got.trust_class, TrustLevel::Untrusted);
    }

    #[test]
    fn elevate_trust_succeeds_for_registered_peer() {
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer("peer-1")).unwrap();
        assert_eq!(reg.get_peer("peer-1").unwrap().trust_class, TrustLevel::Untrusted);
        reg.elevate_trust("peer-1", TrustLevel::Verified).unwrap();
        assert_eq!(reg.get_peer("peer-1").unwrap().trust_class, TrustLevel::Verified);
    }

    #[test]
    fn elevate_trust_fails_for_unknown_peer() {
        let mut reg = DefaultPeerRegistry::new();
        assert!(reg.elevate_trust("ghost", TrustLevel::Verified).is_err());
    }

    #[test]
    fn elevate_trust_can_set_any_level() {
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer("peer-1")).unwrap();
        reg.elevate_trust("peer-1", TrustLevel::Trusted).unwrap();
        assert_eq!(reg.get_peer("peer-1").unwrap().trust_class, TrustLevel::Trusted);
        reg.elevate_trust("peer-1", TrustLevel::BuiltIn).unwrap();
        assert_eq!(reg.get_peer("peer-1").unwrap().trust_class, TrustLevel::BuiltIn);
    }

    #[test]
    fn version_compatibility_same_version() {
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer("peer-1")).unwrap();
        let compat = reg.check_version_compatibility("peer-1", "0.1.0").unwrap();
        assert_eq!(compat, VersionCompatibility::Compatible);
    }

    #[test]
    fn version_compatibility_same_major_different_minor() {
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer("peer-1")).unwrap();
        let compat = reg.check_version_compatibility("peer-1", "0.2.0").unwrap();
        assert_eq!(compat, VersionCompatibility::BackwardCompatible);
    }

    #[test]
    fn version_compatibility_same_major_different_patch() {
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer("peer-1")).unwrap();
        let compat = reg.check_version_compatibility("peer-1", "0.1.3").unwrap();
        assert_eq!(compat, VersionCompatibility::BackwardCompatible);
    }

    #[test]
    fn version_compatibility_different_major() {
        let mut reg = DefaultPeerRegistry::new();
        let mut peer = make_peer("peer-1");
        peer.version = "2.0.0".to_string();
        reg.register_peer(peer).unwrap();
        let compat = reg.check_version_compatibility("peer-1", "1.0.0").unwrap();
        match compat {
            VersionCompatibility::Incompatible { reason } => {
                assert!(reason.contains("major version mismatch"));
            }
            other => panic!("expected Incompatible, got {:?}", other),
        }
    }

    #[test]
    fn version_compatibility_unknown_peer() {
        let reg = DefaultPeerRegistry::new();
        assert!(reg.check_version_compatibility("ghost", "0.1.0").is_err());
    }

    #[test]
    fn advertisement_carries_current_load() {
        let mut spec = make_peer("loaded-peer");
        spec.current_load = 0.75;
        let ad = PeerAdvertisement::from_spec(&spec);
        assert!((ad.current_load - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn update_last_seen_succeeds() {
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer("peer-1")).unwrap();
        let new_time = chrono::Utc::now() + chrono::Duration::hours(1);
        reg.update_last_seen("peer-1", new_time).unwrap();
        let got = reg.get_peer("peer-1").unwrap();
        assert_eq!(got.last_seen, new_time);
    }

    #[test]
    fn update_last_seen_unknown_peer_fails() {
        let mut reg = DefaultPeerRegistry::new();
        assert!(reg.update_last_seen("ghost", chrono::Utc::now()).is_err());
    }

    #[test]
    fn update_load_succeeds() {
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer("peer-1")).unwrap();
        reg.update_load("peer-1", 0.85).unwrap();
        let got = reg.get_peer("peer-1").unwrap();
        assert!((got.current_load - 0.85).abs() < f64::EPSILON);
    }

    #[test]
    fn update_load_unknown_peer_fails() {
        let mut reg = DefaultPeerRegistry::new();
        assert!(reg.update_load("ghost", 0.5).is_err());
    }

    #[test]
    fn peer_ids_returns_all() {
        let mut reg = DefaultPeerRegistry::new();
        reg.register_peer(make_peer("peer-1")).unwrap();
        reg.register_peer(make_peer("peer-2")).unwrap();
        let mut ids = reg.peer_ids();
        ids.sort();
        assert_eq!(ids, vec!["peer-1".to_string(), "peer-2".to_string()]);
    }

    #[test]
    fn peer_ids_empty_registry() {
        let reg = DefaultPeerRegistry::new();
        assert!(reg.peer_ids().is_empty());
    }
}
