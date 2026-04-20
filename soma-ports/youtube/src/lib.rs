use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use semver::Version;
use soma_port_sdk::prelude::*;

pub struct YoutubePort {
    spec: PortSpec,
}

impl Default for YoutubePort {
    fn default() -> Self {
        Self::new()
    }
}

impl YoutubePort {
    pub fn new() -> Self {
        Self {
            spec: Self::build_spec(),
        }
    }

    fn output_dir() -> PathBuf {
        PathBuf::from(
            std::env::var("SOMA_YTDLP_OUTPUT_DIR")
                .unwrap_or_else(|_| "/tmp/soma-ytdlp".to_string()),
        )
    }

    fn ensure_ytdlp() -> std::result::Result<(), PortError> {
        Command::new("yt-dlp")
            .arg("--version")
            .output()
            .map_err(|_| {
                PortError::DependencyUnavailable(
                    "yt-dlp is not installed. Install via: brew install yt-dlp (macOS) or pip install yt-dlp".to_string(),
                )
            })?;
        Ok(())
    }

    fn ensure_output_dir() -> std::result::Result<PathBuf, PortError> {
        let dir = Self::output_dir();
        std::fs::create_dir_all(&dir).map_err(|e| {
            PortError::Internal(format!("failed to create output directory {}: {e}", dir.display()))
        })?;
        Ok(dir)
    }

    fn run_ytdlp(args: &[&str]) -> std::result::Result<String, PortError> {
        let output = Command::new("yt-dlp")
            .args(args)
            .output()
            .map_err(|e| PortError::ExternalError(format!("failed to execute yt-dlp: {e}")))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(PortError::ExternalError(format!("yt-dlp error: {stderr}")))
        }
    }

    fn do_get_info(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, PortError> {
        Self::ensure_ytdlp()?;
        let url = input.get("url").and_then(|v| v.as_str())
            .ok_or_else(|| PortError::Validation("missing 'url' field".into()))?;

        let stdout = Self::run_ytdlp(&["--dump-json", "--no-download", url])?;
        let info: serde_json::Value = serde_json::from_str(&stdout)
            .map_err(|e| PortError::ExternalError(format!("failed to parse yt-dlp JSON: {e}")))?;

        Ok(serde_json::json!({
            "title": info.get("title"),
            "duration": info.get("duration"),
            "uploader": info.get("uploader"),
            "upload_date": info.get("upload_date"),
            "view_count": info.get("view_count"),
            "like_count": info.get("like_count"),
            "description": info.get("description"),
            "thumbnail": info.get("thumbnail"),
            "webpage_url": info.get("webpage_url"),
            "format": info.get("format"),
            "resolution": info.get("resolution"),
            "filesize_approx": info.get("filesize_approx"),
        }))
    }

    fn do_list_formats(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, PortError> {
        Self::ensure_ytdlp()?;
        let url = input.get("url").and_then(|v| v.as_str())
            .ok_or_else(|| PortError::Validation("missing 'url' field".into()))?;

        let stdout = Self::run_ytdlp(&["--dump-json", "--no-download", url])?;
        let info: serde_json::Value = serde_json::from_str(&stdout)
            .map_err(|e| PortError::ExternalError(format!("failed to parse yt-dlp JSON: {e}")))?;

        let formats = info.get("formats").cloned().unwrap_or(serde_json::json!([]));
        let summary: Vec<serde_json::Value> = formats.as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|f| serde_json::json!({
                "format_id": f.get("format_id"),
                "ext": f.get("ext"),
                "resolution": f.get("resolution"),
                "fps": f.get("fps"),
                "vcodec": f.get("vcodec"),
                "acodec": f.get("acodec"),
                "filesize": f.get("filesize"),
                "format_note": f.get("format_note"),
            }))
            .collect();

        Ok(serde_json::json!({ "formats": summary }))
    }

    fn do_download_video(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, PortError> {
        Self::ensure_ytdlp()?;
        let url = input.get("url").and_then(|v| v.as_str())
            .ok_or_else(|| PortError::Validation("missing 'url' field".into()))?;
        let dir = Self::ensure_output_dir()?;

        let format = input.get("format").and_then(|v| v.as_str()).unwrap_or("bestvideo+bestaudio/best");
        let output_template = dir.join("%(title)s.%(ext)s");
        let output_str = output_template.to_string_lossy();

        let mut args = vec![
            "-f", format,
            "-o", &output_str,
            "--print", "after_move:filepath",
            "--no-simulate",
        ];

        let merge_format_val;
        if let Some(merge_format) = input.get("merge_format").and_then(|v| v.as_str()) {
            merge_format_val = merge_format.to_string();
            args.push("--merge-output-format");
            args.push(&merge_format_val);
        }

        args.push(url);
        let stdout = Self::run_ytdlp(&args)?;
        let filepath = stdout.trim().lines().last().unwrap_or("").to_string();

        Ok(serde_json::json!({
            "downloaded": true,
            "filepath": filepath,
            "format": format,
        }))
    }

    fn do_download_audio(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, PortError> {
        Self::ensure_ytdlp()?;
        let url = input.get("url").and_then(|v| v.as_str())
            .ok_or_else(|| PortError::Validation("missing 'url' field".into()))?;
        let dir = Self::ensure_output_dir()?;

        let audio_format = input.get("audio_format").and_then(|v| v.as_str()).unwrap_or("mp3");
        let output_template = dir.join("%(title)s.%(ext)s");
        let output_str = output_template.to_string_lossy();

        let args = vec![
            "-x",
            "--audio-format", audio_format,
            "-o", &output_str,
            "--print", "after_move:filepath",
            "--no-simulate",
            url,
        ];
        let stdout = Self::run_ytdlp(&args)?;
        let filepath = stdout.trim().lines().last().unwrap_or("").to_string();

        Ok(serde_json::json!({
            "downloaded": true,
            "filepath": filepath,
            "audio_format": audio_format,
        }))
    }

    fn build_spec() -> PortSpec {
        let any_schema = SchemaRef { schema: serde_json::json!({ "type": "object" }) };
        let any_output = SchemaRef { schema: serde_json::json!({ "description": "any" }) };

        let fast_latency = LatencyProfile {
            expected_latency_ms: 500,
            p95_latency_ms: 5_000,
            max_latency_ms: 30_000,
        };
        let download_latency = LatencyProfile {
            expected_latency_ms: 5_000,
            p95_latency_ms: 60_000,
            max_latency_ms: 600_000,
        };
        let low_cost = CostProfile {
            cpu_cost_class: CostClass::Low,
            memory_cost_class: CostClass::Low,
            io_cost_class: CostClass::Low,
            network_cost_class: CostClass::Medium,
            energy_cost_class: CostClass::Low,
        };
        let download_cost = CostProfile {
            cpu_cost_class: CostClass::Medium,
            memory_cost_class: CostClass::Medium,
            io_cost_class: CostClass::High,
            network_cost_class: CostClass::High,
            energy_cost_class: CostClass::Medium,
        };

        let capabilities = vec![
            PortCapabilitySpec {
                capability_id: "get_info".to_string(),
                name: "get_info".to_string(),
                purpose: "Fetch video metadata (title, duration, uploader, etc.) without downloading".to_string(),
                input_schema: any_schema.clone(),
                output_schema: any_output.clone(),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::PartiallyDeterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: fast_latency.clone(),
                cost_profile: low_cost.clone(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "list_formats".to_string(),
                name: "list_formats".to_string(),
                purpose: "List available download formats and qualities for a video".to_string(),
                input_schema: any_schema.clone(),
                output_schema: any_output.clone(),
                effect_class: SideEffectClass::ReadOnly,
                rollback_support: RollbackSupport::Irreversible,
                determinism_class: DeterminismClass::PartiallyDeterministic,
                idempotence_class: IdempotenceClass::Idempotent,
                risk_class: RiskClass::Negligible,
                latency_profile: fast_latency.clone(),
                cost_profile: low_cost.clone(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "download_video".to_string(),
                name: "download_video".to_string(),
                purpose: "Download video to disk in best quality (or specified format)".to_string(),
                input_schema: any_schema.clone(),
                output_schema: any_output.clone(),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::PartiallyDeterministic,
                idempotence_class: IdempotenceClass::ConditionallyIdempotent,
                risk_class: RiskClass::Low,
                latency_profile: download_latency.clone(),
                cost_profile: download_cost.clone(),
                remote_exposable: false,
                auth_override: None,
            },
            PortCapabilitySpec {
                capability_id: "download_audio".to_string(),
                name: "download_audio".to_string(),
                purpose: "Download audio-only track, extracting to mp3/m4a/etc.".to_string(),
                input_schema: any_schema.clone(),
                output_schema: any_output.clone(),
                effect_class: SideEffectClass::ExternalStateMutation,
                rollback_support: RollbackSupport::CompensatingAction,
                determinism_class: DeterminismClass::PartiallyDeterministic,
                idempotence_class: IdempotenceClass::ConditionallyIdempotent,
                risk_class: RiskClass::Low,
                latency_profile: download_latency.clone(),
                cost_profile: download_cost,
                remote_exposable: false,
                auth_override: None,
            },
        ];

        PortSpec {
            port_id: "soma.ports.youtube".to_string(),
            name: "youtube".to_string(),
            version: Version::new(0, 1, 0),
            kind: PortKind::Custom,
            description: "YouTube video/audio downloading and metadata via yt-dlp".to_string(),
            namespace: "soma.ports.youtube".to_string(),
            trust_level: TrustLevel::Verified,
            capabilities,
            input_schema: any_schema,
            output_schema: any_output,
            failure_modes: vec![
                PortFailureClass::ValidationError,
                PortFailureClass::DependencyUnavailable,
                PortFailureClass::ExternalError,
                PortFailureClass::Timeout,
            ],
            side_effect_class: SideEffectClass::ExternalStateMutation,
            latency_profile: download_latency,
            cost_profile: low_cost,
            auth_requirements: AuthRequirements {
                methods: vec![AuthMethod::LocalProcessTrust],
                required: false,
            },
            sandbox_requirements: SandboxRequirements {
                filesystem_access: true,
                network_access: true,
                device_access: false,
                process_access: true,
                memory_limit_mb: None,
                cpu_limit_percent: None,
                time_limit_ms: Some(600_000),
                syscall_limit: None,
            },
            observable_fields: vec![],
            validation_rules: vec![],
            remote_exposure: false,
        }
    }
}

impl Port for YoutubePort {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(&self, capability_id: &str, input: serde_json::Value) -> soma_port_sdk::Result<PortCallRecord> {
        let start = Instant::now();
        let result = match capability_id {
            "get_info" => self.do_get_info(&input),
            "list_formats" => self.do_list_formats(&input),
            "download_video" => self.do_download_video(&input),
            "download_audio" => self.do_download_audio(&input),
            other => return Err(PortError::Validation(format!("unknown capability: {other}"))),
        };
        let latency_ms = start.elapsed().as_millis() as u64;
        Ok(match result {
            Ok(v) => PortCallRecord::success(&self.spec.port_id, capability_id, v, latency_ms),
            Err(e) => {
                let fc = e.failure_class();
                PortCallRecord::failure(&self.spec.port_id, capability_id, fc, &e.to_string(), latency_ms)
            }
        })
    }

    fn validate_input(&self, capability_id: &str, input: &serde_json::Value) -> soma_port_sdk::Result<()> {
        if !input.is_object() {
            return Err(PortError::Validation("input must be a JSON object".into()));
        }
        match capability_id {
            "get_info" | "list_formats" | "download_video" | "download_audio" => {
                if input.get("url").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'url' field".into()));
                }
            }
            other => return Err(PortError::Validation(format!("unknown capability: {other}"))),
        }
        Ok(())
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        PortLifecycleState::Active
    }
}

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    let port = YoutubePort::new();
    Box::into_raw(Box::new(port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec_valid() {
        let port = YoutubePort::new();
        let spec = port.spec();
        assert_eq!(spec.port_id, "soma.ports.youtube");
        assert_eq!(spec.capabilities.len(), 4);
        assert!(!spec.failure_modes.is_empty());
        assert!(spec.latency_profile.expected_latency_ms <= spec.latency_profile.p95_latency_ms);
        assert!(spec.latency_profile.p95_latency_ms <= spec.latency_profile.max_latency_ms);
        for cap in &spec.capabilities {
            assert!(cap.latency_profile.expected_latency_ms <= cap.latency_profile.p95_latency_ms);
            assert!(cap.latency_profile.p95_latency_ms <= cap.latency_profile.max_latency_ms);
        }
    }

    #[test]
    fn test_validate_input_missing_url() {
        let port = YoutubePort::new();
        let input = serde_json::json!({});
        assert!(port.validate_input("get_info", &input).is_err());
        assert!(port.validate_input("download_video", &input).is_err());
    }

    #[test]
    fn test_validate_input_valid() {
        let port = YoutubePort::new();
        let input = serde_json::json!({"url": "https://youtube.com/watch?v=test"});
        assert!(port.validate_input("get_info", &input).is_ok());
        assert!(port.validate_input("list_formats", &input).is_ok());
        assert!(port.validate_input("download_video", &input).is_ok());
        assert!(port.validate_input("download_audio", &input).is_ok());
    }

    #[test]
    fn test_validate_input_unknown_capability() {
        let port = YoutubePort::new();
        let input = serde_json::json!({"url": "https://youtube.com/watch?v=test"});
        assert!(port.validate_input("nonexistent", &input).is_err());
    }

    #[test]
    fn test_capability_ids_unique() {
        let port = YoutubePort::new();
        let ids: Vec<&str> = port.spec().capabilities.iter().map(|c| c.capability_id.as_str()).collect();
        let mut deduped = ids.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(ids.len(), deduped.len());
    }
}
