use std::process::Command;
use std::time::Instant;

use soma_port_sdk::prelude::*;

const PORT_ID: &str = "soma.git";

pub struct GitPort {
    spec: PortSpec,
}

impl GitPort {
    pub fn new() -> Self {
        Self { spec: build_spec() }
    }
}

impl Default for GitPort {
    fn default() -> Self {
        Self::new()
    }
}

impl Port for GitPort {
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
            "status" => self.git_status(&input),
            "diff" => self.git_diff(&input),
            "log" => self.git_log(&input),
            "blame" => self.git_blame(&input),
            "branch_list" => self.git_branch_list(&input),
            "changed_files" => self.git_changed_files(&input),
            "add" => self.git_add(&input),
            "commit" => self.git_commit(&input),
            "init" => self.git_init(&input),
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
            "status" | "branch_list" | "changed_files" => {
                require_field(input, "cwd")?;
            }
            "diff" => {
                require_field(input, "cwd")?;
            }
            "log" => {
                require_field(input, "cwd")?;
            }
            "blame" => {
                require_field(input, "cwd")?;
                require_field(input, "file")?;
            }
            "add" => {
                require_field(input, "cwd")?;
                require_field(input, "paths")?;
            }
            "commit" => {
                require_field(input, "cwd")?;
                require_field(input, "message")?;
            }
            "init" => {
                require_field(input, "cwd")?;
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

// ---------------------------------------------------------------------------
// Capability implementations
// ---------------------------------------------------------------------------

impl GitPort {
    fn run_git(&self, cwd: &str, args: &[&str]) -> soma_port_sdk::Result<(String, String, i32)> {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .map_err(|e| PortError::DependencyUnavailable(format!("git not found: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let code = output.status.code().unwrap_or(-1);
        Ok((stdout, stderr, code))
    }

    fn git_status(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let cwd = get_str(input, "cwd")?;
        let (stdout, stderr, code) = self.run_git(cwd, &["status", "--porcelain=v1"])?;

        if code != 0 {
            return Err(PortError::ExternalError(format!("git status failed: {stderr}")));
        }

        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        let mut untracked = Vec::new();

        for line in stdout.lines() {
            if line.len() < 3 {
                continue;
            }
            let index = line.as_bytes()[0];
            let worktree = line.as_bytes()[1];
            let file = line[3..].to_string();

            if index == b'?' {
                untracked.push(file);
            } else {
                if index != b' ' && index != b'?' {
                    staged.push(serde_json::json!({
                        "status": String::from_utf8_lossy(&[index]).to_string(),
                        "file": &file,
                    }));
                }
                if worktree != b' ' && worktree != b'?' {
                    unstaged.push(serde_json::json!({
                        "status": String::from_utf8_lossy(&[worktree]).to_string(),
                        "file": &file,
                    }));
                }
            }
        }

        Ok(serde_json::json!({
            "staged": staged,
            "unstaged": unstaged,
            "untracked": untracked,
            "clean": staged.is_empty() && unstaged.is_empty() && untracked.is_empty(),
        }))
    }

    fn git_diff(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let cwd = get_str(input, "cwd")?;
        let base = input.get("base").and_then(|v| v.as_str());
        let staged = input.get("staged").and_then(|v| v.as_bool()).unwrap_or(false);
        let file = input.get("file").and_then(|v| v.as_str());

        let mut args = vec!["diff"];
        if staged {
            args.push("--cached");
        }
        if let Some(b) = base {
            args.push(b);
        }
        if let Some(f) = file {
            args.push("--");
            args.push(f);
        }

        let (stdout, stderr, code) = self.run_git(cwd, &args)?;
        if code != 0 {
            return Err(PortError::ExternalError(format!("git diff failed: {stderr}")));
        }

        let stat_args = {
            let mut a = vec!["diff", "--stat"];
            if staged {
                a.push("--cached");
            }
            if let Some(b) = base {
                a.push(b);
            }
            a
        };
        let (stat_out, _, _) = self.run_git(cwd, &stat_args)?;

        Ok(serde_json::json!({
            "diff": stdout,
            "stat": stat_out.trim(),
            "empty": stdout.is_empty(),
        }))
    }

    fn git_log(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let cwd = get_str(input, "cwd")?;
        let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);
        let limit_str = format!("-{limit}");

        let (stdout, stderr, code) = self.run_git(
            cwd,
            &["log", &limit_str, "--pretty=format:%H|%an|%ae|%at|%s"],
        )?;
        if code != 0 {
            return Err(PortError::ExternalError(format!("git log failed: {stderr}")));
        }

        let commits: Vec<serde_json::Value> = stdout
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(5, '|').collect();
                if parts.len() == 5 {
                    Some(serde_json::json!({
                        "hash": parts[0],
                        "author_name": parts[1],
                        "author_email": parts[2],
                        "timestamp": parts[3],
                        "subject": parts[4],
                    }))
                } else {
                    None
                }
            })
            .collect();

        Ok(serde_json::json!({
            "commits": commits,
            "count": commits.len(),
        }))
    }

    fn git_blame(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let cwd = get_str(input, "cwd")?;
        let file = get_str(input, "file")?;

        let mut args = vec!["blame", "--porcelain"];

        let line_range;
        if let (Some(start), Some(end)) = (
            input.get("start_line").and_then(|v| v.as_u64()),
            input.get("end_line").and_then(|v| v.as_u64()),
        ) {
            line_range = format!("-L {start},{end}");
            args.push(&line_range);
        }
        args.push(file);

        let (stdout, stderr, code) = self.run_git(cwd, &args)?;
        if code != 0 {
            return Err(PortError::ExternalError(format!("git blame failed: {stderr}")));
        }

        Ok(serde_json::json!({
            "blame": stdout,
            "file": file,
        }))
    }

    fn git_branch_list(
        &self,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let cwd = get_str(input, "cwd")?;
        let (stdout, stderr, code) =
            self.run_git(cwd, &["branch", "--format=%(refname:short)|%(HEAD)"])?;
        if code != 0 {
            return Err(PortError::ExternalError(format!("git branch failed: {stderr}")));
        }

        let mut branches = Vec::new();
        let mut current = String::new();
        for line in stdout.lines() {
            let parts: Vec<&str> = line.splitn(2, '|').collect();
            if parts.len() == 2 {
                let name = parts[0].to_string();
                let is_current = parts[1].trim() == "*";
                if is_current {
                    current = name.clone();
                }
                branches.push(serde_json::json!({
                    "name": name,
                    "current": is_current,
                }));
            }
        }

        Ok(serde_json::json!({
            "branches": branches,
            "current": current,
        }))
    }

    fn git_changed_files(
        &self,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let cwd = get_str(input, "cwd")?;
        let base = input
            .get("base")
            .and_then(|v| v.as_str())
            .unwrap_or("HEAD");

        let (stdout, stderr, code) =
            self.run_git(cwd, &["diff", "--name-status", base])?;
        if code != 0 {
            return Err(PortError::ExternalError(format!(
                "git diff --name-status failed: {stderr}"
            )));
        }

        let files: Vec<serde_json::Value> = stdout
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(2, '\t').collect();
                if parts.len() == 2 {
                    Some(serde_json::json!({
                        "status": parts[0],
                        "file": parts[1],
                    }))
                } else {
                    None
                }
            })
            .collect();

        Ok(serde_json::json!({
            "files": files,
            "count": files.len(),
        }))
    }

    fn git_add(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let cwd = get_str(input, "cwd")?;
        let paths = input
            .get("paths")
            .and_then(|v| v.as_array())
            .ok_or_else(|| PortError::Validation("paths must be an array of strings".into()))?;

        let path_strs: Vec<&str> = paths
            .iter()
            .filter_map(|v| v.as_str())
            .collect();

        let mut args = vec!["add"];
        args.extend(path_strs.iter());

        let (_, stderr, code) = self.run_git(cwd, &args)?;
        if code != 0 {
            return Err(PortError::ExternalError(format!("git add failed: {stderr}")));
        }

        Ok(serde_json::json!({
            "added": path_strs,
            "count": path_strs.len(),
        }))
    }

    fn git_commit(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let cwd = get_str(input, "cwd")?;
        let message = get_str(input, "message")?;

        let (stdout, stderr, code) = self.run_git(cwd, &["commit", "-m", message])?;
        if code != 0 {
            return Err(PortError::ExternalError(format!("git commit failed: {stderr}")));
        }

        let (hash_out, _, _) = self.run_git(cwd, &["rev-parse", "HEAD"])?;

        Ok(serde_json::json!({
            "output": stdout.trim(),
            "hash": hash_out.trim(),
        }))
    }

    fn git_init(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let cwd = get_str(input, "cwd")?;
        let (stdout, stderr, code) = self.run_git(cwd, &["init"])?;
        if code != 0 {
            return Err(PortError::ExternalError(format!("git init failed: {stderr}")));
        }

        Ok(serde_json::json!({
            "output": stdout.trim(),
        }))
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
    let fast = LatencyProfile {
        expected_latency_ms: 50,
        p95_latency_ms: 500,
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
        name: "git".into(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Custom,
        description: "Git operations: status, diff, log, blame, branch, add, commit".into(),
        namespace: "soma.git".into(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![
            PortCapabilitySpec {
                capability_id: "status".into(),
                name: "status".into(),
                purpose: "Get working tree status (staged, unstaged, untracked)".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "cwd": {"type": "string", "description": "Repository path"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "staged": {"type": "array"}, "unstaged": {"type": "array"},
                    "untracked": {"type": "array"}, "clean": {"type": "boolean"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: fast.clone(),
                cost_profile: io_cost.clone(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "diff".into(),
                name: "diff".into(),
                purpose: "Get diff output (optionally staged, against a base, or for a file)"
                    .into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "cwd": {"type": "string", "description": "Repository path"},
                    "base": {"type": "string", "description": "Base ref (e.g. HEAD~1, main)"},
                    "staged": {"type": "boolean", "description": "Show staged changes only"},
                    "file": {"type": "string", "description": "Limit diff to one file"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "diff": {"type": "string"}, "stat": {"type": "string"},
                    "empty": {"type": "boolean"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: fast.clone(),
                cost_profile: io_cost.clone(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "log".into(),
                name: "log".into(),
                purpose: "Get commit log (hash, author, timestamp, subject)".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "cwd": {"type": "string", "description": "Repository path"},
                    "limit": {"type": "integer", "description": "Max commits to return (default 10)"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "commits": {"type": "array"}, "count": {"type": "integer"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: fast.clone(),
                cost_profile: io_cost.clone(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "blame".into(),
                name: "blame".into(),
                purpose: "Get blame info for a file (optionally a line range)".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "cwd": {"type": "string", "description": "Repository path"},
                    "file": {"type": "string", "description": "File path relative to repo root"},
                    "start_line": {"type": "integer", "description": "Start line (1-based)"},
                    "end_line": {"type": "integer", "description": "End line (1-based)"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "blame": {"type": "string"}, "file": {"type": "string"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: fast.clone(),
                cost_profile: io_cost.clone(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "branch_list".into(),
                name: "branch_list".into(),
                purpose: "List branches and identify the current one".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "cwd": {"type": "string", "description": "Repository path"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "branches": {"type": "array"}, "current": {"type": "string"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: fast.clone(),
                cost_profile: io_cost.clone(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "changed_files".into(),
                name: "changed_files".into(),
                purpose: "List files changed relative to a base ref".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "cwd": {"type": "string", "description": "Repository path"},
                    "base": {"type": "string", "description": "Base ref (default HEAD)"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "files": {"type": "array"}, "count": {"type": "integer"},
                })),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Stochastic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: fast.clone(),
                cost_profile: io_cost.clone(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "add".into(),
                name: "add".into(),
                purpose: "Stage files for commit".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "cwd": {"type": "string", "description": "Repository path"},
                    "paths": {"type": "array", "description": "File paths to stage"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "added": {"type": "array"}, "count": {"type": "integer"},
                })),
                effect_class: SideEffectClass::LocalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Low,
                latency_profile: fast.clone(),
                cost_profile: io_cost.clone(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "commit".into(),
                name: "commit".into(),
                purpose: "Create a commit from staged changes".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "cwd": {"type": "string", "description": "Repository path"},
                    "message": {"type": "string", "description": "Commit message"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "output": {"type": "string"}, "hash": {"type": "string"},
                })),
                effect_class: SideEffectClass::LocalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::NonIdempotent,
                risk_class: RiskClass::Low,
                latency_profile: fast.clone(),
                cost_profile: io_cost.clone(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "init".into(),
                name: "init".into(),
                purpose: "Initialize a new git repository".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "cwd": {"type": "string", "description": "Directory to initialize"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "output": {"type": "string"},
                })),
                effect_class: SideEffectClass::LocalStateMutation,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::Deterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Low,
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
            "hash".into(),
            "file".into(),
            "clean".into(),
            "count".into(),
        ],
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
    Box::into_raw(Box::new(GitPort::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec() {
        let port = GitPort::new();
        assert_eq!(port.spec().port_id, "soma.git");
        assert_eq!(port.spec().capabilities.len(), 9);
    }

    #[test]
    fn test_status_in_repo() {
        let port = GitPort::new();
        let record = port
            .invoke(
                "status",
                serde_json::json!({"cwd": env!("CARGO_MANIFEST_DIR")}),
            )
            .unwrap();
        assert!(record.success);
    }

    #[test]
    fn test_log_in_repo() {
        let port = GitPort::new();
        let record = port
            .invoke(
                "log",
                serde_json::json!({"cwd": env!("CARGO_MANIFEST_DIR"), "limit": 3}),
            )
            .unwrap();
        assert!(record.success);
        assert!(record.raw_result["count"].as_u64().unwrap() <= 3);
    }

    #[test]
    fn test_branch_list() {
        let port = GitPort::new();
        let record = port
            .invoke(
                "branch_list",
                serde_json::json!({"cwd": env!("CARGO_MANIFEST_DIR")}),
            )
            .unwrap();
        assert!(record.success);
        assert!(!record.raw_result["current"].as_str().unwrap().is_empty());
    }
}
