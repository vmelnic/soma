use std::fs;
use std::path::Path;
use web_time::Instant;

use chrono::Utc;
use semver::Version;
use uuid::Uuid;

use crate::errors::{Result, SomaError};
use crate::runtime::port::Port;
use crate::types::common::{
    AuthRequirements, CostClass, CostProfile, DeterminismClass, IdempotenceClass,
    LatencyProfile, PortFailureClass, RiskClass, RollbackSupport, SandboxRequirements, SchemaRef,
    SideEffectClass, TrustLevel,
};
use crate::types::observation::PortCallRecord;
use crate::types::port::{PortCapabilitySpec, PortKind, PortLifecycleState, PortSpec};

/// Filesystem port adapter providing real OS filesystem operations.
///
/// Capabilities: readdir, readfile, writefile, stat, mkdir, rmdir, rm.
pub struct FilesystemPort {
    spec: PortSpec,
}

impl FilesystemPort {
    pub fn new() -> Self {
        Self {
            spec: build_filesystem_port_spec(),
        }
    }
}

impl Default for FilesystemPort {
    fn default() -> Self {
        Self::new()
    }
}

impl Port for FilesystemPort {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(
        &self,
        capability_id: &str,
        input: serde_json::Value,
    ) -> Result<PortCallRecord> {
        let start = Instant::now();

        let result = match capability_id {
            "readdir" => self.do_readdir(&input),
            "readfile" => self.do_readfile(&input),
            "writefile" => self.do_writefile(&input),
            "stat" => self.do_stat(&input),
            "mkdir" => self.do_mkdir(&input),
            "rmdir" => self.do_rmdir(&input),
            "rm" => self.do_rm(&input),
            other => {
                return Err(SomaError::Port(format!(
                    "unknown capability '{}' on filesystem port",
                    other,
                )));
            }
        };

        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(structured) => Ok(PortCallRecord {
                observation_id: Uuid::new_v4(),
                port_id: self.spec.port_id.clone(),
                capability_id: capability_id.to_string(),
                invocation_id: Uuid::new_v4(),
                success: true,
                failure_class: None,
                raw_result: structured.clone(),
                structured_result: structured,
                effect_patch: None,
                side_effect_summary: Some(side_effect_for(capability_id).to_string()),
                latency_ms,
                resource_cost: 0.001,
                confidence: 1.0,
                timestamp: Utc::now(),
                retry_safe: true,
                input_hash: None,
                session_id: None,
                goal_id: None,
                caller_identity: None,
                auth_result: None,
                policy_result: None,
                sandbox_result: None,
            }),
            Err(e) => {
                let failure_class = classify_fs_error(&e);
                Ok(PortCallRecord {
                    observation_id: Uuid::new_v4(),
                    port_id: self.spec.port_id.clone(),
                    capability_id: capability_id.to_string(),
                    invocation_id: Uuid::new_v4(),
                    success: false,
                    failure_class: Some(failure_class),
                    raw_result: serde_json::Value::Null,
                    structured_result: serde_json::json!({ "error": e.to_string() }),
                    effect_patch: None,
                    side_effect_summary: Some("none".to_string()),
                    latency_ms,
                    resource_cost: 0.0,
                    confidence: 0.0,
                    timestamp: Utc::now(),
                    retry_safe: false,
                    input_hash: None,
                    session_id: None,
                    goal_id: None,
                    caller_identity: None,
                    auth_result: None,
                    policy_result: None,
                    sandbox_result: None,
                })
            }
        }
    }

    fn validate_input(
        &self,
        capability_id: &str,
        input: &serde_json::Value,
    ) -> Result<()> {
        match capability_id {
            "readdir" | "readfile" | "stat" | "mkdir" | "rmdir" | "rm" => {
                let obj = input.as_object().ok_or_else(|| {
                    SomaError::Port("input must be a JSON object".to_string())
                })?;
                if !obj.contains_key("path") {
                    return Err(SomaError::Port("missing required field 'path'".to_string()));
                }
                if !obj["path"].is_string() {
                    return Err(SomaError::Port("'path' must be a string".to_string()));
                }
                Ok(())
            }
            "writefile" => {
                let obj = input.as_object().ok_or_else(|| {
                    SomaError::Port("input must be a JSON object".to_string())
                })?;
                if !obj.contains_key("path") {
                    return Err(SomaError::Port("missing required field 'path'".to_string()));
                }
                if !obj["path"].is_string() {
                    return Err(SomaError::Port("'path' must be a string".to_string()));
                }
                if !obj.contains_key("content") {
                    return Err(SomaError::Port(
                        "missing required field 'content'".to_string(),
                    ));
                }
                Ok(())
            }
            other => Err(SomaError::Port(format!(
                "unknown capability '{}' on filesystem port",
                other,
            ))),
        }
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        PortLifecycleState::Active
    }
}

// ---------------------------------------------------------------------------
// Capability implementations
// ---------------------------------------------------------------------------

impl FilesystemPort {
    fn get_path(input: &serde_json::Value) -> Result<&str> {
        input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SomaError::Port("missing required field 'path'".to_string()))
    }

    fn do_readdir(&self, input: &serde_json::Value) -> Result<serde_json::Value> {
        let path_str = Self::get_path(input)?;
        let path = Path::new(path_str);

        let entries: Vec<serde_json::Value> = fs::read_dir(path)
            .map_err(|e| SomaError::Port(format!("readdir '{}': {}", path_str, e)))?
            .filter_map(|entry| entry.ok())
            .map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                let file_type = entry.file_type().ok();
                let is_dir = file_type.as_ref().is_some_and(|ft| ft.is_dir());
                let is_file = file_type.as_ref().is_some_and(|ft| ft.is_file());
                let is_symlink = file_type.as_ref().is_some_and(|ft| ft.is_symlink());
                serde_json::json!({
                    "name": name,
                    "path": entry.path().to_string_lossy(),
                    "is_dir": is_dir,
                    "is_file": is_file,
                    "is_symlink": is_symlink,
                })
            })
            .collect();

        Ok(serde_json::json!({
            "path": path_str,
            "entries": entries,
            "count": entries.len(),
        }))
    }

    fn do_readfile(&self, input: &serde_json::Value) -> Result<serde_json::Value> {
        let path_str = Self::get_path(input)?;
        let content = fs::read_to_string(path_str)
            .map_err(|e| SomaError::Port(format!("readfile '{}': {}", path_str, e)))?;
        Ok(serde_json::json!({
            "path": path_str,
            "content": content,
            "size": content.len(),
        }))
    }

    fn do_writefile(&self, input: &serde_json::Value) -> Result<serde_json::Value> {
        let path_str = Self::get_path(input)?;
        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SomaError::Port("missing required field 'content'".to_string()))?;
        fs::write(path_str, content)
            .map_err(|e| SomaError::Port(format!("writefile '{}': {}", path_str, e)))?;
        Ok(serde_json::json!({
            "path": path_str,
            "bytes_written": content.len(),
        }))
    }

    fn do_stat(&self, input: &serde_json::Value) -> Result<serde_json::Value> {
        let path_str = Self::get_path(input)?;
        let meta = fs::metadata(path_str)
            .map_err(|e| SomaError::Port(format!("stat '{}': {}", path_str, e)))?;
        Ok(serde_json::json!({
            "path": path_str,
            "is_file": meta.is_file(),
            "is_dir": meta.is_dir(),
            "is_symlink": meta.is_symlink(),
            "size": meta.len(),
            "readonly": meta.permissions().readonly(),
        }))
    }

    fn do_mkdir(&self, input: &serde_json::Value) -> Result<serde_json::Value> {
        let path_str = Self::get_path(input)?;
        fs::create_dir_all(path_str)
            .map_err(|e| SomaError::Port(format!("mkdir '{}': {}", path_str, e)))?;
        Ok(serde_json::json!({
            "path": path_str,
            "created": true,
        }))
    }

    fn do_rmdir(&self, input: &serde_json::Value) -> Result<serde_json::Value> {
        let path_str = Self::get_path(input)?;
        fs::remove_dir(path_str)
            .map_err(|e| SomaError::Port(format!("rmdir '{}': {}", path_str, e)))?;
        Ok(serde_json::json!({
            "path": path_str,
            "removed": true,
        }))
    }

    fn do_rm(&self, input: &serde_json::Value) -> Result<serde_json::Value> {
        let path_str = Self::get_path(input)?;
        fs::remove_file(path_str)
            .map_err(|e| SomaError::Port(format!("rm '{}': {}", path_str, e)))?;
        Ok(serde_json::json!({
            "path": path_str,
            "removed": true,
        }))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn side_effect_for(capability_id: &str) -> &'static str {
    match capability_id {
        "readdir" | "readfile" | "stat" => "read_only",
        "writefile" | "mkdir" => "local_state_mutation",
        "rmdir" | "rm" => "destructive",
        _ => "none",
    }
}

fn classify_fs_error(err: &SomaError) -> PortFailureClass {
    let msg = err.to_string();
    if msg.contains("not found") || msg.contains("No such file") {
        PortFailureClass::ExternalError
    } else if msg.contains("permission") || msg.contains("Permission") {
        PortFailureClass::AuthorizationDenied
    } else {
        PortFailureClass::ExternalError
    }
}

// ---------------------------------------------------------------------------
// PortSpec builder
// ---------------------------------------------------------------------------

fn fs_latency() -> LatencyProfile {
    LatencyProfile {
        expected_latency_ms: 1,
        p95_latency_ms: 10,
        max_latency_ms: 1000,
    }
}

fn fs_cost() -> CostProfile {
    CostProfile {
        cpu_cost_class: CostClass::Negligible,
        memory_cost_class: CostClass::Low,
        io_cost_class: CostClass::Low,
        network_cost_class: CostClass::Negligible,
        energy_cost_class: CostClass::Negligible,
    }
}

fn path_input_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["path"],
        "properties": {
            "path": { "type": "string" }
        }
    })
}

fn write_input_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["path", "content"],
        "properties": {
            "path": { "type": "string" },
            "content": { "type": "string" }
        }
    })
}

fn readdir_output_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "path": { "type": "string" },
            "entries": { "type": "array" },
            "count": { "type": "integer" }
        }
    })
}

fn file_output_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "path": { "type": "string" },
            "content": { "type": "string" },
            "size": { "type": "integer" }
        }
    })
}

fn write_output_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "path": { "type": "string" },
            "bytes_written": { "type": "integer" }
        }
    })
}

fn stat_output_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "path": { "type": "string" },
            "is_file": { "type": "boolean" },
            "is_dir": { "type": "boolean" },
            "is_symlink": { "type": "boolean" },
            "size": { "type": "integer" },
            "readonly": { "type": "boolean" }
        }
    })
}

fn bool_result_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "path": { "type": "string" },
            "created": { "type": "boolean" },
            "removed": { "type": "boolean" }
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn make_cap(
    id: &str,
    name: &str,
    purpose: &str,
    input: serde_json::Value,
    output: serde_json::Value,
    effect: SideEffectClass,
    rollback: RollbackSupport,
    idem: IdempotenceClass,
    risk: RiskClass,
) -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: id.to_string(),
        name: name.to_string(),
        purpose: purpose.to_string(),
        input_schema: SchemaRef { schema: input },
        output_schema: SchemaRef { schema: output },
        effect_class: effect,
        rollback_support: rollback,
        determinism_class: DeterminismClass::Deterministic,
        idempotence_class: idem,
        risk_class: risk,
        latency_profile: fs_latency(),
        cost_profile: fs_cost(),
        remote_exposable: false,
        auth_override: None,
    }
}

fn build_filesystem_port_spec() -> PortSpec {
    let capabilities = vec![
        make_cap(
            "readdir",
            "Read Directory",
            "List entries in a directory",
            path_input_schema(),
            readdir_output_schema(),
            SideEffectClass::ReadOnly,
            RollbackSupport::Irreversible,
            IdempotenceClass::Idempotent,
            RiskClass::Negligible,
        ),
        make_cap(
            "readfile",
            "Read File",
            "Read the contents of a file as UTF-8 text",
            path_input_schema(),
            file_output_schema(),
            SideEffectClass::ReadOnly,
            RollbackSupport::Irreversible,
            IdempotenceClass::Idempotent,
            RiskClass::Negligible,
        ),
        make_cap(
            "writefile",
            "Write File",
            "Write text content to a file, creating or overwriting",
            write_input_schema(),
            write_output_schema(),
            SideEffectClass::LocalStateMutation,
            RollbackSupport::CompensatingAction,
            IdempotenceClass::Idempotent,
            RiskClass::Medium,
        ),
        make_cap(
            "stat",
            "File Stat",
            "Get metadata about a file or directory",
            path_input_schema(),
            stat_output_schema(),
            SideEffectClass::ReadOnly,
            RollbackSupport::Irreversible,
            IdempotenceClass::Idempotent,
            RiskClass::Negligible,
        ),
        make_cap(
            "mkdir",
            "Make Directory",
            "Create a directory and any missing parent directories",
            path_input_schema(),
            bool_result_schema(),
            SideEffectClass::LocalStateMutation,
            RollbackSupport::CompensatingAction,
            IdempotenceClass::Idempotent,
            RiskClass::Low,
        ),
        make_cap(
            "rmdir",
            "Remove Directory",
            "Remove an empty directory",
            path_input_schema(),
            bool_result_schema(),
            SideEffectClass::Destructive,
            RollbackSupport::Irreversible,
            IdempotenceClass::Idempotent,
            RiskClass::Medium,
        ),
        make_cap(
            "rm",
            "Remove File",
            "Remove a file",
            path_input_schema(),
            bool_result_schema(),
            SideEffectClass::Destructive,
            RollbackSupport::Irreversible,
            IdempotenceClass::Idempotent,
            RiskClass::Medium,
        ),
    ];

    PortSpec {
        port_id: "filesystem".to_string(),
        name: "Filesystem".to_string(),
        version: Version::new(1, 0, 0),
        kind: PortKind::Filesystem,
        description: "Local filesystem port for directory and file operations".to_string(),
        namespace: "reference".to_string(),
        trust_level: TrustLevel::BuiltIn,
        capabilities,
        input_schema: SchemaRef {
            schema: path_input_schema(),
        },
        output_schema: SchemaRef {
            schema: serde_json::json!({
                "type": "object"
            }),
        },
        failure_modes: vec![
            PortFailureClass::ExternalError,
            PortFailureClass::AuthorizationDenied,
            PortFailureClass::ValidationError,
        ],
        side_effect_class: SideEffectClass::LocalStateMutation,
        latency_profile: fs_latency(),
        cost_profile: fs_cost(),
        auth_requirements: AuthRequirements {
            methods: vec![],
            required: false,
        },
        sandbox_requirements: SandboxRequirements {
            filesystem_access: true,
            network_access: false,
            device_access: false,
            process_access: false,
            memory_limit_mb: None,
            cpu_limit_percent: None,
            time_limit_ms: Some(5000),
            syscall_limit: None,
        },
        observable_fields: vec![
            "path".to_string(),
        ],
        validation_rules: vec![],
        remote_exposure: false,
        backend: crate::types::port::PortBackend::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filesystem_port_spec_is_valid() {
        let port = FilesystemPort::new();
        let spec = port.spec();

        assert_eq!(spec.port_id, "filesystem");
        assert_eq!(spec.kind, PortKind::Filesystem);
        assert_eq!(spec.capabilities.len(), 7);

        let cap_ids: Vec<&str> = spec
            .capabilities
            .iter()
            .map(|c| c.capability_id.as_str())
            .collect();
        assert!(cap_ids.contains(&"readdir"));
        assert!(cap_ids.contains(&"readfile"));
        assert!(cap_ids.contains(&"writefile"));
        assert!(cap_ids.contains(&"stat"));
        assert!(cap_ids.contains(&"mkdir"));
        assert!(cap_ids.contains(&"rmdir"));
        assert!(cap_ids.contains(&"rm"));
    }

    #[test]
    fn filesystem_port_lifecycle_is_active() {
        let port = FilesystemPort::new();
        assert_eq!(port.lifecycle_state(), PortLifecycleState::Active);
    }

    #[test]
    fn readdir_on_temp_dir() {
        let dir = std::env::temp_dir().join("soma_fs_port_test_readdir");
        let _ = fs::create_dir_all(&dir);
        let _ = fs::write(dir.join("hello.txt"), "world");
        let _ = fs::write(dir.join("bye.txt"), "later");

        let port = FilesystemPort::new();
        let input = serde_json::json!({ "path": dir.to_string_lossy() });
        let record = port.invoke("readdir", input).unwrap();

        assert!(record.success);
        let entries = record.structured_result["entries"].as_array().unwrap();
        let names: Vec<&str> = entries
            .iter()
            .filter_map(|e| e["name"].as_str())
            .collect();
        assert!(names.contains(&"hello.txt"));
        assert!(names.contains(&"bye.txt"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn readdir_nonexistent_returns_failure() {
        let port = FilesystemPort::new();
        let input = serde_json::json!({ "path": "/tmp/soma_fs_port_test_nonexistent_dir_xyz" });
        let record = port.invoke("readdir", input).unwrap();
        assert!(!record.success);
        assert!(record.failure_class.is_some());
    }

    #[test]
    fn readfile_and_writefile_roundtrip() {
        let dir = std::env::temp_dir().join("soma_fs_port_test_rw");
        let _ = fs::create_dir_all(&dir);
        let file_path = dir.join("test.txt");

        let port = FilesystemPort::new();
        let write_input = serde_json::json!({
            "path": file_path.to_string_lossy(),
            "content": "hello soma",
        });
        let write_record = port.invoke("writefile", write_input).unwrap();
        assert!(write_record.success);

        let read_input = serde_json::json!({ "path": file_path.to_string_lossy() });
        let read_record = port.invoke("readfile", read_input).unwrap();
        assert!(read_record.success);
        assert_eq!(read_record.structured_result["content"], "hello soma");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stat_on_file() {
        let dir = std::env::temp_dir().join("soma_fs_port_test_stat");
        let _ = fs::create_dir_all(&dir);
        let file_path = dir.join("statme.txt");
        fs::write(&file_path, "data").unwrap();

        let port = FilesystemPort::new();
        let input = serde_json::json!({ "path": file_path.to_string_lossy() });
        let record = port.invoke("stat", input).unwrap();
        assert!(record.success);
        assert_eq!(record.structured_result["is_file"], true);
        assert_eq!(record.structured_result["is_dir"], false);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mkdir_and_rmdir() {
        let dir = std::env::temp_dir().join("soma_fs_port_test_mkdir");
        let _ = fs::remove_dir_all(&dir);

        let port = FilesystemPort::new();
        let mk_input = serde_json::json!({ "path": dir.to_string_lossy() });
        let mk_record = port.invoke("mkdir", mk_input).unwrap();
        assert!(mk_record.success);
        assert!(dir.exists());

        let rm_input = serde_json::json!({ "path": dir.to_string_lossy() });
        let rm_record = port.invoke("rmdir", rm_input).unwrap();
        assert!(rm_record.success);
        assert!(!dir.exists());
    }

    #[test]
    fn rm_file() {
        let dir = std::env::temp_dir().join("soma_fs_port_test_rm");
        let _ = fs::create_dir_all(&dir);
        let file_path = dir.join("deleteme.txt");
        fs::write(&file_path, "gone").unwrap();

        let port = FilesystemPort::new();
        let input = serde_json::json!({ "path": file_path.to_string_lossy() });
        let record = port.invoke("rm", input).unwrap();
        assert!(record.success);
        assert!(!file_path.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_input_rejects_missing_path() {
        let port = FilesystemPort::new();
        let input = serde_json::json!({});
        assert!(port.validate_input("readdir", &input).is_err());
    }

    #[test]
    fn validate_input_rejects_non_string_path() {
        let port = FilesystemPort::new();
        let input = serde_json::json!({ "path": 42 });
        assert!(port.validate_input("readdir", &input).is_err());
    }

    #[test]
    fn validate_input_rejects_unknown_capability() {
        let port = FilesystemPort::new();
        let input = serde_json::json!({ "path": "/tmp" });
        assert!(port.validate_input("frobnicate", &input).is_err());
    }

    #[test]
    fn validate_writefile_requires_content() {
        let port = FilesystemPort::new();
        let input = serde_json::json!({ "path": "/tmp/test" });
        assert!(port.validate_input("writefile", &input).is_err());
    }
}
