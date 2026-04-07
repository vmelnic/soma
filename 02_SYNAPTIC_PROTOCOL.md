# Synaptic Protocol v2 — Specification

**Status:** Design  
**Depends on:** SOMA Core  
**Blocks:** All SOMA-to-SOMA communication, Interface SOMA, Backend SOMAs

---

## 1. Purpose

The Synaptic Protocol is the ONLY way SOMAs communicate. Not HTTP. Not gRPC. Not MQTT. One protocol for everything: text intents, structured data, binary files, audio/video streaming, real-time bidirectional signals, chunked uploads, and peer discovery.

It replaces HTTP, WebSocket, SSE, gRPC, and MQTT between SOMAs. HTTP exists only as a plugin for legacy browser compatibility — and even that is temporary until Interface SOMAs replace browsers.

---

## 2. Design Principles

- **Universal.** One protocol carries text, binary, streaming, and control signals.
- **Lightweight.** Minimal framing overhead. No HTTP headers, no verb/path/header ceremony.
- **Bidirectional.** Both sides send signals at any time. No request/response constraint.
- **Multiplexed.** Multiple channels on one connection (like HTTP/2 streams, but simpler).
- **Backpressure-aware.** Slow consumers can signal the sender to throttle.
- **Resumable.** Interrupted transfers (large files) can resume from last acknowledged chunk.
- **Binary-native.** Not text-based. Binary framing with optional human-readable debug mode.

---

## 3. Transport Layer

### 3.1 Connection Types

| Transport | Use Case |
|---|---|
| TCP | Default. Reliable, ordered. For most SOMA-to-SOMA communication. |
| Unix Domain Socket | Same-host SOMAs. Zero network overhead. |
| QUIC (future) | Unreliable networks, mobile. Built-in multiplexing and resumption. |
| In-process channel | SOMAs in same binary (cluster-in-one). Zero-copy. |

The protocol is transport-agnostic. Signals are framed the same regardless of transport. A SOMA doesn't know (or care) whether a synapse is TCP, Unix socket, or in-process.

### 3.2 Connection Lifecycle

```
SOMA-A                          SOMA-B
  │                                │
  ├── TCP connect ────────────────▶│
  │                                │
  ├── HANDSHAKE signal ───────────▶│
  │   {soma_id, version,           │
  │    capabilities, plugins}      │
  │                                │
  │◀── HANDSHAKE_ACK ─────────────┤
  │   {soma_id, version,           │
  │    capabilities, plugins}      │
  │                                │
  │   [connection established]     │
  │   [bidirectional signals flow] │
  │                                │
  ├── signals ◄──────────────────▶ │
  │                                │
  ├── CLOSE signal ───────────────▶│
  │                                │
  └── TCP close ──────────────────▶│
```

---

## 4. Signal Format

### 4.1 Wire Format (Binary)

```
┌──────────────────────────────────────────────┐
│ Magic (2 bytes): 0xS0 0xMA                    │
│ Version (1 byte): 0x02                        │
│ Flags (1 byte): see below                     │
│ Signal Type (1 byte): see below               │
│ Channel ID (4 bytes): uint32, big-endian      │
│ Sequence (4 bytes): uint32, big-endian        │
│ Sender ID length (1 byte)                     │
│ Sender ID (variable, UTF-8)                   │
│ Metadata length (4 bytes): uint32             │
│ Metadata (variable, MessagePack-encoded)      │
│ Payload length (4 bytes): uint32              │
│ Payload (variable, raw bytes)                 │
│ Checksum (4 bytes): CRC32 of entire frame     │
└──────────────────────────────────────────────┘

Total overhead: 22 bytes + sender_id + metadata
Compare: HTTP request averages 500-2000 bytes of headers
```

### 4.2 Flags Byte

```
Bit 0: COMPRESSED    — payload is zstd compressed
Bit 1: ENCRYPTED     — payload is encrypted (ChaCha20-Poly1305)
Bit 2: CHUNKED       — this is part of a multi-chunk transfer
Bit 3: FINAL_CHUNK   — this is the last chunk in a chunked transfer
Bit 4: ACK_REQUESTED — sender wants acknowledgment
Bit 5: PRIORITY      — high-priority signal (skip queue)
Bit 6-7: reserved
```

### 4.3 Signal Types

```
0x01  HANDSHAKE        — connection initialization
0x02  HANDSHAKE_ACK    — connection accepted
0x03  CLOSE            — graceful disconnect

0x10  INTENT           — natural language intent for processing
0x11  RESULT           — execution result

0x20  DATA             — structured data (JSON/MessagePack in payload)
0x21  BINARY           — raw binary data (file, image, audio frame)
0x22  STREAM_START     — begin a named stream on a channel
0x23  STREAM_DATA      — stream frame (audio/video/SSE-like)
0x24  STREAM_END       — end a named stream

0x30  CHUNK_START      — begin chunked transfer (large file)
0x31  CHUNK_DATA       — file chunk
0x32  CHUNK_END        — final chunk + reassembly info
0x33  CHUNK_ACK        — acknowledge received chunk (for resumption)

0x40  DISCOVER         — presence announcement
0x41  DISCOVER_ACK     — presence response with capabilities
0x42  PEER_QUERY       — ask about other known peers
0x43  PEER_LIST        — response with known peers

0x50  SUBSCRIBE        — subscribe to a channel
0x51  UNSUBSCRIBE      — unsubscribe from a channel

0xF0  PING             — keepalive
0xF1  PONG             — keepalive response
0xFE  ERROR            — error signal
0xFF  CONTROL          — protocol-level control
```

---

## 5. Channels

### 5.1 What Channels Are

A channel is a named, multiplexed stream within a single connection. Multiple channels share one TCP connection. Each signal carries a channel ID.

### 5.2 Channel Types

| Channel | Purpose | Example |
|---|---|---|
| 0 (control) | Protocol-level signals | HANDSHAKE, PING, DISCOVER |
| 1 (default) | General data/intent signals | Intents, results, data |
| N (named) | Application-specific streams | "video-call-42", "file-upload-7" |

### 5.3 Channel Lifecycle

```
STREAM_START {channel: 42, metadata: {type: "video", codec: "h264"}}
STREAM_DATA  {channel: 42, payload: [video frame bytes]}
STREAM_DATA  {channel: 42, payload: [video frame bytes]}
...
STREAM_END   {channel: 42}
```

Channels are lightweight. Creating/destroying a channel is one signal each. No negotiation. The receiver handles it or ignores it.

---

## 6. Data Patterns

### 6.1 Simple Intent/Response

```
A → B: INTENT {payload: "list files in /tmp"}
B → A: RESULT {payload: {files: ["a.txt", "b.txt"]}}
```

### 6.2 Semantic UI Signal

```
Backend → Interface: DATA {
  payload: {
    view: "contact_list",
    data: [{name: "Ana", service: "Stylist", online: true}],
    actions: ["chat", "book", "favorite"]
  }
}
```

### 6.3 Large File Upload (Chunked)

```
A → B: CHUNK_START {
  channel: 7,
  metadata: {
    filename: "profile.jpg",
    total_size: 2_500_000,
    chunk_size: 65536,
    total_chunks: 39,
    content_type: "image/jpeg",
    checksum_sha256: "abc123..."
  }
}

A → B: CHUNK_DATA {channel: 7, seq: 0, payload: [65536 bytes]}
B → A: CHUNK_ACK  {channel: 7, seq: 0}

A → B: CHUNK_DATA {channel: 7, seq: 1, payload: [65536 bytes]}
B → A: CHUNK_ACK  {channel: 7, seq: 1}
...
A → B: CHUNK_END  {channel: 7, seq: 38, flags: FINAL_CHUNK}
B → A: CHUNK_ACK  {channel: 7, seq: 38, metadata: {reassembled: true, verified: true}}
```

If connection drops after chunk 20, resumption:

```
A → B: CHUNK_START {channel: 7, metadata: {resume_from: 21, ...}}
A → B: CHUNK_DATA {channel: 7, seq: 21, ...}
...
```

### 6.4 Audio/Video Streaming

```
A → B: STREAM_START {
  channel: 42,
  metadata: {
    type: "audio",
    codec: "opus",
    sample_rate: 48000,
    channels: 1
  }
}

A → B: STREAM_DATA {channel: 42, payload: [opus frame]}
A → B: STREAM_DATA {channel: 42, payload: [opus frame]}
...
A → B: STREAM_END {channel: 42}
```

For video calls (WebRTC-like), each participant streams on their own channel. The WebRTC plugin handles codec encoding/decoding. Synaptic Protocol carries the frames.

For actual WebRTC (peer-to-peer media), the Synaptic Protocol carries only SIGNALING (offer/answer/ICE candidates as DATA signals). The media flows directly between peers via WebRTC's own transport. The SOMA orchestrates the signaling, the WebRTC plugin handles the media.

### 6.5 Server-Sent Events (SSE-like)

```
A subscribes to real-time updates:

A → B: SUBSCRIBE {channel: 100, metadata: {topic: "chat:room-5"}}

B pushes updates as they happen:
B → A: STREAM_DATA {channel: 100, payload: {type: "message", from: "Ana", text: "hello"}}
B → A: STREAM_DATA {channel: 100, payload: {type: "typing", from: "Ion"}}
B → A: STREAM_DATA {channel: 100, payload: {type: "message", from: "Ion", text: "hi"}}

A unsubscribes:
A → B: UNSUBSCRIBE {channel: 100}
```

### 6.6 Image/Video Preview (Upload + Preview Generation)

```
1. Interface SOMA → Backend SOMA: CHUNK_START + CHUNK_DATA (profile photo)
2. Backend SOMA: receives all chunks, reassembles
3. Backend SOMA → [Image Processing Plugin]: generate thumbnail
4. Backend SOMA → [Storage Plugin]: store original + thumbnail
5. Backend SOMA → Interface SOMA: DATA {
     payload: {
       type: "upload_complete",
       original_url: "soma://storage/photos/abc123.jpg",
       thumbnail: [inline base64 or binary signal with thumbnail bytes],
       size: 2500000,
       dimensions: {width: 3024, height: 4032}
     }
   }
6. Interface SOMA: renders preview from thumbnail data
```

---

## 7. Discovery

### 7.1 Presence Broadcasting (Chemical Gradient)

SOMAs announce themselves to known peers:

```
DISCOVER {
  sender: "soma-backend-1",
  payload: {
    address: "192.168.1.10:9001",
    plugins: ["postgres", "redis", "auth", "messaging"],
    conventions: ["query", "cache_get", "authenticate", ...],
    load: 0.3,        // current load factor
    capacity: 1000     // max concurrent signals/sec
  }
}
```

Receiving SOMAs store peer info and can forward discovery signals to their own peers (with decaying TTL), creating a gradient — nearby SOMAs are discovered quickly, distant ones eventually.

### 7.2 Peer Query

A SOMA can ask a peer about other SOMAs it knows:

```
A → B: PEER_QUERY {payload: {need_plugin: "stripe"}}
B → A: PEER_LIST {payload: {peers: [{id: "soma-payments", address: "...", plugins: ["stripe"]}]}}
```

---

## 8. Security

### 8.1 Encryption

- Signals can be encrypted per-signal (ENCRYPTED flag) using ChaCha20-Poly1305
- Key exchange during HANDSHAKE using X25519 Diffie-Hellman
- Or: the entire transport can be wrapped in TLS (for TCP) or DTLS (for QUIC)

### 8.2 Authentication

- HANDSHAKE includes a signed token proving SOMA identity
- Tokens signed with Ed25519 keys
- Key management: each SOMA has a keypair generated at first run, stored in its checkpoint

### 8.3 Authorization

- Each SOMA declares capabilities it exposes to peers
- A SOMA can restrict which peers can send which signal types
- Authorization rules are part of the SOMA's configuration

---

## 9. Backpressure and Flow Control

### 9.1 Window-Based Flow Control

Each channel has a receive window (similar to TCP flow control). The sender tracks how many unacknowledged bytes are in flight. If the window fills, the sender pauses.

### 9.2 Priority Signals

The PRIORITY flag allows critical signals (errors, control) to bypass the normal queue. This ensures protocol-level signals are never blocked by data congestion.

---

## 10. Comparison with Existing Protocols

| Feature | HTTP | WebSocket | gRPC | MQTT | Synaptic |
|---|---|---|---|---|---|
| Bidirectional | No | Yes | Yes | Yes | Yes |
| Multiplexed | HTTP/2 only | No | Yes | No | Yes |
| Binary-native | No | Yes | Yes | Yes | Yes |
| Streaming | Chunked/SSE | Yes | Yes | No | Yes |
| Resumable uploads | No | No | No | No | Yes |
| Discovery | No | No | No | No | Yes |
| Overhead per message | 500-2000B | 2-6B | ~20B | 2-5B | 22B + ids |
| Semantic signals | No | No | No | No | Yes |

---

## 11. Implementation Notes

### 11.1 Rust Implementation

```rust
pub struct Signal {
    pub signal_type: SignalType,
    pub channel_id: u32,
    pub sequence: u32,
    pub flags: SignalFlags,
    pub sender_id: String,
    pub metadata: Value,         // MessagePack-encoded
    pub payload: Bytes,          // raw bytes
}

pub struct SynapseConnection {
    transport: Transport,        // TCP, Unix, InProcess
    peer_id: String,
    channels: HashMap<u32, ChannelState>,
    encryption: Option<CipherState>,
}

impl SynapseConnection {
    pub async fn send(&self, signal: Signal) -> Result<()>;
    pub async fn recv(&self) -> Result<Signal>;
    pub async fn open_channel(&self, id: u32, metadata: Value) -> Result<()>;
    pub async fn close_channel(&self, id: u32) -> Result<()>;
}
```

### 11.2 Serialization

- Metadata: MessagePack (compact binary, schema-free, faster than JSON)
- Payload: raw bytes (interpretation is signal-type dependent)
- Debug mode: JSON metadata + hex payload dump for development

### 11.3 Metadata Naming Conventions

Metadata is schema-free MessagePack, but standardized field names ensure interoperability between SOMAs from different authors.

**Reserved metadata fields (protocol-level, used by Core):**

| Field | Type | Used In | Description |
|---|---|---|---|
| `trace_id` | string | All | Distributed trace correlation ID |
| `content_type` | string | DATA, BINARY | MIME type of payload (e.g., "application/json", "image/jpeg") |
| `encoding` | string | DATA | Payload encoding ("json", "msgpack") |
| `total_size` | uint | CHUNK_START | Total transfer size in bytes |
| `chunk_size` | uint | CHUNK_START | Size per chunk |
| `total_chunks` | uint | CHUNK_START | Total chunk count |
| `resume_from` | uint | CHUNK_START | Resume chunked transfer from this sequence |
| `checksum_sha256` | string | CHUNK_START | SHA-256 of complete data for verification |
| `topic` | string | SUBSCRIBE | Pub/sub topic name |
| `last_seen_sequence` | uint | SUBSCRIBE | For durable catch-up |
| `codec` | string | STREAM_START | Media codec ("opus", "h264", etc.) |
| `sample_rate` | uint | STREAM_START | Audio sample rate |
| `relay_path` | list[string] | Any relayed | SOMAs that have relayed this signal |
| `max_hops` | uint | Any relayed | TTL for relay |
| `hop_count` | uint | Any relayed | Current hop count |
| `retry_after_ms` | uint | CONTROL | Backpressure: retry delay suggestion |

**Application-level metadata conventions:**

Application-specific metadata uses namespaced keys to avoid collisions:

```
Correct:    metadata: { "helperbook.chat_id": "chat_5", "helperbook.msg_type": "text" }
Wrong:      metadata: { "chat_id": "chat_5" }  ← could collide with another app
```

Protocol-reserved keys (above table) are NOT namespaced. Everything else should be.

**Semantic signal conventions (for Interface SOMA):**

Semantic signals (Backend → Interface) use standardized top-level fields in the payload:

```json
{
  "view": "string — which view to render (contact_list, chat, calendar, ...)",
  "data": "array or object — the actual data to display",
  "actions": "array — available user actions",
  "filters": "array — available filter options",
  "pagination": "object — {page, per_page, total}",
  "error": "object — {code, message} if request failed",
  "loading": "bool — is data still being fetched"
}
```

These field names are conventions, not enforced by the protocol. But consistent naming means Interface SOMAs can develop reusable rendering patterns.

### 11.4 Connection Pooling

A SOMA maintains a pool of connections to frequently-used peers. Signals are multiplexed across pooled connections. New channels reuse existing connections rather than opening new TCP streams.

---

## 12. Protocol Versioning and Negotiation

### 12.1 Version Format

Protocol version is a single byte: `major.minor` packed as `(major << 4) | minor`. Current version: `0x20` (v2.0).

Major version change = wire format incompatible. Minor version change = new signal types added, old ones unchanged.

### 12.2 Handshake Negotiation

```
SOMA-A → SOMA-B: HANDSHAKE {
  metadata: {
    protocol_version: "2.0",
    supported_versions: ["2.0", "1.0"],
    soma_id: "helperbook-backend",
    soma_core_version: "0.2.0",
    capabilities: ["streaming", "chunked", "compression", "encryption"],
    plugins: ["postgres", "redis", "messaging"],
    max_signal_size: 10485760,
    max_channels: 256
  }
}

SOMA-B → SOMA-A: HANDSHAKE_ACK {
  metadata: {
    protocol_version: "2.0",
    negotiated_version: "2.0",          // highest mutually supported
    negotiated_capabilities: ["streaming", "chunked", "encryption"],  // intersection
    soma_id: "helperbook-interface",
    compression: "zstd",                // agreed compression algorithm
    encryption: "chacha20-poly1305",    // agreed encryption
    max_signal_size: 4194304            // min of both sides
  }
}
```

### 12.3 Version Mismatch Rules

| Scenario | Behavior |
|---|---|
| Same major, different minor | Use lower minor. Higher side disables features not in lower. |
| Different major, overlap in `supported_versions` | Use highest common major. |
| No overlap | Reject connection. Send ERROR signal with reason "incompatible_protocol". |
| Unknown signal type received | Ignore signal. Log warning. Do NOT close connection. |

### 12.4 Feature Capabilities

Capabilities are negotiated during handshake. A signal type is only used if both sides declared the corresponding capability:

| Capability | Enables Signal Types |
|---|---|
| `streaming` | STREAM_START, STREAM_DATA, STREAM_END |
| `chunked` | CHUNK_START, CHUNK_DATA, CHUNK_END, CHUNK_ACK |
| `compression` | COMPRESSED flag on any signal |
| `encryption` | ENCRYPTED flag on any signal |
| `pubsub` | SUBSCRIBE, UNSUBSCRIBE |
| `relay` | Multi-hop forwarding (Section 15) |

If a SOMA receives a signal requiring a capability it didn't negotiate, it sends ERROR with reason "capability_not_negotiated".

---

## 13. Signal Ordering Guarantees

### 13.1 Within a Channel

**Guaranteed in-order.** Signals on the same channel are always delivered in the order sent. This holds for all transport types:

- TCP: inherently ordered
- QUIC: ordered within a stream (each channel maps to a QUIC stream)
- Unix Domain Socket: inherently ordered
- In-process: channel (mpsc) preserves order

This guarantee is critical for:
- Chunked transfers: chunks must arrive in sequence for reassembly
- Streaming: audio/video frames must maintain temporal order
- Message delivery: chat messages must appear in sent order

### 13.2 Across Channels

**No ordering guarantee.** Signal on channel 5 may arrive before or after a signal on channel 10, regardless of send order. Channels are independent streams.

If cross-channel ordering matters (rare), the application layer must use sequence numbers in metadata and reorder at the receiver.

### 13.3 Across Connections

**No ordering guarantee.** If SOMA-A has two connections to SOMA-B (e.g., reconnected after drop), signals on different connections have no ordering relationship. The receiver uses sequence numbers to detect duplicates and reorder if needed.

### 13.4 Sequence Numbers

Every signal carries a `sequence` field (uint32). Sequence numbers are:
- Per-connection, monotonically increasing
- Wrap around at `u32::MAX` (4 billion — not a practical concern)
- Used for: request-response correlation, duplicate detection, ordering verification, chunk reassembly

---

## 14. Connection Recovery and Reconnection

### 14.1 Detecting Disconnection

| Method | Detects |
|---|---|
| TCP RST/FIN | Immediate hard disconnect |
| PING/PONG timeout | Silent disconnect (peer died without closing) |
| Write failure | Connection broken mid-signal |
| OS-level keepalive | Network path failure (configured via TCP_KEEPALIVE) |

### 14.2 Auto-Reconnect

When a connection drops, the SOMA automatically attempts reconnection:

```
Connection lost
       │
  [Wait 100ms]
  [Attempt 1: connect]
       │ fail
  [Wait 500ms]
  [Attempt 2: connect]
       │ fail
  [Wait 2s]
  [Attempt 3: connect]
       │ fail
  [Wait 5s]
  [Attempt 4: connect]
       │ ...
  [Exponential backoff, max 60s between attempts]
  [Continue indefinitely until connected or SOMA shuts down]
```

On successful reconnection:
1. New HANDSHAKE (full capability negotiation)
2. Re-authenticate (session token in HANDSHAKE)
3. Re-establish subscriptions (Section 14.3)

### 14.3 Subscription Recovery

When a connection drops, all SUBSCRIBE registrations on that connection are lost. On reconnection:

```
1. Reconnect established
2. SOMA replays all active subscriptions:
   SUBSCRIBE {channel: 100, metadata: {topic: "chat:room-5"}}
   SUBSCRIBE {channel: 101, metadata: {topic: "presence"}}
   ...
3. Publisher may send a "catch-up" burst of missed signals
   (if it implements durable subscriptions — see Section 16)
```

The SOMA keeps a local list of active subscriptions and automatically re-subscribes after reconnection. The application layer doesn't know a disconnection happened (unless it lasted long enough to cause visible delays).

### 14.4 Stream Recovery

Active streams (audio, video, file upload) are interrupted on disconnect. Recovery depends on stream type:

| Stream Type | Recovery |
|---|---|
| Chunked file transfer | Resume from last CHUNK_ACK'd sequence (Section 6.3) |
| Audio/video stream | Restart from current position (no backfill — live streams don't replay) |
| SSE-like data stream | Re-subscribe; publisher sends catch-up if available |

### 14.5 Session Continuity

A session token (from HANDSHAKE) identifies a logical session across reconnections. The receiver recognizes: "this is the same SOMA reconnecting, not a new peer." This enables:
- Subscription recovery without re-authorization
- Resume of pending request-response pairs
- Continuity of chunked transfers

Session token expires after configurable duration (default 24h). After expiry, full re-authentication is required.

---

## 15. Multi-Hop Relay and Forwarding

### 15.1 When Relay Is Needed

SOMA-A wants to reach SOMA-C but doesn't have a direct connection. SOMA-B is connected to both.

```
SOMA-A ←──── connected ────→ SOMA-B ←──── connected ────→ SOMA-C
       (no direct connection between A and C)
```

### 15.2 Relay Protocol

SOMA-A sends a signal with `recipient: "soma-c"`. SOMA-B receives it, sees that the recipient is not itself, and forwards:

```
A → B: Signal {
  sender: "soma-a",
  recipient: "soma-c",
  metadata: {
    relay_path: ["soma-a"],        // tracks hops
    max_hops: 3,                   // TTL
    ...
  },
  payload: ...
}

B sees: recipient is "soma-c", not me.
B knows peer "soma-c" (from discovery).
B forwards:

B → C: Signal {
  sender: "soma-a",               // original sender preserved
  recipient: "soma-c",
  metadata: {
    relay_path: ["soma-a", "soma-b"],  // append self
    max_hops: 3,
    hop_count: 1,
    ...
  },
  payload: ...                    // unchanged
}
```

### 15.3 Relay Rules

- **max_hops**: Signal is dropped if `hop_count >= max_hops`. Prevents infinite loops. Default: 3.
- **relay_path**: Tracks which SOMAs have relayed this signal. A SOMA never relays a signal it has already relayed (loop prevention).
- **Capability gating**: A SOMA only relays if it negotiated the `relay` capability with both the sender and the next hop.
- **No payload inspection**: The relaying SOMA does NOT read or modify the payload. It's a transparent forward. If the signal is encrypted, the relay cannot decrypt it (end-to-end encryption between A and C).
- **Backpressure**: If the relay SOMA is overloaded, it sends CONTROL/backpressure to the sender rather than silently dropping.

### 15.4 Relay Discovery

SOMA-A discovers SOMA-C exists via PEER_QUERY:

```
A → B: PEER_QUERY {payload: {need_plugin: "calendar"}}
B → A: PEER_LIST {payload: {
  peers: [{
    id: "soma-c",
    plugins: ["calendar"],
    reachable_via: "soma-b",      // relay hint
    estimated_latency_ms: 15
  }]
}}
```

SOMA-A now knows to address signals to "soma-c" and they'll be relayed via SOMA-B.

---

## 16. Pub/Sub Semantics

### 16.1 Subscription Model

A SOMA subscribes to a topic on a channel. The publisher sends signals on that channel to all subscribers.

```
Interface-1 → Backend: SUBSCRIBE {channel: 100, metadata: {topic: "chat:room-5"}}
Interface-2 → Backend: SUBSCRIBE {channel: 100, metadata: {topic: "chat:room-5"}}

// New message arrives
Backend → Interface-1: STREAM_DATA {channel: 100, payload: {type: "new_message", ...}}
Backend → Interface-2: STREAM_DATA {channel: 100, payload: {type: "new_message", ...}}
```

### 16.2 Fan-Out

The publisher maintains a subscriber list per topic. When a signal is published on a topic, it's sent to ALL subscribers. This is per-connection fan-out — the publisher sends N copies (one per subscriber connection).

For large fan-out (thousands of subscribers), the publisher can delegate to a fan-out SOMA (a SOMA specialized in high-throughput pub/sub, with a Redis plugin for subscriber tracking).

### 16.3 Message Durability

Two modes, configured per topic:

**Ephemeral (default):** If a subscriber is disconnected when a signal is published, the signal is lost. The subscriber misses it. Suitable for: typing indicators, presence updates, live video frames.

**Durable:** The publisher stores recent signals per topic (in-memory ring buffer or via a storage plugin). When a subscriber reconnects and re-subscribes, the publisher sends missed signals as a catch-up burst:

```
Interface reconnects after 30s offline:

Interface → Backend: SUBSCRIBE {
  channel: 100,
  metadata: {
    topic: "chat:room-5",
    last_seen_sequence: 42        // "I saw up to signal 42"
  }
}

Backend: checks buffer, finds signals 43-47 were published while offline

Backend → Interface: STREAM_DATA {channel: 100, seq: 43, payload: ...}
Backend → Interface: STREAM_DATA {channel: 100, seq: 44, payload: ...}
Backend → Interface: STREAM_DATA {channel: 100, seq: 45, payload: ...}
Backend → Interface: STREAM_DATA {channel: 100, seq: 46, payload: ...}
Backend → Interface: STREAM_DATA {channel: 100, seq: 47, payload: ...}
// now live
Backend → Interface: STREAM_DATA {channel: 100, seq: 48, payload: ...}
```

Durable buffer size is configurable. Signals older than the buffer are permanently lost — the subscriber must do a full data fetch instead.

### 16.4 Topic Patterns

Topics are strings. Hierarchical with `/` separator:

```
"chat:room-5"           — specific chat room
"presence:user-abc"     — specific user's presence
"notifications:*"       — all notifications (wildcard)
"calendar:changes:*"    — all calendar changes
```

Wildcard subscriptions (`*`) match any suffix. A subscriber to `notifications:*` receives signals published to `notifications:new_message`, `notifications:appointment`, etc.

---

## 17. Wire Format — Complete Byte Specification

### 17.1 Endianness

ALL multi-byte fields are **big-endian** (network byte order). This includes:

- channel_id (4 bytes, big-endian uint32)
- sequence (4 bytes, big-endian uint32)
- metadata_length (4 bytes, big-endian uint32)
- payload_length (4 bytes, big-endian uint32)
- checksum (4 bytes, big-endian CRC32)

ESP32 (Xtensa) and ARM are little-endian natively. The protocol implementation must byte-swap on these platforms. This is standard practice for network protocols (TCP/IP uses big-endian) and Rust's `u32::from_be_bytes()` / `u32::to_be_bytes()` handle it efficiently.

### 17.2 Complete Frame Layout

```
Offset  Size    Field               Notes
------  ------  ------------------  ---------------------------------
0       2       magic               0x53 0x4D ("SM" for Soma)
2       1       version             0x20 = v2.0
3       1       flags               bit field (see Section 4.2)
4       1       signal_type         enum value (see Section 4.3)
5       4       channel_id          big-endian uint32
9       4       sequence            big-endian uint32
13      1       sender_id_length    0-255
14      N       sender_id           UTF-8 bytes (N = sender_id_length)
14+N    4       metadata_length     big-endian uint32 (M)
18+N    M       metadata            MessagePack-encoded bytes
18+N+M  4       payload_length      big-endian uint32 (P)
22+N+M  P       payload             raw bytes
22+N+M+P 4      checksum            CRC32 of bytes [0 .. 22+N+M+P)
```

Total frame overhead: 26 bytes + sender_id + metadata (minimum frame with empty metadata, empty payload, 1-byte sender_id: 31 bytes).

### 17.3 Maximum Frame Size

Maximum frame size = 26 + 255 (max sender_id) + max_metadata + max_payload + 4 (checksum).

Negotiated during handshake via `max_signal_size`. Default: 10MB. Embedded: 4KB.

Signals exceeding the negotiated max are rejected by the receiver with ERROR "signal_too_large".

### 17.4 Checksum

CRC32 (ISO 3309 / ITU-T V.42) over all bytes from offset 0 to end of payload (exclusive of checksum itself). Receiver computes CRC32 over received bytes and compares. Mismatch → drop frame, send ERROR "checksum_mismatch".

On reliable transports (TCP, Unix socket), checksum failure indicates a bug, not corruption — TCP has its own checksums. On QUIC/UDP (future), the checksum provides an additional integrity layer.

---

## 18. Heartbeat and Dead Peer Detection

### 18.1 PING/PONG Mechanism

```
Keepalive interval: configurable (default 30s)
Pong timeout: configurable (default 10s)
Max missed pongs: configurable (default 3)

Timeline:
  t=0s:   Send PING {sequence: N}
  t=0-10s: Expect PONG {sequence: N}
  t=10s:  PONG not received → missed_pong_count += 1
  t=30s:  Send PING {sequence: N+1}
  t=40s:  PONG not received → missed_pong_count += 1
  t=60s:  Send PING {sequence: N+2}
  t=70s:  PONG not received → missed_pong_count = 3 → PEER DEAD
```

### 18.2 Dead Peer Handling

When a peer is declared dead:

1. Close the connection (if not already closed by transport)
2. Move all pending request-response pairs for this peer to error state (timeout)
3. Mark all active streams on this connection as interrupted
4. Remove peer from active peer list
5. Cancel subscriptions that were sourced from this peer
6. Log: "Peer {id} declared dead after {N} missed pongs"
7. Start auto-reconnect loop (Section 14.2)

In-flight signals (already sent but not acknowledged) are considered lost. The sender's application layer handles retransmission if needed (e.g., resend an intent to a different peer).

### 18.3 Asymmetric Heartbeat

Both sides independently send PINGs. A connection is kept alive if EITHER side receives data (any signal, not just PONG). Data signals reset the missed-pong counter — no need for PING if the connection is actively exchanging signals.

```rust
fn on_signal_received(&mut self) {
    self.last_received = Instant::now();
    self.missed_pong_count = 0;  // any data = peer is alive
}

fn should_send_ping(&self) -> bool {
    self.last_received.elapsed() > self.keepalive_interval
}
```

### 18.4 Embedded Heartbeat

On embedded, keepalive is less frequent (default 60s) to conserve bandwidth and power. Dead peer detection is slower (3 missed × 60s = 3 minutes). This is acceptable — embedded SOMAs typically communicate infrequently.

Battery-powered embedded SOMAs can disable keepalive entirely and rely on connection-on-demand: connect, exchange signals, disconnect. No persistent connection. Reconnect next time signals need to be sent.

---

## 19. Signal Compression

### 19.1 Algorithm

**zstd** (Zstandard) — fast compression and decompression, good ratios, widely available, Rust crate `zstd` is mature.

### 19.2 Negotiation

Compression support is negotiated during HANDSHAKE:

```
A: capabilities: ["compression"]
B: capabilities: ["compression"]
→ negotiated_capabilities: ["compression"], compression: "zstd"
```

If either side doesn't support compression, COMPRESSED flag is never set.

### 19.3 When to Compress

| Condition | Compress? |
|---|---|
| Payload < 256 bytes | No (overhead exceeds savings) |
| Payload 256B - 1KB | Optional (compress, keep smaller of compressed/original) |
| Payload > 1KB | Yes (almost always saves bytes) |
| Binary payload (already compressed image/video) | No (incompressible data gets larger) |
| STREAM_DATA (real-time) | No (latency matters more than size) |

The sender decides per-signal. The COMPRESSED flag tells the receiver whether decompression is needed.

### 19.4 Compression Level

- Default: zstd level 3 (fast, decent ratio)
- Configurable per SOMA: `protocol.compression_level = 3`
- Real-time signals use level 1 (fastest)
- Large file chunks use level 6 (better ratio, more time is acceptable)

### 19.5 Embedded Compression

ESP32 can run zstd decompression but compression is expensive (CPU and RAM). Embedded SOMAs typically:
- RECEIVE compressed signals (decompress — fast, low memory)
- SEND uncompressed signals (avoid compression overhead)

This is asymmetric but valid — the server-side SOMA compresses, the embedded SOMA decompresses.

---

## 20. Protocol-Level Rate Limiting

### 20.1 Why

Application-level rate limiting (in the Core spec) limits business operations. Protocol-level rate limiting protects the SOMA from signal floods — a misbehaving or compromised peer sending millions of signals per second.

### 20.2 Limits

| Metric | Default | Embedded Default |
|---|---|---|
| Signals per second (per connection) | 10,000 | 100 |
| Bytes per second (per connection) | 100MB/s | 100KB/s |
| Channels per connection | 256 | 8 |
| Pending chunks per connection | 16 | 2 |
| Subscriptions per connection | 100 | 4 |
| HANDSHAKE attempts per minute (per IP) | 10 | 3 |

### 20.3 Enforcement

```
Signal arrives
  │
  ├── Check: signals_this_second >= limit?
  │     Yes → send CONTROL/rate_limit {retry_after_ms: 100}
  │           drop signal
  │
  ├── Check: bytes_this_second >= limit?
  │     Yes → send CONTROL/rate_limit
  │           drop signal
  │
  └── Pass → route signal normally
```

Rate limit state is per-connection. A SOMA with 10 connections can handle 10× the per-connection limit in aggregate.

### 20.4 Graduated Response

| Violation Level | Response |
|---|---|
| First hit | CONTROL/rate_limit with retry_after |
| Sustained (>10s) | Log warning, reduce window size for this connection |
| Severe (>60s continuous) | Close connection, blacklist peer for 5 minutes |

---

## 21. Mobile and Unreliable Networks

### 21.1 Network Transition

Mobile devices switch between WiFi and cellular. Each switch typically breaks the TCP connection.

```
WiFi connected → TCP to backend SOMA established
  │
User leaves WiFi range
  │
TCP connection breaks (RST or timeout)
  │
Auto-reconnect triggers (Section 14.2)
  │
Cellular connection available → new TCP connection
  │
HANDSHAKE with same session token → session continuity
  │
Re-subscribe to all active topics → catch-up burst
  │
User experience: brief interruption (<5s), then resume
```

### 21.2 App Background/Foreground

When a mobile app goes to background:

```
Foreground: Synaptic connection active, real-time signals flowing

App → Background:
  Option A (aggressive): Close Synaptic connection immediately.
    Push notifications take over for critical signals.
    On foreground: reconnect, catch-up.
    
  Option B (gentle): Keep connection alive for configurable duration (60s).
    If app returns to foreground within 60s: seamless.
    After 60s: close connection, rely on push.
    
  Option C (persistent): Keep connection alive with reduced keepalive (120s).
    Consumes battery. Only for apps that need true real-time background.
```

The Interface SOMA decides based on its proprioception (battery level, user preferences, OS restrictions).

### 21.3 Connection Quality Sensing

The SOMA monitors connection quality:

```rust
pub struct ConnectionQuality {
    pub rtt_ms: f32,              // round-trip time (from PING/PONG)
    pub rtt_jitter_ms: f32,       // RTT variance
    pub signal_loss_rate: f32,    // fraction of signals that needed retry
    pub bandwidth_bytes_sec: u64, // estimated available bandwidth
    pub connection_age: Duration, // how long this connection has been alive
}
```

Based on quality, the SOMA can:
- Switch to lower compression (reduce latency on slow CPUs)
- Reduce signal frequency (batch updates instead of real-time)
- Pause non-critical streams (disable typing indicators on bad connections)
- Alert the Interface SOMA to show a "poor connection" indicator

### 21.4 Offline Queue

When no connection is available, the SOMA queues outbound signals:

```rust
pub struct OfflineQueue {
    signals: VecDeque<QueuedSignal>,
    max_size: usize,              // max queued signals
    max_age: Duration,            // drop signals older than this
}

pub struct QueuedSignal {
    signal: Signal,
    queued_at: Instant,
    priority: u8,                 // higher = sent first on reconnect
    max_retries: u8,
}
```

On reconnect, the queue drains in priority order. Expired signals are dropped. If the queue exceeds max_size, lowest-priority signals are dropped first.

Critical signals (intent results, data responses) have high priority. Ephemeral signals (typing indicators, presence) have low priority and short max_age (10s).

---

## 22. Size Limits

### 22.1 Per-Component Limits

| Component | Max Size | Embedded Max | Rationale |
|---|---|---|---|
| sender_id | 255 bytes | 32 bytes | Fits in 1-byte length field |
| metadata (encoded) | 64KB | 256 bytes | MessagePack is compact but can grow |
| payload | 10MB (negotiated) | 4KB | Configurable via handshake |
| total frame | ~10MB + overhead | ~4.3KB | Sum of above |
| channel_id | uint32 (4 billion) | uint32 | Never exhausted |
| sequence | uint32 (4 billion) | uint32 | Wraps, not exhausted |
| topic string | 256 bytes | 64 bytes | Hierarchical but bounded |
| relay_path | 16 entries | 4 entries | max_hops limits this |
| peer_id | 255 bytes | 32 bytes | Same as sender_id |
| chunk count | uint32 | uint16 | Max file: 4B × 64KB = 256TB (server), 65K × 4KB = 256MB (embedded) |

### 22.2 Connection Limits

| Limit | Server Default | Embedded Default |
|---|---|---|
| Max concurrent connections | 10,000 | 3 |
| Max channels per connection | 256 | 8 |
| Max subscriptions per connection | 1,000 | 4 |
| Max pending chunks per connection | 64 | 2 |
| Max queued outbound signals | 10,000 | 50 |
| Max inflight requests (awaiting response) | 1,000 | 4 |

### 22.3 Enforcement

Limits are enforced at connection setup (handshake negotiation) and at runtime (reject signals that would exceed limits). The receiver's limits take precedence — a server SOMA sending to an embedded SOMA must respect the embedded SOMA's small signal size and low channel count.

```
Handshake negotiation:
  Server offers: max_signal_size = 10MB, max_channels = 256
  Embedded offers: max_signal_size = 4KB, max_channels = 8
  Negotiated: max_signal_size = 4KB, max_channels = 8
  (minimum of both sides)
```

---

## 23. Testing and Debugging

### 23.1 Signal Capture (soma-dump)

A command-line tool that captures and displays Synaptic Protocol traffic, analogous to `tcpdump` for TCP:

```bash
# Capture all signals on port 9001
soma-dump --port 9001

# Output:
# 14:30:01.234 INTENT soma-interface→soma-backend ch=1 seq=42 
#   payload(23B): "list files in /tmp"
# 14:30:01.257 RESULT soma-backend→soma-interface ch=1 seq=42 
#   payload(156B): {files: ["a.txt", ...]}
# 14:30:05.100 STREAM_DATA soma-backend→soma-interface ch=100 seq=89 
#   payload(45B): {type: "typing", user: "Ana"}

# Filter by signal type
soma-dump --port 9001 --type INTENT,RESULT

# Filter by sender
soma-dump --port 9001 --sender soma-backend

# Hex dump payloads
soma-dump --port 9001 --hex

# Save to file for replay
soma-dump --port 9001 --output capture.synaptic
```

### 23.2 Signal Replay

Replay captured signals for testing:

```bash
# Replay a capture against a SOMA
soma-replay --input capture.synaptic --target localhost:9001

# Replay at 2x speed
soma-replay --input capture.synaptic --target localhost:9001 --speed 2.0

# Replay only INTENT signals (test mind inference)
soma-replay --input capture.synaptic --target localhost:9001 --type INTENT
```

### 23.3 Mock Peer

A mock SOMA for testing that responds with configurable patterns:

```bash
# Start a mock that echoes all intents back as results
soma-mock --port 9002 --mode echo

# Mock that responds to specific intents with fixed data
soma-mock --port 9002 --responses responses.json

# Mock that simulates slow responses (latency testing)
soma-mock --port 9002 --mode echo --delay 500ms

# Mock that drops 10% of signals (reliability testing)
soma-mock --port 9002 --mode echo --drop-rate 0.1
```

### 23.4 Protocol Conformance Tests

A test suite that verifies any Synaptic Protocol implementation against the spec:

```bash
soma-test-protocol --target localhost:9001

# Tests:
#  ✓ Handshake with valid credentials
#  ✓ Handshake version negotiation (downgrade)
#  ✓ Handshake with unknown version (reject)
#  ✓ Signal ordering within channel
#  ✓ Checksum validation (reject corrupted frame)
#  ✓ Max signal size enforcement
#  ✓ Channel limit enforcement
#  ✓ Rate limit response
#  ✓ PING/PONG keepalive
#  ✓ Graceful close
#  ✓ Chunked transfer with resume
#  ✓ Stream start/data/end lifecycle
#  ✓ Subscribe/unsubscribe
#  ✓ Compression round-trip
#  ✓ Encryption round-trip
#  ✓ Unknown signal type (ignored, not crash)
#  ✓ Oversized signal (rejected)
#  ✓ Concurrent channels
#  ✓ Connection recovery after drop
```

### 23.5 Embedded Protocol Testing

For embedded targets, testing is done via a host-side test harness:

```
[Host PC: soma-test-protocol]  ←── UART/TCP ──→  [ESP32: soma-embedded]

Host sends signals, verifies ESP32 responses.
Tests: handshake, basic intent/result, chunked transfer within 4KB limit,
keepalive with 60s interval, reconnection after simulated drop.
```
