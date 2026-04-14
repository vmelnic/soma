# Distributed Runtime

## Overview

SOMA instances communicate peer-to-peer for skill delegation, state synchronization, and resource sharing. Three transports serve different deployment needs: TCP/TLS for production clusters, WebSocket for browser/UI connections, and Unix sockets for fast local IPC. All transports share the same wire protocol (`TransportMessage`/`TransportResponse` JSON envelopes with length-prefixed framing).

## Transports

**TCP/TLS** (`TcpTransport`, `TlsTcpTransport`) -- Production transport. Plain TCP by default; when `tls_cert` and `tls_key` are configured, upgrades to TLS via rustls. The `TlsTcpTransport` builds a `rustls::ServerConfig` from PEM files. Falls back to plain TCP if TLS setup fails. Rate limiting is integrated at the transport layer.

**WebSocket** (`WsRemoteExecutor`, `start_ws_listener_background`) -- Browser and UI connections via `tokio-tungstenite`. Uses JSON text frames over the same `TransportMessage` protocol. Each call currently opens a fresh connection; connection pooling is deferred.

**Unix Socket** (`UnixRemoteExecutor`, `start_unix_listener_background`) -- Fast local IPC. Same 4-byte big-endian length prefix + JSON framing as TCP. Max frame size: 16 MB. Available only on Unix platforms (`#[cfg(unix)]`).

All three transports implement `RemoteExecutor` for outbound calls and `IncomingHandler` for inbound dispatch:

```rust
pub trait IncomingHandler: Send + Sync + 'static {
    fn handle(&self, msg: TransportMessage) -> TransportResponse;
}
```

`LocalDispatchHandler` bridges incoming requests to the local runtime, executing skills on behalf of remote peers and managing chunked transfer receivers.

## Configuration

The `[distributed]` section in `soma.toml`:

| Field | Default | Description |
|-------|---------|-------------|
| `bind` | `127.0.0.1:9999` | TCP listener bind address |
| `tls_cert` | -- | PEM certificate path (enables TLS) |
| `tls_key` | -- | PEM private key path |
| `tls_ca` | -- | CA certificate for client verification |
| `rate_limit_rps` | (default) | Max sustained requests/sec per peer |
| `burst_limit` | (default) | Extra burst capacity above steady-state |
| `blacklist_threshold` | (default) | Consecutive violations before peer ban |
| `rate_limit_enabled` | `true` | Toggle per-peer rate limiting |
| `blacklist_enabled` | `true` | Toggle blacklist mechanism |

CLI flags:

```
--listen <addr>       Start TCP listener (e.g., 127.0.0.1:9999)
--ws-listen <addr>    Start WebSocket listener
--unix-listen <path>  Start Unix socket listener
--peer <addr>         Register a remote TCP peer
--unix-peer <path>    Register a remote Unix socket peer
--discover-lan        Start an mDNS browser for `_soma._tcp.local.` and
                      auto-register any SOMA peers found on the LAN.
```

When TLS config is present and `--listen` is given, the runtime attempts TLS first, falling back to plain TCP on failure.

## LAN Peer Discovery (mDNS)

`--discover-lan` spawns a background `mdns-sd` browser thread that watches the multicast DNS service type `_soma._tcp.local.`. For every `ServiceResolved` event the browser inserts the discovered address into a shared peer-address map and appends a derived peer ID into the MCP server's shared peer list. `ServiceRemoved` events evict the entry. The flag is additive — it coexists with static `--peer` / `--unix-peer` registrations.

**Peer ID derivation.** The mDNS instance name (the left-most label of the fullname, e.g. `soma-esp32-ccdba79df9e8`) is prefixed with `lan-` to produce a stable `peer_id` the MCP layer uses for `invoke_remote_skill`, `transfer_routine`, and `list_peers`. The prefix avoids collision with static peer IDs from the config file. Because the MAC is baked into the instance name, the same physical leaf gets the same ID across reboots.

**Shared state between the MCP server and the listener runtime.** In MCP mode, the runtime spawns a separate listener runtime for incoming connections, independent of the MCP server. Both need visibility into the discovered peer list. The `peer_ids` field on the MCP server is `Arc<Mutex<Vec<String>>>` and the same `Arc` is passed to the discovery thread via `with_remote_shared(executor, peer_ids)`. Discovered peers appear in `list_peers` the instant the mDNS browser resolves them.

**Working against an embedded leaf.** The ESP32 leaf firmware advertises the same service type. After DHCP assigns an IPv4 address, the leaf's `MdnsResponder` (built on `edge-mdns` + a smoltcp UDP socket bound to `224.0.0.251:5353`) emits an unsolicited announcement for `soma-<chip>-<mac>._soma._tcp.local.` pointing at its TCP listener on port 9100. A server SOMA running `--discover-lan` sees the announcement within a few hundred milliseconds, registers the peer, and can invoke skills on the leaf without any static configuration. This is how `soma-project-esp32` drives real hardware from Claude Code or any other MCP client: the LLM calls `list_peers` → finds the leaf → calls `invoke_remote_skill {peer_id, skill_id, input}`.

## RemoteExecutor Trait

The core abstraction for all outbound distributed operations:

```rust
pub trait RemoteExecutor: Send + Sync {
    fn submit_goal(&self, peer_id: &str, request: &RemoteGoalRequest) -> Result<RemoteGoalResponse>;
    fn invoke_skill(&self, peer_id: &str, skill_id: &str, input: Value) -> Result<RemoteSkillResponse>;
    fn query_resource(&self, peer_id: &str, resource_type: &str, resource_id: &str) -> Result<RemoteResourceResponse>;
    fn transfer_schema(&self, peer_id: &str, schema: &SchemaTransfer) -> Result<()>;
    fn transfer_routine(&self, peer_id: &str, routine: &RoutineTransfer) -> Result<()>;
}
```

`RemoteGoalResponse` carries three statuses: `Accepted` (peer created a session), `Rejected`, or `RequestStricterPolicy` (peer needs more budget or tighter policy). `RemoteSkillResponse` includes skill ID, peer ID, success flag, observation, latency, and a trace ID for audit. `RemoteResourceResponse` supports `Snapshot` or `Delta` mode with versioning, provenance, and freshness.

`RemoteInvocationContext` can be attached to validate session budget and policy before any network call.

## Skill Delegation

The `DelegationManager` trait handles 5 delegation units:

| Unit | Method | Description |
|------|--------|-------------|
| Skill | `delegate_skill` | Single skill invocation on a remote peer |
| Subgoal | `delegate_subgoal` | Subgoal delegation, local session retained |
| Resource op | `delegate_resource_op` | Remote resource read/write |
| Schema/routine lookup | `delegate_schema_routine_lookup` | Remote schema or routine query |
| Session | `migrate_session` | Full session ownership transfer |

Every delegation carries a `DelegationContext` preserving: session ID, budget remaining, trust requirement, policy boundaries, trace cursor, and attribution. A `DelegationHandle` tracks the delegation lifecycle through statuses: `Pending`, `Accepted`, `Running`, `Completed`, `Failed`, `Refused`.

Session migration transfers all 8 required fields and fails closed if any cannot be sent. Session mirroring replicates state for redundancy without transferring authority.

## Peer Authentication

`PeerAuthenticator` trait -- all peers are untrusted by default:

```rust
pub trait PeerAuthenticator: Send + Sync {
    fn authenticate(&mut self, peer_id: &str, credentials: &PeerCredentials) -> Result<AuthResult>;
    fn is_authenticated(&self, peer_id: &str) -> bool;
    fn revoke(&mut self, peer_id: &str);
}
```

`PeerCredentials` carries a method (token, mTLS, signed capability) and an optional bearer token. `AuthResult` is either `Authenticated` (with optional elevated trust level) or `Rejected`. The default implementation grants `TrustLevel::Verified` on any non-empty token; production deployments should verify signatures or certificates.

Guard functions `require_authenticated` and `require_trust` enforce auth and minimum trust before distributed operations proceed.

## Heartbeat and Peer Liveness

`HeartbeatManager` sends periodic `Ping` messages (with nonce for RTT measurement) to all known peers. Configuration:

| Field | Default | Description |
|-------|---------|-------------|
| `interval_ms` | 5000 | Milliseconds between heartbeat rounds |
| `max_missed` | 3 | Consecutive misses before marking unavailable |
| `timeout_ms` | 2000 | Per-peer ping timeout |

`PeerHealth` tracks: last heartbeat timestamp, RTT in milliseconds, missed count, and alive flag. The manager updates `PeerSpec.last_seen` and `PeerSpec.current_load` in the peer registry and marks peers unavailable when they stop responding.

## Rate Limiting

Token bucket algorithm per peer with graduated response:

| Excess count | Decision | Effect |
|-------------|----------|--------|
| 1 | `Throttle` | Suggest backoff (`wait_ms`) |
| 2 -- N | `Deny` | Reject request outright |
| > threshold | `Blacklisted` | Peer banned for configurable period |

`PeerRateState` tracks fractional tokens, refill rate, and consecutive violation count. Tokens refill continuously based on elapsed time. Violation count resets to zero when a request is allowed. The `RateLimiter` is integrated into both `TcpTransport` and `TlsTcpTransport` behind `Arc<Mutex<>>`.

## Chunked Transfer

Resumable transfer with SHA-256 integrity for large payloads (schemas, routines). Default chunk size: 64 KB. Max payload: 256 MB.

Flow:
1. Sender transmits a `TransferManifest` (transfer ID, total bytes/chunks, overall SHA-256, per-chunk hashes).
2. Sender streams `Chunk` messages (index, data, per-chunk SHA-256). Chunks may arrive out of order.
3. Receiver verifies each chunk independently against its manifest hash.
4. On interruption, receiver sends `ResumeRequest` listing already-verified chunk indices; sender skips them.
5. Final reassembly verified against the manifest's overall SHA-256.

Wire messages: `ChunkedTransferStart`, `ChunkedTransferData`, `ChunkedTransferResume` in the `TransportMessage` enum. `LocalDispatchHandler` manages active `ChunkedReceiver` instances keyed by transfer ID.

## State Synchronization

`BeliefSync` trait for belief and resource synchronization between peers:

- **Belief sync** (`sync_belief`) -- Merges belief summaries with provenance and versioning. Distinguishes 5 fact types: asserted, observed, inferred, stale, remote. Returns `SyncResult` with outcome (`Merged`, `Conflict`, `Stale`), conflict state, and freshness.
- **Resource subscriptions** (`subscribe_resource`, `unsubscribe`) -- Three sync modes: `Snapshot`, `Delta`, `EventStream`. `SubscriptionRecord` tracks the peer, resource type, sync mode, and last seen version. `check_subscriptions` emits `ResourceChangeNotification` when changes are detected.

Staleness enforcement: `SyncResult::enforce_staleness(max_ms)` marks results stale when freshness exceeds the threshold.

## Result Streaming

`ObservationStreaming` trait for ordered observation delivery during distributed execution:

```rust
pub trait ObservationStreaming: Send + Sync {
    fn open_stream(&mut self, session_id: Uuid, source_peer: &str, replay_supported: bool) -> Result<StreamId>;
    fn receive_observation(&mut self, stream_id: &StreamId, observation: &StreamedObservation) -> Result<DeliveryStatus>;
    fn request_replay(&self, stream_id: &StreamId, from_sequence: u64) -> Result<Vec<StreamedObservation>>;
    fn close_stream(&mut self, stream_id: &StreamId) -> Result<()>;
}
```

Streams are ordered by sequence number within a session. `DeliveryStatus` detects: `InOrder`, `OutOfOrder`, `Duplicate`, `MissingPredecessors`, `StaleReplay`. Replay requests return stored observations from a given sequence point. The default implementation stores observations locally and forwards replay requests to the source peer when local data is unavailable.

## Failure Classification

Every `DistributedFailure` variant maps to a recoverability category:

| Recoverability | Failures | Recovery action |
|---------------|----------|-----------------|
| DelegatableToAnotherPeer | PeerUnreachable, UnsupportedSkill, UnsupportedResource, DelegationRefusal | Retry on different peer |
| Retryable | TransportFailure, StaleData, Timeout, PartialObservationStream | Retry same operation |
| TerminalForSession | AuthenticationFailure, BudgetExhaustion, MigrationFailure | Abort session |
| TerminalForActionOnly | AuthorizationFailure, TrustValidationFailure, ConflictingData, ReplayRejection, PolicyViolation | Abort action, session continues |

`classify_failure()` and `build_structured_failure()` in `remote.rs` produce `StructuredFailure` with the correct recovery guidance.

## Wire Protocol

All messages are `TransportMessage` variants (serde JSON, `#[serde(tag = "type")]`):

`InvokeSkill`, `QueryResource`, `SubmitGoal`, `TransferSchema`, `TransferRoutine`, `ChunkedTransferStart`, `ChunkedTransferData`, `ChunkedTransferResume`, `Ping`, `ListCapabilities`, `RemoveRoutine`.

Responses are `TransportResponse` variants: `SkillResult`, `ResourceResult`, `GoalResult`, `SchemaOk`, `RoutineOk`, `ChunkedAck`, `Error`, `Capabilities`, `RoutineStored`, `RoutineRemoved`.

`ListCapabilities`, `RemoveRoutine`, `Capabilities`, `RoutineStored`, and `RoutineRemoved` were added to support the embedded leaf use case where the server needs to introspect the leaf's registered primitives and revoke previously-transferred routines. All 1225 soma-next tests pass with the additions; existing peers that don't send these variants are unaffected (backward compatible).

Framing: 4-byte big-endian length prefix followed by JSON payload. Same format across TCP and Unix socket transports. WebSocket uses native text frames. Max frame size is 16 MB on all transports.

### Schema Tolerance for Embedded Leaves

Embedded leaves (soma-project-esp32) sometimes return a `SkillResult` with a subset of the fields soma-next uses internally — the leaf has no `peer_id` concept, no `trace_id`, and constructs the observation envelope by hand rather than going through the full runtime invocation path. To keep the server-side deserializer forgiving, `RemoteSkillResponse` uses:

```rust
pub struct RemoteSkillResponse {
    pub skill_id: String,
    #[serde(default)]
    pub peer_id: String,
    pub success: bool,
    #[serde(alias = "structured_result")]
    pub observation: serde_json::Value,
    pub latency_ms: u64,
    #[serde(default = "default_remote_skill_timestamp")]
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub trace_id: Uuid,
}
```

`#[serde(default)]` on `peer_id`, `timestamp`, and `trace_id` lets the server accept responses missing those fields. `#[serde(alias = "structured_result")]` maps the leaf's field name onto the server's `observation` field. `TcpRemoteExecutor` fills in the `peer_id` after deserialization so downstream code sees a fully-populated response regardless of the wire shape.
