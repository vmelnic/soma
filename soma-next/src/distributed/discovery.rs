// mDNS / zeroconf LAN peer discovery.
//
// Starts an mdns-sd browser in a background thread, watches for the
// `_soma._tcp.local.` service type, and pushes each resolved peer into
// the shared `PeerAddressMap` so the existing `TcpRemoteExecutor` can
// reach it. Also appends the peer ID into the shared peer-id list that
// `handle_list_peers` reports, so the brain sees the peer without any
// explicit `--peer` flag.
//
// Peers that announce, disappear, or re-announce are all handled:
//   ServiceResolved  — add or refresh peer entry
//   ServiceRemoved   — remove peer entry
//
// The peer ID generated from a discovered service is stable across
// announce/remove cycles, derived from the service's instance name
// (`soma-esp32-<mac>`). That means a chip that drops off and rejoins
// WiFi keeps the same peer ID and any routines/schemas already
// transferred don't need to be re-addressed.
//
// Multicast gotcha: many consumer APs drop 224.0.0.251 between wireless
// clients (client isolation) or between wired/wireless segments. If
// discovery isn't finding a known-good peer, fall back to `--peer
// <host:port>` — the explicit TCP path is unaffected by multicast
// forwarding policy.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use mdns_sd::{ServiceDaemon, ServiceEvent};
use tracing::{info, warn};

use super::transport::PeerAddressMap;

/// Service type we browse for. Matches the announcement sent by the
/// ESP32 leaf firmware's mDNS responder.
pub const SOMA_SERVICE_TYPE: &str = "_soma._tcp.local.";

/// Start the mDNS browser on a background thread.
///
/// The browser writes discovered peer addresses into `peer_map` and
/// tracks which peer IDs it added so it can remove them when the
/// service goes away. The same IDs are pushed into `peer_ids` (the
/// list the MCP `list_peers` handler reports).
///
/// Returns a handle to the ServiceDaemon so the caller can keep it
/// alive for the lifetime of the process. Dropping the handle stops
/// the browser.
pub fn spawn_lan_browser(
    peer_map: PeerAddressMap,
    peer_ids: Arc<Mutex<Vec<String>>>,
) -> Result<ServiceDaemon, mdns_sd::Error> {
    let daemon = ServiceDaemon::new()?;
    let receiver = daemon.browse(SOMA_SERVICE_TYPE)?;

    info!("[discovery] mDNS browser started for {}", SOMA_SERVICE_TYPE);

    // Track which peer IDs we added so we can clean them up on
    // ServiceRemoved. Peer IDs are derived from the service instance
    // fullname so a peer that re-announces after a brief drop lands
    // back in the same slot.
    let mut our_peers: HashSet<String> = HashSet::new();
    // Instance fullname -> peer_id mapping. mdns-sd's ServiceRemoved
    // gives us the fullname, so we map it back to the peer_id we
    // assigned.
    let mut fullname_to_peer_id: HashMap<String, String> = HashMap::new();

    thread::spawn(move || {
        while let Ok(event) = receiver.recv() {
            match event {
                ServiceEvent::ServiceResolved(info) => {
                    let instance = info.get_fullname().to_string();
                    let peer_id = peer_id_from_instance(&instance);

                    // Pick an IPv4 address from the advertised set.
                    // mdns-sd returns `ScopedIp` values which wrap an
                    // IpAddr plus optional interface scope. Embedded
                    // leaves only have one IP so this is trivially
                    // the first one.
                    let ipv4 = info.get_addresses().iter().find_map(|scoped| {
                        if let std::net::IpAddr::V4(v4) = scoped.to_ip_addr() {
                            Some(v4)
                        } else {
                            None
                        }
                    });

                    let Some(ipv4) = ipv4 else {
                        warn!(
                            "[discovery] {} resolved with no IPv4 address, ignoring",
                            instance
                        );
                        continue;
                    };

                    let addr = SocketAddr::new(std::net::IpAddr::V4(ipv4), info.get_port());
                    info!(
                        "[discovery] peer resolved: {} (id={}) at {}",
                        instance, peer_id, addr
                    );

                    // Insert into peer_map (address lookup for the
                    // TcpRemoteExecutor) and into peer_ids (the list
                    // list_peers reports). Both are idempotent — if
                    // the peer re-announces with the same ID we just
                    // update the address.
                    peer_map.lock().unwrap().insert(peer_id.clone(), addr);

                    let mut ids = peer_ids.lock().unwrap();
                    if !ids.contains(&peer_id) {
                        ids.push(peer_id.clone());
                    }
                    drop(ids);

                    our_peers.insert(peer_id.clone());
                    fullname_to_peer_id.insert(instance, peer_id);
                }
                ServiceEvent::ServiceRemoved(_service_type, instance) => {
                    if let Some(peer_id) = fullname_to_peer_id.remove(&instance) {
                        info!(
                            "[discovery] peer removed: {} (id={})",
                            instance, peer_id
                        );
                        peer_map.lock().unwrap().remove(&peer_id);
                        peer_ids.lock().unwrap().retain(|pid| pid != &peer_id);
                        our_peers.remove(&peer_id);
                    }
                }
                ServiceEvent::SearchStarted(_)
                | ServiceEvent::ServiceFound(_, _)
                | ServiceEvent::SearchStopped(_) => {
                    // Not actionable on their own — we only care about
                    // fully resolved instances and explicit removes.
                }
                // mdns-sd adds new event variants over time; swallow
                // unknowns so older firmware announcing with optional
                // fields doesn't panic the browser loop.
                _ => {}
            }
        }
    });

    Ok(daemon)
}

/// Derive a stable peer ID from an mDNS service instance fullname.
///
/// Input: `soma-esp32-ccdba79df9e8._soma._tcp.local.`
/// Output: `lan-soma-esp32-ccdba79df9e8`
///
/// Strips the service suffix to keep the ID short and prefixes with
/// `lan-` so discovered peers are visually distinct from static
/// `peer-N` IDs assigned by the `--peer` CLI path.
fn peer_id_from_instance(fullname: &str) -> String {
    // Take everything before the first `._soma._tcp.` segment.
    let instance = fullname.split("._soma._tcp").next().unwrap_or(fullname);
    format!("lan-{}", instance)
}

/// Convenience: compute how long the caller should sleep before
/// expecting the first discovery event. Mostly useful in tests and
/// in `main.rs` startup banners. mDNS spec allows up to 1 second
/// for first response; we budget 3 to be safe with congested APs.
pub fn typical_discovery_delay() -> Duration {
    Duration::from_secs(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_id_strips_service_suffix() {
        assert_eq!(
            peer_id_from_instance("soma-esp32-ccdba79df9e8._soma._tcp.local."),
            "lan-soma-esp32-ccdba79df9e8"
        );
    }

    #[test]
    fn peer_id_handles_fullname_without_service() {
        // Fallback — shouldn't happen with mdns-sd but be defensive
        assert_eq!(peer_id_from_instance("weird-name"), "lan-weird-name");
    }

    #[test]
    fn peer_id_is_stable_across_calls() {
        let a = peer_id_from_instance("soma-esp32-aa._soma._tcp.local.");
        let b = peer_id_from_instance("soma-esp32-aa._soma._tcp.local.");
        assert_eq!(a, b);
    }
}
