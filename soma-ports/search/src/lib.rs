use std::process::Command;
use std::time::Instant;

use soma_port_sdk::prelude::*;

const PORT_ID: &str = "soma.search";

pub struct SearchPort {
    spec: PortSpec,
}

impl SearchPort {
    pub fn new() -> Self {
        Self { spec: build_spec() }
    }
}

impl Default for SearchPort {
    fn default() -> Self {
        Self::new()
    }
}

impl Port for SearchPort {
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
            "text_search" => self.text_search(&input),
            "file_search" => self.file_search(&input),
            "symbol_search" => self.symbol_search(&input),
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
            "text_search" | "symbol_search" => {
                require_field(input, "cwd")?;
                require_field(input, "pattern")?;
            }
            "file_search" => {
                require_field(input, "cwd")?;
                require_field(input, "glob")?;
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

impl SearchPort {
    fn run_rg(&self, cwd: &str, args: &[&str]) -> soma_port_sdk::Result<(String, String, i32)> {
        let output = Command::new("rg")
            .args(args)
            .current_dir(cwd)
            .output()
            .map_err(|e| PortError::DependencyUnavailable(format!("rg (ripgrep) not found: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let code = output.status.code().unwrap_or(-1);
        Ok((stdout, stderr, code))
    }

    fn text_search(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let cwd = get_str(input, "cwd")?;
        let pattern = get_str(input, "pattern")?;
        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(50);
        let glob_filter = input.get("glob").and_then(|v| v.as_str());
        let case_insensitive = input
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let max_str = max_results.to_string();
        let mut args = vec![
            "--json",
            "--max-count",
            &max_str,
        ];
        if case_insensitive {
            args.push("-i");
        }
        if let Some(g) = glob_filter {
            args.push("--glob");
            args.push(g);
        }
        args.push(pattern);

        let (stdout, stderr, code) = self.run_rg(cwd, &args)?;

        // code 1 = no matches (not an error)
        if code > 1 {
            return Err(PortError::ExternalError(format!("rg failed: {stderr}")));
        }

        let mut matches = Vec::new();
        for line in stdout.lines() {
            if let Ok(obj) = serde_json::from_str::<serde_json::Value>(line) {
                if obj.get("type").and_then(|t| t.as_str()) == Some("match") {
                    let data = &obj["data"];
                    matches.push(serde_json::json!({
                        "file": data["path"]["text"],
                        "line_number": data["line_number"],
                        "text": data["lines"]["text"],
                    }));
                }
            }
        }

        Ok(serde_json::json!({
            "matches": matches,
            "count": matches.len(),
            "pattern": pattern,
        }))
    }

    fn file_search(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let cwd = get_str(input, "cwd")?;
        let glob = get_str(input, "glob")?;

        let (stdout, stderr, code) = self.run_rg(cwd, &["--files", "--glob", glob])?;

        if code > 1 {
            return Err(PortError::ExternalError(format!("rg --files failed: {stderr}")));
        }

        let files: Vec<&str> = stdout.lines().collect();

        Ok(serde_json::json!({
            "files": files,
            "count": files.len(),
            "glob": glob,
        }))
    }

    fn symbol_search(
        &self,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<serde_json::Value> {
        let cwd = get_str(input, "cwd")?;
        let pattern = get_str(input, "pattern")?;
        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(30);

        // Search for symbol definitions: function/class/const/export patterns
        let sym_pattern = format!(
            r"(function\s+{p}|class\s+{p}|const\s+{p}|let\s+{p}|var\s+{p}|module\.exports\.\s*{p}|exports\.{p}|{p}\s*[:=]\s*(function|async|\())",
            p = pattern
        );

        let max_str = max_results.to_string();
        let (stdout, stderr, code) = self.run_rg(
            cwd,
            &["--json", "--max-count", &max_str, &sym_pattern],
        )?;

        if code > 1 {
            return Err(PortError::ExternalError(format!(
                "rg symbol search failed: {stderr}"
            )));
        }

        let mut symbols = Vec::new();
        for line in stdout.lines() {
            if let Ok(obj) = serde_json::from_str::<serde_json::Value>(line) {
                if obj.get("type").and_then(|t| t.as_str()) == Some("match") {
                    let data = &obj["data"];
                    symbols.push(serde_json::json!({
                        "file": data["path"]["text"],
                        "line_number": data["line_number"],
                        "text": data["lines"]["text"],
                    }));
                }
            }
        }

        Ok(serde_json::json!({
            "symbols": symbols,
            "count": symbols.len(),
            "pattern": pattern,
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
        expected_latency_ms: 30,
        p95_latency_ms: 300,
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
        name: "search".into(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Custom,
        description: "Code search via ripgrep: text, file glob, and symbol search".into(),
        namespace: "soma.search".into(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![
            PortCapabilitySpec {
                capability_id: "text_search".into(),
                name: "text_search".into(),
                purpose: "Search for a regex pattern in file contents".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "cwd": {"type": "string", "description": "Directory to search in"},
                    "pattern": {"type": "string", "description": "Regex pattern"},
                    "max_results": {"type": "integer", "description": "Max matches per file (default 50)"},
                    "glob": {"type": "string", "description": "File glob filter (e.g. *.js)"},
                    "case_insensitive": {"type": "boolean", "description": "Case insensitive search"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "matches": {"type": "array"}, "count": {"type": "integer"},
                    "pattern": {"type": "string"},
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
                capability_id: "file_search".into(),
                name: "file_search".into(),
                purpose: "Find files matching a glob pattern".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "cwd": {"type": "string", "description": "Directory to search in"},
                    "glob": {"type": "string", "description": "Glob pattern (e.g. **/*.js)"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "files": {"type": "array"}, "count": {"type": "integer"},
                    "glob": {"type": "string"},
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
                capability_id: "symbol_search".into(),
                name: "symbol_search".into(),
                purpose: "Search for symbol definitions (function, class, const, export)".into(),
                input_schema: SchemaRef::object(serde_json::json!({
                    "cwd": {"type": "string", "description": "Directory to search in"},
                    "pattern": {"type": "string", "description": "Symbol name or pattern"},
                    "max_results": {"type": "integer", "description": "Max results (default 30)"},
                })),
                output_schema: SchemaRef::object(serde_json::json!({
                    "symbols": {"type": "array"}, "count": {"type": "integer"},
                    "pattern": {"type": "string"},
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
        ],
        input_schema: SchemaRef::any(),
        output_schema: SchemaRef::any(),
        failure_modes: vec![
            PortFailureClass::ValidationError,
            PortFailureClass::ExternalError,
            PortFailureClass::DependencyUnavailable,
        ],
        side_effect_class: SideEffectClass::ReadOnly,
        latency_profile: fast,
        cost_profile: io_cost,
        auth_requirements: AuthRequirements::default(),
        sandbox_requirements: SandboxRequirements {
            filesystem_access: true,
            process_access: true,
            ..Default::default()
        },
        observable_fields: vec![
            "count".into(),
            "pattern".into(),
            "file".into(),
        ],
        validation_rules: vec![],
        remote_exposure: false,
    }
}

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    Box::into_raw(Box::new(SearchPort::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec() {
        let port = SearchPort::new();
        assert_eq!(port.spec().port_id, "soma.search");
        assert_eq!(port.spec().capabilities.len(), 3);
    }

    #[test]
    fn test_file_search() {
        let port = SearchPort::new();
        let record = port
            .invoke(
                "file_search",
                serde_json::json!({
                    "cwd": env!("CARGO_MANIFEST_DIR"),
                    "glob": "*.toml",
                }),
            )
            .unwrap();
        assert!(record.success);
        assert!(record.raw_result["count"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn test_text_search() {
        let port = SearchPort::new();
        let record = port
            .invoke(
                "text_search",
                serde_json::json!({
                    "cwd": env!("CARGO_MANIFEST_DIR"),
                    "pattern": "soma_port_init",
                }),
            )
            .unwrap();
        assert!(record.success);
    }
}
