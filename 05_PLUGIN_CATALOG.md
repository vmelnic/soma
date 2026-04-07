# SOMA Plugin Catalog

**Status:** Reference  
**Purpose:** Complete catalog of plugins to build for the SOMA ecosystem. Each entry is detailed enough to start implementation.

---

## Plugin Priority Tiers

| Tier | Meaning |
|---|---|
| **T0 — Core** | Required for any useful SOMA. Build first. |
| **T1 — Foundation** | Required for web/mobile applications. Build second. |
| **T2 — Features** | Common application features. Build as needed. |
| **T3 — Specialized** | Domain-specific or advanced. Build on demand. |

---

## 1. MCP Bridge — `mcp`

**Tier:** T0  
**Platforms:** server, desktop  
**Dependencies:** none

### What It Does

Connects SOMA to the Model Context Protocol ecosystem. One plugin that bridges to ALL MCP servers — instantly giving SOMA access to hundreds of external services without per-service plugins.

Also exposes SOMA as an MCP server, allowing external AI models (Claude, GPT, etc.) to orchestrate SOMAs.

### Conventions (Dynamic)

Conventions are discovered at runtime from connected MCP servers. Each MCP tool becomes a SOMA convention:

```
mcp.{server_name}.{tool_name}(args...)
```

Examples with a GitHub MCP server connected:
```
mcp.github.create_issue(repo, title, body) → issue_url
mcp.github.list_repos(org) → [repos]
mcp.github.create_pr(repo, branch, title, body) → pr_url
mcp.github.search_code(query) → [results]
```

### MCP Client Mode (SOMA uses MCP servers)

```toml
[plugins.mcp]
mode = "client"  # or "server" or "both"

[[plugins.mcp.servers]]
name = "github"
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "${SOMA_GITHUB_TOKEN}" }

[[plugins.mcp.servers]]
name = "slack"
transport = "sse"
url = "https://slack-mcp.example.com/sse"

[[plugins.mcp.servers]]
name = "filesystem"
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/home/data"]

[[plugins.mcp.servers]]
name = "postgres"
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-postgres"]
env = { DATABASE_URL = "${SOMA_DATABASE_URL}" }
```

### MCP Server Mode (External AI uses SOMA)

SOMA exposes all its loaded plugin conventions as MCP tools:

```
External AI → MCP → SOMA:
  Tool: "soma.process_intent"
  Args: { intent: "list all bookings for tomorrow" }
  
SOMA executes, returns result via MCP.
```

This lets Claude Code, Cursor, or any MCP-compatible AI orchestrate SOMA instances.

### Discovery Flow

```
1. Plugin loads → reads MCP server configs
2. For each server: spawn process / connect SSE
3. Call MCP tools/list → receive available tools
4. Convert each MCP tool to a SOMA CallingConvention:
   - name: "mcp.{server}.{tool_name}"
   - args: mapped from MCP inputSchema (JSON Schema → ArgSpec)
   - returns: Value::Map (MCP results are JSON objects)
5. Register conventions in catalog
6. If server disconnects: mark its conventions as unavailable
7. If server reconnects: re-discover, re-register
```

### LoRA Knowledge

The MCP plugin itself has minimal LoRA — it's a bridge, not a domain. However, each MCP server connection can ship LoRA training data:

```json
{
  "mcp_server": "github",
  "examples": [
    {
      "intents": ["create an issue about the login bug"],
      "program": [
        { "convention": "mcp.github.create_issue", "args": {...} }
      ]
    }
  ]
}
```

This can be auto-generated from MCP tool descriptions + example prompts.

### Implementation Notes

- Use the `mcp-client` Rust crate (or implement MCP protocol directly — it's JSON-RPC over stdio/SSE)
- MCP servers are child processes managed by this plugin
- Each MCP server runs in its own process for isolation
- If an MCP server crashes, only its conventions become unavailable — other MCP servers unaffected
- MCP resources (not just tools) can be exposed as read-only data sources

---

## 2. PostgreSQL — `postgres`

**Tier:** T0  
**Platforms:** server, desktop, rpi  
**Dependencies:** none  
**Native lib:** libpq

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `postgres.query` | sql: String, params: List | List<Map> | Execute SELECT, return rows |
| `postgres.execute` | sql: String, params: List | Int | Execute INSERT/UPDATE/DELETE, return affected rows |
| `postgres.query_one` | sql: String, params: List | Map or Null | Single row or null |
| `postgres.begin` | — | Handle | Start transaction |
| `postgres.commit` | txn: Handle | Void | Commit transaction |
| `postgres.rollback` | txn: Handle | Void | Rollback transaction |
| `postgres.query_stream` | sql: String, params: List | Stream<Map> | Streaming cursor for large results |
| `postgres.create_table` | name: String, columns: Map | Void | DDL: create table |
| `postgres.alter_table` | name: String, changes: Map | Void | DDL: alter table |
| `postgres.table_exists` | name: String | Bool | Check if table exists |
| `postgres.list_tables` | — | List<String> | List all tables |
| `postgres.table_schema` | name: String | Map | Get column definitions |

### Config

```toml
[plugins.postgres]
host = "localhost"
port = 5432
database = "helperbook"
username = "soma"
password_env = "SOMA_PG_PASSWORD"
max_connections = 10
query_timeout = "30s"
ssl_mode = "prefer"
```

### Cleanup Specs

- `begin` → cleanup: `rollback` (pass transaction handle)

### Rust Crate

`tokio-postgres` for async queries. Connection pool via `deadpool-postgres`.

---

## 3. Redis — `redis`

**Tier:** T0  
**Platforms:** server, desktop, rpi  
**Dependencies:** none

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `redis.get` | key: String | String or Null | Get value |
| `redis.set` | key: String, value: String, ttl: Int (optional) | Void | Set value with optional TTL |
| `redis.delete` | key: String | Bool | Delete key |
| `redis.exists` | key: String | Bool | Check existence |
| `redis.incr` | key: String | Int | Atomic increment |
| `redis.expire` | key: String, seconds: Int | Void | Set TTL on existing key |
| `redis.hget` | key: String, field: String | String or Null | Hash field get |
| `redis.hset` | key: String, field: String, value: String | Void | Hash field set |
| `redis.hgetall` | key: String | Map | Get all hash fields |
| `redis.lpush` | key: String, value: String | Int | Push to list |
| `redis.lrange` | key: String, start: Int, stop: Int | List<String> | List range |
| `redis.publish` | channel: String, message: String | Int | Pub/sub publish |
| `redis.subscribe` | channel: String | Stream<String> | Pub/sub subscribe (streaming) |
| `redis.keys` | pattern: String | List<String> | Find keys matching pattern |

### Config

```toml
[plugins.redis]
url = "redis://localhost:6379/0"
max_connections = 5
connection_timeout = "5s"
```

### Rust Crate

`redis-rs` with `tokio-comp` feature for async.

---

## 4. Filesystem — `filesystem`

**Tier:** T0  
**Platforms:** all (server, desktop, embedded, rpi)  
**Dependencies:** none

### Conventions

| Convention | Args | Returns | Cleanup |
|---|---|---|---|
| `fs.read_file` | path: String | String | — |
| `fs.read_bytes` | path: String | Bytes | — |
| `fs.write_file` | path: String, content: String | Void | — |
| `fs.write_bytes` | path: String, data: Bytes | Void | — |
| `fs.append_file` | path: String, content: String | Void | — |
| `fs.delete` | path: String | Void | — |
| `fs.rename` | from: String, to: String | Void | — |
| `fs.copy` | from: String, to: String | Void | — |
| `fs.exists` | path: String | Bool | — |
| `fs.list_dir` | path: String | List<String> | — |
| `fs.create_dir` | path: String | Void | — |
| `fs.file_info` | path: String | Map (size, modified, mode) | — |
| `fs.open_read` | path: String | Handle | `fs.close` |
| `fs.open_write` | path: String | Handle | `fs.close` |
| `fs.close` | handle: Handle | Void | — |
| `fs.read_chunk` | handle: Handle, size: Int | Bytes | — |
| `fs.write_chunk` | handle: Handle, data: Bytes | Int | — |

### Embedded Variant

On ESP32, filesystem conventions map to SPIFFS/LittleFS flash filesystem or SD card. Same convention names, different implementation.

### Rust Crate

`tokio::fs` for async. `std::fs` for sync/embedded.

---

## 5. HTTP Bridge — `http-bridge`

**Tier:** T1  
**Platforms:** server, desktop  
**Dependencies:** none

### What It Does

Bridges the legacy web to SOMA. Runs an HTTP server that converts HTTP requests to Synaptic signals and HTTP responses from SOMA results. This is how browsers that don't have an Interface SOMA communicate with a Backend SOMA.

Also provides HTTP client capabilities for calling external APIs.

### Server Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `http.listen` | port: Int | Handle | Start HTTP listener |
| `http.stop` | handle: Handle | Void | Stop listener |

Server mode is mostly automatic — the plugin receives HTTP requests and converts them to INTENT or DATA signals internally. The Mind doesn't explicitly manage HTTP serving.

### Client Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `http.get` | url: String, headers: Map (optional) | Map (status, headers, body) | HTTP GET |
| `http.post` | url: String, body: String, headers: Map | Map | HTTP POST |
| `http.put` | url: String, body: String, headers: Map | Map | HTTP PUT |
| `http.delete` | url: String, headers: Map | Map | HTTP DELETE |
| `http.request` | method: String, url: String, headers: Map, body: Bytes | Map | Generic request |

### Config

```toml
[plugins.http-bridge]
listen_port = 8080
cors_origins = ["*"]
max_request_size = "10MB"
request_timeout = "30s"
# Reverse proxy headers
trust_proxy = true
real_ip_header = "X-Forwarded-For"
```

### Rust Crate

`axum` for server, `reqwest` for client.

---

## 6. S3-Compatible Storage — `s3`

**Tier:** T1  
**Platforms:** server, desktop  
**Dependencies:** none

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `s3.put_object` | bucket: String, key: String, data: Bytes, content_type: String | Map (url, etag) | Upload object |
| `s3.get_object` | bucket: String, key: String | Bytes | Download object |
| `s3.delete_object` | bucket: String, key: String | Void | Delete object |
| `s3.list_objects` | bucket: String, prefix: String | List<Map> | List objects |
| `s3.presigned_url` | bucket: String, key: String, expires: Int | String | Generate presigned URL |
| `s3.head_object` | bucket: String, key: String | Map (size, type, modified) | Object metadata |
| `s3.multipart_start` | bucket: String, key: String, content_type: String | Handle | Start multipart upload |
| `s3.multipart_upload` | handle: Handle, part: Int, data: Bytes | String (etag) | Upload part |
| `s3.multipart_complete` | handle: Handle, parts: List | Map | Complete multipart |
| `s3.multipart_abort` | handle: Handle | Void | Abort multipart |

### Config

```toml
[plugins.s3]
endpoint = "https://s3.amazonaws.com"
bucket = "helperbook-media"
region = "eu-west-1"
access_key_env = "SOMA_S3_KEY"
secret_key_env = "SOMA_S3_SECRET"
# Also works with MinIO, Cloudflare R2, Backblaze B2
```

### Cleanup Specs

- `multipart_start` → cleanup: `multipart_abort`

### Rust Crate

`aws-sdk-s3` or `rust-s3`.

---

## 7. SMTP Email — `smtp`

**Tier:** T1  
**Platforms:** server, desktop  
**Dependencies:** none

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `smtp.send` | to: String, subject: String, body_text: String | Map (message_id) | Send plain text email |
| `smtp.send_html` | to: String, subject: String, body_html: String, body_text: String | Map | Send HTML email with text fallback |
| `smtp.send_with_attachment` | to: String, subject: String, body: String, attachment: Bytes, filename: String | Map | Send with attachment |
| `smtp.send_template` | to: String, template: String, variables: Map | Map | Send from named template |
| `smtp.verify_address` | email: String | Bool | Verify email format and MX record |

### Config

```toml
[plugins.smtp]
host = "smtp.provider.com"
port = 587
encryption = "starttls"
username_env = "SOMA_SMTP_USER"
password_env = "SOMA_SMTP_PASS"
from_address = "noreply@helperbook.app"
from_name = "HelperBook"
rate_limit = 100  # per hour
```

### Rust Crate

`lettre`.

---

## 8. SMS / Twilio — `twilio`

**Tier:** T1  
**Platforms:** server  
**Dependencies:** `http-bridge` (for webhook receiving)

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `twilio.send_sms` | to: String, body: String | Map (sid, status) | Send SMS |
| `twilio.send_otp` | to: String | Map (sid) | Send verification OTP |
| `twilio.verify_otp` | to: String, code: String | Bool | Verify OTP code |
| `twilio.lookup` | phone: String | Map (country, carrier, type) | Phone number lookup |

### Config

```toml
[plugins.twilio]
account_sid_env = "SOMA_TWILIO_SID"
auth_token_env = "SOMA_TWILIO_TOKEN"
from_number = "+1234567890"
verify_service_sid_env = "SOMA_TWILIO_VERIFY_SID"
```

### Rust Crate

`reqwest` (Twilio API is REST-based). Custom implementation wrapping their HTTP API.

---

## 9. Push Notifications — `push`

**Tier:** T1  
**Platforms:** server  
**Dependencies:** none

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `push.send_apns` | device_token: String, title: String, body: String, data: Map | Map (apns_id) | Apple Push Notification |
| `push.send_fcm` | device_token: String, title: String, body: String, data: Map | Map (message_id) | Firebase Cloud Messaging |
| `push.send` | device_token: String, platform: String, title: String, body: String, data: Map | Map | Auto-detect platform |
| `push.send_batch` | tokens: List<Map>, title: String, body: String, data: Map | List<Map> | Send to multiple devices |
| `push.register_token` | user_id: String, token: String, platform: String | Void | Register device token |
| `push.unregister_token` | token: String | Void | Unregister device token |

### Config

```toml
[plugins.push]
apns_key_file = "./certs/apns_key.p8"
apns_key_id = "ABC123"
apns_team_id = "DEF456"
apns_topic = "app.helperbook"
apns_environment = "production"  # or "sandbox"

fcm_credentials_file = "./certs/firebase-service-account.json"
```

### Rust Crate

`a2` (APNs), `fcm-rust` or direct HTTP/2 to FCM API.

---

## 10. Authentication — `auth`

**Tier:** T1  
**Platforms:** server  
**Dependencies:** `postgres` (or `sqlite`), `crypto`

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `auth.generate_otp` | phone: String | Map (otp_id, expires_at) | Generate and store OTP |
| `auth.verify_otp` | phone: String, code: String | Map (valid, user_id) | Verify OTP |
| `auth.create_session` | user_id: String, device_info: Map | Map (token, expires_at) | Create session token |
| `auth.validate_session` | token: String | Map (valid, user_id, device) | Validate session |
| `auth.revoke_session` | token: String | Void | Revoke session |
| `auth.revoke_all_sessions` | user_id: String | Int (count revoked) | Revoke all user sessions |
| `auth.list_sessions` | user_id: String | List<Map> | List active sessions |
| `auth.google_verify` | id_token: String | Map (user_id, email, name) | Verify Google Sign-In token |
| `auth.apple_verify` | id_token: String | Map (user_id, email) | Verify Apple Sign-In token |
| `auth.generate_totp_secret` | user_id: String | Map (secret, qr_url) | Setup 2FA |
| `auth.verify_totp` | user_id: String, code: String | Bool | Verify 2FA code |
| `auth.hash_token` | token: String | String | Hash for storage |

### Rust Crate

`jsonwebtoken`, `totp-rs`, `argon2` (password hashing for tokens).

---

## 11. Cryptography — `crypto`

**Tier:** T0  
**Platforms:** all  
**Dependencies:** none

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `crypto.hash_sha256` | data: Bytes | String (hex) | SHA-256 hash |
| `crypto.hash_argon2` | password: String | String | Password hash |
| `crypto.verify_argon2` | password: String, hash: String | Bool | Verify password hash |
| `crypto.random_bytes` | count: Int | Bytes | Cryptographic random bytes |
| `crypto.random_hex` | count: Int | String | Random hex string |
| `crypto.random_uuid` | — | String | UUID v4 |
| `crypto.sign_ed25519` | data: Bytes, key: Bytes | Bytes | Sign data |
| `crypto.verify_ed25519` | data: Bytes, signature: Bytes, pubkey: Bytes | Bool | Verify signature |
| `crypto.encrypt_aead` | plaintext: Bytes, key: Bytes | Bytes (nonce + ciphertext) | ChaCha20-Poly1305 encrypt |
| `crypto.decrypt_aead` | ciphertext: Bytes, key: Bytes | Bytes | ChaCha20-Poly1305 decrypt |
| `crypto.jwt_sign` | claims: Map, secret: String | String | Sign JWT |
| `crypto.jwt_verify` | token: String, secret: String | Map (claims) or Null | Verify JWT |
| `crypto.hmac_sha256` | data: Bytes, key: Bytes | Bytes | HMAC-SHA256 |

### Embedded Variant

On ESP32, uses hardware crypto acceleration (AES, SHA) where available. Subset of conventions: hash, random, hmac. No JWT on embedded (too heavy).

### Rust Crate

`ring` or `rustcrypto` family (`sha2`, `chacha20poly1305`, `ed25519-dalek`, `argon2`). `jsonwebtoken` for JWT.

---

## 12. Image Processing — `image-proc`

**Tier:** T1  
**Platforms:** server, desktop  
**Dependencies:** none

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `image.thumbnail` | data: Bytes, width: Int, height: Int | Bytes | Generate thumbnail |
| `image.resize` | data: Bytes, width: Int, height: Int, fit: String | Bytes | Resize (cover/contain/fill) |
| `image.crop` | data: Bytes, x: Int, y: Int, w: Int, h: Int | Bytes | Crop region |
| `image.format_convert` | data: Bytes, target_format: String | Bytes | Convert format (jpg→webp, etc.) |
| `image.strip_exif` | data: Bytes | Bytes | Remove EXIF metadata |
| `image.info` | data: Bytes | Map (width, height, format, size) | Image metadata |
| `image.compress` | data: Bytes, quality: Int | Bytes | Compress (0-100 quality) |
| `image.blur_region` | data: Bytes, x: Int, y: Int, w: Int, h: Int | Bytes | Blur a region (privacy) |

### Config

```toml
[plugins.image-proc]
max_input_size = "20MB"
max_output_size = "10MB"
default_thumbnail_size = [200, 200]
default_quality = 85
allowed_formats = ["jpeg", "png", "webp", "gif"]
```

### Rust Crate

`image` crate. `kamadak-exif` for EXIF stripping.

---

## 13. Audio Processing — `audio-proc`

**Tier:** T2  
**Platforms:** server, desktop  
**Dependencies:** none

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `audio.info` | data: Bytes | Map (duration, format, channels, sample_rate) | Audio metadata |
| `audio.convert` | data: Bytes, target_format: String | Bytes | Format conversion |
| `audio.normalize` | data: Bytes | Bytes | Normalize volume |
| `audio.trim` | data: Bytes, start_ms: Int, end_ms: Int | Bytes | Trim audio |
| `audio.duration` | data: Bytes | Float (seconds) | Get duration |
| `audio.waveform` | data: Bytes, width: Int | List<Float> | Generate waveform data for visualization |

### Rust Crate

`symphonia` for decoding, `opus` for encoding. Custom FFI to `libopus` for voice message codec.

---

## 14. Video Processing — `video-proc`

**Tier:** T2  
**Platforms:** server  
**Dependencies:** none (bundles ffmpeg libs)

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `video.info` | data: Bytes | Map (duration, width, height, codec, fps) | Video metadata |
| `video.thumbnail` | data: Bytes, time_ms: Int | Bytes (image) | Extract frame as thumbnail |
| `video.compress` | data: Bytes, quality: String | Bytes | Compress (low/medium/high) |
| `video.convert` | data: Bytes, target_format: String | Bytes | Format conversion |
| `video.trim` | data: Bytes, start_ms: Int, end_ms: Int | Bytes | Trim video |

### Rust Crate

FFI to `ffmpeg` libraries, or `gstreamer` Rust bindings.

---

## 15. WebRTC — `webrtc`

**Tier:** T2  
**Platforms:** server (signaling), browser/mobile (media)  
**Dependencies:** none

### What It Does

Handles WebRTC signaling through SOMA's Synaptic Protocol. The actual media (audio/video) flows peer-to-peer between clients. The SOMA orchestrates the connection setup.

### Conventions (Signaling — Server Side)

| Convention | Args | Returns | Description |
|---|---|---|---|
| `webrtc.create_room` | room_id: String | Map (room_id, ice_servers) | Create signaling room |
| `webrtc.join_room` | room_id: String, peer_id: String | Map (existing_peers) | Join room |
| `webrtc.leave_room` | room_id: String, peer_id: String | Void | Leave room |
| `webrtc.relay_signal` | room_id: String, from: String, to: String, signal: Map | Void | Relay SDP/ICE between peers |

### Conventions (Client Side — Interface SOMA Plugin)

| Convention | Args | Returns | Description |
|---|---|---|---|
| `webrtc.create_offer` | config: Map | Map (sdp) | Create WebRTC offer |
| `webrtc.create_answer` | offer_sdp: Map | Map (sdp) | Create WebRTC answer |
| `webrtc.add_ice` | candidate: Map | Void | Add ICE candidate |
| `webrtc.get_stream` | — | Handle (media stream) | Get local media stream |

### Config

```toml
[plugins.webrtc]
ice_servers = [
  { urls = "stun:stun.l.google.com:19302" },
  { urls = "turn:turn.example.com:3478", username = "soma", credential_env = "SOMA_TURN_PASS" }
]
max_rooms = 100
room_timeout = "1h"
```

### Rust Crate

`webrtc-rs` for server-side. Client-side uses browser/native WebRTC APIs via the renderer plugin.

---

## 16. SQLite — `sqlite`

**Tier:** T1  
**Platforms:** desktop, mobile, embedded (with enough flash)  
**Dependencies:** none

### Conventions

Same as `postgres` conventions but with SQLite-specific behavior:

| Convention | Args | Returns |
|---|---|---|
| `sqlite.query` | sql: String, params: List | List<Map> |
| `sqlite.execute` | sql: String, params: List | Int |
| `sqlite.query_one` | sql: String, params: List | Map or Null |
| `sqlite.begin` | — | Handle |
| `sqlite.commit` | txn: Handle | Void |
| `sqlite.rollback` | txn: Handle | Void |
| `sqlite.table_exists` | name: String | Bool |
| `sqlite.list_tables` | — | List<String> |
| `sqlite.table_schema` | name: String | Map |

### Config

```toml
[plugins.sqlite]
path = "./data/helperbook.db"
journal_mode = "wal"
busy_timeout = "5s"
```

### Rust Crate

`rusqlite` with `bundled` feature (includes SQLite).

---

## 17. Geolocation — `geo`

**Tier:** T1  
**Platforms:** server, desktop, mobile  
**Dependencies:** none

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `geo.distance` | lat1: Float, lon1: Float, lat2: Float, lon2: Float | Float (km) | Haversine distance |
| `geo.within_radius` | lat: Float, lon: Float, radius_km: Float, points: List<Map> | List<Map> | Filter points within radius |
| `geo.geocode` | address: String | Map (lat, lon, formatted_address) | Address to coordinates |
| `geo.reverse_geocode` | lat: Float, lon: Float | Map (address, city, country) | Coordinates to address |
| `geo.bounding_box` | lat: Float, lon: Float, radius_km: Float | Map (min_lat, max_lat, min_lon, max_lon) | Bounding box for radius query |

### Config

```toml
[plugins.geo]
geocoding_provider = "nominatim"  # or "google", "mapbox"
geocoding_api_key_env = "SOMA_GEO_API_KEY"  # not needed for nominatim
```

### Rust Crate

`geo` crate for calculations. `reqwest` for geocoding API calls.

---

## 18. Text Search — `search`

**Tier:** T1  
**Platforms:** server, desktop  
**Dependencies:** none

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `search.index` | collection: String, id: String, text: String, metadata: Map | Void | Index a document |
| `search.search` | collection: String, query: String, limit: Int | List<Map> (id, score, highlights) | Full-text search |
| `search.semantic_search` | collection: String, query: String, limit: Int | List<Map> | Embedding-based semantic search |
| `search.remove` | collection: String, id: String | Void | Remove from index |
| `search.suggest` | collection: String, prefix: String, limit: Int | List<String> | Autocomplete suggestions |
| `search.synonym_match` | query: String, candidates: List<String> | List<Map> (candidate, score) | Semantic similarity matching |

### Config

```toml
[plugins.search]
engine = "tantivy"  # or "meilisearch" (external)
index_dir = "./data/search"
embedding_model = "sentence-transformers/all-MiniLM-L6-v2"
```

### Rust Crate

`tantivy` for full-text search. `ort` for embedding inference (sentence transformers).

---

## 19. Calendar / Scheduling — `calendar`

**Tier:** T2  
**Platforms:** server, desktop  
**Dependencies:** `postgres` or `sqlite`

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `calendar.create_event` | title: String, start: String, end: String, data: Map | Map (event_id) | Create event |
| `calendar.update_event` | event_id: String, changes: Map | Void | Update event |
| `calendar.delete_event` | event_id: String | Void | Delete event |
| `calendar.list_events` | start: String, end: String, filters: Map | List<Map> | List events in range |
| `calendar.check_conflict` | start: String, end: String, participant: String | Bool | Check for conflicts |
| `calendar.find_available` | participant: String, date: String, duration: Int | List<Map> (available slots) | Find open slots |
| `calendar.set_schedule` | participant: String, schedule: Map | Void | Set working hours |
| `calendar.get_schedule` | participant: String | Map | Get working hours |
| `calendar.create_reminder` | event_id: String, before_minutes: Int | Map (reminder_id) | Set reminder |
| `calendar.export_ical` | event_id: String | String (iCal format) | Export as iCal |
| `calendar.recurrence` | event_id: String, pattern: Map | Void | Set recurrence (daily, weekly, monthly) |

---

## 20. Messaging — `messaging`

**Tier:** T2  
**Platforms:** server  
**Dependencies:** `postgres` or `sqlite`, `redis` (optional, for pub/sub)

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `msg.send` | chat_id: String, sender: String, type: String, content: String | Map (message_id, timestamp) | Send message |
| `msg.get_messages` | chat_id: String, before: String, limit: Int | List<Map> | Get message history |
| `msg.mark_read` | chat_id: String, user_id: String, message_id: String | Void | Mark as read |
| `msg.mark_delivered` | message_ids: List<String> | Void | Mark as delivered |
| `msg.edit` | message_id: String, new_content: String | Void | Edit message (within time limit) |
| `msg.delete` | message_id: String, mode: String | Void | Delete (for_me / for_all) |
| `msg.create_chat` | type: String, members: List<String>, name: String | Map (chat_id) | Create chat (direct/group) |
| `msg.add_member` | chat_id: String, user_id: String | Void | Add group member |
| `msg.remove_member` | chat_id: String, user_id: String | Void | Remove group member |
| `msg.typing` | chat_id: String, user_id: String | Void | Send typing indicator |
| `msg.unread_count` | user_id: String | Map (total, per_chat) | Get unread counts |
| `msg.search` | user_id: String, query: String | List<Map> | Search messages |
| `msg.pin` | chat_id: String | Void | Pin chat |
| `msg.mute` | chat_id: String, until: String | Void | Mute notifications |
| `msg.archive` | chat_id: String | Void | Archive chat |

---

## 21. Reviews / Ratings — `reviews`

**Tier:** T2  
**Platforms:** server  
**Dependencies:** `postgres` or `sqlite`

### Conventions

| Convention | Args | Returns |
|---|---|---|
| `reviews.create` | reviewer: String, reviewed: String, appointment_id: String, rating: Int, feedback: String, tags: List | Map (review_id) |
| `reviews.respond` | review_id: String, response: String | Void |
| `reviews.get_for_user` | user_id: String, limit: Int, offset: Int | List<Map> |
| `reviews.aggregate` | user_id: String | Map (avg_rating, count, tag_counts) |
| `reviews.report` | review_id: String, reason: String | Void |

---

## 22. Analytics — `analytics`

**Tier:** T2  
**Platforms:** server  
**Dependencies:** `postgres` or `sqlite`

### Conventions

| Convention | Args | Returns |
|---|---|---|
| `analytics.track` | event: String, user_id: String, properties: Map | Void |
| `analytics.query` | metric: String, period: String, filters: Map | Map (values, labels) |
| `analytics.dashboard` | user_id: String, period: String | Map (comprehensive stats) |
| `analytics.funnel` | steps: List<String>, period: String | List<Map> (step, count, rate) |
| `analytics.cohort` | period: String, group_by: String | Map (retention matrix) |

---

## 23. Localization — `i18n`

**Tier:** T2  
**Platforms:** all  
**Dependencies:** none

### Conventions

| Convention | Args | Returns |
|---|---|---|
| `i18n.translate` | key: String, locale: String, params: Map | String |
| `i18n.format_date` | date: String, locale: String, format: String | String |
| `i18n.format_currency` | amount: Float, currency: String, locale: String | String |
| `i18n.format_distance` | km: Float, locale: String | String |
| `i18n.detect_language` | text: String | String (locale code) |
| `i18n.available_locales` | — | List<String> |

### Config

```toml
[plugins.i18n]
default_locale = "en"
supported_locales = ["en", "ro", "ru"]
translations_dir = "./data/translations"
```

---

## 24. AI Inference — `ai`

**Tier:** T2  
**Platforms:** server, desktop  
**Dependencies:** none

### What It Does

Runs additional ML models for features beyond the Mind's core inference. Smart replies, intent detection in chat, embeddings for semantic search, text classification.

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `ai.smart_replies` | conversation: List<Map>, count: Int | List<String> | Generate reply suggestions |
| `ai.detect_intent` | text: String | Map (intent, entities, confidence) | Detect scheduling/pricing/location intent |
| `ai.summarize` | messages: List<String> | String | Conversation summary |
| `ai.embed_text` | text: String | List<Float> | Text embedding vector |
| `ai.embed_batch` | texts: List<String> | List<List<Float>> | Batch embeddings |
| `ai.classify` | text: String, labels: List<String> | Map (label, confidence) | Zero-shot classification |
| `ai.translate` | text: String, source: String, target: String | String | Machine translation |
| `ai.moderate` | text: String | Map (safe, categories) | Content moderation |
| `ai.sentiment` | text: String | Map (sentiment, score) | Sentiment analysis |

### Config

```toml
[plugins.ai]
models_dir = "./models/ai"
smart_reply_model = "distilgpt2"
embedding_model = "all-MiniLM-L6-v2"
intent_model = "custom-intent-detector"
device = "cpu"  # or "cuda", "metal"
```

### Rust Crate

`ort` (ONNX Runtime) for inference. Models exported from Hugging Face to ONNX.

---

## 25. ID Verification — `id-verify`

**Tier:** T2  
**Platforms:** server  
**Dependencies:** `s3` (for document storage), `ai` (for face matching)

### Conventions

| Convention | Args | Returns |
|---|---|---|
| `id-verify.submit` | user_id: String, selfie: Bytes, document_front: Bytes, document_back: Bytes | Map (verification_id, status) |
| `id-verify.check_status` | verification_id: String | Map (status, result) |
| `id-verify.face_match` | selfie: Bytes, document_photo: Bytes | Map (match, confidence) |

### Config

```toml
[plugins.id-verify]
provider = "internal"  # or "onfido", "jumio"
face_match_threshold = 0.85
storage_bucket = "helperbook-verification"
retention_days = 90
```

---

## 26. DOM Renderer — `dom-renderer`

**Tier:** T1  
**Platforms:** wasm32 (browser only)  
**Dependencies:** none

### What It Does

The Interface SOMA's body in the browser. Converts Mind-generated programs into DOM operations. The Mind composes UI by generating programs of DOM primitives.

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `dom.create` | tag: String, attrs: Map | Handle (element) | Create DOM element |
| `dom.set_attr` | el: Handle, name: String, value: String | Void | Set attribute |
| `dom.set_style` | el: Handle, prop: String, value: String | Void | Set CSS property |
| `dom.set_text` | el: Handle, text: String | Void | Set text content |
| `dom.set_html` | el: Handle, html: String | Void | Set inner HTML |
| `dom.append` | parent: Handle, child: Handle | Void | Append child |
| `dom.prepend` | parent: Handle, child: Handle | Void | Prepend child |
| `dom.remove` | el: Handle | Void | Remove element |
| `dom.replace` | old: Handle, new: Handle | Void | Replace element |
| `dom.add_class` | el: Handle, class: String | Void | Add CSS class |
| `dom.remove_class` | el: Handle, class: String | Void | Remove CSS class |
| `dom.on_event` | el: Handle, event: String, channel: Int | Void | Listen for event, send to Synaptic channel |
| `dom.query` | selector: String | Handle or Null | Query selector |
| `dom.query_all` | selector: String | List<Handle> | Query selector all |
| `dom.set_value` | el: Handle, value: String | Void | Set input value |
| `dom.get_value` | el: Handle | String | Get input value |
| `dom.animate` | el: Handle, keyframes: List<Map>, options: Map | Void | CSS animation |
| `dom.scroll_to` | el: Handle, options: Map | Void | Scroll element into view |
| `dom.focus` | el: Handle | Void | Focus element |

### Cleanup Specs

- `on_event` → cleanup: remove event listener (automatic when element is removed)

### Rust Crate

`web-sys`, `wasm-bindgen` for browser DOM access from Rust/WASM.

---

## 27. Design Knowledge — `design`

**Tier:** T1  
**Platforms:** wasm32 (browser), desktop  
**Dependencies:** `dom-renderer` (or other renderer)

### What It Does

Absorbs design specifications (from pencil.dev, Figma exports, or custom design tokens) and provides the Interface SOMA's Mind with knowledge of how to render aesthetically and consistently. This is a LoRA-only plugin — it has no conventions, only knowledge.

### LoRA Knowledge Includes

- Color palette (primary, secondary, accent, surface, error, etc.)
- Typography scale (font families, sizes, weights, line heights)
- Spacing system (4px/8px grid, padding/margin tokens)
- Border radius, shadows, elevation
- Component patterns (how a "card" looks, how a "button" looks, how a "list" looks)
- Responsive breakpoints
- Animation/transition defaults
- Dark mode variants

### Design File Ingestion

```
pencil.dev .pen file → parse JSON → extract design tokens → train LoRA
Figma export (.fig) → parse → extract styles, components → train LoRA
Custom tokens (JSON/TOML) → parse → train LoRA
```

### Config

```toml
[plugins.design]
source = "./design/helperbook.pen"  # or .fig, or tokens.json
mode = "absorb"  # absorb design language into LoRA
theme = "light"  # or "dark", "auto"
```

---

## 28. Offline Cache — `offline`

**Tier:** T2  
**Platforms:** wasm32, mobile, desktop  
**Dependencies:** `sqlite` (or IndexedDB on browser)

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `offline.cache_set` | key: String, data: Any, ttl: Int | Void | Cache data locally |
| `offline.cache_get` | key: String | Any or Null | Retrieve cached data |
| `offline.cache_invalidate` | key: String | Void | Remove from cache |
| `offline.queue_signal` | signal: Signal | Void | Queue outbound signal for when online |
| `offline.flush_queue` | — | Int (signals sent) | Send all queued signals |
| `offline.is_online` | — | Bool | Check network status |
| `offline.sync_state` | — | Map (pending, last_sync) | Sync diagnostics |

---

## 29. GPIO — `gpio`

**Tier:** T1 (embedded only)  
**Platforms:** esp32, rpi  
**Dependencies:** none

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `gpio.set_mode` | pin: Int, mode: String | Void | Set pin mode (input/output/input_pullup) |
| `gpio.write` | pin: Int, value: Int | Void | Digital write (0/1) |
| `gpio.read` | pin: Int | Int (0/1) | Digital read |
| `gpio.pwm_start` | pin: Int, frequency: Int, duty: Int | Void | Start PWM |
| `gpio.pwm_stop` | pin: Int | Void | Stop PWM |
| `gpio.adc_read` | pin: Int | Int (0-4095) | Analog read |

### Built-In

Always compiled into the embedded SOMA binary. Not dynamically loaded.

---

## 30. I2C — `i2c`

**Tier:** T2 (embedded only)  
**Platforms:** esp32, rpi  
**Dependencies:** none

### Conventions

| Convention | Args | Returns |
|---|---|---|
| `i2c.scan` | — | List<Int> (addresses found) |
| `i2c.write` | address: Int, data: Bytes | Void |
| `i2c.read` | address: Int, length: Int | Bytes |
| `i2c.write_read` | address: Int, write_data: Bytes, read_length: Int | Bytes |

---

## 31. Timer — `timer`

**Tier:** T1 (embedded)  
**Platforms:** all  
**Dependencies:** none

### Conventions

| Convention | Args | Returns |
|---|---|---|
| `timer.now` | — | String (ISO timestamp) |
| `timer.now_unix` | — | Int (unix seconds) |
| `timer.now_millis` | — | Int (unix millis) |
| `timer.sleep` | ms: Int | Void |
| `timer.set_interval` | ms: Int, channel: Int | Handle |
| `timer.clear_interval` | handle: Handle | Void |
| `timer.set_timeout` | ms: Int, channel: Int | Handle |
| `timer.clear_timeout` | handle: Handle | Void |

Timer events are sent as Synaptic signals on the specified channel, allowing the Mind to react to timed events.

---

## 32. SPI — `spi`

**Tier:** T3 (embedded only)  
**Platforms:** esp32, rpi  
**Dependencies:** none

### Conventions

| Convention | Args | Returns |
|---|---|---|
| `spi.transfer` | data: Bytes | Bytes |
| `spi.write` | data: Bytes | Void |
| `spi.read` | length: Int | Bytes |

---

## 33. UART / Serial — `uart`

**Tier:** T3 (embedded only)  
**Platforms:** esp32, rpi  
**Dependencies:** none

### Conventions

| Convention | Args | Returns |
|---|---|---|
| `uart.open` | port: String, baud: Int | Handle |
| `uart.write` | handle: Handle, data: Bytes | Int |
| `uart.read` | handle: Handle, max_bytes: Int | Bytes |
| `uart.close` | handle: Handle | Void |
| `uart.available` | handle: Handle | Int (bytes available) |

---

## 34. Bluetooth LE — `ble`

**Tier:** T3 (embedded, mobile)  
**Platforms:** esp32, mobile  
**Dependencies:** none

### Conventions

| Convention | Args | Returns |
|---|---|---|
| `ble.scan` | duration_ms: Int | List<Map> (devices) |
| `ble.connect` | address: String | Handle |
| `ble.disconnect` | handle: Handle | Void |
| `ble.discover_services` | handle: Handle | List<Map> (services + characteristics) |
| `ble.read_characteristic` | handle: Handle, service: String, char: String | Bytes |
| `ble.write_characteristic` | handle: Handle, service: String, char: String, data: Bytes | Void |
| `ble.subscribe` | handle: Handle, service: String, char: String, channel: Int | Void |

---

## 35. WiFi — `wifi`

**Tier:** T2 (embedded only)  
**Platforms:** esp32  
**Dependencies:** none

### Conventions

| Convention | Args | Returns |
|---|---|---|
| `wifi.scan` | — | List<Map> (ssid, rssi, security) |
| `wifi.connect` | ssid: String, password: String | Map (ip, gateway) |
| `wifi.disconnect` | — | Void |
| `wifi.status` | — | Map (connected, ssid, ip, rssi) |
| `wifi.start_ap` | ssid: String, password: String | Map (ip) |

---

---

## 36. MQTT — `mqtt`

**Tier:** T2  
**Platforms:** server, embedded, rpi  
**Dependencies:** none

### What It Does

MQTT is the standard IoT messaging protocol. Most sensor networks, home automation systems, and industrial IoT use MQTT. This plugin allows a SOMA to participate in existing IoT ecosystems — publish sensor data, subscribe to topics, communicate with MQTT brokers (Mosquitto, HiveMQ, AWS IoT Core).

On embedded, MQTT may be the primary communication method before Synaptic Protocol is available on peer devices. SOMA speaks MQTT to legacy devices and Synaptic Protocol to other SOMAs.

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `mqtt.connect` | broker: String, port: Int, client_id: String | Handle | Connect to broker |
| `mqtt.disconnect` | handle: Handle | Void | Disconnect |
| `mqtt.publish` | handle: Handle, topic: String, payload: String, qos: Int | Void | Publish message |
| `mqtt.subscribe` | handle: Handle, topic: String, qos: Int, channel: Int | Void | Subscribe (signals on Synaptic channel) |
| `mqtt.unsubscribe` | handle: Handle, topic: String | Void | Unsubscribe |

### Config

```toml
[plugins.mqtt]
broker = "mqtt://localhost:1883"
client_id = "soma-greenhouse"
username_env = "SOMA_MQTT_USER"
password_env = "SOMA_MQTT_PASS"
keepalive = 60
clean_session = true
```

### Embedded Variant

On ESP32, uses a lightweight MQTT client. Connects to WiFi, then to broker. Publishes sensor readings, subscribes to command topics.

### Rust Crate

`rumqttc` for async MQTT client.

---

## 37. Job Queue / Background Tasks — `jobs`

**Tier:** T2  
**Platforms:** server  
**Dependencies:** `redis` or `postgres`

### What It Does

Handles long-running tasks that shouldn't block intent processing: video transcoding, email batch sending, report generation, bulk data processing. Jobs are queued, executed by worker SOMAs, and results are delivered via Synaptic Protocol.

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `jobs.enqueue` | queue: String, task: String, args: Map, priority: Int | Map (job_id) | Enqueue a job |
| `jobs.status` | job_id: String | Map (status, progress, result) | Check job status |
| `jobs.cancel` | job_id: String | Bool | Cancel pending/running job |
| `jobs.list` | queue: String, status: String | List<Map> | List jobs by status |
| `jobs.retry` | job_id: String | Map (new_job_id) | Retry a failed job |
| `jobs.schedule` | queue: String, task: String, args: Map, run_at: String | Map (job_id) | Schedule for future |
| `jobs.recurring` | queue: String, task: String, args: Map, cron: String | Map (job_id) | Recurring job (cron syntax) |
| `jobs.dead_letter` | queue: String | List<Map> | List permanently failed jobs |

### Job Lifecycle

```
enqueued → running → completed
                  → failed → retry → running
                          → dead_letter (max retries exceeded)
           → cancelled
```

### Config

```toml
[plugins.jobs]
backend = "redis"          # or "postgres"
max_retries = 3
retry_backoff = "exponential"  # 1s, 2s, 4s, ...
default_timeout = "5m"
max_concurrent = 10
dead_letter_retention = "7d"
```

### Rust Crate

Custom implementation using `redis` or `postgres` as backing store. Inspired by Sidekiq/Bull design.

---

## 38. Webhook Receiver — `webhooks`

**Tier:** T2  
**Platforms:** server  
**Dependencies:** `http-bridge`, `crypto`

### What It Does

Receives webhook events from external services (Stripe, GitHub, Twilio, etc.), verifies signatures, ensures idempotency, and converts them to Synaptic signals that the SOMA can process.

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `webhooks.register` | name: String, path: String, secret_env: String, signature_header: String | Void | Register a webhook endpoint |
| `webhooks.unregister` | name: String | Void | Remove endpoint |
| `webhooks.list` | — | List<Map> | List registered endpoints |
| `webhooks.replay` | event_id: String | Map | Replay a past event |
| `webhooks.history` | name: String, limit: Int | List<Map> | Recent events |

### How It Works

```
1. External service POSTs to /webhooks/{name}
2. http-bridge routes request to webhooks plugin
3. Plugin verifies signature (Stripe: HMAC-SHA256, GitHub: HMAC-SHA1, etc.)
4. Plugin checks idempotency key (has this event been processed before?)
5. Plugin converts to Synaptic signal: DATA {type: "webhook", source: "stripe", event: {...}}
6. SOMA Mind processes the event
7. Plugin stores event ID for idempotency (dedup window: 24h)
```

### Config

```toml
[plugins.webhooks]
idempotency_window = "24h"
max_payload_size = "1MB"
event_retention = "7d"

[[plugins.webhooks.endpoints]]
name = "stripe"
path = "/webhooks/stripe"
secret_env = "SOMA_STRIPE_WEBHOOK_SECRET"
signature_header = "Stripe-Signature"
signature_algo = "hmac-sha256"

[[plugins.webhooks.endpoints]]
name = "github"
path = "/webhooks/github"
secret_env = "SOMA_GITHUB_WEBHOOK_SECRET"
signature_header = "X-Hub-Signature-256"
signature_algo = "hmac-sha256"

[[plugins.webhooks.endpoints]]
name = "twilio"
path = "/webhooks/twilio"
secret_env = "SOMA_TWILIO_AUTH_TOKEN"
signature_header = "X-Twilio-Signature"
signature_algo = "twilio-hmac"
```

---

## 39. PDF Generation — `pdf`

**Tier:** T3  
**Platforms:** server, desktop  
**Dependencies:** none

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `pdf.from_html` | html: String, options: Map | Bytes | HTML to PDF |
| `pdf.from_template` | template: String, data: Map | Bytes | Template-based generation |
| `pdf.merge` | pdfs: List<Bytes> | Bytes | Merge multiple PDFs |
| `pdf.page_count` | data: Bytes | Int | Get page count |
| `pdf.extract_text` | data: Bytes | String | Extract text from PDF |
| `pdf.add_watermark` | data: Bytes, text: String | Bytes | Add text watermark |

### Config

```toml
[plugins.pdf]
engine = "weasyprint"   # or "wkhtmltopdf", "chromium-headless"
templates_dir = "./templates/pdf"
default_page_size = "A4"
```

### Rust Crate

`printpdf` for native generation, or FFI to `weasyprint` / headless Chromium for HTML-to-PDF.

---

## 40. Data Export — `export`

**Tier:** T3  
**Platforms:** server, desktop  
**Dependencies:** none

### Conventions

| Convention | Args | Returns | Description |
|---|---|---|---|
| `export.csv` | data: List<Map>, columns: List<String> | Bytes | Export to CSV |
| `export.json` | data: Any | Bytes | Export to formatted JSON |
| `export.xlsx` | sheets: Map<String, List<Map>> | Bytes | Export to Excel |
| `export.zip` | files: Map<String, Bytes> | Bytes | Bundle files into ZIP |
| `export.ical` | events: List<Map> | String | Export to iCal format |

### Rust Crate

`csv`, `serde_json`, `rust_xlsxwriter`, `zip`.

---

## Summary Table

| # | Plugin | Tier | Platforms | Key Use |
|---|---|---|---|---|
| 1 | `mcp` | T0 | server, desktop | Bridge to entire MCP ecosystem |
| 2 | `postgres` | T0 | server, desktop, rpi | Primary database |
| 3 | `redis` | T0 | server, desktop, rpi | Caching, sessions, pub/sub |
| 4 | `filesystem` | T0 | all | File I/O |
| 5 | `http-bridge` | T1 | server, desktop | Legacy browser compatibility |
| 6 | `s3` | T1 | server, desktop | Object/media storage |
| 7 | `smtp` | T1 | server, desktop | Email |
| 8 | `twilio` | T1 | server | SMS, OTP |
| 9 | `push` | T1 | server | APNs + FCM |
| 10 | `auth` | T1 | server | Authentication, sessions |
| 11 | `crypto` | T0 | all | Hashing, signing, encryption |
| 12 | `image-proc` | T1 | server, desktop | Thumbnails, resize, EXIF |
| 13 | `audio-proc` | T2 | server, desktop | Voice messages |
| 14 | `video-proc` | T2 | server | Video thumbnails, compression |
| 15 | `webrtc` | T2 | server + browser | Video/voice calls |
| 16 | `sqlite` | T1 | desktop, mobile, embedded | Local database |
| 17 | `geo` | T1 | server, desktop, mobile | Distance, geocoding |
| 18 | `search` | T1 | server, desktop | Full-text + semantic search |
| 19 | `calendar` | T2 | server, desktop | Scheduling, reminders |
| 20 | `messaging` | T2 | server | Chat, delivery, read receipts |
| 21 | `reviews` | T2 | server | Ratings, reviews |
| 22 | `analytics` | T2 | server | Event tracking, dashboards |
| 23 | `i18n` | T2 | all | Localization, formatting |
| 24 | `ai` | T2 | server, desktop | Smart replies, embeddings, NLP |
| 25 | `id-verify` | T2 | server | Face matching, document check |
| 26 | `dom-renderer` | T1 | wasm32 | Browser DOM manipulation |
| 27 | `design` | T1 | wasm32, desktop | Design language absorption |
| 28 | `offline` | T2 | wasm32, mobile, desktop | Offline cache + sync |
| 29 | `gpio` | T1-embedded | esp32, rpi | Digital/analog I/O |
| 30 | `i2c` | T2-embedded | esp32, rpi | Sensor bus |
| 31 | `timer` | T1 | all | Time, intervals, timeouts |
| 32 | `spi` | T3-embedded | esp32, rpi | High-speed peripheral bus |
| 33 | `uart` | T3-embedded | esp32, rpi | Serial communication |
| 34 | `ble` | T3 | esp32, mobile | Bluetooth LE |
| 35 | `wifi` | T2-embedded | esp32 | WiFi management |
| 36 | `mqtt` | T2 | server, embedded, rpi | IoT messaging protocol |
| 37 | `jobs` | T2 | server | Background task queue |
| 38 | `webhooks` | T2 | server | External event ingestion |
| 39 | `pdf` | T3 | server, desktop | PDF generation/manipulation |
| 40 | `export` | T3 | server, desktop | CSV, JSON, Excel, ZIP export |
