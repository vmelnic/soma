use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::{DateTime, Utc};
use tracing::{debug, info, warn};

use super::peer::PeerRegistry;
use super::transport::{PeerAddressMap, TcpRemoteExecutor, TransportResponse};
use crate::types::peer::PeerAvailability;

// ---------------------------------------------------------------------------
// PeerHealth — per-peer heartbeat state
// ---------------------------------------------------------------------------

/// Health state tracked for each peer. Updated on every heartbeat cycle.
#[derive(Debug, Clone)]
pub struct PeerHealth {
    /// Timestamp of the last successful heartbeat response.
    pub last_heartbeat: DateTime<Utc>,
    /// Most recently measured round-trip time in milliseconds.
    pub rtt_ms: u64,
    /// Number of consecutive heartbeat attempts that received no response.
    pub missed_count: u32,
    /// Whether the peer is considered alive based on heartbeat responses.
    pub alive: bool,
}

impl PeerHealth {
    fn new() -> Self {
        Self {
            last_heartbeat: Utc::now(),
            rtt_ms: 0,
            missed_count: 0,
            alive: true,
        }
    }
}

// ---------------------------------------------------------------------------
// HeartbeatConfig — tunables
// ---------------------------------------------------------------------------

/// Configuration for the heartbeat manager.
#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    /// Interval between heartbeat rounds, in milliseconds.
    pub interval_ms: u64,
    /// Number of consecutive missed heartbeats before marking a peer unavailable.
    pub max_missed: u32,
    /// Per-peer ping timeout in milliseconds (connect + response).
    pub timeout_ms: u64,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            interval_ms: 5000,
            max_missed: 3,
            timeout_ms: 2000,
        }
    }
}

// ---------------------------------------------------------------------------
// HeartbeatManager — tracks peer health via periodic pings
// ---------------------------------------------------------------------------

/// Manages periodic heartbeat pings to all known peers. Tracks RTT, updates
/// PeerSpec.last_seen and PeerSpec.current_load in the registry, and marks
/// peers unavailable when they stop responding.
pub struct HeartbeatManager {
    peers: HashMap<String, PeerHealth>,
    config: HeartbeatConfig,
    nonce_counter: u64,
}

impl HeartbeatManager {
    pub fn new(config: HeartbeatConfig) -> Self {
        Self {
            peers: HashMap::new(),
            config,
            nonce_counter: 0,
        }
    }

    /// Get an immutable view of tracked peer health states.
    pub fn peer_health(&self) -> &HashMap<String, PeerHealth> {
        &self.peers
    }

    /// Get health for a specific peer.
    pub fn get_health(&self, peer_id: &str) -> Option<&PeerHealth> {
        self.peers.get(peer_id)
    }

    /// Run one heartbeat cycle: ping every known peer, measure RTT, update
    /// the registry. This is designed to be called periodically from a
    /// background task.
    pub fn tick(
        &mut self,
        registry: &mut dyn PeerRegistry,
        executor: &TcpRemoteExecutor,
    ) {
        let peer_ids = registry.peer_ids();

        for peer_id in &peer_ids {
            self.nonce_counter = self.nonce_counter.wrapping_add(1);
            let nonce = self.nonce_counter;

            let health = self.peers.entry(peer_id.clone()).or_insert_with(PeerHealth::new);

            let start = Instant::now();
            match executor.send_ping(peer_id, nonce) {
                Ok(TransportResponse::Pong {
                    nonce: resp_nonce,
                    load,
                }) if resp_nonce == nonce => {
                    let rtt = start.elapsed().as_millis() as u64;
                    let now = Utc::now();

                    health.last_heartbeat = now;
                    health.rtt_ms = rtt;
                    health.missed_count = 0;
                    health.alive = true;

                    // Push fresh data into the peer registry.
                    if let Err(e) = registry.update_last_seen(peer_id, now) {
                        debug!(peer = %peer_id, error = %e, "failed to update last_seen");
                    }
                    if let Err(e) = registry.update_load(peer_id, load) {
                        debug!(peer = %peer_id, error = %e, "failed to update load");
                    }

                    debug!(peer = %peer_id, rtt_ms = rtt, load = load, "heartbeat ok");
                }
                Ok(other) => {
                    // Got a response but not a valid pong (nonce mismatch or wrong type).
                    health.missed_count += 1;
                    warn!(
                        peer = %peer_id,
                        missed = health.missed_count,
                        response = ?other,
                        "unexpected heartbeat response"
                    );
                    self.maybe_mark_unavailable(peer_id, registry);
                }
                Err(e) => {
                    health.missed_count += 1;
                    debug!(
                        peer = %peer_id,
                        missed = health.missed_count,
                        error = %e,
                        "heartbeat failed"
                    );
                    self.maybe_mark_unavailable(peer_id, registry);
                }
            }
        }

        // Clean up health entries for peers that are no longer in the registry.
        self.peers.retain(|id, _| peer_ids.contains(id));
    }

    /// If a peer has exceeded the miss threshold, mark it unavailable.
    fn maybe_mark_unavailable(&mut self, peer_id: &str, registry: &mut dyn PeerRegistry) {
        let health = match self.peers.get_mut(peer_id) {
            Some(h) => h,
            None => return,
        };

        if health.missed_count >= self.config.max_missed && health.alive {
            health.alive = false;
            info!(
                peer = %peer_id,
                missed = health.missed_count,
                "peer marked unavailable after missed heartbeats"
            );
            if let Err(e) = registry.update_availability(peer_id, PeerAvailability::Offline) {
                debug!(peer = %peer_id, error = %e, "failed to update availability");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Background task launcher
// ---------------------------------------------------------------------------

/// Start a background tokio task that runs heartbeat cycles at the configured
/// interval. Returns a `JoinHandle` that can be used to abort or await the task.
///
/// The `registry` is behind an `Arc<Mutex<>>` so the heartbeat can take a
/// write lock for updates while other code reads the registry concurrently.
pub fn start_heartbeat_task(
    config: HeartbeatConfig,
    registry: Arc<Mutex<dyn PeerRegistry>>,
    peer_addresses: PeerAddressMap,
) -> tokio::task::JoinHandle<()> {
    let interval_ms = config.interval_ms;

    tokio::spawn(async move {
        let mut manager = HeartbeatManager::new(config);
        let executor = TcpRemoteExecutor::new(peer_addresses);
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(interval_ms));

        info!(interval_ms = interval_ms, "heartbeat task started");

        loop {
            interval.tick().await;
            let mut reg = registry.lock().unwrap();
            manager.tick(&mut *reg, &executor);
        }
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distributed::peer::{DefaultPeerRegistry, PeerRegistry};
    use crate::distributed::transport::{
        IncomingHandler, PeerAddressMap, TcpRemoteExecutor, TransportMessage, TransportResponse,
        start_listener_background,
    };
    
    use crate::types::common::{
        DeterminismClass, LatencyProfile, RiskClass, RollbackSupport, SchemaRef, TrustLevel,
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

    fn make_peer(id: &str, endpoint: &str) -> PeerSpec {
        PeerSpec {
            peer_id: id.to_string(),
            version: "0.1.0".to_string(),
            trust_class: TrustLevel::Verified,
            supported_transports: vec![Transport::Tcp],
            reachable_endpoints: vec![endpoint.to_string()],
            current_availability: PeerAvailability::Available,
            policy_limits: vec![],
            exposed_packs: vec!["core".to_string()],
            exposed_skills: vec![make_skill_ad("file.list")],
            exposed_resources: vec![make_resource_ad("filesystem")],
            latency_class: "low".to_string(),
            cost_class: "low".to_string(),
            current_load: 0.0,
            last_seen: chrono::Utc::now() - chrono::Duration::seconds(60),
            replay_support: true,
            observation_streaming: true,
            advertisement_version: 1,
            advertisement_expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        }
    }

    /// Handler that responds to Ping with Pong, and stubs everything else.
    struct PingPongHandler {
        load: f64,
    }

    impl IncomingHandler for PingPongHandler {
        fn handle(&self, msg: TransportMessage) -> TransportResponse {
            match msg {
                TransportMessage::Ping { nonce } => TransportResponse::Pong {
                    nonce,
                    load: self.load,
                },
                _ => TransportResponse::Error {
                    details: "unexpected message type".to_string(),
                },
            }
        }
    }

    fn free_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    #[test]
    fn heartbeat_manager_new_has_empty_state() {
        let mgr = HeartbeatManager::new(HeartbeatConfig::default());
        assert!(mgr.peer_health().is_empty());
    }

    #[test]
    fn peer_health_defaults() {
        let health = PeerHealth::new();
        assert!(health.alive);
        assert_eq!(health.missed_count, 0);
        assert_eq!(health.rtt_ms, 0);
    }

    #[test]
    fn heartbeat_config_defaults() {
        let config = HeartbeatConfig::default();
        assert_eq!(config.interval_ms, 5000);
        assert_eq!(config.max_missed, 3);
        assert_eq!(config.timeout_ms, 2000);
    }

    #[test]
    fn tick_updates_health_on_successful_ping() {
        let port = free_port();
        let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
        let handler: Arc<dyn IncomingHandler> = Arc::new(PingPongHandler { load: 0.75 });
        let _listener = start_listener_background(addr, handler);
        std::thread::sleep(std::time::Duration::from_millis(100));

        let peer_map: PeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map.lock().unwrap().insert("peer-1".to_string(), addr);
        let executor = TcpRemoteExecutor::new(peer_map);

        let mut registry = DefaultPeerRegistry::new();
        registry
            .register_peer(make_peer("peer-1", &format!("127.0.0.1:{}", port)))
            .unwrap();

        let mut mgr = HeartbeatManager::new(HeartbeatConfig::default());
        mgr.tick(&mut registry, &executor);

        // Health should be tracked and alive.
        let health = mgr.get_health("peer-1").unwrap();
        assert!(health.alive);
        assert_eq!(health.missed_count, 0);

        // Registry should have updated load from the pong.
        let spec = registry.get_peer("peer-1").unwrap();
        assert!((spec.current_load - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn tick_increments_missed_on_unreachable_peer() {
        // Point to a port where nothing is listening.
        let port = free_port();
        let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();

        let peer_map: PeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map.lock().unwrap().insert("peer-dead".to_string(), addr);
        let executor = TcpRemoteExecutor::new(peer_map);

        let mut registry = DefaultPeerRegistry::new();
        registry
            .register_peer(make_peer("peer-dead", &format!("127.0.0.1:{}", port)))
            .unwrap();

        let mut mgr = HeartbeatManager::new(HeartbeatConfig {
            max_missed: 3,
            ..HeartbeatConfig::default()
        });

        // First tick: missed_count = 1, still alive (threshold is 3).
        mgr.tick(&mut registry, &executor);
        let health = mgr.get_health("peer-dead").unwrap();
        assert_eq!(health.missed_count, 1);
        assert!(health.alive);

        // Second tick: missed_count = 2, still alive.
        mgr.tick(&mut registry, &executor);
        assert_eq!(mgr.get_health("peer-dead").unwrap().missed_count, 2);
        assert!(mgr.get_health("peer-dead").unwrap().alive);

        // Third tick: missed_count = 3, marked unavailable.
        mgr.tick(&mut registry, &executor);
        let health = mgr.get_health("peer-dead").unwrap();
        assert_eq!(health.missed_count, 3);
        assert!(!health.alive);

        // Registry should have been updated to Offline.
        let spec = registry.get_peer("peer-dead").unwrap();
        assert_eq!(spec.current_availability, PeerAvailability::Offline);
    }

    #[test]
    fn tick_recovers_after_peer_comes_back() {
        let port = free_port();
        let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();

        let peer_map: PeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map.lock().unwrap().insert("peer-flaky".to_string(), addr);
        let executor = TcpRemoteExecutor::new(Arc::clone(&peer_map));

        let mut registry = DefaultPeerRegistry::new();
        registry
            .register_peer(make_peer("peer-flaky", &format!("127.0.0.1:{}", port)))
            .unwrap();

        let mut mgr = HeartbeatManager::new(HeartbeatConfig {
            max_missed: 2,
            ..HeartbeatConfig::default()
        });

        // Two ticks with no listener -> marked dead.
        mgr.tick(&mut registry, &executor);
        mgr.tick(&mut registry, &executor);
        assert!(!mgr.get_health("peer-flaky").unwrap().alive);

        // Now start a listener.
        let handler: Arc<dyn IncomingHandler> = Arc::new(PingPongHandler { load: 0.1 });
        let _listener = start_listener_background(addr, handler);
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Next tick should succeed and bring the peer back.
        mgr.tick(&mut registry, &executor);
        let health = mgr.get_health("peer-flaky").unwrap();
        assert!(health.alive);
        assert_eq!(health.missed_count, 0);
    }

    #[test]
    fn tick_with_no_peers_does_nothing() {
        let peer_map: PeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        let executor = TcpRemoteExecutor::new(peer_map);
        let mut registry = DefaultPeerRegistry::new();
        let mut mgr = HeartbeatManager::new(HeartbeatConfig::default());

        mgr.tick(&mut registry, &executor);
        assert!(mgr.peer_health().is_empty());
    }

    #[test]
    fn tick_cleans_up_removed_peers() {
        let port = free_port();
        let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
        let handler: Arc<dyn IncomingHandler> = Arc::new(PingPongHandler { load: 0.0 });
        let _listener = start_listener_background(addr, handler);
        std::thread::sleep(std::time::Duration::from_millis(100));

        let peer_map: PeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map.lock().unwrap().insert("peer-temp".to_string(), addr);
        let executor = TcpRemoteExecutor::new(peer_map);

        let mut registry = DefaultPeerRegistry::new();
        registry
            .register_peer(make_peer("peer-temp", &format!("127.0.0.1:{}", port)))
            .unwrap();

        let mut mgr = HeartbeatManager::new(HeartbeatConfig::default());
        mgr.tick(&mut registry, &executor);
        assert!(mgr.get_health("peer-temp").is_some());

        // Remove peer from registry.
        registry.remove_peer("peer-temp").unwrap();

        // Next tick should clean up the stale health entry.
        mgr.tick(&mut registry, &executor);
        assert!(mgr.get_health("peer-temp").is_none());
    }

    #[test]
    fn send_ping_roundtrip() {
        let port = free_port();
        let addr: std::net::SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
        let handler: Arc<dyn IncomingHandler> = Arc::new(PingPongHandler { load: 0.5 });
        let _listener = start_listener_background(addr, handler);
        std::thread::sleep(std::time::Duration::from_millis(100));

        let peer_map: PeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        peer_map.lock().unwrap().insert("peer-1".to_string(), addr);
        let executor = TcpRemoteExecutor::new(peer_map);

        let resp = executor.send_ping("peer-1", 42).unwrap();
        match resp {
            TransportResponse::Pong { nonce, load } => {
                assert_eq!(nonce, 42);
                assert!((load - 0.5).abs() < f64::EPSILON);
            }
            other => panic!("expected Pong, got {:?}", other),
        }
    }

    #[test]
    fn multiple_peers_tracked_independently() {
        let port1 = free_port();
        let port2 = free_port();
        let addr1: std::net::SocketAddr = format!("127.0.0.1:{}", port1).parse().unwrap();
        let addr2: std::net::SocketAddr = format!("127.0.0.1:{}", port2).parse().unwrap();

        // Only start a listener for peer-1.
        let handler: Arc<dyn IncomingHandler> = Arc::new(PingPongHandler { load: 0.3 });
        let _listener = start_listener_background(addr1, handler);
        std::thread::sleep(std::time::Duration::from_millis(100));

        let peer_map: PeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
        {
            let mut map = peer_map.lock().unwrap();
            map.insert("peer-1".to_string(), addr1);
            map.insert("peer-2".to_string(), addr2);
        }
        let executor = TcpRemoteExecutor::new(peer_map);

        let mut registry = DefaultPeerRegistry::new();
        registry
            .register_peer(make_peer("peer-1", &format!("127.0.0.1:{}", port1)))
            .unwrap();
        registry
            .register_peer(make_peer("peer-2", &format!("127.0.0.1:{}", port2)))
            .unwrap();

        let mut mgr = HeartbeatManager::new(HeartbeatConfig {
            max_missed: 1,
            ..HeartbeatConfig::default()
        });

        mgr.tick(&mut registry, &executor);

        // peer-1 should be alive.
        let h1 = mgr.get_health("peer-1").unwrap();
        assert!(h1.alive);
        assert_eq!(h1.missed_count, 0);

        // peer-2 should have one miss (threshold is 1, so it's now dead).
        let h2 = mgr.get_health("peer-2").unwrap();
        assert!(!h2.alive);
        assert_eq!(h2.missed_count, 1);
    }
}
