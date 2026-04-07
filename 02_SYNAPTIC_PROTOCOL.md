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

### 11.3 Connection Pooling

A SOMA maintains a pool of connections to frequently-used peers. Signals are multiplexed across pooled connections. New channels reuse existing connections rather than opening new TCP streams.
