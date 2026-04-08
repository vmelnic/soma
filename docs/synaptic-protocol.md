# Synaptic Protocol

## Overview

The Synaptic Protocol is the binary wire protocol for all SOMA-to-SOMA communication. It replaces HTTP, WebSocket, SSE, gRPC, and MQTT between SOMAs with a single, unified protocol that carries text, binary, streaming, and control signals.

**Synaptic Protocol vs MCP.** Two protocols coexist in the SOMA binary. MCP (Model Context Protocol) handles LLM-to-SOMA communication via JSON-RPC over HTTP. The Synaptic Protocol handles SOMA-to-SOMA communication via binary signals over TCP, Unix Domain Socket, or WebSocket. They serve separate roles and do not compete.

### Design Principles

- **Universal.** One protocol carries text, binary, streaming, and control signals.
- **Lightweight.** 26-byte minimum frame overhead (vs HTTP's 500-2000 bytes of headers).
- **Bidirectional.** Both sides send signals at any time. No request/response constraint.
- **Multiplexed.** Multiple channels on one connection (similar to HTTP/2 streams).
- **Backpressure-aware.** Slow consumers signal the sender to throttle.
- **Resumable.** Interrupted transfers resume from the last acknowledged chunk.
- **Binary-native.** Binary framing with optional human-readable debug mode.
- **Transport-agnostic.** Signals are framed identically regardless of transport.

---

## Signal Format

### Wire Format

Every signal is encoded as a single binary frame with this layout:

```
Offset  Size    Field               Notes
------  ------  ------------------  ----------------------------------------
0       2       magic               0x53 0x4D ("SM" for Soma)
2       1       version             0x20 = v2.0
3       1       flags               bit field (see Flags Byte below)
4       1       signal_type         enum value (see Signal Types below)
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

Minimum frame size (empty sender_id, metadata, and payload): 26 bytes.

All multi-byte fields use **big-endian** (network byte order). Rust's `u32::from_be_bytes()` and `u32::to_be_bytes()` handle byte swapping on little-endian platforms.

### Flags Byte

```
Bit 0: COMPRESSED      payload is zstd compressed
Bit 1: ENCRYPTED       payload is encrypted (ChaCha20-Poly1305)
Bit 2: CHUNKED         this is part of a multi-chunk transfer
Bit 3: FINAL_CHUNK     this is the last chunk in a chunked transfer
Bit 4: ACK_REQUESTED   sender wants acknowledgment
Bit 5: PRIORITY        high-priority signal (skip queue)
Bit 6-7: reserved
```

The PRIORITY flag ensures protocol-level signals (errors, control) bypass data congestion and are never blocked.

### Checksum

CRC32 (ISO 3309) computed over all bytes from offset 0 through end of payload, exclusive of the checksum itself. On checksum mismatch the receiver drops the frame and sends an ERROR signal with reason `checksum_mismatch`.

### Maximum Frame Size

Negotiated during handshake via `max_signal_size`. Default: 10 MB. Embedded: 4 KB. Signals exceeding the negotiated maximum are rejected with ERROR `signal_too_large`.

---

## Signal Types (24)

Every signal carries a 1-byte type code. Signal types are grouped by function:

### Protocol (connection lifecycle)

| Code   | Name           | Purpose                                        |
|--------|----------------|------------------------------------------------|
| `0x01` | HANDSHAKE      | Connection initialization with capabilities    |
| `0x02` | HANDSHAKE_ACK  | Connection accepted, negotiated parameters     |
| `0x03` | CLOSE          | Graceful disconnect                            |
| `0xF0` | PING           | Keepalive probe                                |
| `0xF1` | PONG           | Keepalive response                             |
| `0xFE` | ERROR          | Error signal (checksum, rate limit, etc.)      |
| `0xFF` | CONTROL        | Protocol-level control (backpressure, etc.)    |

### Data (intent and payload exchange)

| Code   | Name    | Purpose                                            |
|--------|---------|----------------------------------------------------|
| `0x10` | INTENT  | Natural language intent for processing             |
| `0x11` | RESULT  | Execution result (response to an intent)           |
| `0x20` | DATA    | Structured data (JSON/MessagePack in payload)      |
| `0x21` | BINARY  | Raw binary data (file, image, audio frame)         |

### Streaming (ordered frame sequences)

| Code   | Name         | Purpose                              |
|--------|--------------|--------------------------------------|
| `0x22` | STREAM_START | Begin a named stream on a channel    |
| `0x23` | STREAM_DATA  | Stream frame (audio/video/events)    |
| `0x24` | STREAM_END   | End a named stream                   |

### Chunked Transfer (resumable large files)

| Code   | Name       | Purpose                                       |
|--------|------------|-----------------------------------------------|
| `0x30` | CHUNK_START| Begin chunked transfer with metadata          |
| `0x31` | CHUNK_DATA | File chunk                                    |
| `0x32` | CHUNK_END  | Final chunk with reassembly info              |
| `0x33` | CHUNK_ACK  | Acknowledge received chunk (enables resumption)|

### Discovery (peer finding)

| Code   | Name         | Purpose                                   |
|--------|--------------|-------------------------------------------|
| `0x40` | DISCOVER     | Presence announcement with capabilities   |
| `0x41` | DISCOVER_ACK | Presence response                         |
| `0x42` | PEER_QUERY   | Ask about other known peers               |
| `0x43` | PEER_LIST    | Response with known peers                 |

### Pub/Sub (topic subscriptions)

| Code   | Name        | Purpose                       |
|--------|-------------|-------------------------------|
| `0x50` | SUBSCRIBE   | Subscribe to a channel/topic  |
| `0x51` | UNSUBSCRIBE | Unsubscribe from a channel    |

Unknown signal types received by a SOMA are silently ignored (logged as warning). The connection is NOT closed.

---

## Channels

A channel is a multiplexed stream within a single connection. Multiple channels share one TCP connection. Each signal carries a `channel_id` (uint32).

### Channel Types

| Channel ID   | Purpose                                      |
|--------------|----------------------------------------------|
| 0 (control)  | Protocol-level signals: HANDSHAKE, PING, DISCOVER |
| 1 (default)  | General data/intent signals                  |
| N (named)    | Application-specific streams: video, file upload |

### Channel Lifecycle

Channels are lightweight. Creating a channel costs one signal; destroying it costs one signal. No negotiation required.

```
STREAM_START  {channel: 42, metadata: {type: "video", codec: "h264"}}
STREAM_DATA   {channel: 42, payload: [video frame bytes]}
STREAM_DATA   {channel: 42, payload: [video frame bytes]}
...
STREAM_END    {channel: 42}
```

The receiver handles the channel or ignores it. Limits on concurrent channels per connection are negotiated during handshake (default: 256, embedded: 8).

---

## Transport Layer

### Connection Types

| Transport          | Use Case                                     |
|--------------------|----------------------------------------------|
| TCP                | Default. Reliable, ordered. Most SOMA-to-SOMA traffic. |
| WebSocket          | Browser-based renderers (browsers cannot open raw TCP). |
| Unix Domain Socket | Same-host SOMAs. Zero network overhead.      |
| QUIC (future)      | Unreliable networks, mobile. Built-in multiplexing. |
| In-process channel | SOMAs in same binary. Zero-copy.             |

The protocol is transport-agnostic. A SOMA does not know or care whether a connection is TCP, WebSocket, or Unix socket.

### Connection Lifecycle

```
SOMA-A                              SOMA-B
  |                                    |
  |-- TCP connect ------------------->|
  |                                    |
  |-- HANDSHAKE signal -------------->|
  |   {soma_id, version,              |
  |    capabilities, plugins}         |
  |                                    |
  |<-- HANDSHAKE_ACK -----------------|
  |   {soma_id, negotiated_version,   |
  |    negotiated_capabilities}       |
  |                                    |
  |   [connection established]        |
  |   [bidirectional signals flow]    |
  |                                    |
  |-- signals <---------------------> |
  |                                    |
  |-- CLOSE signal ------------------>|
  |                                    |
  |-- TCP close --------------------->|
```

### Handshake Negotiation

The HANDSHAKE signal carries the initiator's capabilities. The HANDSHAKE_ACK returns negotiated parameters.

```
SOMA-A -> SOMA-B: HANDSHAKE {
  metadata: {
    protocol_version: "2.0",
    supported_versions: ["2.0", "1.0"],
    soma_id: "helperbook-backend",
    capabilities: ["streaming", "chunked", "compression", "encryption"],
    plugins: ["postgres", "redis", "messaging"],
    max_signal_size: 10485760,
    max_channels: 256
  }
}

SOMA-B -> SOMA-A: HANDSHAKE_ACK {
  metadata: {
    protocol_version: "2.0",
    negotiated_version: "2.0",
    negotiated_capabilities: ["streaming", "chunked", "encryption"],
    soma_id: "helperbook-interface",
    compression: "zstd",
    encryption: "chacha20-poly1305",
    max_signal_size: 4194304
  }
}
```

Negotiated values use the minimum of both sides (e.g., the smaller `max_signal_size`). Capabilities use the intersection.

### Heartbeat (PING/PONG)

```
Keepalive interval:   30s (default), 60s (embedded)
Pong timeout:         10s (default)
Max missed pongs:     3 (default)

Timeline:
  t=0s:   Send PING {sequence: N}
  t=0-10s: Expect PONG {sequence: N}
  t=10s:  No PONG -> missed_pong_count += 1
  t=30s:  Send PING {sequence: N+1}
  ...
  3 consecutive missed pongs -> peer declared dead
```

Any received signal (not just PONG) resets the missed-pong counter. Active connections that exchange data signals do not need explicit PINGs.

### Session Tokens

Session tokens are exchanged during HANDSHAKE. They identify a logical session across reconnections, enabling subscription recovery and chunked transfer resumption without re-authorization. Default expiry: 24 hours.

---

## Data Patterns

### Intent/Response

The basic request/reply pattern:

```
A -> B: INTENT  {payload: "list files in /tmp"}
B -> A: RESULT  {payload: {files: ["a.txt", "b.txt"]}}
```

### Semantic UI Signals

Backend SOMAs send meaning, not markup. The Interface SOMA renders:

```
Backend -> Interface: DATA {
  payload: {
    view: "contact_list",
    data: [{name: "Ana", service: "Stylist", online: true}],
    actions: ["chat", "book", "favorite"]
  }
}
```

Standardized payload fields for semantic signals: `view`, `data`, `actions`, `filters`, `pagination`, `error`, `loading`.

### Chunked File Upload (Resumable)

```
A -> B: CHUNK_START {
  channel: 7,
  metadata: {
    filename: "profile.jpg",
    total_size: 2500000,
    chunk_size: 65536,
    total_chunks: 39,
    content_type: "image/jpeg",
    checksum_sha256: "abc123..."
  }
}

A -> B: CHUNK_DATA  {channel: 7, seq: 0, payload: [65536 bytes]}
B -> A: CHUNK_ACK   {channel: 7, seq: 0}

A -> B: CHUNK_DATA  {channel: 7, seq: 1, payload: [65536 bytes]}
B -> A: CHUNK_ACK   {channel: 7, seq: 1}
...
A -> B: CHUNK_END   {channel: 7, seq: 38, flags: FINAL_CHUNK}
B -> A: CHUNK_ACK   {channel: 7, seq: 38, metadata: {reassembled: true, verified: true}}
```

**Resumption after connection drop at chunk 20:**

```
A -> B: CHUNK_START {channel: 7, metadata: {resume_from: 21, ...}}
A -> B: CHUNK_DATA  {channel: 7, seq: 21, ...}
...
```

SHA-256 checksum in CHUNK_START enables end-to-end verification after reassembly.

### Streaming

For audio, video, or event streams:

```
A -> B: STREAM_START {
  channel: 42,
  metadata: {type: "audio", codec: "opus", sample_rate: 48000, channels: 1}
}

A -> B: STREAM_DATA {channel: 42, payload: [opus frame]}
A -> B: STREAM_DATA {channel: 42, payload: [opus frame]}
...
A -> B: STREAM_END  {channel: 42}
```

For WebRTC, the Synaptic Protocol carries only signaling (offer/answer/ICE candidates as DATA signals). Media flows directly peer-to-peer via WebRTC's own transport.

### Pub/Sub (SSE-like)

```
A -> B: SUBSCRIBE   {channel: 100, metadata: {topic: "chat:room-5"}}

B -> A: STREAM_DATA {channel: 100, payload: {type: "message", from: "Ana", text: "hello"}}
B -> A: STREAM_DATA {channel: 100, payload: {type: "typing", from: "Ion"}}

A -> B: UNSUBSCRIBE {channel: 100}
```

---

## Discovery

### Presence Broadcasting (Chemical Gradient)

SOMAs announce themselves to known peers via DISCOVER signals:

```
DISCOVER {
  sender: "soma-backend-1",
  payload: {
    address: "192.168.1.10:9001",
    plugins: ["postgres", "redis", "auth", "messaging"],
    conventions: ["query", "cache_get", "authenticate"],
    load: 0.3,
    capacity: 1000
  }
}
```

Receiving SOMAs store peer info in a PeerRegistry and may forward discovery signals to their own peers with decaying TTL. This creates a gradient: nearby SOMAs are discovered quickly, distant ones eventually.

### Peer Query

A SOMA can ask a peer about other SOMAs with specific capabilities:

```
A -> B: PEER_QUERY {payload: {need_plugin: "stripe"}}
B -> A: PEER_LIST  {payload: {
  peers: [{
    id: "soma-payments",
    address: "192.168.1.20:9003",
    plugins: ["stripe"],
    reachable_via: "soma-b",
    estimated_latency_ms: 15
  }]
}}
```

The `reachable_via` field serves as a relay hint for multi-hop routing.

---

## Security

### Encryption

Per-signal encryption using **ChaCha20-Poly1305** (AEAD). When the ENCRYPTED flag is set, the payload is encrypted. The 12-byte nonce and 16-byte authentication tag are appended to the frame.

### Key Exchange

**X25519 Diffie-Hellman** during the HANDSHAKE phase. Both sides exchange ephemeral public keys and derive a shared session key. The session key encrypts all subsequent signals on that connection (when the ENCRYPTED flag is used).

### Identity

**Ed25519 signing** for SOMA identity verification. Each SOMA has a keypair generated at first run and stored in its checkpoint. HANDSHAKE includes a signed token proving identity.

### Authorization

Each SOMA declares capabilities it exposes to peers. Authorization rules (which peers can send which signal types) are part of the SOMA's configuration. These are enforced at the connection level after handshake.

All cryptographic operations use the `dalek` crate family (x25519-dalek, ed25519-dalek, chacha20poly1305). These are real cryptographic implementations, not placeholders.

---

## Rate Limiting

### Per-Connection Limits

| Metric                        | Server Default | Embedded Default |
|-------------------------------|----------------|------------------|
| Signals per second            | 10,000         | 100              |
| Bytes per second              | 100 MB/s       | 100 KB/s         |
| Channels per connection       | 256            | 8                |
| Pending chunks per connection | 16             | 2                |
| Subscriptions per connection  | 100            | 4                |
| HANDSHAKE attempts/min per IP | 10             | 3                |

### Graduated Response

| Violation Level              | Response                                              |
|------------------------------|-------------------------------------------------------|
| First hit                    | CONTROL signal with `retry_after_ms`; drop signal     |
| Sustained (>10 seconds)     | Log warning, reduce window size for this connection   |
| Severe (>60 seconds)        | Close connection, blacklist peer for 5 minutes        |

The PeerBlacklist tracks blacklisted peers with TTL-based expiry. Rate limit state is per-connection; a SOMA with 10 connections handles 10x the per-connection limit in aggregate.

---

## Connection Recovery

### Auto-Reconnect

When a connection drops, the SOMA automatically attempts reconnection with exponential backoff:

```
Connection lost
  -> Wait 100ms  -> Attempt 1
  -> Wait 500ms  -> Attempt 2
  -> Wait 2s     -> Attempt 3
  -> Wait 5s     -> Attempt 4
  -> ...
  -> Exponential backoff, max 60 seconds between attempts
  -> Continue indefinitely until connected or SOMA shuts down
```

On successful reconnection:
1. New HANDSHAKE (full capability negotiation)
2. Re-authenticate (session token in HANDSHAKE)
3. Replay all active subscriptions

### Subscription Recovery

The SOMA keeps a local list of active subscriptions and automatically re-subscribes after reconnection:

```
1. Reconnect established
2. Replay all subscriptions:
   SUBSCRIBE {channel: 100, metadata: {topic: "chat:room-5"}}
   SUBSCRIBE {channel: 101, metadata: {topic: "presence"}}
3. Publisher sends catch-up burst of missed signals (if durable)
```

The application layer is unaware a disconnection occurred (unless it lasted long enough to cause visible delays).

### Stream Recovery

| Stream Type           | Recovery Strategy                                        |
|-----------------------|----------------------------------------------------------|
| Chunked file transfer | Resume from last CHUNK_ACK'd sequence                   |
| Audio/video stream    | Restart from current position (live streams don't replay)|
| SSE-like data stream  | Re-subscribe; publisher sends catch-up if available      |

### Offline Queue

When no connection is available, outbound signals queue locally:

- **Priority ordering**: Critical signals (intent results, data responses) drain first.
- **Expiry**: Ephemeral signals (typing indicators, presence) expire after 10 seconds.
- **Overflow**: When the queue exceeds `max_size`, lowest-priority signals are dropped first.
- **Limits**: Server default 10,000 queued signals; embedded default 50.

---

## Protocol Versioning

### Version Format

Protocol version is a single byte: `(major << 4) | minor`. Current version: `0x20` (v2.0).

### Version Mismatch Rules

| Scenario                                 | Behavior                                          |
|------------------------------------------|---------------------------------------------------|
| Same major, different minor              | Use lower minor. Higher side disables new features.|
| Different major, overlap in supported    | Use highest common major.                         |
| No overlap                               | Reject connection. ERROR: `incompatible_protocol`.|
| Unknown signal type received             | Ignore signal. Log warning. Do NOT close.         |

### Feature Capabilities

Capabilities are negotiated during handshake. A signal type is only used if both sides declared the corresponding capability:

| Capability    | Enables                                        |
|---------------|------------------------------------------------|
| `streaming`   | STREAM_START, STREAM_DATA, STREAM_END          |
| `chunked`     | CHUNK_START, CHUNK_DATA, CHUNK_END, CHUNK_ACK  |
| `compression` | COMPRESSED flag on any signal                  |
| `encryption`  | ENCRYPTED flag on any signal                   |
| `pubsub`      | SUBSCRIBE, UNSUBSCRIBE                         |
| `relay`       | Multi-hop forwarding                           |

If a SOMA receives a signal requiring a capability it did not negotiate, it sends ERROR with reason `capability_not_negotiated`.

---

## Signal Ordering

### Within a Channel

**Guaranteed in-order.** Signals on the same channel are delivered in send order. This holds across all transports (TCP is inherently ordered; each channel maps to a QUIC stream; Unix sockets are ordered; in-process channels preserve order).

This guarantee is critical for chunked transfers (chunks must arrive in sequence), streaming (frames maintain temporal order), and message delivery (chat messages appear in sent order).

### Across Channels

**No ordering guarantee.** A signal on channel 5 may arrive before or after a signal on channel 10, regardless of send order. Channels are independent streams.

### Across Connections

**No ordering guarantee.** If SOMA-A has two connections to SOMA-B, signals on different connections have no ordering relationship. The receiver uses sequence numbers for duplicate detection and reordering if needed.

### Sequence Numbers

Every signal carries a `sequence` field (uint32). Sequence numbers are:
- Per-connection, monotonically increasing
- Wrap around at `u32::MAX` (4 billion; not a practical concern)
- Used for request-response correlation, duplicate detection, ordering verification, and chunk reassembly

---

## Relay

### Multi-Hop Signal Forwarding

When SOMA-A wants to reach SOMA-C but has no direct connection, SOMA-B (connected to both) acts as relay:

```
SOMA-A <--- connected ---> SOMA-B <--- connected ---> SOMA-C
       (no direct connection between A and C)
```

### Relay Protocol

```
A -> B: Signal {
  sender: "soma-a",
  recipient: "soma-c",
  metadata: {
    relay_path: ["soma-a"],
    max_hops: 3
  },
  payload: ...
}

B sees: recipient is "soma-c", not me. B knows "soma-c" from discovery.

B -> C: Signal {
  sender: "soma-a",            // original sender preserved
  recipient: "soma-c",
  metadata: {
    relay_path: ["soma-a", "soma-b"],
    max_hops: 3,
    hop_count: 1
  },
  payload: ...                 // unchanged
}
```

### Relay Rules

- **max_hops**: Signal dropped if `hop_count >= max_hops`. Default: 3.
- **Loop prevention**: `relay_path` tracks visited SOMAs. A SOMA never relays a signal it has already relayed.
- **Capability gating**: Relay only if both sides negotiated the `relay` capability.
- **No payload inspection**: Relay nodes do not read or modify the payload. Encrypted signals remain encrypted end-to-end between originator and recipient.
- **Backpressure**: Overloaded relay nodes send CONTROL/backpressure rather than silently dropping.

---

## Pub/Sub Semantics

### Subscription Model

A SOMA subscribes to a topic on a channel. The publisher fans out signals to all subscribers:

```
Interface-1 -> Backend: SUBSCRIBE {channel: 100, metadata: {topic: "chat:room-5"}}
Interface-2 -> Backend: SUBSCRIBE {channel: 100, metadata: {topic: "chat:room-5"}}

Backend -> Interface-1: STREAM_DATA {channel: 100, payload: {type: "new_message", ...}}
Backend -> Interface-2: STREAM_DATA {channel: 100, payload: {type: "new_message", ...}}
```

### Topic Patterns

Topics are hierarchical strings with `/` separator. Wildcards (`*`) match any suffix:

```
"chat:room-5"            specific chat room
"presence:user-abc"      specific user's presence
"notifications:*"        all notifications (wildcard)
"calendar:changes:*"     all calendar changes
```

### Durability Modes

**Ephemeral (default).** If a subscriber is disconnected when a signal is published, the signal is lost. Suitable for typing indicators, presence updates, live video frames.

**Durable.** The publisher stores recent signals per topic in a ring buffer. On reconnect, the subscriber sends `last_seen_sequence` and receives a catch-up burst:

```
Interface reconnects after 30s offline:

Interface -> Backend: SUBSCRIBE {
  channel: 100,
  metadata: {topic: "chat:room-5", last_seen_sequence: 42}
}

Backend sends missed signals 43-47, then continues live:
Backend -> Interface: STREAM_DATA {channel: 100, seq: 43, payload: ...}
Backend -> Interface: STREAM_DATA {channel: 100, seq: 44, payload: ...}
...
Backend -> Interface: STREAM_DATA {channel: 100, seq: 48, payload: ...}  // live
```

Durable buffer size is configurable. Signals older than the buffer are permanently lost; the subscriber must perform a full data fetch.

---

## Signal Compression

### Algorithm

**zstd** (Zstandard) at level 3 (default). Configurable per SOMA via `protocol.compression_level`. Real-time signals use level 1 (fastest); large file chunks use level 6 (better ratio).

### When to Compress

| Condition                          | Compress? |
|------------------------------------|-----------|
| Payload < 256 bytes               | No (overhead exceeds savings)                     |
| Payload 256 B - 1 KB              | Optional (keep smaller of compressed/original)    |
| Payload > 1 KB                    | Yes                                               |
| Already-compressed binary (image) | No (incompressible data grows)                    |
| STREAM_DATA (real-time)           | No (latency matters more than size)               |

The sender decides per-signal. The COMPRESSED flag tells the receiver whether decompression is needed.

---

## Metadata Conventions

Metadata is schema-free MessagePack. Standardized field names ensure interoperability.

### Reserved Fields (protocol-level)

| Field              | Type         | Used In       | Description                              |
|--------------------|--------------|---------------|------------------------------------------|
| `trace_id`         | string       | All           | Distributed trace correlation ID         |
| `content_type`     | string       | DATA, BINARY  | MIME type of payload                     |
| `encoding`         | string       | DATA          | Payload encoding ("json", "msgpack")     |
| `total_size`       | uint         | CHUNK_START   | Total transfer size in bytes             |
| `chunk_size`       | uint         | CHUNK_START   | Size per chunk                           |
| `total_chunks`     | uint         | CHUNK_START   | Total chunk count                        |
| `resume_from`      | uint         | CHUNK_START   | Resume from this sequence                |
| `checksum_sha256`  | string       | CHUNK_START   | SHA-256 of complete data                 |
| `topic`            | string       | SUBSCRIBE     | Pub/sub topic name                       |
| `last_seen_sequence`| uint        | SUBSCRIBE     | For durable catch-up                     |
| `codec`            | string       | STREAM_START  | Media codec ("opus", "h264")             |
| `sample_rate`      | uint         | STREAM_START  | Audio sample rate                        |
| `relay_path`       | list[string] | Any relayed   | SOMAs that have relayed this signal      |
| `max_hops`         | uint         | Any relayed   | TTL for relay                            |
| `hop_count`        | uint         | Any relayed   | Current hop count                        |
| `retry_after_ms`   | uint         | CONTROL       | Backpressure retry delay                 |

### Application Metadata

Application-specific metadata uses namespaced keys to avoid collisions:

```
Correct:  {"helperbook.chat_id": "chat_5", "helperbook.msg_type": "text"}
Wrong:    {"chat_id": "chat_5"}   // could collide with another app
```

Protocol-reserved keys (above table) are NOT namespaced. Everything else should be.

---

## Size Limits

### Per-Component

| Component        | Server Max      | Embedded Max |
|------------------|-----------------|--------------|
| sender_id        | 255 bytes       | 32 bytes     |
| metadata         | 64 KB           | 256 bytes    |
| payload          | 10 MB (negotiated) | 4 KB     |
| total frame      | ~10 MB + overhead | ~4.3 KB   |
| topic string     | 256 bytes       | 64 bytes     |
| relay_path       | 16 entries      | 4 entries    |

### Per-Connection

| Limit                           | Server Default | Embedded Default |
|---------------------------------|----------------|------------------|
| Max concurrent connections      | 10,000         | 3                |
| Max channels per connection     | 256            | 8                |
| Max subscriptions per connection| 1,000          | 4                |
| Max pending chunks              | 64             | 2                |
| Max queued outbound signals     | 10,000         | 50               |
| Max inflight requests           | 1,000          | 4                |

The receiver's limits take precedence. Negotiation during handshake selects the minimum of both sides.

---

## Testing and Debugging

### soma-dump (Signal Capture)

A command-line tool analogous to `tcpdump` for Synaptic Protocol traffic:

```bash
# Capture all signals on a target
soma-dump 127.0.0.1:9999

# Example output:
# 14:30:01.234 INTENT soma-interface->soma-backend ch=1 seq=42
#   payload(23B): "list files in /tmp"
# 14:30:01.257 RESULT soma-backend->soma-interface ch=1 seq=42
#   payload(156B): {files: ["a.txt", ...]}
```

### Protocol Comparison

| Feature              | HTTP      | WebSocket | gRPC | MQTT | MCP         | Synaptic |
|----------------------|-----------|-----------|------|------|-------------|----------|
| Bidirectional        | No        | Yes       | Yes  | Yes  | No          | Yes      |
| Multiplexed          | HTTP/2    | No        | Yes  | No   | No          | Yes      |
| Binary-native        | No        | Yes       | Yes  | Yes  | No (JSON)   | Yes      |
| Streaming            | SSE       | Yes       | Yes  | No   | SSE         | Yes      |
| Resumable uploads    | No        | No        | No   | No   | No          | Yes      |
| Discovery            | No        | No        | No   | No   | No          | Yes      |
| Per-message overhead | 500-2000B | 2-6B      | ~20B | 2-5B | ~200B       | 26B+     |
| Semantic signals     | No        | No        | No   | No   | No          | Yes      |

---

## Implementation Reference

The Rust implementation lives in `soma-core/src/protocol/` with one module per concern:

| Module            | Spec Sections | Purpose                                     |
|-------------------|---------------|---------------------------------------------|
| `signal.rs`       | 4.2, 4.3      | SignalType enum (24 types), SignalFlags      |
| `codec.rs`        | 4, 17         | Binary wire format encode/decode, CRC32     |
| `connection.rs`   | 3, 14, 18     | TCP transport, heartbeat, session tokens    |
| `server.rs`       | 3, 12         | Listener, capability enforcement, metrics   |
| `client.rs`       | 14            | Auto-reconnect, subscription replay         |
| `router.rs`       | 13            | SignalRouter, pending requests, 30s timeout |
| `discovery.rs`    | 7             | PeerRegistry, TTL forwarding, PEER_QUERY    |
| `relay.rs`        | 15            | Multi-hop forwarding, loop prevention       |
| `chunked.rs`      | 6.3           | Resumable transfer, SHA-256 verification    |
| `pubsub.rs`       | 16            | Wildcards, durable mode, catch-up, fan-out  |
| `streaming.rs`    | 6.4           | Stream lifecycle, frame counting            |
| `rate_limit.rs`   | 20            | Graduated response, PeerBlacklist           |
| `offline_queue.rs`| 21            | Priority queue, expiry                      |
| `encryption.rs`   | 8             | ChaCha20-Poly1305, X25519, Ed25519         |
| `websocket.rs`    | 3.1           | WebSocket transport adapter                 |
| `unix_socket.rs`  | 3.1           | Unix Domain Socket transport                |
