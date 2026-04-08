//! SOMA Redis Plugin -- 14 conventions for Redis key-value operations.
//!
//! Provides string get/set/delete/exists/incr, key expiry, hash operations
//! (hget/hset/hgetall), list operations (lpush/lrange), pub/sub (publish/subscribe),
//! and key pattern search.
//!
//! # Connection strategy
//!
//! Uses `redis::aio::ConnectionManager` for connection multiplexing with
//! automatic reconnection. All operations go through `execute_async_inner()`;
//! the sync `execute()` bridges via `tokio::runtime::Handle::current().block_on()`
//! when a Tokio runtime is available, or creates a temporary runtime otherwise.
//!
//! # Convention IDs
//!
//! | ID | Name      | Description                           |
//! |----|-----------|---------------------------------------|
//! |  0 | get       | Get string value by key               |
//! |  1 | set       | Set string value with optional TTL    |
//! |  2 | delete    | Delete a key                          |
//! |  3 | exists    | Check if key exists                   |
//! |  4 | incr      | Atomic increment                      |
//! |  5 | expire    | Set TTL on existing key                |
//! |  6 | hget      | Get hash field value                  |
//! |  7 | hset      | Set hash field value                  |
//! |  8 | hgetall   | Get all hash fields                   |
//! |  9 | lpush     | Push to list head                     |
//! | 10 | lrange    | Get list range                        |
//! | 11 | publish   | Publish to pub/sub channel            |
//! | 12 | subscribe | Subscribe (stub, streaming required)  |
//! | 13 | keys      | Find keys matching glob pattern       |

use soma_plugin_sdk::prelude::*;

use redis::AsyncCommands;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

/// The SOMA Redis plugin.
///
/// Wraps an async `ConnectionManager` behind a `Mutex` so it can be shared
/// across the sync `SomaPlugin` trait boundary. The connection is established
/// in `on_load()` and dropped in `on_unload()`.
pub struct RedisPlugin {
    /// Async connection manager -- initialized in `on_load()`.
    conn: Mutex<Option<redis::aio::ConnectionManager>>,
    /// Redis URL used for connection.
    url: Mutex<String>,
}

impl Default for RedisPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl RedisPlugin {
    /// Create a new `RedisPlugin` with default connection URL (`redis://localhost:6379/0`).
    ///
    /// The plugin is not connected until `on_load()` is called.
    pub fn new() -> Self {
        Self {
            conn: Mutex::new(None),
            url: Mutex::new("redis://localhost:6379/0".to_string()),
        }
    }

    /// Clone the connection manager, or return an error if not yet connected.
    fn get_conn(&self) -> Result<redis::aio::ConnectionManager, PluginError> {
        let guard = self.conn.lock().map_err(|e| {
            PluginError::Failed(format!("lock poisoned: {e}"))
        })?;
        guard.clone().ok_or_else(|| {
            PluginError::ConnectionRefused("Redis not connected -- call on_load first".into())
        })
    }
}

#[allow(clippy::unnecessary_literal_bound)]
impl SomaPlugin for RedisPlugin {
    fn name(&self) -> &str {
        "redis"
    }

    fn version(&self) -> &str {
        "0.1.0"
    }

    fn description(&self) -> &str {
        "Redis key-value store: strings, hashes, lists, pub/sub"
    }

    fn trust_level(&self) -> TrustLevel {
        TrustLevel::Community
    }

    fn permissions(&self) -> PluginPermissions {
        PluginPermissions {
            network: vec!["redis://*".into()],
            ..Default::default()
        }
    }

    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "properties": {
                "url": {
                    "type": "string",
                    "description": "Redis connection URL",
                    "default": "redis://localhost:6379/0"
                }
            }
        }))
    }

    // === Lifecycle ===

    fn on_load(&mut self, config: &PluginConfig) -> Result<(), PluginError> {
        let url = config
            .get_str("url")
            .unwrap_or("redis://localhost:6379/0")
            .to_string();

        (*self.url.lock().map_err(|e| {
            PluginError::Failed(format!("lock poisoned: {e}"))
        })?).clone_from(&url);

        // Build a single-threaded Tokio runtime for the blocking on_load call
        // to establish the initial connection.
        let rt = tokio::runtime::Runtime::new().map_err(|e| {
            PluginError::Failed(format!("failed to create tokio runtime: {e}"))
        })?;

        let client = redis::Client::open(url.as_str()).map_err(|e| {
            PluginError::ConnectionRefused(format!("invalid Redis URL: {e}"))
        })?;

        let mgr = rt.block_on(async {
            redis::aio::ConnectionManager::new(client).await
        }).map_err(|e| {
            PluginError::ConnectionRefused(format!("Redis connection failed: {e}"))
        })?;

        *self.conn.lock().map_err(|e| {
            PluginError::Failed(format!("lock poisoned: {e}"))
        })? = Some(mgr);

        Ok(())
    }

    fn on_unload(&mut self) -> Result<(), PluginError> {
        // Drop the connection manager.
        *self.conn.lock().map_err(|e| {
            PluginError::Failed(format!("lock poisoned: {e}"))
        })? = None;
        Ok(())
    }

    // === Conventions ===

    #[allow(clippy::too_many_lines)]
    fn conventions(&self) -> Vec<Convention> {
        vec![
            // 0: get
            Convention {
                id: 0,
                name: "get".into(),
                description: "Get string value by key".into(),
                call_pattern: "get(key)".into(),
                args: vec![ArgSpec {
                    name: "key".into(),
                    arg_type: ArgType::String,
                    required: true,
                    description: "Redis key to retrieve".into(),
                }],
                returns: ReturnSpec::Value("String".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 100,
                side_effects: vec![],
                cleanup: None,
            },
            // 1: set
            Convention {
                id: 1,
                name: "set".into(),
                description: "Set string value with optional TTL (seconds)".into(),
                call_pattern: "set(key, value, ttl?)".into(),
                args: vec![
                    ArgSpec {
                        name: "key".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Redis key".into(),
                    },
                    ArgSpec {
                        name: "value".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Value to store".into(),
                    },
                    ArgSpec {
                        name: "ttl".into(),
                        arg_type: ArgType::Int,
                        required: false,
                        description: "TTL in seconds (optional, uses SETEX)".into(),
                    },
                ],
                returns: ReturnSpec::Void,
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 100,
                side_effects: vec![SideEffect("database".into())],
                cleanup: None,
            },
            // 2: delete
            Convention {
                id: 2,
                name: "delete".into(),
                description: "Delete a key, returns whether it existed".into(),
                call_pattern: "delete(key)".into(),
                args: vec![ArgSpec {
                    name: "key".into(),
                    arg_type: ArgType::String,
                    required: true,
                    description: "Redis key to delete".into(),
                }],
                returns: ReturnSpec::Value("Bool".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 100,
                side_effects: vec![SideEffect("database".into())],
                cleanup: None,
            },
            // 3: exists
            Convention {
                id: 3,
                name: "exists".into(),
                description: "Check if a key exists".into(),
                call_pattern: "exists(key)".into(),
                args: vec![ArgSpec {
                    name: "key".into(),
                    arg_type: ArgType::String,
                    required: true,
                    description: "Redis key to check".into(),
                }],
                returns: ReturnSpec::Value("Bool".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 100,
                side_effects: vec![],
                cleanup: None,
            },
            // 4: incr
            Convention {
                id: 4,
                name: "incr".into(),
                description: "Atomic increment, returns new value".into(),
                call_pattern: "incr(key)".into(),
                args: vec![ArgSpec {
                    name: "key".into(),
                    arg_type: ArgType::String,
                    required: true,
                    description: "Redis key to increment".into(),
                }],
                returns: ReturnSpec::Value("Int".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 100,
                side_effects: vec![SideEffect("database".into())],
                cleanup: None,
            },
            // 5: expire
            Convention {
                id: 5,
                name: "expire".into(),
                description: "Set TTL on an existing key".into(),
                call_pattern: "expire(key, seconds)".into(),
                args: vec![
                    ArgSpec {
                        name: "key".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Redis key".into(),
                    },
                    ArgSpec {
                        name: "seconds".into(),
                        arg_type: ArgType::Int,
                        required: true,
                        description: "TTL in seconds".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bool".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 100,
                side_effects: vec![SideEffect("database".into())],
                cleanup: None,
            },
            // 6: hget
            Convention {
                id: 6,
                name: "hget".into(),
                description: "Get a hash field value".into(),
                call_pattern: "hget(key, field)".into(),
                args: vec![
                    ArgSpec {
                        name: "key".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Redis hash key".into(),
                    },
                    ArgSpec {
                        name: "field".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Hash field name".into(),
                    },
                ],
                returns: ReturnSpec::Value("String".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 100,
                side_effects: vec![],
                cleanup: None,
            },
            // 7: hset
            Convention {
                id: 7,
                name: "hset".into(),
                description: "Set a hash field value".into(),
                call_pattern: "hset(key, field, value)".into(),
                args: vec![
                    ArgSpec {
                        name: "key".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Redis hash key".into(),
                    },
                    ArgSpec {
                        name: "field".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Hash field name".into(),
                    },
                    ArgSpec {
                        name: "value".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Value to set".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bool".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 100,
                side_effects: vec![SideEffect("database".into())],
                cleanup: None,
            },
            // 8: hgetall
            Convention {
                id: 8,
                name: "hgetall".into(),
                description: "Get all fields and values of a hash".into(),
                call_pattern: "hgetall(key)".into(),
                args: vec![ArgSpec {
                    name: "key".into(),
                    arg_type: ArgType::String,
                    required: true,
                    description: "Redis hash key".into(),
                }],
                returns: ReturnSpec::Value("Map".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 100,
                side_effects: vec![],
                cleanup: None,
            },
            // 9: lpush
            Convention {
                id: 9,
                name: "lpush".into(),
                description: "Push value to head of list, returns new length".into(),
                call_pattern: "lpush(key, value)".into(),
                args: vec![
                    ArgSpec {
                        name: "key".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Redis list key".into(),
                    },
                    ArgSpec {
                        name: "value".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Value to push".into(),
                    },
                ],
                returns: ReturnSpec::Value("Int".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 100,
                side_effects: vec![SideEffect("database".into())],
                cleanup: None,
            },
            // 10: lrange
            Convention {
                id: 10,
                name: "lrange".into(),
                description: "Get a range of elements from a list".into(),
                call_pattern: "lrange(key, start, stop)".into(),
                args: vec![
                    ArgSpec {
                        name: "key".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Redis list key".into(),
                    },
                    ArgSpec {
                        name: "start".into(),
                        arg_type: ArgType::Int,
                        required: true,
                        description: "Start index (0-based, negative from end)".into(),
                    },
                    ArgSpec {
                        name: "stop".into(),
                        arg_type: ArgType::Int,
                        required: true,
                        description: "Stop index (inclusive, negative from end)".into(),
                    },
                ],
                returns: ReturnSpec::Value("List".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 100,
                side_effects: vec![],
                cleanup: None,
            },
            // 11: publish
            Convention {
                id: 11,
                name: "publish".into(),
                description: "Publish a message to a channel, returns receiver count".into(),
                call_pattern: "publish(channel, message)".into(),
                args: vec![
                    ArgSpec {
                        name: "channel".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Pub/sub channel name".into(),
                    },
                    ArgSpec {
                        name: "message".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Message to publish".into(),
                    },
                ],
                returns: ReturnSpec::Value("Int".into()),
                is_deterministic: false,
                estimated_latency_ms: 1,
                max_latency_ms: 100,
                side_effects: vec![SideEffect("database".into())],
                cleanup: None,
            },
            // 12: subscribe
            Convention {
                id: 12,
                name: "subscribe".into(),
                description: "Subscribe to a channel (streaming -- not yet supported)".into(),
                call_pattern: "subscribe(channel)".into(),
                args: vec![ArgSpec {
                    name: "channel".into(),
                    arg_type: ArgType::String,
                    required: true,
                    description: "Pub/sub channel name".into(),
                }],
                returns: ReturnSpec::Stream("String".into()),
                is_deterministic: false,
                estimated_latency_ms: 0,
                max_latency_ms: 0,
                side_effects: vec![],
                cleanup: None,
            },
            // 13: keys
            Convention {
                id: 13,
                name: "keys".into(),
                description: "Find keys matching a glob pattern".into(),
                call_pattern: "keys(pattern)".into(),
                args: vec![ArgSpec {
                    name: "pattern".into(),
                    arg_type: ArgType::String,
                    required: true,
                    description: "Glob pattern (e.g. 'user:*')".into(),
                }],
                returns: ReturnSpec::Value("List".into()),
                is_deterministic: false,
                estimated_latency_ms: 5,
                max_latency_ms: 500,
                side_effects: vec![],
                cleanup: None,
            },
        ]
    }

    // === Sync execution -- bridges to async via current Tokio handle ===

    fn execute(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        // If we are already inside a Tokio runtime, use block_on from the
        // current handle. Otherwise, create a temporary runtime.
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            // We are inside an async runtime -- use block_in_place + block_on
            // so we don't deadlock a single-threaded runtime.
            tokio::task::block_in_place(|| {
                handle.block_on(self.execute_async_inner(convention_id, args))
            })
        } else {
            // No runtime -- build a temporary one.
            let rt = tokio::runtime::Runtime::new().map_err(|e| {
                PluginError::Failed(format!("failed to create tokio runtime: {e}"))
            })?;
            rt.block_on(self.execute_async_inner(convention_id, args))
        }
    }

    // === Async execution -- the primary implementation ===

    fn execute_async(
        &self,
        convention_id: u32,
        args: Vec<Value>,
    ) -> Pin<Box<dyn Future<Output = Result<Value, PluginError>> + Send + '_>> {
        Box::pin(self.execute_async_inner(convention_id, args))
    }
}

impl RedisPlugin {
    /// Core async implementation for all 14 conventions.
    ///
    /// Dispatches on `convention_id` (0..=13) to the corresponding Redis command.
    /// Each branch extracts arguments from the `args` vector, executes the command
    /// via the async connection manager, and maps the result into a `Value`.
    #[allow(clippy::too_many_lines)]
    async fn execute_async_inner(
        &self,
        convention_id: u32,
        args: Vec<Value>,
    ) -> Result<Value, PluginError> {
        let mut conn = self.get_conn()?;

        match convention_id {
            // 0: get(key) -> String | Null
            0 => {
                let key = args.first()
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: key".into()))?
                    .as_str()?;

                let result: Option<String> = conn.get(key).await.map_err(|e| {
                    PluginError::Failed(format!("Redis GET failed: {e}"))
                })?;

                Ok(result.map_or(Value::Null, Value::String))
            }

            // 1: set(key, value, ttl?) -> Null
            1 => {
                let key = args.first()
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: key".into()))?
                    .as_str()?;
                let value = args.get(1)
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: value".into()))?
                    .as_str()?;

                // Check for optional TTL (third argument).
                if let Some(ttl_val) = args.get(2) {
                    // Allow Null to mean "no TTL"
                    if matches!(ttl_val, Value::Null) {
                        let _: () = conn.set(key, value).await.map_err(|e| {
                            PluginError::Failed(format!("Redis SET failed: {e}"))
                        })?;
                    } else {
                        let ttl = ttl_val.as_int()?;
                        if ttl <= 0 {
                            return Err(PluginError::InvalidArg(
                                "TTL must be positive".into(),
                            ));
                        }
                        let _: () = redis::cmd("SETEX")
                            .arg(key)
                            .arg(ttl)
                            .arg(value)
                            .query_async(&mut conn)
                            .await
                            .map_err(|e| {
                                PluginError::Failed(format!("Redis SETEX failed: {e}"))
                            })?;
                    }
                } else {
                    let _: () = conn.set(key, value).await.map_err(|e| {
                        PluginError::Failed(format!("Redis SET failed: {e}"))
                    })?;
                }

                Ok(Value::Null)
            }

            // 2: delete(key) -> Bool
            2 => {
                let key = args.first()
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: key".into()))?
                    .as_str()?;

                let deleted: i64 = conn.del(key).await.map_err(|e| {
                    PluginError::Failed(format!("Redis DEL failed: {e}"))
                })?;

                Ok(Value::Bool(deleted > 0))
            }

            // 3: exists(key) -> Bool
            3 => {
                let key = args.first()
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: key".into()))?
                    .as_str()?;

                let exists: bool = conn.exists(key).await.map_err(|e| {
                    PluginError::Failed(format!("Redis EXISTS failed: {e}"))
                })?;

                Ok(Value::Bool(exists))
            }

            // 4: incr(key) -> Int
            4 => {
                let key = args.first()
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: key".into()))?
                    .as_str()?;

                let new_val: i64 = conn.incr(key, 1i64).await.map_err(|e| {
                    PluginError::Failed(format!("Redis INCR failed: {e}"))
                })?;

                Ok(Value::Int(new_val))
            }

            // 5: expire(key, seconds) -> Bool
            5 => {
                let key = args.first()
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: key".into()))?
                    .as_str()?;
                let seconds = args.get(1)
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: seconds".into()))?
                    .as_int()?;

                let set: bool = conn.expire(key, seconds).await.map_err(|e| {
                    PluginError::Failed(format!("Redis EXPIRE failed: {e}"))
                })?;

                Ok(Value::Bool(set))
            }

            // 6: hget(key, field) -> String | Null
            6 => {
                let key = args.first()
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: key".into()))?
                    .as_str()?;
                let field = args.get(1)
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: field".into()))?
                    .as_str()?;

                let result: Option<String> = conn.hget(key, field).await.map_err(|e| {
                    PluginError::Failed(format!("Redis HGET failed: {e}"))
                })?;

                Ok(result.map_or(Value::Null, Value::String))
            }

            // 7: hset(key, field, value) -> Bool
            7 => {
                let key = args.first()
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: key".into()))?
                    .as_str()?;
                let field = args.get(1)
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: field".into()))?
                    .as_str()?;
                let value = args.get(2)
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: value".into()))?
                    .as_str()?;

                // HSET returns the number of new fields added (1 if new, 0 if updated).
                let added: i64 = conn.hset(key, field, value).await.map_err(|e| {
                    PluginError::Failed(format!("Redis HSET failed: {e}"))
                })?;

                Ok(Value::Bool(added > 0))
            }

            // 8: hgetall(key) -> Map<String, String>
            8 => {
                let key = args.first()
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: key".into()))?
                    .as_str()?;

                let result: HashMap<String, String> =
                    conn.hgetall(key).await.map_err(|e| {
                        PluginError::Failed(format!("Redis HGETALL failed: {e}"))
                    })?;

                let map: HashMap<String, Value> = result
                    .into_iter()
                    .map(|(k, v)| (k, Value::String(v)))
                    .collect();

                Ok(Value::Map(map))
            }

            // 9: lpush(key, value) -> Int (new list length)
            9 => {
                let key = args.first()
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: key".into()))?
                    .as_str()?;
                let value = args.get(1)
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: value".into()))?
                    .as_str()?;

                let len: i64 = conn.lpush(key, value).await.map_err(|e| {
                    PluginError::Failed(format!("Redis LPUSH failed: {e}"))
                })?;

                Ok(Value::Int(len))
            }

            // 10: lrange(key, start, stop) -> List<String>
            #[allow(clippy::cast_possible_truncation)]
            10 => {
                let key = args.first()
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: key".into()))?
                    .as_str()?;
                let start = args.get(1)
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: start".into()))?
                    .as_int()? as isize;
                let stop = args.get(2)
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: stop".into()))?
                    .as_int()? as isize;

                let result: Vec<String> =
                    conn.lrange(key, start, stop).await.map_err(|e| {
                        PluginError::Failed(format!("Redis LRANGE failed: {e}"))
                    })?;

                let list: Vec<Value> = result
                    .into_iter()
                    .map(Value::String)
                    .collect();

                Ok(Value::List(list))
            }

            // 11: publish(channel, message) -> Int (receivers count)
            11 => {
                let channel = args.first()
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: channel".into()))?
                    .as_str()?;
                let message = args.get(1)
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: message".into()))?
                    .as_str()?;

                let receivers: i64 = conn.publish(channel, message).await.map_err(|e| {
                    PluginError::Failed(format!("Redis PUBLISH failed: {e}"))
                })?;

                Ok(Value::Int(receivers))
            }

            // 12: subscribe -- not supported in MVP
            12 => {
                Err(PluginError::Failed(
                    "subscribe requires streaming, use publish/get pattern instead".into(),
                ))
            }

            // 13: keys(pattern) -> List<String>
            13 => {
                let pattern = args.first()
                    .ok_or_else(|| PluginError::InvalidArg("missing argument: pattern".into()))?
                    .as_str()?;

                let result: Vec<String> = conn.keys(pattern).await.map_err(|e| {
                    PluginError::Failed(format!("Redis KEYS failed: {e}"))
                })?;

                let list: Vec<Value> = result
                    .into_iter()
                    .map(Value::String)
                    .collect();

                Ok(Value::List(list))
            }

            _ => Err(PluginError::NotFound(format!(
                "unknown convention id: {convention_id}"
            ))),
        }
    }
}

// === Plugin export (C ABI) ===

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_plugin_init() -> *mut dyn SomaPlugin {
    Box::into_raw(Box::new(RedisPlugin::new()))
}
