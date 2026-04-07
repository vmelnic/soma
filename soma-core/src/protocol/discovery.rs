//! Peer discovery and registry for the Synaptic Protocol.

use std::collections::HashMap;

/// Information about a known peer.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub name: String,
    pub addr: String,
    pub last_seen: u64,
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
        self.peers.insert(
            name.clone(),
            PeerInfo {
                name,
                addr,
                last_seen: 0,
            },
        );
    }

    /// Update the last_seen timestamp for a peer.
    pub fn touch(&mut self, name: &str) {
        if let Some(peer) = self.peers.get_mut(name) {
            peer.last_seen = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
        }
    }

    /// Register or update a discovered peer.
    pub fn register(&mut self, name: String, addr: String) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.peers.insert(
            name.clone(),
            PeerInfo {
                name,
                addr,
                last_seen: now,
            },
        );
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
}
