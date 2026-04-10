// mDNS responder — advertises this leaf as `_soma._tcp.local` so
// soma-next instances running with `--discover-lan` can find it on the
// LAN without any explicit peer configuration.
//
// Wire protocol: RFC 6762 mDNS over UDP port 5353, multicast address
// 224.0.0.251. Uses edge-mdns 0.7 (no_std, no-alloc, with default
// `io` feature disabled) to parse incoming queries and build responses.
// We hand-drive the smoltcp UdpSocket rather than using edge-mdns's
// embassy-based io layer.
//
// What we announce:
//   PTR   _soma._tcp.local.               -> soma-esp32-<mac>._soma._tcp.local.
//   SRV   soma-esp32-<mac>._soma._tcp.local. -> port=9100, target=soma-esp32-<mac>.local.
//   TXT   soma-esp32-<mac>._soma._tcp.local. -> proto=soma-leaf-v1 chip=<chip>
//   A     soma-esp32-<mac>.local.         -> <dhcp-assigned-ipv4>
//
// The soma-next side browses for `_soma._tcp.local.` via `mdns-sd` and
// picks up this announcement, deriving the peer ID `lan-soma-esp32-<mac>`.
//
// Caveats / known limitations:
//   - We only answer IPv4 (no AAAA). ESP32 leafs don't have IPv6.
//   - We send a single unsolicited announcement when DHCP gives us an
//     address, and respond to queries thereafter. No periodic
//     re-announcement — if our lease renews we'll re-announce then.
//   - Many consumer APs drop 224.0.0.251 between wireless clients
//     (client isolation). If discovery isn't seeing the leaf, fall back
//     to soma-next's explicit `--peer` path — unrelated to this code.

use alloc::format;
use alloc::string::String;
use core::net::Ipv4Addr as CoreIpv4Addr;

use edge_mdns::{
    domain::base::Ttl,
    host::{Host, Service, ServiceAnswers},
    HostAnswersMdnsHandler, MdnsHandler, MdnsRequest, MdnsResponse,
};
use esp_println::println;
use smoltcp::iface::SocketSet;
use smoltcp::socket::udp::{self, PacketBuffer as UdpPacketBuffer, Socket as UdpSocket};
use smoltcp::wire::{IpAddress, IpEndpoint, IpListenEndpoint, Ipv4Address};

const MDNS_PORT: u16 = 5353;
const MDNS_MULTICAST_V4: Ipv4Address = Ipv4Address::new(224, 0, 0, 251);
const MDNS_TTL_SEC: u32 = 120;

/// Maximum bytes we accept for a single incoming mDNS query. mDNS
/// messages are almost always well under 512 bytes; we cap at 1024 to
/// allow some headroom for combined questions.
const MDNS_MTU: usize = 1024;

/// State for the ESP32 leaf's mDNS responder.
///
/// Holds:
/// - the smoltcp UdpSocket handle bound to 0.0.0.0:5353
/// - the hostname-derived identifiers as owned strings (so they live
///   at 'static lifetime inside this leaked-to-'static state)
/// - whether we've sent our one-shot announcement yet
///
/// Construction allocates ~1.5 KB for the rx buffer and ~1.5 KB for the
/// tx buffer (both kept small because mDNS packets are tiny).
pub struct MdnsResponder {
    pub socket_handle: smoltcp::iface::SocketHandle,
    /// e.g. "soma-esp32-ccdba79df9e8"
    pub hostname: String,
    /// e.g. "esp32" or "esp32s3"
    pub chip: String,
    /// Set once the one-shot unsolicited announcement has been sent.
    pub announced: bool,
    /// The IPv4 address to put in A records. Updated on DHCP change.
    pub ipv4: Option<Ipv4Address>,
}

impl MdnsResponder {
    /// Add a UdpSocket to the given SocketSet, bind it to port 5353,
    /// and return a new MdnsResponder ready to poll.
    ///
    /// The hostname is derived from the chip's MAC address:
    ///   mac = cc:db:a7:9d:f9:e8 -> "soma-esp32-ccdba79df9e8"
    ///
    /// The caller is expected to also call
    /// `iface.join_multicast_group(MDNS_MULTICAST_V4)` once the IPv4
    /// address is configured — without that smoltcp won't accept
    /// packets destined to 224.0.0.251.
    pub fn new(
        sockets: &mut SocketSet<'static>,
        mac: [u8; 6],
        chip_name: &str,
    ) -> Self {
        // Allocate small buffers on the heap and leak to 'static so the
        // SocketSet can hold them for the lifetime of the program.
        let rx_storage: &'static mut [u8] =
            alloc::vec![0u8; 1536].leak();
        let rx_meta: &'static mut [udp::PacketMetadata] =
            alloc::vec![udp::PacketMetadata::EMPTY; 4].leak();
        let tx_storage: &'static mut [u8] =
            alloc::vec![0u8; 1536].leak();
        let tx_meta: &'static mut [udp::PacketMetadata] =
            alloc::vec![udp::PacketMetadata::EMPTY; 4].leak();

        let mut socket = UdpSocket::new(
            UdpPacketBuffer::new(rx_meta, rx_storage),
            UdpPacketBuffer::new(tx_meta, tx_storage),
        );

        // bind(u16) binds to the unspecified address. That's what we
        // want — mDNS listens on every interface.
        let _ = socket.bind(IpListenEndpoint {
            addr: None,
            port: MDNS_PORT,
        });

        let socket_handle = sockets.add(socket);

        // Build "soma-<chip>-<mac-hex>" hostname. No colons, lowercase.
        let hostname = format!(
            "soma-{}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            chip_name, mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        );

        println!(
            "[mdns] responder bound to UDP/5353, hostname '{}.local'",
            hostname
        );

        Self {
            socket_handle,
            hostname,
            chip: chip_name.into(),
            announced: false,
            ipv4: None,
        }
    }

    /// Update the IPv4 address we advertise in A records. Call this
    /// from the DHCP Configured handler. Also clears the `announced`
    /// flag so the next poll will send a fresh gratuitous announcement
    /// with the new address.
    pub fn set_ipv4(&mut self, addr: Ipv4Address) {
        self.ipv4 = Some(addr);
        self.announced = false;
    }

    /// Clear the IPv4 (called on DHCP lease loss). Drops our A record
    /// and suppresses answering queries until a new address is set.
    pub fn clear_ipv4(&mut self) {
        self.ipv4 = None;
        self.announced = true; // no-op guard
    }

    /// Drive the responder: drain any received mDNS query packets and
    /// respond; send a one-shot announcement if we haven't yet.
    ///
    /// Called from the main dispatch loop every iteration.
    pub fn poll(&mut self, sockets: &mut SocketSet<'static>) {
        let Some(ipv4) = self.ipv4 else {
            return;
        };

        // Build the edge-mdns data structures on the stack each call.
        // They borrow from our owned strings so no alloc per-call.
        let core_ipv4 = CoreIpv4Addr::new(
            ipv4.octets()[0],
            ipv4.octets()[1],
            ipv4.octets()[2],
            ipv4.octets()[3],
        );

        let host = Host {
            hostname: &self.hostname,
            ipv4: core_ipv4,
            ipv6: core::net::Ipv6Addr::UNSPECIFIED,
            ttl: Ttl::from_secs(MDNS_TTL_SEC),
        };

        // The TXT record is built from a borrowed slice of (key, value)
        // pairs. Keep these as local literals so the slice lives for
        // the duration of the call.
        let txt_kvs: [(&str, &str); 2] = [
            ("proto", "soma-leaf-v1"),
            ("chip", self.chip.as_str()),
        ];

        let service = Service {
            name: &self.hostname,
            priority: 0,
            weight: 0,
            service: "_soma",
            protocol: "_tcp",
            port: 9100,
            service_subtypes: &[],
            txt_kvs: &txt_kvs,
        };

        let answers = ServiceAnswers::new(&host, &service);
        let mut handler = HostAnswersMdnsHandler::new(answers);

        let socket = sockets.get_mut::<UdpSocket>(self.socket_handle);

        // Handle any incoming query packets.
        let mut rx_buf = [0u8; MDNS_MTU];
        let mut tx_buf = [0u8; MDNS_MTU];
        let mut recv_count = 0;
        while let Ok((data, meta)) = socket.recv() {
            if data.len() > rx_buf.len() {
                continue;
            }
            let n = data.len();
            rx_buf[..n].copy_from_slice(data);
            let src_is_mdns_port = meta.endpoint.port == MDNS_PORT;
            let request = MdnsRequest::Request {
                legacy: !src_is_mdns_port,
                multicast: true,
                data: &rx_buf[..n],
            };
            match handler.handle(request, &mut tx_buf) {
                Ok(MdnsResponse::Reply { data: reply, .. }) => {
                    // Unicast reply to legacy queries, multicast for
                    // standard mDNS queries.
                    let dest = if src_is_mdns_port {
                        IpEndpoint {
                            addr: IpAddress::Ipv4(MDNS_MULTICAST_V4),
                            port: MDNS_PORT,
                        }
                    } else {
                        meta.endpoint
                    };
                    let _ = socket.send_slice(reply, dest);
                }
                Ok(MdnsResponse::None) => {}
                Err(_) => {}
            }
            recv_count += 1;
            if recv_count >= 4 {
                break;
            }
        }

        // Send the one-shot unsolicited announcement once we know our
        // IP. This populates neighbor caches immediately instead of
        // waiting for the first query.
        if !self.announced {
            match handler.handle(MdnsRequest::None, &mut tx_buf) {
                Ok(MdnsResponse::Reply { data: reply, .. }) => {
                    let dest = IpEndpoint {
                        addr: IpAddress::Ipv4(MDNS_MULTICAST_V4),
                        port: MDNS_PORT,
                    };
                    if socket.send_slice(reply, dest).is_ok() {
                        println!(
                            "[mdns] announced {}._soma._tcp.local on {}",
                            self.hostname, ipv4
                        );
                        self.announced = true;
                    }
                }
                _ => {}
            }
        }
    }
}
