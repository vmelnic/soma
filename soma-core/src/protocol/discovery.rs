//! Peer discovery and registry for the Synaptic Protocol v2 (Spec Section 7).
//!
//! DISCOVER signals include address, plugins, conventions, and load.
//! `PEER_QUERY` finds SOMAs with specific plugins.
//! `PEER_LIST` returns matching peers.

use std::collections::HashMap;

use super::signal::{Signal, SignalType};

/// Information about a known peer.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub name: String,
    pub addr: String,
    pub plugins: Vec<String>,
    pub conventions: Vec<String>,
    pub load: f64,
    #[allow(dead_code)] // Spec feature for peer load balancing
    pub capacity: u64,
    #[allow(dead_code)] // Spec feature for peer health tracking
    pub last_seen: u64,
}

impl PeerInfo {
    /// Create a basic `PeerInfo` with just name and address.
    pub const fn basic(name: String, addr: String) -> Self {
        Self {
            name,
            addr,
            plugins: Vec::new(),
            conventions: Vec::new(),
            load: 0.0,
            capacity: 1000,
            last_seen: 0,
        }
    }
}

/// Registry of known SOMA peers.
pub struct PeerRegistry {
    peers: HashMap<String, PeerInfo>,
}

impl PeerRegistry {
    pub fn new() -> Self {
        Self {
            peers: HashMap::new(),
        }
    }

    /// Register a peer from configuration (static peers).
    pub fn add_static_peer(&mut self, name: String, addr: String) {
        self.peers
            .insert(name.clone(), PeerInfo::basic(name, addr));
    }

    /// Update the `last_seen` timestamp for a peer.
    #[allow(dead_code)] // Spec feature for peer health tracking
    pub fn touch(&mut self, name: &str) {
        if let Some(peer) = self.peers.get_mut(name) {
            peer.last_seen = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
        }
    }

    /// Register or update a discovered peer from a DISCOVER signal.
    pub fn register_from_discover(&mut self, signal: &Signal) {
        let sender = signal.sender_id.clone();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Extract payload data (address, plugins, conventions, load)
        let payload: serde_json::Value = if signal.payload.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&signal.payload).unwrap_or_default()
        };

        let addr = payload
            .get("address")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let plugins: Vec<String> = payload
            .get("plugins")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let conventions: Vec<String> = payload
            .get("conventions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let load = payload
            .get("load")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0);

        let capacity = payload
            .get("capacity")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(1000);

        self.peers.insert(
            sender.clone(),
            PeerInfo {
                name: sender,
                addr,
                plugins,
                conventions,
                load,
                capacity,
                last_seen: now,
            },
        );
    }

    /// Register or update a discovered peer with explicit info.
    #[allow(dead_code)] // Spec feature for peer discovery
    pub fn register(&mut self, name: String, addr: String) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut info = PeerInfo::basic(name.clone(), addr);
        info.last_seen = now;
        self.peers.insert(name, info);
    }

    /// Remove a peer by name.
    pub fn remove(&mut self, name: &str) -> Option<PeerInfo> {
        self.peers.remove(name)
    }

    /// Get a peer by name.
    pub fn get(&self, name: &str) -> Option<&PeerInfo> {
        self.peers.get(name)
    }

    /// List all known peers.
    pub fn list(&self) -> Vec<&PeerInfo> {
        self.peers.values().collect()
    }

    /// Number of known peers.
    pub fn count(&self) -> usize {
        self.peers.len()
    }

    /// Load peers from configuration map (name -> address).
    pub fn load_from_config(&mut self, peers: &HashMap<String, String>) {
        for (name, addr) in peers {
            self.add_static_peer(name.clone(), addr.clone());
        }
    }

    /// Create a DISCOVER signal announcing this SOMA's presence.
    #[allow(dead_code)] // Spec feature for peer discovery
    pub fn create_discover_signal(
        soma_id: &str,
        address: &str,
        plugins: &[String],
        conventions: &[String],
        load: f64,
        capacity: u64,
    ) -> Signal {
        let mut signal = Signal::new(SignalType::Discover, soma_id.to_string());
        signal.channel_id = 0; // control channel
        signal.payload = serde_json::to_vec(&serde_json::json!({
            "address": address,
            "plugins": plugins,
            "conventions": conventions,
            "load": load,
            "capacity": capacity,
        }))
        .unwrap_or_default();

        // Default TTL for discovery forwarding (chemical gradient, Spec 7.1)
        if let serde_json::Value::Object(ref mut map) = signal.metadata {
            map.insert("ttl".to_string(), serde_json::json!(3));
        }

        signal
    }

    /// Create a `DISCOVER_ACK` response signal.
    #[allow(dead_code)] // Spec feature for peer discovery
    pub fn create_discover_ack(
        soma_id: &str,
        address: &str,
        plugins: &[String],
        conventions: &[String],
        load: f64,
        capacity: u64,
    ) -> Signal {
        let mut signal = Signal::new(SignalType::DiscoverAck, soma_id.to_string());
        signal.channel_id = 0;
        signal.payload = serde_json::to_vec(&serde_json::json!({
            "address": address,
            "plugins": plugins,
            "conventions": conventions,
            "load": load,
            "capacity": capacity,
        }))
        .unwrap_or_default();
        signal
    }

    /// Handle a `PEER_QUERY` signal: find peers matching the requested plugin.
    /// Returns a `PEER_LIST` signal.
    pub fn handle_peer_query(&self, query: &Signal, soma_id: &str) -> Signal {
        let query_payload: serde_json::Value = if query.payload.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&query.payload).unwrap_or_default()
        };

        let need_plugin = query_payload
            .get("need_plugin")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let matching_peers: Vec<serde_json::Value> = self
            .peers
            .values()
            .filter(|peer| {
                need_plugin.is_empty()
                    || peer.plugins.iter().any(|p| p == need_plugin)
            })
            .map(|peer| {
                serde_json::json!({
                    "id": peer.name,
                    "address": peer.addr,
                    "plugins": peer.plugins,
                    "load": peer.load,
                    "reachable_via": soma_id,
                })
            })
            .collect();

        let mut response = Signal::new(SignalType::PeerList, soma_id.to_string());
        response.channel_id = 0;
        response.payload = serde_json::to_vec(&serde_json::json!({
            "peers": matching_peers,
        }))
        .unwrap_or_default();
        response
    }
}

/// Check if a DISCOVER signal should be forwarded (TTL > 0).
/// Returns a new signal with decremented TTL for forwarding, or None.
/// Implements the chemical-gradient decay from Spec Section 7.1.
#[allow(dead_code)] // Spec feature for discovery forwarding
pub fn prepare_forward_discover(signal: &Signal, our_id: &str) -> Option<Signal> {
    let ttl = signal
        .metadata
        .get("ttl")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    if ttl == 0 {
        return None;
    }

    // Don't forward our own discoveries
    if signal.sender_id == our_id {
        return None;
    }

    let mut forwarded = signal.clone();
    if let serde_json::Value::Object(ref mut map) = forwarded.metadata {
        map.insert("ttl".to_string(), serde_json::json!(ttl - 1));
        // Add forwarded_by to track gradient path
        let mut path: Vec<String> = map
            .get("forward_path")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        path.push(our_id.to_string());
        map.insert("forward_path".to_string(), serde_json::json!(path));
    }

    Some(forwarded)
}
