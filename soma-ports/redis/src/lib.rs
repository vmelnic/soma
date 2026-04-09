//! SOMA Redis Port — 13 capabilities for Redis key-value operations.
//!
//! Provides string get/set/del, hash operations (hget/hset/hdel/hgetall),
//! list operations (lpush/lpop/lrange), pub/sub (publish/subscribe),
//! and key pattern search.
//!
//! # Connection
//!
//! Reads `SOMA_REDIS_URL` from the environment, falling back to
//! `redis://localhost:6379/0`. Uses `redis::aio::ConnectionManager` for
//! automatic reconnection and connection multiplexing. All operations go
//! through `invoke_async` then bridge to sync via a dedicated Tokio runtime.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use semver::Version;
use soma_port_sdk::prelude::*;

// ---------------------------------------------------------------------------
// RedisPort
// ---------------------------------------------------------------------------

/// SOMA port adapter for Redis.
///
/// Wraps an async `ConnectionManager` behind a `Mutex`. The connection is
/// established during construction and reconnects automatically on failure.
pub struct RedisPort {
    spec: PortSpec,
    conn: Mutex<Option<redis::aio::ConnectionManager>>,
    rt: tokio::runtime::Runtime,
}

impl Default for RedisPort {
    fn default() -> Self {
        Self::new()
    }
}

impl RedisPort {
    /// Create a new Redis port, connecting to `SOMA_REDIS_URL` or localhost.
    pub fn new() -> Self {
        let url = std::env::var("SOMA_REDIS_URL")
            .unwrap_or_else(|_| "redis://localhost:6379/0".to_string());

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to create tokio runtime for redis port");

        let conn = rt.block_on(async {
            let client = redis::Client::open(url.as_str()).ok()?;
            redis::aio::ConnectionManager::new(client).await.ok()
        });

        Self {
            spec: build_spec(),
            conn: Mutex::new(conn),
            rt,
        }
    }

    /// Clone the connection manager, returning an error if not connected.
    fn get_conn(&self) -> soma_port_sdk::Result<redis::aio::ConnectionManager> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| PortError::Internal(format!("lock poisoned: {e}")))?;
        guard
            .clone()
            .ok_or_else(|| PortError::DependencyUnavailable("Redis not connected".into()))
    }

    /// Async implementation for all 13 capabilities.
    async fn invoke_async(
        &self,
        capability_id: &str,
        input: serde_json::Value,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        use redis::AsyncCommands;

        let mut conn = self.get_conn()?;

        match capability_id {
            // --- String operations ---
            "get" => {
                let key = require_str(&input, "key")?;
                let val: Option<String> = conn.get(&key).await.map_err(redis_err)?;
                Ok(match val {
                    Some(s) => serde_json::json!(s),
                    None => serde_json::Value::Null,
                })
            }

            "set" => {
                let key = require_str(&input, "key")?;
                let value = require_str(&input, "value")?;
                let ttl = input.get("ttl").and_then(|v| v.as_i64());

                if let Some(seconds) = ttl {
                    if seconds <= 0 {
                        return Err(PortError::Validation("ttl must be positive".into()));
                    }
                    let _: () = redis::cmd("SETEX")
                        .arg(&key)
                        .arg(seconds)
                        .arg(&value)
                        .query_async(&mut conn)
                        .await
                        .map_err(redis_err)?;
                } else {
                    let _: () = conn.set(&key, &value).await.map_err(redis_err)?;
                }
                Ok(serde_json::json!("OK"))
            }

            "del" => {
                let key = require_str(&input, "key")?;
                let deleted: i64 = conn.del(&key).await.map_err(redis_err)?;
                Ok(serde_json::json!(deleted > 0))
            }

            // --- Hash operations ---
            "hget" => {
                let key = require_str(&input, "key")?;
                let field = require_str(&input, "field")?;
                let val: Option<String> = conn.hget(&key, &field).await.map_err(redis_err)?;
                Ok(match val {
                    Some(s) => serde_json::json!(s),
                    None => serde_json::Value::Null,
                })
            }

            "hset" => {
                let key = require_str(&input, "key")?;
                let field = require_str(&input, "field")?;
                let value = require_str(&input, "value")?;
                let added: i64 = conn.hset(&key, &field, &value).await.map_err(redis_err)?;
                Ok(serde_json::json!(added > 0))
            }

            "hdel" => {
                let key = require_str(&input, "key")?;
                let field = require_str(&input, "field")?;
                let removed: i64 = conn.hdel(&key, &field).await.map_err(redis_err)?;
                Ok(serde_json::json!(removed > 0))
            }

            "hgetall" => {
                let key = require_str(&input, "key")?;
                let map: HashMap<String, String> = conn.hgetall(&key).await.map_err(redis_err)?;
                Ok(serde_json::json!(map))
            }

            // --- List operations ---
            "lpush" => {
                let key = require_str(&input, "key")?;
                let value = require_str(&input, "value")?;
                let len: i64 = conn.lpush(&key, &value).await.map_err(redis_err)?;
                Ok(serde_json::json!(len))
            }

            "lpop" => {
                let key = require_str(&input, "key")?;
                let val: Option<String> = conn.lpop(&key, None).await.map_err(redis_err)?;
                Ok(match val {
                    Some(s) => serde_json::json!(s),
                    None => serde_json::Value::Null,
                })
            }

            "lrange" => {
                let key = require_str(&input, "key")?;
                let start = input.get("start").and_then(|v| v.as_i64()).unwrap_or(0) as isize;
                let stop = input.get("stop").and_then(|v| v.as_i64()).unwrap_or(-1) as isize;
                let items: Vec<String> = conn.lrange(&key, start, stop).await.map_err(redis_err)?;
                Ok(serde_json::json!(items))
            }

            // --- Pub/Sub ---
            "publish" => {
                let channel = require_str(&input, "channel")?;
                let message = require_str(&input, "message")?;
                let receivers: i64 = conn.publish(&channel, &message).await.map_err(redis_err)?;
                Ok(serde_json::json!(receivers))
            }

            "subscribe" => Err(PortError::ExternalError(
                "subscribe requires streaming, not supported in sync port invocation".into(),
            )),

            // --- Key listing ---
            "keys" => {
                let pattern = require_str(&input, "pattern")?;
                let matched: Vec<String> = conn.keys(&pattern).await.map_err(redis_err)?;
                Ok(serde_json::json!(matched))
            }

            _ => Err(PortError::NotFound(format!(
                "unknown capability: {capability_id}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Port trait implementation
// ---------------------------------------------------------------------------

impl Port for RedisPort {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(
        &self,
        capability_id: &str,
        input: serde_json::Value,
    ) -> soma_port_sdk::Result<PortCallRecord> {
        let start = Instant::now();

        match self.rt.block_on(self.invoke_async(capability_id, input)) {
            Ok(result) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                Ok(PortCallRecord::success(
                    "redis",
                    capability_id,
                    result,
                    latency_ms,
                ))
            }
            Err(e) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                Ok(PortCallRecord::failure(
                    "redis",
                    capability_id,
                    e.failure_class(),
                    &e.to_string(),
                    latency_ms,
                ))
            }
        }
    }

    fn validate_input(
        &self,
        capability_id: &str,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<()> {
        match capability_id {
            "get" | "del" | "lpop" => {
                require_str(input, "key")?;
            }
            "set" => {
                require_str(input, "key")?;
                require_str(input, "value")?;
            }
            "hget" | "hdel" => {
                require_str(input, "key")?;
                require_str(input, "field")?;
            }
            "hset" => {
                require_str(input, "key")?;
                require_str(input, "field")?;
                require_str(input, "value")?;
            }
            "hgetall" => {
                require_str(input, "key")?;
            }
            "lpush" => {
                require_str(input, "key")?;
                require_str(input, "value")?;
            }
            "lrange" => {
                require_str(input, "key")?;
            }
            "publish" => {
                require_str(input, "channel")?;
                require_str(input, "message")?;
            }
            "subscribe" => {
                require_str(input, "channel")?;
            }
            "keys" => {
                require_str(input, "pattern")?;
            }
            _ => {
                return Err(PortError::NotFound(format!(
                    "unknown capability: {capability_id}"
                )));
            }
        }
        Ok(())
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        let guard = self.conn.lock().ok();
        match guard {
            Some(ref g) if g.is_some() => PortLifecycleState::Active,
            _ => PortLifecycleState::Degraded,
        }
    }
}

// ---------------------------------------------------------------------------
// C ABI entry point
// ---------------------------------------------------------------------------

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    Box::into_raw(Box::new(RedisPort::new()))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract a required string field from JSON input.
fn require_str(input: &serde_json::Value, field: &str) -> soma_port_sdk::Result<String> {
    input
        .get(field)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| PortError::Validation(format!("missing required field: {field}")))
}

/// Map a redis error into a PortError.
fn redis_err(e: redis::RedisError) -> PortError {
    PortError::ExternalError(format!("Redis: {e}"))
}

// ---------------------------------------------------------------------------
// Capability helpers
// ---------------------------------------------------------------------------

/// Shorthand for a read-only capability with low latency.
fn read_cap(
    id: &str,
    name: &str,
    purpose: &str,
    input_schema: serde_json::Value,
    output_schema: serde_json::Value,
) -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: id.to_string(),
        name: name.to_string(),
        purpose: purpose.to_string(),
        input_schema: SchemaRef {
            schema: input_schema,
        },
        output_schema: SchemaRef {
            schema: output_schema,
        },
        effect_class: SideEffectClass::ReadOnly,
        rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::Stochastic,
        idempotence_class: IdempotenceClass::Idempotent,
        risk_class: RiskClass::Negligible,
        latency_profile: LatencyProfile {
            expected_latency_ms: 1,
            p95_latency_ms: 5,
            max_latency_ms: 100,
        },
        cost_profile: CostProfile::default(),
        remote_exposable: true,
        auth_override: None,
    }
}

/// Shorthand for a write capability (external state mutation).
fn write_cap(
    id: &str,
    name: &str,
    purpose: &str,
    input_schema: serde_json::Value,
    output_schema: serde_json::Value,
) -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: id.to_string(),
        name: name.to_string(),
        purpose: purpose.to_string(),
        input_schema: SchemaRef {
            schema: input_schema,
        },
        output_schema: SchemaRef {
            schema: output_schema,
        },
        effect_class: SideEffectClass::ExternalStateMutation,
        rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::Stochastic,
        idempotence_class: IdempotenceClass::Idempotent,
        risk_class: RiskClass::Low,
        latency_profile: LatencyProfile {
            expected_latency_ms: 1,
            p95_latency_ms: 5,
            max_latency_ms: 100,
        },
        cost_profile: CostProfile::default(),
        remote_exposable: true,
        auth_override: None,
    }
}

// ---------------------------------------------------------------------------
// PortSpec builder
// ---------------------------------------------------------------------------

fn build_spec() -> PortSpec {
    let key_schema = serde_json::json!({
        "type": "object",
        "properties": { "key": { "type": "string" } },
        "required": ["key"]
    });

    let key_value_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "key": { "type": "string" },
            "value": { "type": "string" },
            "ttl": { "type": "integer" }
        },
        "required": ["key", "value"]
    });

    let key_field_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "key": { "type": "string" },
            "field": { "type": "string" }
        },
        "required": ["key", "field"]
    });

    let key_field_value_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "key": { "type": "string" },
            "field": { "type": "string" },
            "value": { "type": "string" }
        },
        "required": ["key", "field", "value"]
    });

    let lrange_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "key": { "type": "string" },
            "start": { "type": "integer" },
            "stop": { "type": "integer" }
        },
        "required": ["key"]
    });

    let channel_message_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "channel": { "type": "string" },
            "message": { "type": "string" }
        },
        "required": ["channel", "message"]
    });

    let channel_schema = serde_json::json!({
        "type": "object",
        "properties": { "channel": { "type": "string" } },
        "required": ["channel"]
    });

    let pattern_schema = serde_json::json!({
        "type": "object",
        "properties": { "pattern": { "type": "string" } },
        "required": ["pattern"]
    });

    let string_schema = serde_json::json!({ "type": "string" });
    let nullable_string = serde_json::json!({
        "oneOf": [{ "type": "string" }, { "type": "null" }]
    });
    let bool_schema = serde_json::json!({ "type": "boolean" });
    let int_schema = serde_json::json!({ "type": "integer" });
    let string_array = serde_json::json!({
        "type": "array", "items": { "type": "string" }
    });
    let object_schema = serde_json::json!({ "type": "object" });

    PortSpec {
        port_id: "redis".to_string(),
        name: "Redis".to_string(),
        version: Version::new(0, 1, 0),
        kind: PortKind::Database,
        description: "Redis key-value store: strings, hashes, lists, pub/sub".to_string(),
        namespace: "soma.ports.redis".to_string(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![
            // String operations
            read_cap(
                "get",
                "get",
                "Get string value by key",
                key_schema.clone(),
                nullable_string.clone(),
            ),
            write_cap(
                "set",
                "set",
                "Set string value with optional TTL",
                key_value_schema.clone(),
                string_schema.clone(),
            ),
            write_cap(
                "del",
                "del",
                "Delete a key",
                key_schema.clone(),
                bool_schema.clone(),
            ),
            // Hash operations
            read_cap(
                "hget",
                "hget",
                "Get a hash field value",
                key_field_schema.clone(),
                nullable_string.clone(),
            ),
            write_cap(
                "hset",
                "hset",
                "Set a hash field value",
                key_field_value_schema.clone(),
                bool_schema.clone(),
            ),
            write_cap(
                "hdel",
                "hdel",
                "Delete a hash field",
                key_field_schema.clone(),
                bool_schema.clone(),
            ),
            read_cap(
                "hgetall",
                "hgetall",
                "Get all hash fields and values",
                key_schema.clone(),
                object_schema.clone(),
            ),
            // List operations
            write_cap(
                "lpush",
                "lpush",
                "Push value to head of list",
                key_value_schema.clone(),
                int_schema.clone(),
            ),
            read_cap(
                "lpop",
                "lpop",
                "Pop value from head of list",
                key_schema.clone(),
                nullable_string.clone(),
            ),
            read_cap(
                "lrange",
                "lrange",
                "Get a range of elements from a list",
                lrange_schema.clone(),
                string_array.clone(),
            ),
            // Pub/Sub
            write_cap(
                "publish",
                "publish",
                "Publish a message to a channel",
                channel_message_schema.clone(),
                int_schema.clone(),
            ),
            {
                let mut cap = read_cap(
                    "subscribe",
                    "subscribe",
                    "Subscribe to a channel (streaming, not yet supported)",
                    channel_schema.clone(),
                    string_schema.clone(),
                );
                cap.effect_class = SideEffectClass::None;
                cap
            },
            // Key listing
            read_cap(
                "keys",
                "keys",
                "Find keys matching a glob pattern",
                pattern_schema.clone(),
                string_array.clone(),
            ),
        ],
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::any(),
        failure_modes: vec![
            PortFailureClass::DependencyUnavailable,
            PortFailureClass::Timeout,
            PortFailureClass::ExternalError,
            PortFailureClass::ValidationError,
        ],
        side_effect_class: SideEffectClass::ExternalStateMutation,
        latency_profile: LatencyProfile {
            expected_latency_ms: 1,
            p95_latency_ms: 10,
            max_latency_ms: 500,
        },
        cost_profile: CostProfile::default(),
        auth_requirements: AuthRequirements::default(),
        sandbox_requirements: SandboxRequirements {
            filesystem_access: false,
            network_access: true,
            device_access: false,
            process_access: false,
            memory_limit_mb: None,
            cpu_limit_percent: None,
            time_limit_ms: None,
            syscall_limit: None,
        },
        observable_fields: vec![
            "connection_status".to_string(),
            "command_count".to_string(),
            "latency_ms".to_string(),
        ],
        validation_rules: vec![],
        remote_exposure: true,
    }
}
