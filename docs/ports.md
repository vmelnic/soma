# Ports

## What Ports Are

Ports are typed adapters that connect the SOMA runtime to external systems -- databases, filesystems, HTTP endpoints, messaging services, object storage, and more. Each port is compiled as a dynamically loaded shared library (`.dylib` on macOS, `.so` on Linux) and exposes a fixed set of **capabilities** through a validated contract.

Two ports are built into the runtime binary (filesystem, http). All others are loaded dynamically from shared libraries at startup based on pack manifests.

## SDK Contract

Every port implements the `Port` trait defined in `soma-port-sdk`:

```rust
pub trait Port: Send + Sync {
    fn spec(&self) -> &PortSpec;
    fn invoke(&self, capability_id: &str, input: serde_json::Value) -> Result<PortCallRecord>;
    fn validate_input(&self, capability_id: &str, input: &serde_json::Value) -> Result<()>;
    fn lifecycle_state(&self) -> PortLifecycleState;
}
```

The runtime calls `validate_input` before every `invoke`. Implementations may assume input has passed schema validation.

### PortSpec

The port's self-declaration. Registered once at load time.

| Field | Type | Purpose |
|-------|------|---------|
| `port_id` | `String` | Unique identifier, must match library name for dynamic ports |
| `name` | `String` | Human-readable name |
| `version` | `semver::Version` | Semantic version |
| `kind` | `PortKind` | Category: Filesystem, Database, Http, Queue, Renderer, Sensor, Actuator, Messaging, DeviceTransport, Custom |
| `description` | `String` | What this port does |
| `namespace` | `String` | Dot-separated namespace for scoping |
| `trust_level` | `TrustLevel` | Untrusted, Restricted, Verified, Trusted, BuiltIn |
| `capabilities` | `Vec<PortCapabilitySpec>` | The operations this port supports |
| `input_schema` | `SchemaRef` | JSON Schema for port-level input |
| `output_schema` | `SchemaRef` | JSON Schema for port-level output |
| `failure_modes` | `Vec<PortFailureClass>` | Expected failure categories |
| `side_effect_class` | `SideEffectClass` | Worst-case side effect across all capabilities |
| `latency_profile` | `LatencyProfile` | Expected/p95/max latency in ms |
| `cost_profile` | `CostProfile` | CPU, memory, IO, network, energy cost classes |
| `auth_requirements` | `AuthRequirements` | Required auth methods and whether auth is mandatory |
| `sandbox_requirements` | `SandboxRequirements` | Filesystem, network, device, process access flags; resource limits |
| `observable_fields` | `Vec<String>` | Fields surfaced to proprioception |
| `validation_rules` | `Vec<ValidationRule>` | Domain-specific validation beyond schema |
| `remote_exposure` | `bool` | Whether this port can be invoked by remote peers |

### PortCapabilitySpec

Per-capability declaration within a port.

| Field | Type | Purpose |
|-------|------|---------|
| `capability_id` | `String` | Unique within the port |
| `name` | `String` | Human-readable name |
| `purpose` | `String` | What this capability does |
| `input_schema` | `SchemaRef` | JSON Schema for capability input |
| `output_schema` | `SchemaRef` | JSON Schema for capability output |
| `effect_class` | `SideEffectClass` | None, ReadOnly, LocalStateMutation, ExternalStateMutation, Destructive, Irreversible |
| `rollback_support` | `RollbackSupport` | FullReversal, CompensatingAction, LogicalUndo, Irreversible |
| `determinism_class` | `DeterminismClass` | Deterministic, PartiallyDeterministic, Stochastic, DelegatedVariant |
| `idempotence_class` | `IdempotenceClass` | Idempotent, NonIdempotent, ConditionallyIdempotent |
| `risk_class` | `RiskClass` | Negligible, Low, Medium, High, Critical |
| `latency_profile` | `LatencyProfile` | Expected/p95/max latency for this capability |
| `cost_profile` | `CostProfile` | Resource cost for this capability |
| `remote_exposable` | `bool` | Whether this capability can be invoked remotely |
| `auth_override` | `Option<AuthRequirements>` | Override port-level auth for this capability |

### PortCallRecord

Structured result of every capability invocation. Always produced, even on failure.

| Field | Type | Purpose |
|-------|------|---------|
| `observation_id` | `Uuid` | Unique observation ID |
| `port_id` | `String` | Which port was invoked |
| `capability_id` | `String` | Which capability was invoked |
| `invocation_id` | `Uuid` | Unique invocation ID |
| `success` | `bool` | Whether the invocation succeeded |
| `failure_class` | `Option<PortFailureClass>` | Failure category if not successful |
| `raw_result` | `serde_json::Value` | Raw JSON result |
| `structured_result` | `serde_json::Value` | Normalized JSON result |
| `effect_patch` | `Option<Value>` | State changes caused by this invocation |
| `side_effect_summary` | `Option<String>` | Human-readable summary of side effects |
| `latency_ms` | `u64` | Wall-clock time in milliseconds |
| `resource_cost` | `f64` | Computed resource cost |
| `confidence` | `f64` | Result confidence (1.0 for success, 0.0 for failure) |
| `timestamp` | `DateTime<Utc>` | When the invocation completed |
| `retry_safe` | `bool` | Whether retrying is safe |
| `input_hash` | `Option<String>` | Hash of the input for deduplication |
| `session_id` | `Option<Uuid>` | Owning session |
| `goal_id` | `Option<String>` | Owning goal |
| `caller_identity` | `Option<String>` | Who invoked this |
| `auth_result` | `Option<Value>` | Auth check outcome |
| `policy_result` | `Option<Value>` | Policy check outcome |
| `sandbox_result` | `Option<Value>` | Sandbox check outcome |

## Dynamic Loading

`DynamicPortLoader` in `soma-next/src/runtime/dynamic_port.rs` handles runtime loading of external port packs.

**Library naming convention**: the loader searches configured directories for `lib<name>.dylib` (macOS) or `lib<name>.so` (Linux). For a port with `port_id` of `"redis"`, the dynamic loader looks for `libsoma_port_redis.dylib`.

**Loading sequence**:

1. Search `ports.plugin_path` directories for a matching library file.
2. Verify Ed25519 signature if `ports.require_signatures` is true (sidecar `.sig` + `.pub` files).
3. Load the shared library via `libloading`.
4. Resolve the `soma_port_init` C-ABI symbol.
5. Call `soma_port_init()` to get a `Box<dyn soma_port_sdk::Port>`.
6. Wrap in `SdkPortAdapter` which bridges SDK types to runtime types via JSON serialization.
7. Retain the library handle for the loader's lifetime.

**SdkPortAdapter**: because `soma_port_sdk::Port` and `soma_next::runtime::port::Port` are separate traits with identical schemas, the adapter bridges between them by serializing `PortSpec` and `PortCallRecord` through JSON. This avoids ABI coupling while maintaining type safety.

**Built-in port routing**: `create_port_adapter` in `soma-next/src/bootstrap.rs` checks the port's `PortKind`. Filesystem and Http kinds are instantiated directly from built-in implementations. All other kinds dispatch on the port's `backend` field (see below).

## Port Backends

Every `PortSpec` has a `backend` field (defaults to `Dylib` for backward compatibility) that tells the bootstrap layer how to load the implementation. Today there are two backends:

| Backend | How it loads | Use case |
|---|---|---|
| `Dylib` | `libloading` opens `libsoma_port_<port_id>.dylib`/`.so`, calls `soma_port_init`, wraps in `SdkPortAdapter`. | Native Rust ports — the 11 in `soma-ports/`. |
| `McpClient` | Spawns a local subprocess (stdio) or connects to a remote URL (http), runs MCP `initialize` + `tools/list`, wraps in `McpClientPort`. | Ports written in any language with an MCP SDK (Node, Python, Bun, PHP, Go, Ruby, …) **and** consumption of remote MCP servers as ports. |

The two `McpClient` transports share a single implementation (`soma-next/src/runtime/mcp_client_port.rs`) — only the transport layer differs:

```json
"backend": {
  "type": "mcp_client",
  "transport": {
    "type": "stdio",
    "command": "python3",
    "args": ["servers/hello_py/server.py"],
    "env": {},
    "working_dir": null
  }
}
```

```json
"backend": {
  "type": "mcp_client",
  "transport": {
    "type": "http",
    "url": "https://example.com/mcp",
    "headers": {"Authorization": "Bearer ..."}
  }
}
```

**Dynamic capability discovery.** The `McpClient` backend runs `tools/list` at load time and merges the returned tools into the port's `PortSpec.capabilities`. The manifest can leave `capabilities: []` and have discovery populate them, or declare capabilities statically (which always win over discovered values — static effect/risk classes are treated as ground truth). Discovered capabilities use safe defaults: `effect_class: ReadOnly`, `risk_class: Low`, `determinism_class: Deterministic`, `idempotence_class: Idempotent`.

**Result shape.** MCP `tools/call` returns `{content: [{type: "text", text: "..."}], isError: false, structuredContent?: ...}`. `McpClientPort::extract_structured` unwraps in this preference order: `structuredContent` → `content[0].text` parsed as JSON → `content[0].text` as a literal string wrapped in `{"text": "..."}`. The full MCP envelope is preserved in `PortCallRecord.raw_result` so the original content is never lost.

**Lifecycle.** Stdio subprocesses are spawned eagerly at bootstrap and reaped on drop: closing stdin lets well-behaved servers exit cleanly, and a 500ms grace period precedes SIGKILL. HTTP transports use a `reqwest::blocking::Client` with a 30s timeout; each invocation POSTs a JSON-RPC request.

**Proven end-to-end** in `soma-project-mcp-bridge/` — a pack manifest points at a ~100-line pure-stdlib Python MCP server exposing `greet` and `reverse` tools. `scripts/test.sh` (smoke test) shows the round trip `mcp-client.mjs → soma-next → McpClientPort → python3 servers/hello_py/server.py → back` producing `{"message": "hello marcu!"}` and `{"reversed": "!ucram olleh"}`. Two chained stdio bridges; no Rust, no FFI, no dylib, no SDK version to match.

## Manifest Format

Ports are declared in pack manifests (`PackSpec` JSON files). Each pack's `ports` array contains `PortSpec` objects. During bootstrap, the runtime iterates over declared ports, creates adapters, registers them, and activates them.

Key rules:
- `port_id` must match the dynamic library name convention (`libsoma_port_<port_id>`) for dynamically loaded ports.
- Built-in ports (Filesystem, Http) are matched by `PortKind`, not by library name.
- Each port transitions through `Declared -> Loaded -> Validated -> Active` lifecycle states.
- Failed port loads are logged as warnings and skipped -- they do not abort bootstrap.

## Port Catalog

### Overview

| port_id | Kind | Caps | Key Env Vars | State Model |
|---------|------|------|-------------|-------------|
| `filesystem` | Filesystem | 7 | -- | Stateless (built-in) |
| `http` | Http | 4 | -- | Stateless (built-in) |
| `auth` | Custom | 10 | -- | In-memory (OTP, sessions, tokens) |
| `crypto` | Custom | 13 | -- | Stateless |
| `geo` | Custom | 5 | `api_key` (per-call) | Stateless |
| `soma.image` | Custom | 5 | -- | Stateless |
| `soma.ports.postgres` | Database | 15 | `SOMA_POSTGRES_URL` | Connection per call |
| `redis` | Database | 13 | `SOMA_REDIS_URL` | Persistent connection (ConnectionManager) |
| `soma.s3` | Database | 5 | `SOMA_S3_DEFAULT_BUCKET`, `SOMA_S3_REGION`, `SOMA_S3_ENDPOINT` | Persistent client (OnceLock) |
| `soma.smtp` | Messaging | 3 | `SOMA_SMTP_HOST`, `SOMA_SMTP_FROM`, `SOMA_SMTP_PORT`, `SOMA_SMTP_USERNAME`, `SOMA_SMTP_PASSWORD`, `SOMA_SMTP_STARTTLS` | Persistent transport config (OnceLock) |
| `soma.push` | Messaging | 4 | -- | In-memory (device registry) |
| `soma.timer` | Custom | 4 | -- | In-memory (timer state) |

### filesystem (built-in)

Local OS filesystem operations. Compiled into the runtime binary.

| capability_id | Purpose | Effect Class | Risk Class |
|--------------|---------|-------------|------------|
| `readdir` | List entries in a directory | ReadOnly | Negligible |
| `readfile` | Read file contents as UTF-8 | ReadOnly | Negligible |
| `writefile` | Write text to a file (create/overwrite) | LocalStateMutation | Medium |
| `stat` | Get file/directory metadata | ReadOnly | Negligible |
| `mkdir` | Create directory with parents | LocalStateMutation | Low |
| `rmdir` | Remove an empty directory | Destructive | Medium |
| `rm` | Remove a file | Destructive | Medium |

**Configuration**: none. Requires `filesystem_access` sandbox permission.

**Notes**: trust level is BuiltIn. Sandbox time limit 5000ms. All destructive operations are idempotent.

### http (built-in)

Synchronous HTTP client via reqwest. Compiled into the runtime binary.

| capability_id | Purpose | Effect Class | Risk Class |
|--------------|---------|-------------|------------|
| `get` | Fetch a resource via HTTP GET | ReadOnly | Low |
| `post` | Send data via HTTP POST | ExternalStateMutation | Medium |
| `put` | Update a resource via HTTP PUT | ExternalStateMutation | Medium |
| `delete` | Delete a resource via HTTP DELETE | Destructive | Medium |

**Configuration**: none. 30-second timeout. Requires `network_access` sandbox permission.

**Notes**: trust level is Trusted. All capabilities are remote-exposable. Sandbox time limit 30000ms.

### auth

Authentication: OTP verification, session management, TOTP, bearer tokens.

| capability_id | Purpose | Effect Class | Risk Class |
|--------------|---------|-------------|------------|
| `otp_generate` | Generate a 6-digit OTP for phone verification | LocalStateMutation | Low |
| `otp_verify` | Verify OTP code (max 5 attempts) | LocalStateMutation | Low |
| `session_create` | Create an authenticated session with expiry | LocalStateMutation | Low |
| `session_validate` | Check if a session token is valid | ReadOnly | Low |
| `session_revoke` | Mark a session as revoked | LocalStateMutation | Low |
| `totp_generate` | Generate TOTP secret and provisioning URI | None | Low |
| `totp_verify` | Verify a TOTP code against a secret | None | Low |
| `token_generate` | Generate a random bearer token with expiry | LocalStateMutation | Low |
| `token_validate` | Check if a bearer token is valid | ReadOnly | Low |
| `token_refresh` | Extend the expiry of a valid bearer token | LocalStateMutation | Low |

**Configuration**: none. All state is in-memory (HashMap behind Mutex).

**Notes**: OTP codes expire after 5 minutes, max 5 verification attempts. Sessions default to 720-hour TTL. Tokens default to 24-hour TTL. All hashing uses SHA-256. A production deployment would back this with a database port.

### crypto

Cryptographic operations. Stateless -- all keying material is supplied per call.

| capability_id | Purpose | Effect Class | Risk Class |
|--------------|---------|-------------|------------|
| `sha256` | Compute SHA-256 hex digest | None | Negligible |
| `sha512` | Compute SHA-512 hex digest | None | Negligible |
| `hmac` | Compute HMAC-SHA256 authentication code | None | Negligible |
| `bcrypt_hash` | Hash password with bcrypt (default cost 12) | None | Negligible |
| `bcrypt_verify` | Verify password against bcrypt hash | None | Negligible |
| `aes_encrypt` | Encrypt with AES-256-GCM (nonce prepended) | None | Negligible |
| `aes_decrypt` | Decrypt AES-256-GCM ciphertext | None | Negligible |
| `rsa_sign` | Sign data with RSA-PKCS1v15-SHA256 | None | Negligible |
| `rsa_verify` | Verify RSA-PKCS1v15-SHA256 signature | None | Negligible |
| `jwt_sign` | Sign JWT with HS256 | None | Negligible |
| `jwt_verify` | Verify JWT and decode claims | None | Negligible |
| `random_bytes` | Generate cryptographically secure random bytes (1-65536) | None | Negligible |
| `random_string` | Generate random alphanumeric string (1-65536) | None | Negligible |

**Configuration**: none.

**Notes**: AES-256-GCM keys must be exactly 32 bytes. Nonce is randomly generated and prepended to ciphertext. RSA keys are provided as PKCS#1 PEM. All operations are idempotent except random generation (stochastic).

### geo

Geolocation: distance calculations, radius filtering, bounding box checks, geocoding stubs.

| capability_id | Purpose | Effect Class | Risk Class |
|--------------|---------|-------------|------------|
| `distance` | Haversine great-circle distance between two points | None | Negligible |
| `radius_filter` | Filter a JSON point array to entries within radius | None | Negligible |
| `geocode` | Address to coordinates (requires API key) | None | Negligible |
| `reverse_geocode` | Coordinates to address (requires API key) | None | Negligible |
| `bounds_check` | Check if a point falls within a bounding box | None | Negligible |

**Configuration**: `api_key` passed per-call for geocoding capabilities.

**Notes**: `distance`, `radius_filter`, and `bounds_check` are pure math with no external dependencies. `geocode` and `reverse_geocode` return errors unless an API key is provided, and the geocoding backend is not yet implemented -- a production deployment would call Nominatim, Google Maps, or similar.

### soma.image

Image processing using the `image` crate (pure Rust, no system dependencies).

| capability_id | Purpose | Effect Class | Risk Class |
|--------------|---------|-------------|------------|
| `thumbnail` | Generate a thumbnail (fast, lower quality) | None | Negligible |
| `resize` | Resize to exact dimensions (Lanczos3) | None | Negligible |
| `crop` | Crop a rectangular region | None | Negligible |
| `format_convert` | Convert between PNG, JPEG, and WebP | None | Negligible |
| `exif_strip` | Strip EXIF metadata by re-encoding pixel data | None | Negligible |

**Configuration**: none. Image data is passed as base64-encoded strings.

**Notes**: max dimension 16384 to prevent multi-gigabyte allocations. Trust level is Verified. All capabilities are remote-exposable and idempotent.

### soma.ports.postgres

PostgreSQL database operations via `tokio-postgres` with synchronous `block_on()` bridging.

| capability_id | Purpose | Effect Class | Risk Class |
|--------------|---------|-------------|------------|
| `query` | Execute SELECT, return rows | ReadOnly | Low |
| `execute` | Execute INSERT/UPDATE/DELETE, return rows affected | ExternalStateMutation | Medium |
| `find` | Find a single row by ID (ORM-style) | ReadOnly | Low |
| `find_many` | Find multiple rows with structured filter | ReadOnly | Low |
| `count` | Count rows matching a filter | ReadOnly | Negligible |
| `aggregate` | Run SUM/AVG/MIN/MAX with optional grouping | ReadOnly | Low |
| `create_table` | DDL CREATE TABLE IF NOT EXISTS | ExternalStateMutation | High |
| `drop_table` | DDL DROP TABLE IF EXISTS | Destructive | Critical |
| `alter_table` | DDL ALTER TABLE | ExternalStateMutation | High |
| `insert` | Insert a row with column/value pairs | ExternalStateMutation | Low |
| `update` | Update rows matching a WHERE filter | ExternalStateMutation | Medium |
| `delete` | Delete rows matching a WHERE filter | Destructive | High |
| `begin_transaction` | Execute multiple statements atomically | ExternalStateMutation | Medium |
| `commit` | Commit a transaction | ExternalStateMutation | Low |
| `rollback` | Rollback a transaction | None | Negligible |

**Configuration**: `SOMA_POSTGRES_URL` (default: `host=localhost dbname=soma`). Requires `network_access` sandbox permission. Sandbox time limit 30000ms.

**Notes**: each invocation creates a fresh connection via `block_on()`. Trust level is Verified. Transaction support: `begin_transaction` accepts a `statements` array and executes them atomically. `commit` and `rollback` are provided for API completeness.

### redis

Redis key-value store operations via `redis` crate with async `ConnectionManager`.

| capability_id | Purpose | Effect Class | Risk Class |
|--------------|---------|-------------|------------|
| `get` | Get string value by key | ReadOnly | Negligible |
| `set` | Set string value with optional TTL | ExternalStateMutation | Low |
| `del` | Delete a key | ExternalStateMutation | Low |
| `hget` | Get a hash field value | ReadOnly | Negligible |
| `hset` | Set a hash field value | ExternalStateMutation | Low |
| `hdel` | Delete a hash field | ExternalStateMutation | Low |
| `hgetall` | Get all hash fields and values | ReadOnly | Negligible |
| `lpush` | Push value to head of list | ExternalStateMutation | Low |
| `lpop` | Pop value from head of list | ReadOnly | Negligible |
| `lrange` | Get a range of elements from a list | ReadOnly | Negligible |
| `publish` | Publish a message to a channel | ExternalStateMutation | Low |
| `subscribe` | Subscribe to a channel (not yet supported) | None | Negligible |
| `keys` | Find keys matching a glob pattern | ReadOnly | Negligible |

**Configuration**: `SOMA_REDIS_URL` (default: `redis://localhost:6379/0`). Requires `network_access` sandbox permission.

**Notes**: uses `ConnectionManager` for automatic reconnection. All operations bridge async to sync via a dedicated Tokio runtime. `subscribe` returns an error because streaming is not supported in the sync port invocation model. Trust level is Verified. Lifecycle state is Degraded when not connected.

### soma.s3

S3-compatible object storage via the AWS SDK for Rust.

| capability_id | Purpose | Effect Class | Risk Class |
|--------------|---------|-------------|------------|
| `put_object` | Upload an object to a bucket | ExternalStateMutation | Low |
| `get_object` | Download an object (returned as base64) | ReadOnly | Negligible |
| `delete_object` | Delete an object | Destructive | Medium |
| `presign_url` | Generate a presigned URL for temporary access | None | Low |
| `list_objects` | List objects with optional prefix filter | ReadOnly | Negligible |

**Configuration**: `SOMA_S3_DEFAULT_BUCKET` (default: `soma-uploads`), `SOMA_S3_REGION` or `AWS_REGION`, `SOMA_S3_ENDPOINT` or `AWS_ENDPOINT_URL_S3`. Requires `network_access` sandbox permission.

**Notes**: endpoint URL enables MinIO/LocalStack usage (force-path-style enabled automatically). Client is initialized lazily via `OnceLock`. `put_object` and `delete_object` use compensating actions. `presign_url` defaults to 3600-second expiry. Trust level is Verified.

### soma.smtp

SMTP email delivery via the `lettre` crate.

| capability_id | Purpose | Effect Class | Risk Class |
|--------------|---------|-------------|------------|
| `send_plain` | Send a plain-text email | ExternalStateMutation | Low |
| `send_html` | Send an HTML email | ExternalStateMutation | Low |
| `send_attachment` | Send an email with a binary attachment (base64) | ExternalStateMutation | Low |

**Configuration**: `SOMA_SMTP_HOST` or `SMTP_HOST` (required), `SOMA_SMTP_FROM` or `SMTP_FROM` (required), `SOMA_SMTP_PORT` or `SMTP_PORT` (default: 587), `SOMA_SMTP_USERNAME` or `SMTP_USERNAME` (optional), `SOMA_SMTP_PASSWORD` or `SMTP_PASSWORD` (optional), `SOMA_SMTP_STARTTLS` or `SMTP_STARTTLS` (default: true, set to `false` for local dev servers like mailcatcher).

**Notes**: lifecycle state is Active only when host, port, and from address are all configured. Credentials are optional (for local dev). All send operations are non-idempotent and irreversible. Trust level is Verified. Requires `network_access` sandbox permission.

### soma.push

Push notifications via FCM and Web Push, with in-memory device registration.

| capability_id | Purpose | Effect Class | Risk Class |
|--------------|---------|-------------|------------|
| `send_fcm` | Send notification via Firebase Cloud Messaging HTTP v1 API | ExternalStateMutation | Low |
| `send_webpush` | Send notification via Web Push protocol (VAPID) | ExternalStateMutation | Low |
| `register_device` | Register a device token for a user/platform | LocalStateMutation | Negligible |
| `unregister_device` | Remove a device registration | LocalStateMutation | Negligible |

**Configuration**: `access_token` and `project_id` passed per-call for FCM. `vapid_key` and `subscription_json` passed per-call for WebPush. No env vars.

**Notes**: device registrations are held in an in-memory `RwLock<HashMap>` keyed by user_id. Supported platforms: android, ios, web. FCM and WebPush sends are non-idempotent and irreversible. Device registration/unregistration supports compensating actions. Trust level is Verified. Requires `network_access` sandbox permission and ApiKey auth.

### soma.timer

Timer/scheduler with in-memory state machine. No external services.

| capability_id | Purpose | Effect Class | Risk Class |
|--------------|---------|-------------|------------|
| `set_timeout` | Set a one-shot timer that fires after a delay | LocalStateMutation | Negligible |
| `set_interval` | Set a recurring timer at regular intervals | LocalStateMutation | Negligible |
| `cancel_timer` | Cancel an active timer by ID | LocalStateMutation | Negligible |
| `list_active` | List all active timers with remaining time | ReadOnly | Negligible |

**Configuration**: none.

**Notes**: timer entries are stored in a `Mutex<HashMap>` keyed by UUID. `delay_ms` must be positive. `cancel_timer` is idempotent. Trust level is BuiltIn.

## Building a Port

1. Create a `cdylib` crate: `cargo init --lib` with `crate-type = ["cdylib"]` in `Cargo.toml`.
2. Add dependency: `soma-port-sdk = { path = "../sdk" }`.
3. Define a struct holding `PortSpec` and any connection state.
4. Implement `Port` for your struct: `spec()`, `invoke()`, `validate_input()`, `lifecycle_state()`.
5. Build the `PortSpec` in a constructor, declaring all capabilities with their schemas, effect classes, risk classes, and latency profiles.
6. Export the C-ABI init function:
   ```rust
   #[allow(improper_ctypes_definitions)]
   #[unsafe(no_mangle)]
   pub extern "C" fn soma_port_init() -> *mut dyn Port {
       Box::into_raw(Box::new(MyPort::new()))
   }
   ```
7. Build with `cargo build --release`. The output is `target/release/libsoma_port_<name>.dylib`.
8. Place the library in a directory listed in `ports.plugin_path` in `soma.toml`.
9. Declare the port in a pack manifest's `ports` array.
10. Optionally, sign the library with Ed25519 and place `.sig` and `.pub` sidecar files alongside it.

---

## Embedded Ports (`no_std`, compile-time composition)

`soma-project-esp32` is a separate port ecosystem for microcontrollers. It does **not** use the `soma-port-sdk` / `cdylib` / dynamic-loading model — microcontrollers can't dynamically load shared libraries, so ports are `rlib` crates composed into the firmware binary at build time via cargo features. The runtime contract is different too: the embedded leaf hosts a `CompositeDispatcher` of `Box<dyn SomaEspPort>` instead of the server's `PortRuntime`, and the wire protocol is a length-prefixed JSON envelope (`TransportMessage` / `TransportResponse`) carried over UART0 or TCP.

### SomaEspPort Trait

```rust
pub trait SomaEspPort {
    fn port_id(&self) -> &'static str;
    fn primitives(&self) -> Vec<CapabilityDescriptor>;
    fn invoke(&mut self, skill_id: &str, input: &serde_json::Value) -> Result<Value, String>;
}
```

Defined in `soma-project-esp32/leaf/src/lib.rs`. Every port crate (`ports/gpio/`, `ports/i2c/`, etc.) implements this trait. The firmware builds a `CompositeDispatcher`, calls `register(Box::new(port))` for each enabled port, and the leaf's wire-protocol handler routes incoming `InvokeSkill` messages to the right port by prefix-matching the skill_id against registered port_ids.

### Embedded Port Catalog

| port_id | Skills | Effect Classes | Notes |
|---|---|---|---|
| `gpio` | `gpio.write`, `gpio.read`, `gpio.toggle` | SM / RO / SM | GPIO is claimed at boot via `gpio_port.claim_output_pin(n, Output::new(...))`. Only claimed pins can be driven. |
| `delay` | `delay.ms`, `delay.us` | RO / RO | Blocking `esp_hal::delay` wrappers. |
| `uart` | `uart.write`, `uart.read` | EX / RO | UART1. UART0 is reserved for host wire-protocol transport. |
| `i2c` | `i2c.write`, `i2c.read`, `i2c.write_read`, `i2c.scan` | SM / RO / SM / RO | **Generic over any `embedded_hal::i2c::I2c`** — takes a raw `esp-hal I2c` or an `embedded-hal-bus RefCellDevice` for the shared-bus path. |
| `spi` | `spi.write`, `spi.read`, `spi.transfer` | SM / RO / SM | SPI3 by default; SPI2 is left free for on-board displays. |
| `adc` | `adc.read`, `adc.read_voltage` | RO / RO | Takes an `AdcReadFn: Box<dyn FnMut() -> Result<u16, AdcError>>` closure so the port crate never depends on `esp-hal`. The chip module builds the closure against a typed `GpioPin<N>` inside a match over valid ADC1 pins. |
| `pwm` | `pwm.set_duty`, `pwm.get_status` | SM / RO | LEDC channel 0. Takes a `PwmSetDutyFn` closure the firmware injects after configuring the timer + channel. |
| `wifi` | `wifi.scan`, `wifi.configure`, `wifi.status`, `wifi.disconnect`, `wifi.forget` | RO / SM / RO / SM / SM | Feature-gated. Pulls in `esp-wifi 0.12` + `smoltcp 0.12`. `wifi.configure` persists credentials to `FlashKvStore` so they survive reboots. |
| `storage` | `storage.get`, `storage.set`, `storage.delete`, `storage.list`, `storage.clear` | RO / SM / SM / RO / SM | Backed by a `FlashKvStore` over `esp-storage` at flash offset `0x3F_F000`. 4 KB sector rewritten in its entirety on every write. |
| `thermistor` | `thermistor.read_temp`, `thermistor.read_temp_calibrated` | RO / RO | Example sensor port (currently simulated; swap in a real ADC read via the closure pattern to hook up a physical thermistor). |
| `board` | `board.chip_info`, `board.pin_map`, `board.configure_pin`, `board.probe_i2c_buses`, `board.reboot` | RO / RO / SM / SM / EX | Diagnostic and runtime pin-configuration skills. Five injected closures (`ChipInfoFn`, `PinMapFn`, `ProbeI2cFn`, `RebootFn`, `ConfigureFn`) capture the chip-specific state. Enables bring-up of new boards without reflashing. |
| `display` | `display.info`, `display.clear`, `display.draw_text`, `display.draw_text_xy`, `display.fill_rect`, `display.set_contrast`, `display.flush` | RO / SM / SM / SM / SM / SM / SM | SSD1306 OLED via seven injected closures. Shares I²C0 with the `i2c` port through `embedded-hal-bus::RefCellDevice`. The port crate has zero `esp-hal`/`ssd1306`/`embedded-graphics` dependencies — the firmware owns the driver, the port owns the wire-protocol surface. |

Effect class legend: RO = ReadOnly, SM = StateMutation, EX = ExternalEffect.

### Type-Erased Closure Pattern

Ports that need hardware access without taking an `esp-hal` dependency follow a uniform pattern: the port struct stores `Box<dyn FnMut(...)>` closures and calls them when a skill arrives. Example from `ports/board/`:

```rust
pub type ChipInfoFn = Box<dyn FnMut() -> ChipInfo + Send>;
pub type ProbeI2cFn = Box<dyn FnMut(&[(u8, u8)]) -> Vec<ProbeResult> + Send>;

pub struct BoardPort {
    chip_info_fn: ChipInfoFn,
    probe_i2c_fn: ProbeI2cFn,
    // ...
}
```

The firmware's `register_all_ports` function builds each closure by capturing chip-specific state (peripherals, FlashKvStore handle, driver instances) via a `move` closure:

```rust
let chip_info_fn: ChipInfoFn = Box::new(move || ChipInfo {
    chip: NAME,
    mac: Efuse::read_base_mac_address(),
    free_heap: esp_alloc::HEAP.free() as u32,
    // ...
});
composite.register(Box::new(BoardPort::new(chip_info_fn, ...)));
```

This keeps port crates chip-agnostic (the same `BoardPort` works on ESP32 and ESP32-S3) and esp-hal-free (port crates have no heavy dependency chain). Adding a new chip means dropping one chip module and wiring the closures — port crates are untouched.

### Shared I²C Bus via `embedded-hal-bus`

The `i2c` and `display` ports share the same I²C0 peripheral on the same physical SDA/SCL pins. The firmware wraps the `esp-hal I2c` instance in a `Box::leak`ed `&'static RefCell` and hands each consumer its own `embedded_hal_bus::i2c::RefCellDevice`:

```rust
let bus_static: &'static RefCell<I2c<'static, Blocking>> =
    Box::leak(Box::new(RefCell::new(i2c)));

let i2c_device = RefCellDevice::new(bus_static);
composite.register(Box::new(I2cPort::new(i2c_device)));

let display_device = RefCellDevice::new(bus_static);
let ssd = Ssd1306::new(I2CDisplayInterface::new(display_device), ...)
    .into_buffered_graphics_mode();
```

`I2cPort<B>` is generic over any `embedded_hal::i2c::I2c` implementor, so the exact same port type works with a raw bus (`display` feature off) or a `RefCellDevice` (shared-bus path). The leaf's dispatch loop is single-threaded, so `RefCell` is safe — no `critical_section::Mutex` is needed.

### Building an Embedded Port

1. Create an `rlib` crate under `soma-project-esp32/ports/<name>/` with `crate-type = ["rlib"]`.
2. Depend on `soma-esp32-leaf = { path = "../../leaf" }` and `serde_json = { version = "1", default-features = false, features = ["alloc"] }`.
3. If the port needs hardware access, define `Box<dyn FnMut(...)>` type aliases for the operations and store them as fields. Do NOT add an `esp-hal` dependency.
4. Implement `SomaEspPort` with a constant `port_id()`, a static `primitives()` capability list, and an `invoke()` method that dispatches on `skill_id`.
5. Add the crate to the workspace's `Cargo.toml` members list.
6. In `firmware/Cargo.toml`, add the crate as `optional = true` and expose a matching feature: `foo = ["dep:soma-esp32-port-foo"]`.
7. Register the port in each `firmware/src/chip/<chip>.rs` under `#[cfg(feature = "foo")] { ... }`, building the hardware-capture closures from peripherals moved out of `peripherals`.
8. Add the feature to `scripts/lib.sh::ALL_PORTS` so `./scripts/build.sh <chip>` picks it up.

The port is then invokable over the wire protocol (UART or TCP) using the same `TransportMessage::InvokeSkill { peer_id, skill_id, input }` shape the server runtime uses.
