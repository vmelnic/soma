# Plugin Catalog

Reference of all SOMA plugins and their calling conventions. Each plugin provides a set of conventions that the Mind can invoke as program steps. Plugins are compiled as `cdylib` crates that export a C ABI `soma_plugin_init` function.

**Current status:** 6 implemented + 5 implementing = 11 plugins, 83 conventions. 29 additional plugins planned (~230 conventions). 40 total catalog.

Organized by implementation status and priority tier:

| Tier | Meaning |
|------|---------|
| **T0 -- Core** | Required for any useful SOMA. Built first. |
| **T1 -- Foundation** | Required for web/mobile applications. Built second. |
| **T2 -- Features** | Common application features. Built as needed. |
| **T3 -- Specialized** | Domain-specific or advanced. Built on demand. |

---

## Implemented Plugins

These plugins exist in `soma-plugins/` and are buildable with `cargo build --release`. Each is a workspace member with a `manifest.json` and full `SomaPlugin` trait implementation.

Additionally, `soma-core` ships a built-in **PosixPlugin** with 22 filesystem/process conventions (see `soma-core/src/plugin/builtin.rs`).

---

### SDK (soma-plugin-sdk)

Not a plugin itself. Provides the shared types every plugin depends on:

- **`SomaPlugin` trait** -- 18 methods (name, version, conventions, execute, lifecycle, etc.)
- **`Value` enum** -- 10 variants: Null, Bool, Int, Float, String, Bytes, List, Map, Handle, Signal
- **`Convention` struct** -- id, name, description, call_pattern, args, returns, latency, side_effects, cleanup
- **`ArgSpec`** / **`ArgType`** / **`ReturnSpec`** / **`CleanupSpec`** / **`TrustLevel`** / **`PluginPermissions`**

All plugins import `soma_plugin_sdk::prelude::*`.

---

### Crypto

13 conventions for cryptographic operations. Pure Rust implementations using `sha2`, `argon2`, `chacha20poly1305`, `ed25519-dalek`, `jsonwebtoken`, `hmac`, `rand`, `uuid`.

| # | Convention | Args | Returns | Latency | Description |
|---|-----------|------|---------|---------|-------------|
| 0 | `hash_sha256` | data: String | String (hex) | ~1ms | SHA-256 hash |
| 1 | `hash_argon2` | password: String | String (PHC) | ~100ms | Argon2 password hash with random salt |
| 2 | `verify_argon2` | password: String, hash: String | Bool | ~100ms | Verify password against Argon2 hash |
| 3 | `random_bytes` | count: Int | Bytes | ~1ms | Cryptographic random bytes |
| 4 | `random_hex` | count: Int | String | ~1ms | Random hex string (2x count length) |
| 5 | `random_uuid` | -- | String | ~1ms | UUID v4 |
| 6 | `sign_ed25519` | data: Bytes, key: Bytes | Bytes | ~1ms | Ed25519 digital signature |
| 7 | `verify_ed25519` | data: Bytes, signature: Bytes, pubkey: Bytes | Bool | ~1ms | Verify Ed25519 signature |
| 8 | `encrypt_aead` | plaintext: Bytes, key: Bytes (32) | Bytes | ~1ms | ChaCha20-Poly1305 encrypt (nonce prepended) |
| 9 | `decrypt_aead` | ciphertext: Bytes, key: Bytes (32) | Bytes | ~1ms | ChaCha20-Poly1305 decrypt (expects nonce prefix) |
| 10 | `jwt_sign` | claims: String (JSON), secret: String | String | ~1ms | Sign JWT with HS256 |
| 11 | `jwt_verify` | token: String, secret: String | Map (claims) or Null | ~1ms | Verify and decode JWT |
| 12 | `hmac_sha256` | data: Bytes, key: Bytes | Bytes | ~1ms | HMAC-SHA256 message authentication |

Trust level: BuiltIn. No external dependencies at runtime.

---

### PostgreSQL

15 conventions for database operations. Uses the synchronous `postgres` crate (not tokio-postgres) to avoid reactor TLS issues in cdylib plugins. Creates a fresh connection per call.

| # | Convention | Args | Returns | Description |
|---|-----------|------|---------|-------------|
| 0 | `query` | sql: String, params: List | List\<Map\> | Execute SELECT, return rows |
| 1 | `execute` | sql: String, params: List | Int | Execute INSERT/UPDATE/DELETE, return affected rows |
| 2 | `query_one` | sql: String, params: List | Map or Null | Single row or null |
| 3 | `begin` | -- | Handle | Start transaction (MVP: unsupported) |
| 4 | `commit` | txn: Handle | Void | Commit transaction |
| 5 | `rollback` | txn: Handle | Void | Rollback transaction |
| 6 | `create_table` | name: String, columns: Map | Void | DDL: create table |
| 7 | `alter_table` | name: String, changes: Map | Void | DDL: alter table |
| 8 | `table_exists` | name: String | Bool | Check if table exists |
| 9 | `list_tables` | -- | List\<String\> | List all tables |
| 10 | `table_schema` | name: String | Map | Get column definitions |
| 11 | `find` | spec: String (JSON) | List\<Map\> | Structured SELECT -- table, select, where, join, group_by, having, order_by, limit, offset |
| 12 | `find_one` | spec: String (JSON) | Map or Null | Same as find but LIMIT 1 |
| 13 | `count` | spec: String (JSON) | Int | Count rows -- table, where |
| 14 | `aggregate` | spec: String (JSON) | List\<Map\> | Aggregation with GROUP BY -- select, group_by, having, order_by, limit |

**ORM-style query building (conventions 11-14):** The Mind generates a JSON spec instead of raw SQL. The plugin builds safe, parameterized SQL from it using a `QueryBuilder` with identifier validation to prevent injection.

**Type mapping:** BOOL, INT2/4/8, FLOAT4/8, TEXT/VARCHAR, JSON/JSONB, UUID, TIMESTAMP/TIMESTAMPTZ, NUMERIC are all mapped to appropriate Value variants. Unknown types fall back to string.

**Config:**

```toml
[plugins.postgres]
connection_string = "host=localhost user=soma dbname=helperbook"
# Or via env: SOMA_PG_CONNECTION_STRING
```

---

### Redis

14 conventions for key-value operations. Uses `redis` crate with `tokio-comp` feature. Connection via `redis::aio::ConnectionManager` with automatic reconnection.

| # | Convention | Args | Returns | Description |
|---|-----------|------|---------|-------------|
| 0 | `get` | key: String | String or Null | Get string value |
| 1 | `set` | key: String, value: String, ttl: Int (opt) | Void | Set value with optional TTL |
| 2 | `delete` | key: String | Bool | Delete key |
| 3 | `exists` | key: String | Bool | Check existence |
| 4 | `incr` | key: String | Int | Atomic increment |
| 5 | `expire` | key: String, seconds: Int | Void | Set TTL on existing key |
| 6 | `hget` | key: String, field: String | String or Null | Hash field get |
| 7 | `hset` | key: String, field: String, value: String | Void | Hash field set |
| 8 | `hgetall` | key: String | Map | Get all hash fields |
| 9 | `lpush` | key: String, value: String | Int | Push to list head |
| 10 | `lrange` | key: String, start: Int, stop: Int | List\<String\> | List range |
| 11 | `publish` | channel: String, message: String | Int | Pub/sub publish |
| 12 | `subscribe` | channel: String | Stream\<String\> | Pub/sub subscribe (streaming) |
| 13 | `keys` | pattern: String | List\<String\> | Find keys matching pattern |

**Config:**

```toml
[plugins.redis]
url = "redis://localhost:6379/0"
```

---

### Auth

10 conventions for authentication. Uses `tokio-postgres` for database storage and `sha2` for token hashing. Auto-creates `_soma_otps` and `_soma_sessions` tables on load.

| # | Convention | Args | Returns | Description |
|---|-----------|------|---------|-------------|
| 0 | `generate_otp` | phone: String | Map (otp_id, debug_code, phone) | Generate 6-digit OTP, store hash in DB |
| 1 | `verify_otp` | phone: String, code: String | Map (valid: Bool, user_id) | Verify OTP (5 attempt limit, TTL-based expiry) |
| 2 | `create_session` | user_id: String, device_info: String | Map (token, expires_at) | Create session token |
| 3 | `validate_session` | token: String | Map (valid, user_id, device) | Validate session |
| 4 | `revoke_session` | token: String | Void | Revoke session |
| 5 | `revoke_all_sessions` | user_id: String | Int (count revoked) | Revoke all user sessions |
| 6 | `list_sessions` | user_id: String | List\<Map\> | List active sessions |
| 7 | `hash_token` | token: String | String (SHA-256 hex) | Hash for secure storage |
| 8 | `generate_totp_secret` | user_id: String | Map (secret, qr_url) | Generate TOTP 2FA secret |
| 9 | `verify_totp` | user_id: String, code: String | Bool | Verify TOTP 2FA code |

**Config:**

```toml
[plugins.auth]
connection_string = "host=localhost user=soma dbname=helperbook"
otp_ttl_minutes = 5
session_ttl_hours = 720
```

---

### Geo

5 conventions for geolocation. Pure Rust distance math using Haversine formula. Geocoding conventions are stubs that return data for well-known locations; a production deployment would use Nominatim or a similar API.

| # | Convention | Args | Returns | Description |
|---|-----------|------|---------|-------------|
| 0 | `distance` | lat1: Float, lon1: Float, lat2: Float, lon2: Float | Float (km) | Haversine distance |
| 1 | `within_radius` | lat: Float, lon: Float, radius_km: Float, points: List\<Map\> | List\<Map\> | Filter points within radius |
| 2 | `bounding_box` | lat: Float, lon: Float, radius_km: Float | Map (min_lat, max_lat, min_lon, max_lon) | Bounding box for radius query |
| 3 | `geocode` | address: String | Map (lat, lon, formatted_address) | Address to coordinates (stub) |
| 4 | `reverse_geocode` | lat: Float, lon: Float | Map (address, city, country) | Coordinates to address (stub) |

Trust level: BuiltIn. No network access required for distance/radius/bbox.

**Config:**

```toml
[plugins.geo]
geocoding_provider = "nominatim"
geocoding_api_key_env = "SOMA_GEO_API_KEY"
```

---

### HTTP Bridge

5 conventions for HTTP client operations. Uses `reqwest` with async execution. Returns status, headers, and body for every response.

| # | Convention | Args | Returns | Description |
|---|-----------|------|---------|-------------|
| 0 | `get` | url: String, headers: String (JSON, opt) | Map (status, headers, body) | HTTP GET |
| 1 | `post` | url: String, body: String, headers: String (JSON, opt) | Map (status, headers, body) | HTTP POST |
| 2 | `put` | url: String, body: String, headers: String (JSON, opt) | Map (status, headers, body) | HTTP PUT |
| 3 | `delete` | url: String, headers: String (JSON, opt) | Map (status, headers, body) | HTTP DELETE |
| 4 | `request` | method: String, url: String, body: String, headers: String (JSON, opt) | Map (status, headers, body) | Generic request with any method |

Note: The current implementation is HTTP client only. The spec also defines HTTP server conventions (`http.listen`, `http.stop`) for routing browser requests to SOMA -- those are not yet implemented.

---

## Implementing Plugins

These plugins are actively being implemented in `soma-plugins/`. Convention sets are finalized, crate dependencies chosen, `SomaPlugin` trait implementation in progress.

---

### Image Processing (soma-plugin-image)

5 conventions for image manipulation. Pure Rust using the `image` crate (no C dependencies).

| # | Convention | Args | Returns | Latency | Description |
|---|-----------|------|---------|---------|-------------|
| 0 | `thumbnail` | data: Bytes, width: Int, height: Int | Bytes | ~10ms | Generate thumbnail |
| 1 | `resize` | data: Bytes, width: Int, height: Int | Bytes | ~10ms | Resize image |
| 2 | `crop` | data: Bytes, x: Int, y: Int, w: Int, h: Int | Bytes | ~5ms | Crop region |
| 3 | `format_convert` | data: Bytes, format: String | Bytes | ~10ms | Convert between formats (PNG, JPEG, WebP) |
| 4 | `exif_strip` | data: Bytes | Bytes | ~1ms | Strip EXIF metadata |

Use case: Profile photos, service gallery images in HelperBook.

**Crate:** `image` (pure Rust).

---

### S3 Storage (soma-plugin-s3)

5 conventions for S3-compatible object storage. Works with AWS S3, MinIO, Cloudflare R2, Backblaze B2.

| # | Convention | Args | Returns | Latency | Description |
|---|-----------|------|---------|---------|-------------|
| 0 | `put_object` | bucket: String, key: String, data: Bytes, content_type: String | String (URL) | ~50ms | Upload object |
| 1 | `get_object` | bucket: String, key: String | Bytes | ~50ms | Download object |
| 2 | `delete_object` | bucket: String, key: String | Bool | ~20ms | Delete object |
| 3 | `presign_url` | bucket: String, key: String, expires_secs: Int | String (URL) | ~1ms | Generate presigned URL |
| 4 | `list_objects` | bucket: String, prefix: String | List | ~20ms | List objects by prefix |

Use case: File uploads, media storage for HelperBook.

**Crate:** `aws-sdk-s3`.

---

### Push Notifications (soma-plugin-push)

4 conventions for push notifications via FCM HTTP v1 API and Web Push.

| # | Convention | Args | Returns | Latency | Description |
|---|-----------|------|---------|---------|-------------|
| 0 | `send_fcm` | device_token: String, title: String, body: String, data: Map | Map (message_id) | ~100ms | Send via Firebase Cloud Messaging |
| 1 | `send_webpush` | subscription: Map, title: String, body: String | Bool | ~100ms | Send Web Push notification |
| 2 | `register_device` | user_id: String, platform: String, token: String | Bool | ~10ms | Register device token |
| 3 | `unregister_device` | token: String | Bool | ~10ms | Unregister device token |

Use case: Message alerts, appointment reminders in HelperBook.

**Crate:** `reqwest` (FCM HTTP v1 API).

---

### Timer (soma-plugin-timer)

4 conventions for time-based scheduling. Pure standard library, no external dependencies.

| # | Convention | Args | Returns | Latency | Description |
|---|-----------|------|---------|---------|-------------|
| 0 | `set_timeout` | callback_intent: String, delay_ms: Int | Handle | ~1ms | One-shot delayed callback |
| 1 | `set_interval` | callback_intent: String, interval_ms: Int | Handle | ~1ms | Recurring callback |
| 2 | `cancel` | handle: Handle | Bool | ~1ms | Cancel timeout or interval |
| 3 | `list_active` | -- | List\<Map\> | ~1ms | List active timers |

Use case: Reminders, periodic tasks, session expiry in HelperBook.

**Crate:** `std` only.

---

### SMTP Email (soma-plugin-smtp)

3 conventions for sending email via SMTP.

| # | Convention | Args | Returns | Latency | Description |
|---|-----------|------|---------|---------|-------------|
| 0 | `send` | to: String, subject: String, body: String | Bool | ~200ms | Send plain text email |
| 1 | `send_html` | to: String, subject: String, html: String | Bool | ~200ms | Send HTML email |
| 2 | `send_with_attachment` | to: String, subject: String, body: String, attachment: Bytes, filename: String | Bool | ~500ms | Send email with attachment |

Use case: Email notifications, appointment confirmations in HelperBook.

**Crate:** `lettre`.

---

## Planned Plugins

These plugins are specified in the catalog but not yet implemented. Listed by priority tier with key conventions, recommended Rust crates, and dependencies.

---

### Tier 0 -- Core

#### MCP Bridge (`mcp`)

Bridges SOMA to the Model Context Protocol ecosystem. In client mode, connects to external MCP servers (GitHub, Slack, filesystem, etc.) and converts their tools into SOMA conventions at runtime. In server mode, exposes SOMA's conventions as MCP tools for external AI orchestration.

**Conventions:** Dynamic -- discovered at runtime from connected MCP servers. Each MCP tool becomes `mcp.{server}.{tool_name}(args...)`.

**Strategic value:** One plugin that instantly gives SOMA access to hundreds of external services without per-service plugins.

**Config:**

```toml
[plugins.mcp]
mode = "client"  # or "server" or "both"

[[plugins.mcp.servers]]
name = "github"
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "${SOMA_GITHUB_TOKEN}" }
```

**Crates:** `mcp-client` or custom JSON-RPC over stdio/SSE.

#### Filesystem (`filesystem`)

File I/O for all platforms. On embedded, maps to SPIFFS/LittleFS.

| Convention | Args | Returns |
|-----------|------|---------|
| `fs.read_file` | path: String | String |
| `fs.read_bytes` | path: String | Bytes |
| `fs.write_file` | path: String, content: String | Void |
| `fs.write_bytes` | path: String, data: Bytes | Void |
| `fs.append_file` | path: String, content: String | Void |
| `fs.delete` | path: String | Void |
| `fs.rename` | from: String, to: String | Void |
| `fs.copy` | from: String, to: String | Void |
| `fs.exists` | path: String | Bool |
| `fs.list_dir` | path: String | List\<String\> |
| `fs.create_dir` | path: String | Void |
| `fs.file_info` | path: String | Map (size, modified, mode) |
| `fs.open_read` | path: String | Handle |
| `fs.open_write` | path: String | Handle |
| `fs.close` | handle: Handle | Void |
| `fs.read_chunk` | handle: Handle, size: Int | Bytes |
| `fs.write_chunk` | handle: Handle, data: Bytes | Int |

Note: `soma-core` already ships a built-in PosixPlugin with 22 conventions covering filesystem and process operations. This planned plugin would provide a cross-platform, plugin-based alternative.

**Crates:** `tokio::fs` (async), `std::fs` (sync/embedded).

---

### Tier 1 -- Foundation

#### S3-Compatible Storage (`s3`)

Core 5 conventions being implemented (see Implementing section). The full planned set adds multipart upload support:

| Convention | Args | Returns |
|-----------|------|---------|
| `s3.head_object` | bucket, key | Map (size, type, modified) |
| `s3.multipart_start` | bucket, key, content_type | Handle |
| `s3.multipart_upload` | handle, part: Int, data: Bytes | String (etag) |
| `s3.multipart_complete` | handle, parts: List | Map |
| `s3.multipart_abort` | handle | Void |

Cleanup: `multipart_start` -> `multipart_abort`.

**Crates:** `aws-sdk-s3` or `rust-s3`.

#### SMTP Email (`smtp`)

Core 3 conventions being implemented (see Implementing section). The full planned set adds template and verification support:

| Convention | Args | Returns |
|-----------|------|---------|
| `smtp.send_template` | to, template, variables: Map | Map |
| `smtp.verify_address` | email | Bool |

**Crates:** `lettre`.

#### SMS / Twilio (`twilio`)

| Convention | Args | Returns |
|-----------|------|---------|
| `twilio.send_sms` | to, body | Map (sid, status) |
| `twilio.send_otp` | to | Map (sid) |
| `twilio.verify_otp` | to, code | Bool |
| `twilio.lookup` | phone | Map (country, carrier, type) |

Dependencies: `http-bridge` (for webhook receiving).

**Crates:** `reqwest` (Twilio API is REST-based).

#### Push Notifications (`push`)

Core 4 conventions being implemented (see Implementing section). The full planned set adds APNs, unified send, and batch support:

| Convention | Args | Returns |
|-----------|------|---------|
| `push.send_apns` | device_token, title, body, data: Map | Map (apns_id) |
| `push.send` | device_token, platform, title, body, data: Map | Map |
| `push.send_batch` | tokens: List\<Map\>, title, body, data: Map | List\<Map\> |

**Crates:** `a2` (APNs), `fcm-rust` or direct HTTP/2.

#### Image Processing (`image-proc`)

Core 5 conventions being implemented (see Implementing section). The full planned set adds info, compress, and blur:

| Convention | Args | Returns |
|-----------|------|---------|
| `image.info` | data: Bytes | Map (width, height, format, size) |
| `image.compress` | data: Bytes, quality: Int | Bytes |
| `image.blur_region` | data: Bytes, x, y, w, h | Bytes |

**Crates:** `image`, `kamadak-exif`.

#### SQLite (`sqlite`)

Same conventions as PostgreSQL but for local/embedded databases.

| Convention | Args | Returns |
|-----------|------|---------|
| `sqlite.query` | sql, params: List | List\<Map\> |
| `sqlite.execute` | sql, params: List | Int |
| `sqlite.query_one` | sql, params: List | Map or Null |
| `sqlite.begin` | -- | Handle |
| `sqlite.commit` | txn: Handle | Void |
| `sqlite.rollback` | txn: Handle | Void |
| `sqlite.table_exists` | name | Bool |
| `sqlite.list_tables` | -- | List\<String\> |
| `sqlite.table_schema` | name | Map |

**Crates:** `rusqlite` with `bundled` feature.

#### Text Search (`search`)

Full-text and semantic search with embedded index.

| Convention | Args | Returns |
|-----------|------|---------|
| `search.index` | collection, id, text, metadata: Map | Void |
| `search.search` | collection, query, limit: Int | List\<Map\> (id, score, highlights) |
| `search.semantic_search` | collection, query, limit: Int | List\<Map\> |
| `search.remove` | collection, id | Void |
| `search.suggest` | collection, prefix, limit: Int | List\<String\> |
| `search.synonym_match` | query, candidates: List\<String\> | List\<Map\> (candidate, score) |

**Crates:** `tantivy` (full-text), `ort` (embedding inference).

#### DOM Renderer (`dom-renderer`)

The Interface SOMA's body in the browser. Converts Mind-generated programs into DOM operations. Browser-only (wasm32).

| Convention | Args | Returns |
|-----------|------|---------|
| `dom.create` | tag, attrs: Map | Handle |
| `dom.set_attr` | el: Handle, name, value | Void |
| `dom.set_style` | el: Handle, prop, value | Void |
| `dom.set_text` | el: Handle, text | Void |
| `dom.set_html` | el: Handle, html | Void |
| `dom.append` | parent: Handle, child: Handle | Void |
| `dom.prepend` | parent: Handle, child: Handle | Void |
| `dom.remove` | el: Handle | Void |
| `dom.replace` | old: Handle, new: Handle | Void |
| `dom.add_class` | el: Handle, class | Void |
| `dom.remove_class` | el: Handle, class | Void |
| `dom.on_event` | el: Handle, event, channel: Int | Void |
| `dom.query` | selector | Handle or Null |
| `dom.query_all` | selector | List\<Handle\> |
| `dom.set_value` | el: Handle, value | Void |
| `dom.get_value` | el: Handle | String |
| `dom.animate` | el: Handle, keyframes: List\<Map\>, options: Map | Void |
| `dom.scroll_to` | el: Handle, options: Map | Void |
| `dom.focus` | el: Handle | Void |

**Crates:** `web-sys`, `wasm-bindgen`.

#### Design Knowledge (`design`)

LoRA-only plugin -- no conventions. Absorbs design specifications (pencil.dev, Figma exports, design tokens) and provides the Interface SOMA's Mind with knowledge of aesthetics: color palette, typography, spacing, component patterns, responsive breakpoints, dark mode.

**Crates:** Custom parser for design token formats.

#### Timer (`timer`)

Core 4 conventions being implemented (see Implementing section). The full planned set adds time queries, sleep, and channel-based callbacks:

| Convention | Args | Returns |
|-----------|------|---------|
| `timer.now` | -- | String (ISO timestamp) |
| `timer.now_unix` | -- | Int (unix seconds) |
| `timer.now_millis` | -- | Int (unix millis) |
| `timer.sleep` | ms: Int | Void |

---

### Tier 2 -- Features

#### Audio Processing (`audio-proc`)

| Convention | Args | Returns |
|-----------|------|---------|
| `audio.info` | data: Bytes | Map (duration, format, channels, sample_rate) |
| `audio.convert` | data: Bytes, target_format | Bytes |
| `audio.normalize` | data: Bytes | Bytes |
| `audio.trim` | data: Bytes, start_ms, end_ms | Bytes |
| `audio.duration` | data: Bytes | Float (seconds) |
| `audio.waveform` | data: Bytes, width: Int | List\<Float\> |

**Crates:** `symphonia` (decoding), `opus` (encoding).

#### Video Processing (`video-proc`)

| Convention | Args | Returns |
|-----------|------|---------|
| `video.info` | data: Bytes | Map (duration, width, height, codec, fps) |
| `video.thumbnail` | data: Bytes, time_ms: Int | Bytes (image) |
| `video.compress` | data: Bytes, quality | Bytes |
| `video.convert` | data: Bytes, target_format | Bytes |
| `video.trim` | data: Bytes, start_ms, end_ms | Bytes |

**Crates:** FFI to `ffmpeg` or `gstreamer` Rust bindings.

#### WebRTC (`webrtc`)

Server-side signaling. Media flows peer-to-peer between clients.

| Convention | Args | Returns |
|-----------|------|---------|
| `webrtc.create_room` | room_id | Map (room_id, ice_servers) |
| `webrtc.join_room` | room_id, peer_id | Map (existing_peers) |
| `webrtc.leave_room` | room_id, peer_id | Void |
| `webrtc.relay_signal` | room_id, from, to, signal: Map | Void |

Client-side conventions (Interface SOMA): `create_offer`, `create_answer`, `add_ice`, `get_stream`.

**Crates:** `webrtc-rs`.

#### Calendar / Scheduling (`calendar`)

Dependencies: `postgres` or `sqlite`.

| Convention | Args | Returns |
|-----------|------|---------|
| `calendar.create_event` | title, start, end, data: Map | Map (event_id) |
| `calendar.update_event` | event_id, changes: Map | Void |
| `calendar.delete_event` | event_id | Void |
| `calendar.list_events` | start, end, filters: Map | List\<Map\> |
| `calendar.check_conflict` | start, end, participant | Bool |
| `calendar.find_available` | participant, date, duration: Int | List\<Map\> |
| `calendar.set_schedule` | participant, schedule: Map | Void |
| `calendar.get_schedule` | participant | Map |
| `calendar.create_reminder` | event_id, before_minutes: Int | Map (reminder_id) |
| `calendar.export_ical` | event_id | String (iCal) |
| `calendar.recurrence` | event_id, pattern: Map | Void |

#### Messaging (`messaging`)

Dependencies: `postgres` or `sqlite`, optionally `redis` for pub/sub.

| Convention | Args | Returns |
|-----------|------|---------|
| `msg.send` | chat_id, sender, type, content | Map (message_id, timestamp) |
| `msg.get_messages` | chat_id, before, limit: Int | List\<Map\> |
| `msg.mark_read` | chat_id, user_id, message_id | Void |
| `msg.mark_delivered` | message_ids: List | Void |
| `msg.edit` | message_id, new_content | Void |
| `msg.delete` | message_id, mode | Void |
| `msg.create_chat` | type, members: List, name | Map (chat_id) |
| `msg.add_member` | chat_id, user_id | Void |
| `msg.remove_member` | chat_id, user_id | Void |
| `msg.typing` | chat_id, user_id | Void |
| `msg.unread_count` | user_id | Map (total, per_chat) |
| `msg.search` | user_id, query | List\<Map\> |
| `msg.pin` | chat_id | Void |
| `msg.mute` | chat_id, until | Void |
| `msg.archive` | chat_id | Void |

#### Reviews / Ratings (`reviews`)

Dependencies: `postgres` or `sqlite`.

| Convention | Args | Returns |
|-----------|------|---------|
| `reviews.create` | reviewer, reviewed, appointment_id, rating: Int, feedback, tags: List | Map (review_id) |
| `reviews.respond` | review_id, response | Void |
| `reviews.get_for_user` | user_id, limit: Int, offset: Int | List\<Map\> |
| `reviews.aggregate` | user_id | Map (avg_rating, count, tag_counts) |
| `reviews.report` | review_id, reason | Void |

#### Analytics (`analytics`)

Dependencies: `postgres` or `sqlite`.

| Convention | Args | Returns |
|-----------|------|---------|
| `analytics.track` | event, user_id, properties: Map | Void |
| `analytics.query` | metric, period, filters: Map | Map (values, labels) |
| `analytics.dashboard` | user_id, period | Map (comprehensive stats) |
| `analytics.funnel` | steps: List, period | List\<Map\> (step, count, rate) |
| `analytics.cohort` | period, group_by | Map (retention matrix) |

#### Localization (`i18n`)

| Convention | Args | Returns |
|-----------|------|---------|
| `i18n.translate` | key, locale, params: Map | String |
| `i18n.format_date` | date, locale, format | String |
| `i18n.format_currency` | amount: Float, currency, locale | String |
| `i18n.format_distance` | km: Float, locale | String |
| `i18n.detect_language` | text | String (locale code) |
| `i18n.available_locales` | -- | List\<String\> |

#### AI Inference (`ai`)

Additional ML models beyond the Mind's core inference: smart replies, embeddings, classification.

| Convention | Args | Returns |
|-----------|------|---------|
| `ai.smart_replies` | conversation: List\<Map\>, count: Int | List\<String\> |
| `ai.detect_intent` | text | Map (intent, entities, confidence) |
| `ai.summarize` | messages: List | String |
| `ai.embed_text` | text | List\<Float\> |
| `ai.embed_batch` | texts: List | List\<List\<Float\>\> |
| `ai.classify` | text, labels: List | Map (label, confidence) |
| `ai.translate` | text, source, target | String |
| `ai.moderate` | text | Map (safe, categories) |
| `ai.sentiment` | text | Map (sentiment, score) |

**Crates:** `ort` (ONNX Runtime).

#### ID Verification (`id-verify`)

Dependencies: `s3`, `ai`.

| Convention | Args | Returns |
|-----------|------|---------|
| `id-verify.submit` | user_id, selfie: Bytes, document_front: Bytes, document_back: Bytes | Map (verification_id, status) |
| `id-verify.check_status` | verification_id | Map (status, result) |
| `id-verify.face_match` | selfie: Bytes, document_photo: Bytes | Map (match, confidence) |

#### Offline Cache (`offline`)

Dependencies: `sqlite` or IndexedDB (browser).

| Convention | Args | Returns |
|-----------|------|---------|
| `offline.cache_set` | key, data: Any, ttl: Int | Void |
| `offline.cache_get` | key | Any or Null |
| `offline.cache_invalidate` | key | Void |
| `offline.queue_signal` | signal: Signal | Void |
| `offline.flush_queue` | -- | Int (signals sent) |
| `offline.is_online` | -- | Bool |
| `offline.sync_state` | -- | Map (pending, last_sync) |

#### MQTT (`mqtt`)

IoT messaging protocol. On embedded, may be the primary communication method.

| Convention | Args | Returns |
|-----------|------|---------|
| `mqtt.connect` | broker, port: Int, client_id | Handle |
| `mqtt.disconnect` | handle | Void |
| `mqtt.publish` | handle, topic, payload, qos: Int | Void |
| `mqtt.subscribe` | handle, topic, qos: Int, channel: Int | Void |
| `mqtt.unsubscribe` | handle, topic | Void |

**Crates:** `rumqttc`.

#### Job Queue (`jobs`)

Background task processing. Dependencies: `redis` or `postgres`.

| Convention | Args | Returns |
|-----------|------|---------|
| `jobs.enqueue` | queue, task, args: Map, priority: Int | Map (job_id) |
| `jobs.status` | job_id | Map (status, progress, result) |
| `jobs.cancel` | job_id | Bool |
| `jobs.list` | queue, status | List\<Map\> |
| `jobs.retry` | job_id | Map (new_job_id) |
| `jobs.schedule` | queue, task, args: Map, run_at | Map (job_id) |
| `jobs.recurring` | queue, task, args: Map, cron | Map (job_id) |
| `jobs.dead_letter` | queue | List\<Map\> |

#### Webhooks (`webhooks`)

Receives webhook events from external services, verifies signatures, ensures idempotency. Dependencies: `http-bridge`, `crypto`.

| Convention | Args | Returns |
|-----------|------|---------|
| `webhooks.register` | name, path, secret_env, signature_header | Void |
| `webhooks.unregister` | name | Void |
| `webhooks.list` | -- | List\<Map\> |
| `webhooks.replay` | event_id | Map |
| `webhooks.history` | name, limit: Int | List\<Map\> |

#### WiFi (`wifi`)

Embedded only (ESP32).

| Convention | Args | Returns |
|-----------|------|---------|
| `wifi.scan` | -- | List\<Map\> (ssid, rssi, security) |
| `wifi.connect` | ssid, password | Map (ip, gateway) |
| `wifi.disconnect` | -- | Void |
| `wifi.status` | -- | Map (connected, ssid, ip, rssi) |
| `wifi.start_ap` | ssid, password | Map (ip) |

---

### Tier 3 -- Specialized

#### SPI (`spi`)

Embedded only (ESP32, RPi). High-speed peripheral bus.

| Convention | Args | Returns |
|-----------|------|---------|
| `spi.transfer` | data: Bytes | Bytes |
| `spi.write` | data: Bytes | Void |
| `spi.read` | length: Int | Bytes |

#### UART / Serial (`uart`)

Embedded only (ESP32, RPi).

| Convention | Args | Returns |
|-----------|------|---------|
| `uart.open` | port, baud: Int | Handle |
| `uart.write` | handle, data: Bytes | Int |
| `uart.read` | handle, max_bytes: Int | Bytes |
| `uart.close` | handle | Void |
| `uart.available` | handle | Int |

#### Bluetooth LE (`ble`)

Embedded and mobile (ESP32, mobile).

| Convention | Args | Returns |
|-----------|------|---------|
| `ble.scan` | duration_ms: Int | List\<Map\> |
| `ble.connect` | address | Handle |
| `ble.disconnect` | handle | Void |
| `ble.discover_services` | handle | List\<Map\> |
| `ble.read_characteristic` | handle, service, char | Bytes |
| `ble.write_characteristic` | handle, service, char, data: Bytes | Void |
| `ble.subscribe` | handle, service, char, channel: Int | Void |

#### PDF (`pdf`)

| Convention | Args | Returns |
|-----------|------|---------|
| `pdf.from_html` | html, options: Map | Bytes |
| `pdf.from_template` | template, data: Map | Bytes |
| `pdf.merge` | pdfs: List\<Bytes\> | Bytes |
| `pdf.page_count` | data: Bytes | Int |
| `pdf.extract_text` | data: Bytes | String |
| `pdf.add_watermark` | data: Bytes, text | Bytes |

**Crates:** `printpdf`, or FFI to `weasyprint` / headless Chromium.

#### Data Export (`export`)

| Convention | Args | Returns |
|-----------|------|---------|
| `export.csv` | data: List\<Map\>, columns: List | Bytes |
| `export.json` | data: Any | Bytes |
| `export.xlsx` | sheets: Map\<String, List\<Map\>\> | Bytes |
| `export.zip` | files: Map\<String, Bytes\> | Bytes |
| `export.ical` | events: List\<Map\> | String |

**Crates:** `csv`, `serde_json`, `rust_xlsxwriter`, `zip`.

---

### Embedded Built-in Plugins

These are compiled directly into the embedded SOMA binary, not dynamically loaded.

#### GPIO (`gpio`)

| Convention | Args | Returns |
|-----------|------|---------|
| `gpio.set_mode` | pin: Int, mode | Void |
| `gpio.write` | pin: Int, value: Int | Void |
| `gpio.read` | pin: Int | Int (0/1) |
| `gpio.pwm_start` | pin: Int, frequency: Int, duty: Int | Void |
| `gpio.pwm_stop` | pin: Int | Void |
| `gpio.adc_read` | pin: Int | Int (0-4095) |

#### I2C (`i2c`)

| Convention | Args | Returns |
|-----------|------|---------|
| `i2c.scan` | -- | List\<Int\> |
| `i2c.write` | address: Int, data: Bytes | Void |
| `i2c.read` | address: Int, length: Int | Bytes |
| `i2c.write_read` | address: Int, write_data: Bytes, read_length: Int | Bytes |

---

## Convention Quick Reference

Compact table of all conventions across implemented and implementing plugins (83 total).

| Plugin | Convention | Args | Returns | Est. Latency |
|--------|-----------|------|---------|--------------|
| crypto | `hash_sha256` | data: String | String (hex) | 1ms |
| crypto | `hash_argon2` | password: String | String (PHC) | 100ms |
| crypto | `verify_argon2` | password: String, hash: String | Bool | 100ms |
| crypto | `random_bytes` | count: Int | Bytes | 1ms |
| crypto | `random_hex` | count: Int | String | 1ms |
| crypto | `random_uuid` | -- | String | 1ms |
| crypto | `sign_ed25519` | data: Bytes, key: Bytes | Bytes | 1ms |
| crypto | `verify_ed25519` | data: Bytes, signature: Bytes, pubkey: Bytes | Bool | 1ms |
| crypto | `encrypt_aead` | plaintext: Bytes, key: Bytes | Bytes | 1ms |
| crypto | `decrypt_aead` | ciphertext: Bytes, key: Bytes | Bytes | 1ms |
| crypto | `jwt_sign` | claims: String, secret: String | String | 1ms |
| crypto | `jwt_verify` | token: String, secret: String | Map or Null | 1ms |
| crypto | `hmac_sha256` | data: Bytes, key: Bytes | Bytes | 1ms |
| postgres | `query` | sql: String, params: List | List\<Map\> | 5ms |
| postgres | `execute` | sql: String, params: List | Int | 5ms |
| postgres | `query_one` | sql: String, params: List | Map or Null | 5ms |
| postgres | `begin` | -- | Handle | 1ms |
| postgres | `commit` | txn: Handle | Void | 1ms |
| postgres | `rollback` | txn: Handle | Void | 1ms |
| postgres | `create_table` | name: String, columns: Map | Void | 10ms |
| postgres | `alter_table` | name: String, changes: Map | Void | 10ms |
| postgres | `table_exists` | name: String | Bool | 5ms |
| postgres | `list_tables` | -- | List\<String\> | 5ms |
| postgres | `table_schema` | name: String | Map | 5ms |
| postgres | `find` | spec: String (JSON) | List\<Map\> | 5ms |
| postgres | `find_one` | spec: String (JSON) | Map or Null | 5ms |
| postgres | `count` | spec: String (JSON) | Int | 5ms |
| postgres | `aggregate` | spec: String (JSON) | List\<Map\> | 10ms |
| redis | `get` | key: String | String or Null | 1ms |
| redis | `set` | key: String, value: String, ttl: Int (opt) | Void | 1ms |
| redis | `delete` | key: String | Bool | 1ms |
| redis | `exists` | key: String | Bool | 1ms |
| redis | `incr` | key: String | Int | 1ms |
| redis | `expire` | key: String, seconds: Int | Void | 1ms |
| redis | `hget` | key: String, field: String | String or Null | 1ms |
| redis | `hset` | key: String, field: String, value: String | Void | 1ms |
| redis | `hgetall` | key: String | Map | 1ms |
| redis | `lpush` | key: String, value: String | Int | 1ms |
| redis | `lrange` | key: String, start: Int, stop: Int | List\<String\> | 1ms |
| redis | `publish` | channel: String, message: String | Int | 1ms |
| redis | `subscribe` | channel: String | Stream\<String\> | 1ms |
| redis | `keys` | pattern: String | List\<String\> | 1ms |
| auth | `generate_otp` | phone: String | Map | 10ms |
| auth | `verify_otp` | phone: String, code: String | Map | 10ms |
| auth | `create_session` | user_id: String, device_info: String | Map | 10ms |
| auth | `validate_session` | token: String | Map | 10ms |
| auth | `revoke_session` | token: String | Void | 10ms |
| auth | `revoke_all_sessions` | user_id: String | Int | 10ms |
| auth | `list_sessions` | user_id: String | List\<Map\> | 10ms |
| auth | `hash_token` | token: String | String | 1ms |
| auth | `generate_totp_secret` | user_id: String | Map | 1ms |
| auth | `verify_totp` | user_id: String, code: String | Bool | 1ms |
| geo | `distance` | lat1, lon1, lat2, lon2: Float | Float (km) | 1ms |
| geo | `within_radius` | lat, lon, radius_km: Float, points: List | List\<Map\> | 1ms |
| geo | `bounding_box` | lat, lon, radius_km: Float | Map | 1ms |
| geo | `geocode` | address: String | Map | 100ms* |
| geo | `reverse_geocode` | lat, lon: Float | Map | 100ms* |
| http-bridge | `get` | url: String, headers: String (opt) | Map | 100ms+ |
| http-bridge | `post` | url: String, body: String, headers: String (opt) | Map | 100ms+ |
| http-bridge | `put` | url: String, body: String, headers: String (opt) | Map | 100ms+ |
| http-bridge | `delete` | url: String, headers: String (opt) | Map | 100ms+ |
| http-bridge | `request` | method, url, body, headers: String (opt) | Map | 100ms+ |
| image | `thumbnail` | data: Bytes, width: Int, height: Int | Bytes | 10ms |
| image | `resize` | data: Bytes, width: Int, height: Int | Bytes | 10ms |
| image | `crop` | data: Bytes, x: Int, y: Int, w: Int, h: Int | Bytes | 5ms |
| image | `format_convert` | data: Bytes, format: String | Bytes | 10ms |
| image | `exif_strip` | data: Bytes | Bytes | 1ms |
| s3 | `put_object` | bucket, key, data: Bytes, content_type: String | String (URL) | 50ms |
| s3 | `get_object` | bucket, key: String | Bytes | 50ms |
| s3 | `delete_object` | bucket, key: String | Bool | 20ms |
| s3 | `presign_url` | bucket, key: String, expires_secs: Int | String (URL) | 1ms |
| s3 | `list_objects` | bucket, prefix: String | List | 20ms |
| push | `send_fcm` | device_token, title, body: String, data: Map | Map | 100ms |
| push | `send_webpush` | subscription: Map, title, body: String | Bool | 100ms |
| push | `register_device` | user_id, platform, token: String | Bool | 10ms |
| push | `unregister_device` | token: String | Bool | 10ms |
| timer | `set_timeout` | callback_intent: String, delay_ms: Int | Handle | 1ms |
| timer | `set_interval` | callback_intent: String, interval_ms: Int | Handle | 1ms |
| timer | `cancel` | handle: Handle | Bool | 1ms |
| timer | `list_active` | -- | List\<Map\> | 1ms |
| smtp | `send` | to, subject, body: String | Bool | 200ms |
| smtp | `send_html` | to, subject, html: String | Bool | 200ms |
| smtp | `send_with_attachment` | to, subject, body: String, attachment: Bytes, filename: String | Bool | 500ms |

\* Geocoding latency depends on external API. Stub implementation returns instantly.

---

## Summary

| # | Plugin | Tier | Status | Conventions | Platforms |
|---|--------|------|--------|-------------|-----------|
| 1 | `crypto` | T0 | **Implemented** | 13 | all |
| 2 | `postgres` | T0 | **Implemented** | 15 | server, desktop, rpi |
| 3 | `redis` | T0 | **Implemented** | 14 | server, desktop, rpi |
| 4 | `auth` | T1 | **Implemented** | 10 | server |
| 5 | `geo` | T1 | **Implemented** | 5 | server, desktop, mobile |
| 6 | `http-bridge` | T1 | **Implemented** | 5 | server, desktop |
| 7 | `mcp` | T0 | Planned | dynamic | server, desktop |
| 8 | `filesystem` | T0 | Planned (built-in exists) | 17 | all |
| 9 | `s3` | T1 | **Implementing** (5) | 10 | server, desktop |
| 10 | `smtp` | T1 | **Implementing** (3) | 5 | server, desktop |
| 11 | `twilio` | T1 | Planned | 4 | server |
| 12 | `push` | T1 | **Implementing** (4) | 6 | server |
| 13 | `image-proc` | T1 | **Implementing** (5) | 8 | server, desktop |
| 14 | `sqlite` | T1 | Planned | 9 | desktop, mobile, embedded |
| 15 | `search` | T1 | Planned | 6 | server, desktop |
| 16 | `dom-renderer` | T1 | Planned | 19 | wasm32 |
| 17 | `design` | T1 | Planned | 0 (LoRA only) | wasm32, desktop |
| 18 | `timer` | T1 | **Implementing** (4) | 8 | all |
| 19 | `audio-proc` | T2 | Planned | 6 | server, desktop |
| 20 | `video-proc` | T2 | Planned | 5 | server |
| 21 | `webrtc` | T2 | Planned | 4+4 | server + browser |
| 22 | `calendar` | T2 | Planned | 11 | server, desktop |
| 23 | `messaging` | T2 | Planned | 15 | server |
| 24 | `reviews` | T2 | Planned | 5 | server |
| 25 | `analytics` | T2 | Planned | 5 | server |
| 26 | `i18n` | T2 | Planned | 6 | all |
| 27 | `ai` | T2 | Planned | 9 | server, desktop |
| 28 | `id-verify` | T2 | Planned | 3 | server |
| 29 | `offline` | T2 | Planned | 7 | wasm32, mobile, desktop |
| 30 | `mqtt` | T2 | Planned | 5 | server, embedded, rpi |
| 31 | `jobs` | T2 | Planned | 8 | server |
| 32 | `webhooks` | T2 | Planned | 5 | server |
| 33 | `wifi` | T2 | Planned | 5 | esp32 |
| 34 | `gpio` | T1-emb | Planned | 6 | esp32, rpi |
| 35 | `i2c` | T2-emb | Planned | 4 | esp32, rpi |
| 36 | `spi` | T3-emb | Planned | 3 | esp32, rpi |
| 37 | `uart` | T3-emb | Planned | 5 | esp32, rpi |
| 38 | `ble` | T3 | Planned | 7 | esp32, mobile |
| 39 | `pdf` | T3 | Planned | 6 | server, desktop |
| 40 | `export` | T3 | Planned | 5 | server, desktop |

**Implemented:** 6 plugins, 62 conventions.
**Implementing:** 5 plugins, 21 conventions (image, s3, push, timer, smtp).
**Total implemented + implementing:** 11 plugins, 83 conventions.
**Planned:** 29 plugins, ~230 conventions (remaining).
**Total catalog:** 40 plugins, ~310 conventions.
