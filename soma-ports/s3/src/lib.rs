//! SOMA S3 Port -- object storage capabilities for S3-compatible backends.
//!
//! Five capabilities:
//!
//! | ID | Name            | Description                                             |
//! |----|-----------------|---------------------------------------------------------|
//! | 0  | `put_object`    | Upload an object to an S3 bucket                        |
//! | 1  | `get_object`    | Download an object from an S3 bucket                    |
//! | 2  | `delete_object` | Delete an object from an S3 bucket                      |
//! | 3  | `presign_url`   | Generate a presigned URL for temporary access            |
//! | 4  | `list_objects`  | List objects in a bucket with optional prefix filter     |
//!
//! The Port trait is synchronous but the AWS SDK is async. A dedicated tokio
//! runtime is created at construction time and `block_on()` bridges async calls
//! into the sync interface.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use aws_sdk_s3::Client as S3Client;
use soma_port_sdk::prelude::*;

const PORT_ID: &str = "soma.s3";

// ---------------------------------------------------------------------------
// Port struct
// ---------------------------------------------------------------------------

pub struct S3Port {
    spec: PortSpec,
    client: OnceLock<S3Client>,
    runtime: OnceLock<tokio::runtime::Runtime>,
    default_bucket: String,
}

impl S3Port {
    pub fn new() -> Self {
        Self {
            spec: build_spec(),
            client: OnceLock::new(),
            runtime: OnceLock::new(),
            default_bucket: "soma-uploads".into(),
        }
    }

    fn client(&self) -> soma_port_sdk::Result<&S3Client> {
        self.client
            .get()
            .ok_or_else(|| PortError::DependencyUnavailable("S3 client not initialized".into()))
    }

    fn rt(&self) -> soma_port_sdk::Result<&tokio::runtime::Runtime> {
        self.runtime
            .get()
            .ok_or_else(|| PortError::DependencyUnavailable("tokio runtime not initialized".into()))
    }

    fn resolve_bucket(&self, input: &serde_json::Value) -> String {
        input["bucket"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.default_bucket.clone())
    }
}

impl Default for S3Port {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Port trait implementation
// ---------------------------------------------------------------------------

impl Port for S3Port {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(
        &self,
        capability_id: &str,
        input: serde_json::Value,
    ) -> soma_port_sdk::Result<PortCallRecord> {
        let start = Instant::now();
        let result = match capability_id {
            "put_object" => self.put_object(&input),
            "get_object" => self.get_object(&input),
            "delete_object" => self.delete_object(&input),
            "presign_url" => self.presign_url(&input),
            "list_objects" => self.list_objects(&input),
            other => {
                return Err(PortError::Validation(format!(
                    "unknown capability: {other}"
                )));
            }
        };
        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(value) => Ok(PortCallRecord::success(
                PORT_ID,
                capability_id,
                value,
                latency_ms,
            )),
            Err(e) => Ok(PortCallRecord::failure(
                PORT_ID,
                capability_id,
                e.failure_class(),
                &e.to_string(),
                latency_ms,
            )),
        }
    }

    fn validate_input(
        &self,
        capability_id: &str,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<()> {
        match capability_id {
            "put_object" => {
                require_field(input, "key")?;
                require_field(input, "data")?;
            }
            "get_object" | "delete_object" | "presign_url" => {
                require_field(input, "key")?;
            }
            "list_objects" => {}
            other => {
                return Err(PortError::Validation(format!(
                    "unknown capability: {other}"
                )));
            }
        }
        Ok(())
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        if self.client.get().is_some() {
            PortLifecycleState::Active
        } else {
            PortLifecycleState::Loaded
        }
    }
}

// ---------------------------------------------------------------------------
// Capability implementations
// ---------------------------------------------------------------------------

impl S3Port {
    fn put_object(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let bucket = self.resolve_bucket(input);
        let key = get_str(input, "key")?;
        let data = get_str(input, "data")?;
        let content_type = input["content_type"]
            .as_str()
            .unwrap_or("application/octet-stream");

        let client = self.client()?;
        let rt = self.rt()?;

        let body = aws_sdk_s3::primitives::ByteStream::from(data.as_bytes().to_vec());

        rt.block_on(async {
            client
                .put_object()
                .bucket(&bucket)
                .key(key)
                .body(body)
                .content_type(content_type)
                .send()
                .await
                .map_err(|e| PortError::ExternalError(format!("S3 PutObject failed: {e}")))?;

            Ok(serde_json::json!({
                "uri": format!("s3://{bucket}/{key}"),
                "bucket": bucket,
                "key": key,
            }))
        })
    }

    fn get_object(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let bucket = self.resolve_bucket(input);
        let key = get_str(input, "key")?;

        let client = self.client()?;
        let rt = self.rt()?;

        rt.block_on(async {
            let resp = client
                .get_object()
                .bucket(&bucket)
                .key(key)
                .send()
                .await
                .map_err(|e| PortError::ExternalError(format!("S3 GetObject failed: {e}")))?;

            let bytes = resp
                .body
                .collect()
                .await
                .map_err(|e| PortError::ExternalError(format!("S3 body read failed: {e}")))?
                .into_bytes();

            use base64::Engine;
            let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);

            Ok(serde_json::json!({
                "data": encoded,
                "size": bytes.len(),
                "bucket": bucket,
                "key": key,
            }))
        })
    }

    fn delete_object(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let bucket = self.resolve_bucket(input);
        let key = get_str(input, "key")?;

        let client = self.client()?;
        let rt = self.rt()?;

        rt.block_on(async {
            client
                .delete_object()
                .bucket(&bucket)
                .key(key)
                .send()
                .await
                .map_err(|e| PortError::ExternalError(format!("S3 DeleteObject failed: {e}")))?;

            Ok(serde_json::json!({
                "deleted": true,
                "bucket": bucket,
                "key": key,
            }))
        })
    }

    fn presign_url(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let bucket = self.resolve_bucket(input);
        let key = get_str(input, "key")?;
        let expires_secs = input["expires_secs"].as_u64().unwrap_or(3600);

        let client = self.client()?;
        let rt = self.rt()?;

        rt.block_on(async {
            let presign_config = aws_sdk_s3::presigning::PresigningConfig::builder()
                .expires_in(Duration::from_secs(expires_secs))
                .build()
                .map_err(|e| PortError::ExternalError(format!("presign config error: {e}")))?;

            let presigned = client
                .get_object()
                .bucket(&bucket)
                .key(key)
                .presigned(presign_config)
                .await
                .map_err(|e| PortError::ExternalError(format!("S3 presign failed: {e}")))?;

            Ok(serde_json::json!({
                "url": presigned.uri().to_string(),
                "expires_secs": expires_secs,
            }))
        })
    }

    fn list_objects(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let bucket = self.resolve_bucket(input);
        let prefix = input["prefix"].as_str().unwrap_or("");

        let client = self.client()?;
        let rt = self.rt()?;

        rt.block_on(async {
            let mut request = client.list_objects_v2().bucket(&bucket);
            if !prefix.is_empty() {
                request = request.prefix(prefix);
            }

            let resp = request
                .send()
                .await
                .map_err(|e| PortError::ExternalError(format!("S3 ListObjectsV2 failed: {e}")))?;

            let objects: Vec<serde_json::Value> = resp
                .contents()
                .iter()
                .map(|obj| {
                    serde_json::json!({
                        "key": obj.key().unwrap_or(""),
                        "size": obj.size().unwrap_or(0),
                        "last_modified": obj.last_modified().map(|t| t.to_string()),
                        "etag": obj.e_tag().unwrap_or(""),
                    })
                })
                .collect();

            Ok(serde_json::json!({
                "objects": objects,
                "count": objects.len(),
                "bucket": bucket,
            }))
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn require_field(input: &serde_json::Value, field: &str) -> soma_port_sdk::Result<()> {
    if input.get(field).is_none() {
        return Err(PortError::Validation(format!("missing field: {field}")));
    }
    Ok(())
}

fn get_str<'a>(input: &'a serde_json::Value, field: &str) -> soma_port_sdk::Result<&'a str> {
    input[field]
        .as_str()
        .ok_or_else(|| PortError::Validation(format!("{field} must be a string")))
}

// ---------------------------------------------------------------------------
// Spec builder
// ---------------------------------------------------------------------------

fn build_spec() -> PortSpec {
    PortSpec {
        port_id: PORT_ID.into(),
        name: "s3".into(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Database,
        description: "S3-compatible object storage: upload, download, delete, presign, list".into(),
        namespace: "soma.s3".into(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![
            PortCapabilitySpec {
                capability_id: "put_object".into(),
                name: "put_object".into(),
                purpose: "Upload an object to an S3 bucket".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "bucket": {"type": "string"}, "key": {"type": "string"},
                    "data": {"type": "string"}, "content_type": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "uri": {"type": "string"}, "bucket": {"type": "string"},
                    "key": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Low,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 200,
                    p95_latency_ms: 2000,
                    max_latency_ms: 30000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Medium,
                    io_cost_class: CostClass::Medium,
                    ..CostProfile::default()
                },
                remote_exposable: true,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "get_object".into(),
                name: "get_object".into(),
                purpose: "Download an object from an S3 bucket".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "bucket": {"type": "string"}, "key": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "data": {"type": "string"}, "size": {"type": "integer"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 150,
                    p95_latency_ms: 1500,
                    max_latency_ms: 30000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Medium,
                    ..CostProfile::default()
                },
                remote_exposable: true,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "delete_object".into(),
                name: "delete_object".into(),
                purpose: "Delete an object from an S3 bucket".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "bucket": {"type": "string"}, "key": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "deleted": {"type": "boolean"},
                })),
                effect_class: SideEffectClass::Destructive,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Medium,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 100,
                    p95_latency_ms: 1000,
                    max_latency_ms: 10000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "presign_url".into(),
                name: "presign_url".into(),
                purpose: "Generate a presigned URL for temporary access to an object".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "bucket": {"type": "string"}, "key": {"type": "string"},
                    "expires_secs": {"type": "integer"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "url": {"type": "string"}, "expires_secs": {"type": "integer"},
                })),
                effect_class: SideEffectClass::None,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Low,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 10,
                    p95_latency_ms: 100,
                    max_latency_ms: 5000,
                },
                cost_profile: CostProfile::default(),
                remote_exposable: true,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "list_objects".into(),
                name: "list_objects".into(),
                purpose: "List objects in an S3 bucket with optional prefix filter".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "bucket": {"type": "string"}, "prefix": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "objects": {"type": "array"}, "count": {"type": "integer"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 100,
                    p95_latency_ms: 1000,
                    max_latency_ms: 15000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: true,
                auth_override: None,
            },
        ],
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::any(),
        failure_modes: vec![
            PortFailureClass::ValidationError,
            PortFailureClass::ExternalError,
            PortFailureClass::Timeout,
            PortFailureClass::AuthorizationDenied,
        ],
        side_effect_class: SideEffectClass::ExternalStateMutation,
        latency_profile: LatencyProfile {
            expected_latency_ms: 150,
            p95_latency_ms: 2000,
            max_latency_ms: 30000,
        },
        cost_profile: CostProfile {
            network_cost_class: CostClass::Medium,
            ..CostProfile::default()
        },
        auth_requirements: AuthRequirements {
            methods: vec![AuthMethod::ApiKey],
            required: true,
        },
        sandbox_requirements: SandboxRequirements {
            network_access: true,
            ..SandboxRequirements::default()
        },
        observable_fields: vec!["bucket".into(), "key".into(), "size".into()],
        validation_rules: vec![],
        remote_exposure: true,
    }
}

// ---------------------------------------------------------------------------
// C ABI entry point
// ---------------------------------------------------------------------------

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    Box::into_raw(Box::new(S3Port::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec() {
        let port = S3Port::new();
        assert_eq!(port.spec().port_id, "soma.s3");
        assert_eq!(port.spec().capabilities.len(), 5);
    }

    #[test]
    fn test_lifecycle_before_init() {
        let port = S3Port::new();
        assert_eq!(port.lifecycle_state(), PortLifecycleState::Loaded);
    }

    #[test]
    fn test_invoke_without_client() {
        let port = S3Port::new();
        let result = port.invoke(
            "put_object",
            serde_json::json!({"key": "test", "data": "abc"}),
        );
        // Should return a PortCallRecord with success=false, not an Err
        let record = result.unwrap();
        assert!(!record.success);
    }

    #[test]
    fn test_unknown_capability() {
        let port = S3Port::new();
        assert!(port.invoke("nonexistent", serde_json::json!({})).is_err());
    }
}
