//! SOMA Google Drive Port -- manage files and folders via the Google Drive API.
//!
//! Five capabilities:
//!
//! | ID | Name            | Description                        |
//! |----|-----------------|------------------------------------|
//! | 0  | `list_files`    | List files (with optional query)   |
//! | 1  | `get_file`      | Get file metadata by ID            |
//! | 2  | `upload_file`   | Upload a file (simple upload)      |
//! | 3  | `delete_file`   | Delete a file by ID                |
//! | 4  | `create_folder` | Create a folder                    |
//!
//! Auth: OAuth2 Bearer token via `SOMA_GOOGLE_ACCESS_TOKEN` or `GOOGLE_ACCESS_TOKEN`.

use std::time::Instant;

use soma_port_sdk::prelude::*;

const PORT_ID: &str = "soma.google.drive";
const BASE_URL: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD_URL: &str = "https://www.googleapis.com/upload/drive/v3";

// ---------------------------------------------------------------------------
// Port struct
// ---------------------------------------------------------------------------

pub struct GoogleDrivePort {
    spec: PortSpec,
}

impl GoogleDrivePort {
    pub fn new() -> Self {
        Self {
            spec: build_spec(),
        }
    }

    fn access_token() -> soma_port_sdk::Result<String> {
        std::env::var("SOMA_GOOGLE_ACCESS_TOKEN")
            .or_else(|_| std::env::var("GOOGLE_ACCESS_TOKEN"))
            .map_err(|_| {
                PortError::DependencyUnavailable(
                    "Google access token not set. Set SOMA_GOOGLE_ACCESS_TOKEN or GOOGLE_ACCESS_TOKEN"
                        .into(),
                )
            })
    }

    fn client() -> reqwest::blocking::Client {
        reqwest::blocking::Client::new()
    }
}

impl Default for GoogleDrivePort {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Port trait implementation
// ---------------------------------------------------------------------------

impl Port for GoogleDrivePort {
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
            "list_files" => self.list_files(&input),
            "get_file" => self.get_file(&input),
            "upload_file" => self.upload_file(&input),
            "delete_file" => self.delete_file(&input),
            "create_folder" => self.create_folder(&input),
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
            "list_files" => {}
            "get_file" => {
                require_field(input, "file_id")?;
            }
            "upload_file" => {
                require_field(input, "name")?;
                require_field(input, "content")?;
            }
            "delete_file" => {
                require_field(input, "file_id")?;
            }
            "create_folder" => {
                require_field(input, "name")?;
            }
            other => {
                return Err(PortError::Validation(format!(
                    "unknown capability: {other}"
                )));
            }
        }
        Ok(())
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        if Self::access_token().is_ok() {
            PortLifecycleState::Active
        } else {
            PortLifecycleState::Loaded
        }
    }
}

// ---------------------------------------------------------------------------
// Capability implementations
// ---------------------------------------------------------------------------

impl GoogleDrivePort {
    fn list_files(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let token = Self::access_token()?;

        let mut request = Self::client()
            .get(format!("{BASE_URL}/files"))
            .bearer_auth(&token);

        if let Some(query) = input.get("query").and_then(|v| v.as_str()) {
            request = request.query(&[("q", query)]);
        }
        if let Some(page_size) = input.get("page_size").and_then(|v| v.as_u64()) {
            request = request.query(&[("pageSize", &page_size.to_string())]);
        }
        if let Some(page_token) = input.get("page_token").and_then(|v| v.as_str()) {
            request = request.query(&[("pageToken", page_token)]);
        }

        let resp = request
            .send()
            .map_err(|e| PortError::TransportError(format!("HTTP request failed: {e}")))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| PortError::ExternalError(format!("failed to parse response: {e}")))?;

        if !status.is_success() {
            return Err(PortError::ExternalError(format!(
                "Google Drive API error {status}: {body}"
            )));
        }

        Ok(body)
    }

    fn get_file(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let token = Self::access_token()?;
        let file_id = get_str(input, "file_id")?;

        let resp = Self::client()
            .get(format!("{BASE_URL}/files/{file_id}"))
            .bearer_auth(&token)
            .query(&[("fields", "id,name,mimeType,size,createdTime,modifiedTime,parents")])
            .send()
            .map_err(|e| PortError::TransportError(format!("HTTP request failed: {e}")))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| PortError::ExternalError(format!("failed to parse response: {e}")))?;

        if !status.is_success() {
            return Err(PortError::ExternalError(format!(
                "Google Drive API error {status}: {body}"
            )));
        }

        Ok(body)
    }

    fn upload_file(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let token = Self::access_token()?;
        let name = get_str(input, "name")?;
        let content = get_str(input, "content")?;
        let mime_type = input
            .get("mime_type")
            .and_then(|v| v.as_str())
            .unwrap_or("application/octet-stream");

        let mut metadata = serde_json::json!({ "name": name });
        if let Some(parent_id) = input.get("parent_id").and_then(|v| v.as_str()) {
            metadata["parents"] = serde_json::json!([parent_id]);
        }

        let metadata_part = reqwest::blocking::multipart::Part::text(metadata.to_string())
            .mime_str("application/json; charset=UTF-8")
            .map_err(|e| PortError::Internal(format!("failed to build metadata part: {e}")))?;

        let file_part = reqwest::blocking::multipart::Part::text(content.to_string())
            .mime_str(mime_type)
            .map_err(|e| PortError::Internal(format!("failed to build file part: {e}")))?;

        let form = reqwest::blocking::multipart::Form::new()
            .part("metadata", metadata_part)
            .part("file", file_part);

        let resp = Self::client()
            .post(format!("{UPLOAD_URL}/files?uploadType=multipart"))
            .bearer_auth(&token)
            .multipart(form)
            .send()
            .map_err(|e| PortError::TransportError(format!("HTTP request failed: {e}")))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| PortError::ExternalError(format!("failed to parse response: {e}")))?;

        if !status.is_success() {
            return Err(PortError::ExternalError(format!(
                "Google Drive API error {status}: {body}"
            )));
        }

        Ok(body)
    }

    fn delete_file(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let token = Self::access_token()?;
        let file_id = get_str(input, "file_id")?;

        let resp = Self::client()
            .delete(format!("{BASE_URL}/files/{file_id}"))
            .bearer_auth(&token)
            .send()
            .map_err(|e| PortError::TransportError(format!("HTTP request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let body: serde_json::Value = resp.json().unwrap_or_default();
            return Err(PortError::ExternalError(format!(
                "Google Drive API error {status}: {body}"
            )));
        }

        Ok(serde_json::json!({
            "deleted": true,
            "file_id": file_id,
        }))
    }

    fn create_folder(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let token = Self::access_token()?;
        let name = get_str(input, "name")?;

        let mut metadata = serde_json::json!({
            "name": name,
            "mimeType": "application/vnd.google-apps.folder",
        });
        if let Some(parent_id) = input.get("parent_id").and_then(|v| v.as_str()) {
            metadata["parents"] = serde_json::json!([parent_id]);
        }

        let resp = Self::client()
            .post(format!("{BASE_URL}/files"))
            .bearer_auth(&token)
            .json(&metadata)
            .send()
            .map_err(|e| PortError::TransportError(format!("HTTP request failed: {e}")))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| PortError::ExternalError(format!("failed to parse response: {e}")))?;

        if !status.is_success() {
            return Err(PortError::ExternalError(format!(
                "Google Drive API error {status}: {body}"
            )));
        }

        Ok(body)
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
        name: "google-drive".into(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Http,
        description: "Google Drive API: list, get, upload, delete files and create folders".into(),
        namespace: "soma.google.drive".into(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![
            PortCapabilitySpec {
                capability_id: "list_files".into(),
                name: "list_files".into(),
                purpose: "List files in Google Drive with optional search query".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "query": {"type": "string", "description": "Google Drive search query (e.g. \"name contains 'report'\")"},
                    "page_size": {"type": "integer", "description": "Maximum number of files to return"},
                    "page_token": {"type": "string", "description": "Token for fetching next page"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "description": "any",
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 300,
                    p95_latency_ms: 2000,
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
                capability_id: "get_file".into(),
                name: "get_file".into(),
                purpose: "Get metadata for a single file by ID".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "file_id": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "description": "any",
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 200,
                    p95_latency_ms: 1500,
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
                capability_id: "upload_file".into(),
                name: "upload_file".into(),
                purpose: "Upload a file to Google Drive".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "name": {"type": "string"},
                    "content": {"type": "string", "description": "File content as text"},
                    "mime_type": {"type": "string", "description": "MIME type (default: application/octet-stream)"},
                    "parent_id": {"type": "string", "description": "Parent folder ID"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "description": "any",
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Low,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 1000,
                    p95_latency_ms: 5000,
                    max_latency_ms: 30000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Medium,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "delete_file".into(),
                name: "delete_file".into(),
                purpose: "Delete a file from Google Drive by ID".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "file_id": {"type": "string"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "deleted": {"type": "boolean"},
                    "file_id": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::High,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 300,
                    p95_latency_ms: 2000,
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
                capability_id: "create_folder".into(),
                name: "create_folder".into(),
                purpose: "Create a folder in Google Drive".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "name": {"type": "string"},
                    "parent_id": {"type": "string", "description": "Parent folder ID"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "description": "any",
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Low,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 500,
                    p95_latency_ms: 3000,
                    max_latency_ms: 10000,
                },
                cost_profile: CostProfile {
                    network_cost_class: CostClass::Low,
                    ..CostProfile::default()
                },
                remote_exposable: false,
                auth_override: None,
            },
        ],
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::any(),
        failure_modes: vec![
            PortFailureClass::ValidationError,
            PortFailureClass::ExternalError,
            PortFailureClass::TransportError,
            PortFailureClass::Timeout,
            PortFailureClass::DependencyUnavailable,
            PortFailureClass::AuthorizationDenied,
        ],
        side_effect_class: SideEffectClass::ExternalStateMutation,
        latency_profile: LatencyProfile {
            expected_latency_ms: 500,
            p95_latency_ms: 5000,
            max_latency_ms: 30000,
        },
        cost_profile: CostProfile {
            network_cost_class: CostClass::Medium,
            ..CostProfile::default()
        },
        auth_requirements: AuthRequirements {
            methods: vec![AuthMethod::BearerToken],
            required: true,
        },
        sandbox_requirements: SandboxRequirements {
            network_access: true,
            ..SandboxRequirements::default()
        },
        observable_fields: vec![],
        validation_rules: vec![],
        remote_exposure: false,
    }
}

// ---------------------------------------------------------------------------
// C ABI entry point
// ---------------------------------------------------------------------------

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    Box::into_raw(Box::new(GoogleDrivePort::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec() {
        let port = GoogleDrivePort::new();
        assert_eq!(port.spec().port_id, "soma.google.drive");
        assert_eq!(port.spec().capabilities.len(), 5);
    }

    #[test]
    fn test_lifecycle_without_token() {
        unsafe { std::env::remove_var("SOMA_GOOGLE_ACCESS_TOKEN") };
        unsafe { std::env::remove_var("GOOGLE_ACCESS_TOKEN") };
        let port = GoogleDrivePort::new();
        assert_eq!(port.lifecycle_state(), PortLifecycleState::Loaded);
    }

    #[test]
    fn test_validate_list_files_no_required_fields() {
        let port = GoogleDrivePort::new();
        assert!(port
            .validate_input("list_files", &serde_json::json!({}))
            .is_ok());
    }

    #[test]
    fn test_validate_upload_file_missing_fields() {
        let port = GoogleDrivePort::new();
        assert!(port
            .validate_input("upload_file", &serde_json::json!({}))
            .is_err());
    }

    #[test]
    fn test_validate_upload_file_all_fields() {
        let port = GoogleDrivePort::new();
        let input = serde_json::json!({"name": "test.txt", "content": "hello"});
        assert!(port.validate_input("upload_file", &input).is_ok());
    }

    #[test]
    fn test_validate_get_file_missing_id() {
        let port = GoogleDrivePort::new();
        assert!(port
            .validate_input("get_file", &serde_json::json!({}))
            .is_err());
    }

    #[test]
    fn test_validate_delete_file_missing_id() {
        let port = GoogleDrivePort::new();
        assert!(port
            .validate_input("delete_file", &serde_json::json!({}))
            .is_err());
    }

    #[test]
    fn test_validate_create_folder_missing_name() {
        let port = GoogleDrivePort::new();
        assert!(port
            .validate_input("create_folder", &serde_json::json!({}))
            .is_err());
    }

    #[test]
    fn test_unknown_capability() {
        let port = GoogleDrivePort::new();
        assert!(port.invoke("nonexistent", serde_json::json!({})).is_err());
    }
}
