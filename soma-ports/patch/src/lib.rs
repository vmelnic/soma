use std::process::Command;
use std::time::Instant;

use soma_port_sdk::prelude::*;

const PORT_ID: &str = "soma.patch";

pub struct PatchPort {
    spec: PortSpec,
}

impl PatchPort {
    pub fn new() -> Self {
        Self { spec: build_spec() }
    }
}

impl Default for PatchPort {
    fn default() -> Self {
        Self::new()
    }
}

impl Port for PatchPort {
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
            "apply_patch" => self.apply_patch(&input),
            "check_patch" => self.check_patch(&input),
            "create_patch" => self.create_patch(&input),
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
            "apply_patch" | "check_patch" => {
                require_field(input, "cwd")?;
                require_field(input, "patch")?;
            }
            "create_patch" => {
                require_field(input, "cwd")?;
                require_field(input, "file")?;
                require_field(input, "content")?;
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
        PortLifecycleState::Active
    }
}

impl PatchPort {
    fn apply_patch(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let cwd = get_str(input, "cwd")?;
        let patch = get_str(input, "patch")?;

        let output = Command::new("git")
            .args(["apply", "--verbose", "-"])
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(ref mut stdin) = child.stdin {
                    stdin.write_all(patch.as_bytes())?;
                }
                child.wait_with_output()
            })
            .map_err(|e| PortError::DependencyUnavailable(format!("git not found: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let code = output.status.code().unwrap_or(-1);

        if code != 0 {
            return Ok(serde_json::json!({
                "applied": false,
                "error": stderr.trim(),
            }));
        }

        Ok(serde_json::json!({
            "applied": true,
            "output": stdout.trim(),
        }))
    }

    fn check_patch(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let cwd = get_str(input, "cwd")?;
        let patch = get_str(input, "patch")?;

        let output = Command::new("git")
            .args(["apply", "--check", "-"])
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(ref mut stdin) = child.stdin {
                    stdin.write_all(patch.as_bytes())?;
                }
                child.wait_with_output()
            })
            .map_err(|e| PortError::DependencyUnavailable(format!("git not found: {e}")))?;

        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let code = output.status.code().unwrap_or(-1);

        Ok(serde_json::json!({
            "valid": code == 0,
            "error": if code != 0 { stderr.trim() } else { "" },
        }))
    }

    fn create_patch(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let cwd = get_str(input, "cwd")?;
        let file = get_str(input, "file")?;
        let content = get_str(input, "content")?;

        let file_path = std::path::Path::new(cwd).join(file);

        let original = if file_path.exists() {
            std::fs::read_to_string(&file_path)
                .map_err(|e| PortError::ExternalError(format!("failed to read {file}: {e}")))?
        } else {
            String::new()
        };

        let patch = generate_unified_diff(file, &original, content);

        Ok(serde_json::json!({
            "patch": patch,
            "file": file,
            "original_lines": original.lines().count(),
            "new_lines": content.lines().count(),
        }))
    }
}

fn generate_unified_diff(file: &str, original: &str, new: &str) -> String {
    let orig_lines: Vec<&str> = if original.is_empty() {
        vec![]
    } else {
        original.lines().collect()
    };
    let new_lines: Vec<&str> = if new.is_empty() {
        vec![]
    } else {
        new.lines().collect()
    };

    let mut result = String::new();
    let a_path = if original.is_empty() {
        "/dev/null".to_string()
    } else {
        format!("a/{file}")
    };
    let b_path = format!("b/{file}");

    result.push_str(&format!("--- {a_path}\n"));
    result.push_str(&format!("+++ {b_path}\n"));

    if original.is_empty() {
        result.push_str(&format!("@@ -0,0 +1,{} @@\n", new_lines.len()));
        for line in &new_lines {
            result.push_str(&format!("+{line}\n"));
        }
    } else if new.is_empty() {
        result.push_str(&format!("@@ -1,{} +0,0 @@\n", orig_lines.len()));
        for line in &orig_lines {
            result.push_str(&format!("-{line}\n"));
        }
    } else {
        result.push_str(&format!(
            "@@ -1,{} +1,{} @@\n",
            orig_lines.len(),
            new_lines.len()
        ));
        for line in &orig_lines {
            result.push_str(&format!("-{line}\n"));
        }
        for line in &new_lines {
            result.push_str(&format!("+{line}\n"));
        }
    }

    result
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
    let fast = LatencyProfile {
        expected_latency_ms: 20,
        p95_latency_ms: 200,
        max_latency_ms: 5000,
    };
    let io_cost = CostProfile {
        cpu_cost_class: CostClass::Low,
        memory_cost_class: CostClass::Low,
        io_cost_class: CostClass::Medium,
        network_cost_class: CostClass::Negligible,
        energy_cost_class: CostClass::Negligible,
    };

    PortSpec {
        port_id: PORT_ID.into(),
        name: "patch".into(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Custom,
        description: "Patch operations: apply unified diffs, validate patches, create patches"
            .into(),
        namespace: "soma.patch".into(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![
            PortCapabilitySpec {
                capability_id: "apply_patch".into(),
                name: "apply_patch".into(),
                purpose: "Apply a unified diff patch to the working directory".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "cwd": {"type": "string", "description": "Repository/project root"},
                    "patch": {"type": "string", "description": "Unified diff content"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "applied": {"type": "boolean"},
                    "output": {"type": "string"},
                    "error": {"type": "string"},
                })),
                effect_class: SideEffectClass::LocalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Medium,
                latency_profile: fast.clone(),
                cost_profile: io_cost.clone(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "check_patch".into(),
                name: "check_patch".into(),
                purpose: "Check if a patch applies cleanly without modifying files".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "cwd": {"type": "string", "description": "Repository/project root"},
                    "patch": {"type": "string", "description": "Unified diff content"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "valid": {"type": "boolean"}, "error": {"type": "string"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: fast.clone(),
                cost_profile: io_cost.clone(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "create_patch".into(),
                name: "create_patch".into(),
                purpose: "Generate a unified diff from current file content vs new content".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "cwd": {"type": "string", "description": "Project root"},
                    "file": {"type": "string", "description": "File path relative to cwd"},
                    "content": {"type": "string", "description": "New file content"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "patch": {"type": "string"}, "file": {"type": "string"},
                    "original_lines": {"type": "integer"}, "new_lines": {"type": "integer"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: fast.clone(),
                cost_profile: io_cost.clone(),
                remote_exposable: false,
                auth_override: None,
            },
        ],
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::any(),
        failure_modes: vec![
            PortFailureClass::ValidationError,
            PortFailureClass::ExternalError,
            PortFailureClass::DependencyUnavailable,
        ],
        side_effect_class: SideEffectClass::LocalStateMutation,
        latency_profile: fast,
        cost_profile: io_cost,
        auth_requirements: AuthRequirements::default(),
        sandbox_requirements: SandboxRequirements {
            filesystem_access: true,
            process_access: true,
            ..Default::default()
        },
        observable_fields: vec![
            "applied".into(),
            "valid".into(),
            "file".into(),
        ],
        validation_rules: vec![],
        remote_exposure: false,
    }
}

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    Box::into_raw(Box::new(PatchPort::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec() {
        let port = PatchPort::new();
        assert_eq!(port.spec().port_id, "soma.patch");
        assert_eq!(port.spec().capabilities.len(), 3);
    }

    #[test]
    fn test_create_patch_new_file() {
        let port = PatchPort::new();
        let record = port
            .invoke(
                "create_patch",
                serde_json::json!({
                    "cwd": "/tmp",
                    "file": "nonexistent_test_file.txt",
                    "content": "hello\nworld\n",
                }),
            )
            .unwrap();
        assert!(record.success);
        let patch = record.raw_result["patch"].as_str().unwrap();
        assert!(patch.contains("--- /dev/null"));
        assert!(patch.contains("+hello"));
    }

    #[test]
    fn test_generate_unified_diff() {
        let diff = generate_unified_diff("test.js", "old line\n", "new line\n");
        assert!(diff.contains("--- a/test.js"));
        assert!(diff.contains("+++ b/test.js"));
        assert!(diff.contains("-old line"));
        assert!(diff.contains("+new line"));
    }
}
