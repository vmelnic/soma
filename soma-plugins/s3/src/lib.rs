//! SOMA S3 Plugin -- object storage conventions for S3-compatible backends.
//!
//! Five conventions:
//!
//! | ID | Name           | Description                                             |
//! |----|----------------|---------------------------------------------------------|
//! | 0  | `put_object`   | Upload an object to an S3 bucket                        |
//! | 1  | `get_object`   | Download an object from an S3 bucket                    |
//! | 2  | `delete_object` | Delete an object from an S3 bucket                     |
//! | 3  | `presign_url`  | Generate a presigned URL for temporary access            |
//! | 4  | `list_objects`  | List objects in a bucket with optional prefix filter    |
//!
//! # Why `tokio::runtime::Runtime::block_on()`?
//!
//! The `SomaPlugin` trait is synchronous, but the AWS SDK for Rust (`aws-sdk-s3`)
//! is async.  This plugin creates a dedicated tokio runtime at `on_load()` time
//! and uses `block_on()` to bridge async calls into the sync trait.  This is the
//! same strategy used when bridging any async SDK into SOMA's sync plugin model.
//!
//! # Configuration
//!
//! Settings from `[plugins.s3]` in `soma.toml`:
//!
//! | Key              | Type   | Default                     | Description                        |
//! |------------------|--------|-----------------------------|------------------------------------|
//! | `region`         | string | `"eu-central-1"`            | AWS region                         |
//! | `endpoint`       | string | `"https://s3.amazonaws.com"` | S3 endpoint (MinIO, Ceph, etc.)   |
//! | `access_key_env` | string | `"AWS_ACCESS_KEY_ID"`       | Env var name for access key        |
//! | `secret_key_env` | string | `"AWS_SECRET_ACCESS_KEY"`   | Env var name for secret key        |
//! | `default_bucket` | string | `"soma-uploads"`            | Bucket used when none specified    |

use soma_plugin_sdk::prelude::*;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use aws_sdk_s3::Client as S3Client;

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

/// The SOMA S3 plugin.
///
/// Holds a lazily-initialized S3 client and tokio runtime, set during
/// [`SomaPlugin::on_load`].  Each convention call uses `block_on()` to
/// bridge async AWS SDK operations into the sync trait.
pub struct S3Plugin {
    /// The AWS S3 client, initialized in `on_load`.
    client: OnceLock<S3Client>,
    /// A dedicated tokio runtime for running async AWS SDK calls.
    runtime: OnceLock<tokio::runtime::Runtime>,
    /// Default bucket from config, used when a convention omits the bucket arg.
    default_bucket: OnceLock<String>,
    /// Endpoint URL for constructing presigned URL fallbacks.
    endpoint: OnceLock<String>,
}

impl Default for S3Plugin {
    fn default() -> Self {
        Self::new()
    }
}

impl S3Plugin {
    /// Create a new unconfigured plugin instance.
    ///
    /// The S3 client is not initialized until [`SomaPlugin::on_load`] is called
    /// by the plugin manager with the appropriate `PluginConfig`.
    pub const fn new() -> Self {
        Self {
            client: OnceLock::new(),
            runtime: OnceLock::new(),
            default_bucket: OnceLock::new(),
            endpoint: OnceLock::new(),
        }
    }

    /// Get the S3 client, returning an error if not yet configured.
    fn client(&self) -> Result<&S3Client, PluginError> {
        self.client
            .get()
            .ok_or_else(|| PluginError::Failed("S3 not configured -- call on_load first".into()))
    }

    /// Get the tokio runtime, returning an error if not yet configured.
    fn runtime(&self) -> Result<&tokio::runtime::Runtime, PluginError> {
        self.runtime
            .get()
            .ok_or_else(|| PluginError::Failed("S3 runtime not initialized".into()))
    }

    /// Resolve the bucket name: use the provided arg, or fall back to the default.
    fn resolve_bucket(&self, bucket_arg: &str) -> String {
        if bucket_arg.is_empty() {
            self.default_bucket
                .get()
                .cloned()
                .unwrap_or_else(|| "soma-uploads".to_string())
        } else {
            bucket_arg.to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// SomaPlugin implementation
// ---------------------------------------------------------------------------

#[allow(clippy::unnecessary_literal_bound)]
impl SomaPlugin for S3Plugin {
    fn name(&self) -> &str {
        "s3"
    }

    fn version(&self) -> &str {
        "0.1.0"
    }

    fn description(&self) -> &str {
        "S3-compatible object storage: upload, download, delete, presign, list"
    }

    fn trust_level(&self) -> TrustLevel {
        TrustLevel::Community
    }

    #[allow(clippy::too_many_lines)]
    fn conventions(&self) -> Vec<Convention> {
        vec![
            // 0: put_object
            Convention {
                id: 0,
                name: "put_object".into(),
                description: "Upload an object to an S3 bucket".into(),
                call_pattern: "put_object(bucket, key, data, content_type)".into(),
                args: vec![
                    ArgSpec {
                        name: "bucket".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "S3 bucket name".into(),
                    },
                    ArgSpec {
                        name: "key".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Object key (path within the bucket)".into(),
                    },
                    ArgSpec {
                        name: "data".into(),
                        arg_type: ArgType::Bytes,
                        required: true,
                        description: "Binary data to upload".into(),
                    },
                    ArgSpec {
                        name: "content_type".into(),
                        arg_type: ArgType::String,
                        required: false,
                        description: "MIME content type (default: application/octet-stream)"
                            .into(),
                    },
                ],
                returns: ReturnSpec::Value("String".into()),
                is_deterministic: false,
                estimated_latency_ms: 200,
                max_latency_ms: 30_000,
                side_effects: vec![SideEffect("writes object storage".into())],
                cleanup: None,
            },
            // 1: get_object
            Convention {
                id: 1,
                name: "get_object".into(),
                description: "Download an object from an S3 bucket".into(),
                call_pattern: "get_object(bucket, key)".into(),
                args: vec![
                    ArgSpec {
                        name: "bucket".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "S3 bucket name".into(),
                    },
                    ArgSpec {
                        name: "key".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Object key (path within the bucket)".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bytes".into()),
                is_deterministic: false,
                estimated_latency_ms: 150,
                max_latency_ms: 30_000,
                side_effects: vec![SideEffect("reads object storage".into())],
                cleanup: None,
            },
            // 2: delete_object
            Convention {
                id: 2,
                name: "delete_object".into(),
                description: "Delete an object from an S3 bucket".into(),
                call_pattern: "delete_object(bucket, key)".into(),
                args: vec![
                    ArgSpec {
                        name: "bucket".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "S3 bucket name".into(),
                    },
                    ArgSpec {
                        name: "key".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Object key (path within the bucket)".into(),
                    },
                ],
                returns: ReturnSpec::Value("Bool".into()),
                is_deterministic: false,
                estimated_latency_ms: 100,
                max_latency_ms: 10_000,
                side_effects: vec![SideEffect("deletes from object storage".into())],
                cleanup: None,
            },
            // 3: presign_url
            Convention {
                id: 3,
                name: "presign_url".into(),
                description: "Generate a presigned URL for temporary access to an object".into(),
                call_pattern: "presign_url(bucket, key, expires_secs)".into(),
                args: vec![
                    ArgSpec {
                        name: "bucket".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "S3 bucket name".into(),
                    },
                    ArgSpec {
                        name: "key".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Object key (path within the bucket)".into(),
                    },
                    ArgSpec {
                        name: "expires_secs".into(),
                        arg_type: ArgType::Int,
                        required: false,
                        description: "URL expiration in seconds (default: 3600)".into(),
                    },
                ],
                returns: ReturnSpec::Value("String".into()),
                is_deterministic: false,
                estimated_latency_ms: 10,
                max_latency_ms: 5_000,
                side_effects: vec![],
                cleanup: None,
            },
            // 4: list_objects
            Convention {
                id: 4,
                name: "list_objects".into(),
                description: "List objects in an S3 bucket with optional prefix filter".into(),
                call_pattern: "list_objects(bucket, prefix)".into(),
                args: vec![
                    ArgSpec {
                        name: "bucket".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "S3 bucket name".into(),
                    },
                    ArgSpec {
                        name: "prefix".into(),
                        arg_type: ArgType::String,
                        required: false,
                        description: "Key prefix to filter by (e.g. \"uploads/\")".into(),
                    },
                ],
                returns: ReturnSpec::Value("List".into()),
                is_deterministic: false,
                estimated_latency_ms: 100,
                max_latency_ms: 15_000,
                side_effects: vec![SideEffect("reads object storage".into())],
                cleanup: None,
            },
        ]
    }

    fn execute(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        match convention_id {
            0 => self.put_object(&args),
            1 => self.get_object(&args),
            2 => self.delete_object(&args),
            3 => self.presign_url(&args),
            4 => self.list_objects(&args),
            _ => Err(PluginError::NotFound(format!(
                "unknown convention_id: {convention_id}"
            ))),
        }
    }

    fn on_load(&mut self, config: &PluginConfig) -> Result<(), PluginError> {
        let region = config
            .get_str("region")
            .unwrap_or("eu-central-1")
            .to_string();

        let endpoint = config
            .get_str("endpoint")
            .unwrap_or("https://s3.amazonaws.com")
            .to_string();

        let access_key_env = config
            .get_str("access_key_env")
            .unwrap_or("AWS_ACCESS_KEY_ID");

        let secret_key_env = config
            .get_str("secret_key_env")
            .unwrap_or("AWS_SECRET_ACCESS_KEY");

        let default_bucket = config
            .get_str("default_bucket")
            .unwrap_or("soma-uploads")
            .to_string();

        // Read credentials from the named environment variables.
        let access_key = std::env::var(access_key_env).map_err(|_| {
            PluginError::Failed(format!(
                "S3 access key env var '{access_key_env}' not set"
            ))
        })?;

        let secret_key = std::env::var(secret_key_env).map_err(|_| {
            PluginError::Failed(format!(
                "S3 secret key env var '{secret_key_env}' not set"
            ))
        })?;

        // Build a dedicated tokio runtime for async AWS SDK calls.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| PluginError::Failed(format!("failed to create tokio runtime: {e}")))?;

        // Build the AWS S3 client inside the runtime context.
        let client = runtime.block_on(async {
            let creds =
                aws_sdk_s3::config::Credentials::new(&access_key, &secret_key, None, None, "soma");

            let config = aws_sdk_s3::config::Builder::new()
                .region(aws_sdk_s3::config::Region::new(region))
                .endpoint_url(&endpoint)
                .credentials_provider(creds)
                .force_path_style(true)
                .build();

            S3Client::from_conf(config)
        });

        let _ = self.client.set(client);
        let _ = self.runtime.set(runtime);
        let _ = self.default_bucket.set(default_bucket);
        let _ = self.endpoint.set(endpoint);

        Ok(())
    }

    fn on_unload(&mut self) -> Result<(), PluginError> {
        // The tokio runtime and S3 client are dropped when the plugin is dropped.
        // No explicit cleanup needed.
        Ok(())
    }

    fn permissions(&self) -> PluginPermissions {
        PluginPermissions {
            filesystem: vec![],
            network: vec!["https:*:443".into(), "http:*:*".into()],
            env_vars: vec![
                "AWS_ACCESS_KEY_ID".into(),
                "AWS_SECRET_ACCESS_KEY".into(),
            ],
            process_spawn: false,
        }
    }

    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "region":         {"type": "string", "default": "eu-central-1"},
                "endpoint":       {"type": "string", "default": "https://s3.amazonaws.com"},
                "access_key_env": {"type": "string", "default": "AWS_ACCESS_KEY_ID"},
                "secret_key_env": {"type": "string", "default": "AWS_SECRET_ACCESS_KEY"},
                "default_bucket": {"type": "string", "default": "soma-uploads"}
            }
        }))
    }
}

// ---------------------------------------------------------------------------
// Convention implementations
// ---------------------------------------------------------------------------

impl S3Plugin {
    /// Convention 0 -- Upload an object to S3.
    ///
    /// Args: bucket (String), key (String), data (Bytes), content_type (String, optional).
    /// Returns: the object key as confirmation.
    fn put_object(&self, args: &[Value]) -> Result<Value, PluginError> {
        let bucket_arg = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: bucket".into()))?
            .as_str()?;
        let bucket = self.resolve_bucket(bucket_arg);

        let key = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: key".into()))?
            .as_str()?;

        let data = args
            .get(2)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: data".into()))?
            .as_bytes()?;

        let content_type = args
            .get(3)
            .and_then(|v| v.as_str().ok())
            .unwrap_or("application/octet-stream");

        let client = self.client()?;
        let rt = self.runtime()?;

        let body = aws_sdk_s3::primitives::ByteStream::from(data.to_vec());

        rt.block_on(async {
            client
                .put_object()
                .bucket(&bucket)
                .key(key)
                .body(body)
                .content_type(content_type)
                .send()
                .await
                .map_err(|e| PluginError::Failed(format!("S3 PutObject failed: {e}")))?;

            Ok(Value::String(format!("s3://{bucket}/{key}")))
        })
    }

    /// Convention 1 -- Download an object from S3.
    ///
    /// Args: bucket (String), key (String).
    /// Returns: the object data as Bytes.
    fn get_object(&self, args: &[Value]) -> Result<Value, PluginError> {
        let bucket_arg = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: bucket".into()))?
            .as_str()?;
        let bucket = self.resolve_bucket(bucket_arg);

        let key = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: key".into()))?
            .as_str()?;

        let client = self.client()?;
        let rt = self.runtime()?;

        rt.block_on(async {
            let resp = client
                .get_object()
                .bucket(&bucket)
                .key(key)
                .send()
                .await
                .map_err(|e| {
                    let msg = format!("{e}");
                    if msg.contains("NoSuchKey") || msg.contains("not found") {
                        PluginError::NotFound(format!("s3://{bucket}/{key}"))
                    } else {
                        PluginError::Failed(format!("S3 GetObject failed: {e}"))
                    }
                })?;

            let bytes = resp
                .body
                .collect()
                .await
                .map_err(|e| PluginError::Failed(format!("S3 GetObject body read failed: {e}")))?
                .into_bytes();

            Ok(Value::Bytes(bytes.to_vec()))
        })
    }

    /// Convention 2 -- Delete an object from S3.
    ///
    /// Args: bucket (String), key (String).
    /// Returns: Bool(true) on success.
    fn delete_object(&self, args: &[Value]) -> Result<Value, PluginError> {
        let bucket_arg = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: bucket".into()))?
            .as_str()?;
        let bucket = self.resolve_bucket(bucket_arg);

        let key = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: key".into()))?
            .as_str()?;

        let client = self.client()?;
        let rt = self.runtime()?;

        rt.block_on(async {
            client
                .delete_object()
                .bucket(&bucket)
                .key(key)
                .send()
                .await
                .map_err(|e| PluginError::Failed(format!("S3 DeleteObject failed: {e}")))?;

            Ok(Value::Bool(true))
        })
    }

    /// Convention 3 -- Generate a presigned URL for temporary access.
    ///
    /// Args: bucket (String), key (String), expires_secs (Int, optional, default 3600).
    /// Returns: the presigned URL as a String.
    ///
    /// Uses the `aws-sdk-s3` presigning support.  If presigning fails (e.g. the
    /// endpoint does not support it), falls back to constructing a direct URL.
    fn presign_url(&self, args: &[Value]) -> Result<Value, PluginError> {
        let bucket_arg = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: bucket".into()))?
            .as_str()?;
        let bucket = self.resolve_bucket(bucket_arg);

        let key = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing argument: key".into()))?
            .as_str()?;

        #[allow(clippy::cast_sign_loss)]
        let expires_secs = args
            .get(2)
            .and_then(|v| v.as_int().ok())
            .unwrap_or(3600) as u64;

        let client = self.client()?;
        let rt = self.runtime()?;

        rt.block_on(async {
            let presign_config = aws_sdk_s3::presigning::PresigningConfig::builder()
                .expires_in(Duration::from_secs(expires_secs))
                .build()
                .map_err(|e| {
                    PluginError::Failed(format!("S3 presign config error: {e}"))
                })?;

            let presigned = client
                .get_object()
                .bucket(&bucket)
                .key(key)
                .presigned(presign_config)
                .await
                .map_err(|e| PluginError::Failed(format!("S3 presign failed: {e}")))?;

            Ok(Value::String(presigned.uri().to_string()))
        })
    }

    /// Convention 4 -- List objects in an S3 bucket.
    ///
    /// Args: bucket (String), prefix (String, optional).
    /// Returns: a List of Maps, each with `key`, `size`, `last_modified`.
    fn list_objects(&self, args: &[Value]) -> Result<Value, PluginError> {
        let bucket_arg = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing argument: bucket".into()))?
            .as_str()?;
        let bucket = self.resolve_bucket(bucket_arg);

        let prefix = args
            .get(1)
            .and_then(|v| v.as_str().ok())
            .unwrap_or("");

        let client = self.client()?;
        let rt = self.runtime()?;

        rt.block_on(async {
            let mut request = client.list_objects_v2().bucket(&bucket);

            if !prefix.is_empty() {
                request = request.prefix(prefix);
            }

            let resp = request
                .send()
                .await
                .map_err(|e| PluginError::Failed(format!("S3 ListObjectsV2 failed: {e}")))?;

            let objects: Vec<Value> = resp
                .contents()
                .iter()
                .map(|obj| {
                    let mut map = HashMap::new();

                    map.insert(
                        "key".into(),
                        Value::String(obj.key().unwrap_or("").to_string()),
                    );
                    map.insert(
                        "size".into(),
                        Value::Int(obj.size().unwrap_or(0)),
                    );
                    if let Some(modified) = obj.last_modified() {
                        map.insert(
                            "last_modified".into(),
                            Value::String(modified.to_string()),
                        );
                    }
                    if let Some(etag) = obj.e_tag() {
                        map.insert("etag".into(), Value::String(etag.to_string()));
                    }

                    Value::Map(map)
                })
                .collect();

            Ok(Value::List(objects))
        })
    }
}

// ---------------------------------------------------------------------------
// C ABI entry point
// ---------------------------------------------------------------------------

/// Create a heap-allocated `S3Plugin` and return a raw pointer for dynamic loading.
///
/// Called by the SOMA runtime's `libloading`-based plugin loader.  The runtime
/// takes ownership of the pointer and drops it on unload.
#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_plugin_init() -> *mut dyn SomaPlugin {
    Box::into_raw(Box::new(S3Plugin::new()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_name() {
        let plugin = S3Plugin::new();
        assert_eq!(plugin.name(), "s3");
    }

    #[test]
    fn test_plugin_version() {
        let plugin = S3Plugin::new();
        assert_eq!(plugin.version(), "0.1.0");
    }

    #[test]
    fn test_conventions_count() {
        let plugin = S3Plugin::new();
        assert_eq!(plugin.conventions().len(), 5);
    }

    #[test]
    fn test_convention_names() {
        let plugin = S3Plugin::new();
        let conventions = plugin.conventions();
        let names: Vec<&str> = conventions.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["put_object", "get_object", "delete_object", "presign_url", "list_objects"]
        );
    }

    #[test]
    fn test_convention_ids_sequential() {
        let plugin = S3Plugin::new();
        let conventions = plugin.conventions();
        for (i, c) in conventions.iter().enumerate() {
            assert_eq!(c.id as usize, i, "convention '{}' has unexpected id", c.name);
        }
    }

    #[test]
    fn test_execute_without_init_put_object() {
        let plugin = S3Plugin::new();
        let result = plugin.execute(
            0,
            vec![
                Value::String("test-bucket".into()),
                Value::String("test-key".into()),
                Value::Bytes(vec![1, 2, 3]),
            ],
        );
        assert!(result.is_err(), "expected error when S3 client not initialized");
        let err = result.unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("not configured") || msg.contains("not initialized"),
            "error should mention not configured/initialized, got: {msg}"
        );
    }

    #[test]
    fn test_execute_without_init_get_object() {
        let plugin = S3Plugin::new();
        let result = plugin.execute(
            1,
            vec![
                Value::String("test-bucket".into()),
                Value::String("test-key".into()),
            ],
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_execute_without_init_list_objects() {
        let plugin = S3Plugin::new();
        let result = plugin.execute(
            4,
            vec![Value::String("test-bucket".into())],
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_unknown_convention() {
        let plugin = S3Plugin::new();
        let result = plugin.execute(99, vec![]);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_bucket_empty_uses_default() {
        let plugin = S3Plugin::new();
        let resolved = plugin.resolve_bucket("");
        assert_eq!(resolved, "soma-uploads");
    }

    #[test]
    fn test_resolve_bucket_explicit() {
        let plugin = S3Plugin::new();
        let resolved = plugin.resolve_bucket("my-bucket");
        assert_eq!(resolved, "my-bucket");
    }

    #[test]
    fn test_trust_level() {
        let plugin = S3Plugin::new();
        assert_eq!(plugin.trust_level(), TrustLevel::Community);
    }

    #[test]
    fn test_permissions_network() {
        let plugin = S3Plugin::new();
        let perms = plugin.permissions();
        assert!(!perms.network.is_empty(), "S3 plugin should declare network permissions");
    }

    #[test]
    fn test_config_schema_present() {
        let plugin = S3Plugin::new();
        let schema = plugin.config_schema();
        assert!(schema.is_some());
    }
}
