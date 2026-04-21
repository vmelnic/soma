use std::collections::HashMap;
use std::process::Command;
use std::time::Instant;

use soma_port_sdk::prelude::*;

const PORT_ID: &str = "soma.runner";

pub struct RunnerPort {
    spec: PortSpec,
}

impl RunnerPort {
    pub fn new() -> Self {
        Self { spec: build_spec() }
    }
}

impl Default for RunnerPort {
    fn default() -> Self {
        Self::new()
    }
}

impl Port for RunnerPort {
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
            "exec" => self.run_exec(&input),
            "npm_install" => self.npm_install(&input),
            "npm_test" => self.npm_test(&input),
            "npm_run" => self.npm_run(&input),
            "node_run" => self.node_run(&input),
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
            "exec" => {
                require_field(input, "cwd")?;
                require_field(input, "command")?;
            }
            "npm_install" | "npm_test" => {
                require_field(input, "cwd")?;
            }
            "npm_run" => {
                require_field(input, "cwd")?;
                require_field(input, "script")?;
            }
            "node_run" => {
                require_field(input, "cwd")?;
                require_field(input, "file")?;
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

impl RunnerPort {
    fn run_command(
        &self,
        cwd: &str,
        program: &str,
        args: &[&str],
        env: &HashMap<String, String>,
        timeout_ms: u64,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let mut cmd = Command::new(program);
        cmd.args(args).current_dir(cwd);
        for (k, v) in env {
            cmd.env(k, v);
        }

        let output = cmd
            .output()
            .map_err(|e| PortError::DependencyUnavailable(format!("{program} not found: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        let max_len = 8192;
        let stdout_truncated = if stdout.len() > max_len {
            format!("{}...[truncated, {} total bytes]", &stdout[..max_len], stdout.len())
        } else {
            stdout
        };
        let stderr_truncated = if stderr.len() > max_len {
            format!("{}...[truncated, {} total bytes]", &stderr[..max_len], stderr.len())
        } else {
            stderr
        };

        Ok(serde_json::json!({
            "exit_code": exit_code,
            "success": exit_code == 0,
            "stdout": stdout_truncated,
            "stderr": stderr_truncated,
            "timeout_ms": timeout_ms,
        }))
    }

    // Intentional: this port provides controlled shell execution gated by the
    // runtime's policy engine. It is not user-facing web code.
    fn run_exec(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let cwd = get_str(input, "cwd")?;
        let command = get_str(input, "command")?;
        let timeout_ms = input
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(30000);

        let env_map: HashMap<String, String> = input
            .get("env")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        self.run_command(cwd, "sh", &["-c", command], &env_map, timeout_ms)
    }

    fn npm_install(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let cwd = get_str(input, "cwd")?;
        self.run_command(cwd, "npm", &["install"], &HashMap::new(), 60000)
    }

    fn npm_test(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let cwd = get_str(input, "cwd")?;
        let extra_args: Vec<String> = input
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        let mut args = vec!["test"];
        if !extra_args.is_empty() {
            args.push("--");
            for a in &extra_args {
                args.push(a);
            }
        }

        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_ref()).collect();
        self.run_command(cwd, "npm", &arg_refs, &HashMap::new(), 60000)
    }

    fn npm_run(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let cwd = get_str(input, "cwd")?;
        let script = get_str(input, "script")?;
        self.run_command(cwd, "npm", &["run", script], &HashMap::new(), 60000)
    }

    fn node_run(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let cwd = get_str(input, "cwd")?;
        let file = get_str(input, "file")?;
        self.run_command(cwd, "node", &[file], &HashMap::new(), 30000)
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
    let medium = LatencyProfile {
        expected_latency_ms: 1000,
        p95_latency_ms: 15000,
        max_latency_ms: 60000,
    };
    let exec_cost = CostProfile {
        cpu_cost_class: CostClass::Medium,
        memory_cost_class: CostClass::Medium,
        io_cost_class: CostClass::Medium,
        network_cost_class: CostClass::Low,
        energy_cost_class: CostClass::Low,
    };

    PortSpec {
        port_id: PORT_ID.into(),
        name: "runner".into(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Custom,
        description: "Command execution: shell, npm install/test/run, node run".into(),
        namespace: "soma.runner".into(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![
            PortCapabilitySpec {
                capability_id: "exec".into(),
                name: "exec".into(),
                purpose: "Execute a shell command and capture structured output".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "cwd": {"type": "string", "description": "Working directory"},
                    "command": {"type": "string", "description": "Shell command to execute"},
                    "env": {"type": "object", "description": "Additional environment variables"},
                    "timeout_ms": {"type": "integer", "description": "Timeout in ms (default 30000)"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "exit_code": {"type": "integer"}, "success": {"type": "boolean"},
                    "stdout": {"type": "string"}, "stderr": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Medium,
                latency_profile: medium.clone(),
                cost_profile: exec_cost.clone(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "npm_install".into(),
                name: "npm_install".into(),
                purpose: "Run npm install in a project directory".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "cwd": {"type": "string", "description": "Project directory"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "exit_code": {"type": "integer"}, "success": {"type": "boolean"},
                    "stdout": {"type": "string"}, "stderr": {"type": "string"},
                })),
                effect_class: SideEffectClass::LocalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::PartiallyDeterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Low,
                latency_profile: LatencyProfile {
                    expected_latency_ms: 5000,
                    p95_latency_ms: 30000,
                    max_latency_ms: 60000,
                },
                cost_profile: exec_cost.clone(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "npm_test".into(),
                name: "npm_test".into(),
                purpose: "Run npm test and capture structured test output".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "cwd": {"type": "string", "description": "Project directory"},
                    "args": {"type": "array", "description": "Extra args passed after --"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "exit_code": {"type": "integer"}, "success": {"type": "boolean"},
                    "stdout": {"type": "string"}, "stderr": {"type": "string"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::PartiallyDeterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: medium.clone(),
                cost_profile: exec_cost.clone(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "npm_run".into(),
                name: "npm_run".into(),
                purpose: "Run a named npm script".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "cwd": {"type": "string", "description": "Project directory"},
                    "script": {"type": "string", "description": "npm script name"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "exit_code": {"type": "integer"}, "success": {"type": "boolean"},
                    "stdout": {"type": "string"}, "stderr": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Low,
                latency_profile: medium.clone(),
                cost_profile: exec_cost.clone(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "node_run".into(),
                name: "node_run".into(),
                purpose: "Execute a JavaScript file with Node.js".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "cwd": {"type": "string", "description": "Working directory"},
                    "file": {"type": "string", "description": "JS file to execute"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "exit_code": {"type": "integer"}, "success": {"type": "boolean"},
                    "stdout": {"type": "string"}, "stderr": {"type": "string"},
                })),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Low,
                latency_profile: medium.clone(),
                cost_profile: exec_cost.clone(),
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
            PortFailureClass::Timeout,
        ],
        side_effect_class: SideEffectClass::ExternalStateMutation,
        latency_profile: medium,
        cost_profile: exec_cost,
        auth_requirements: AuthRequirements::default(),
        sandbox_requirements: SandboxRequirements {
            filesystem_access: true,
            network_access: true,
            process_access: true,
            ..Default::default()
        },
        observable_fields: vec![
            "exit_code".into(),
            "success".into(),
        ],
        validation_rules: vec![],
        remote_exposure: false,
    }
}

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    Box::into_raw(Box::new(RunnerPort::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec() {
        let port = RunnerPort::new();
        assert_eq!(port.spec().port_id, "soma.runner");
        assert_eq!(port.spec().capabilities.len(), 5);
    }

    #[test]
    fn test_exec_echo() {
        let port = RunnerPort::new();
        let record = port
            .invoke(
                "exec",
                serde_json::json!({
                    "cwd": env!("CARGO_MANIFEST_DIR"),
                    "command": "echo hello",
                }),
            )
            .unwrap();
        assert!(record.success);
        assert_eq!(record.raw_result["exit_code"], 0);
        assert!(record.raw_result["stdout"]
            .as_str()
            .unwrap()
            .contains("hello"));
    }

    #[test]
    fn test_exec_failing_command() {
        let port = RunnerPort::new();
        let record = port
            .invoke(
                "exec",
                serde_json::json!({
                    "cwd": env!("CARGO_MANIFEST_DIR"),
                    "command": "false",
                }),
            )
            .unwrap();
        assert!(record.success);
        assert_ne!(record.raw_result["exit_code"], 0);
        assert_eq!(record.raw_result["success"], false);
    }
}
