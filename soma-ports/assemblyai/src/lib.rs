use std::sync::OnceLock;
use std::time::Instant;

use semver::Version;
use soma_port_sdk::prelude::*;

type Res<T> = soma_port_sdk::Result<T>;

const DEFAULT_BASE_URL: &str = "https://api.assemblyai.com";

pub struct AssemblyAiPort {
    spec: PortSpec,
    runtime: OnceLock<tokio::runtime::Runtime>,
    base_url: OnceLock<String>,
    api_key: OnceLock<String>,
}

impl Default for AssemblyAiPort {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
struct Cap {
    effect: SideEffectClass,
    rollback: RollbackSupport,
    determinism: DeterminismClass,
    idempotence: IdempotenceClass,
    risk: RiskClass,
}

impl AssemblyAiPort {
    pub fn new() -> Self {
        Self {
            spec: Self::build_spec(),
            runtime: OnceLock::new(),
            base_url: OnceLock::new(),
            api_key: OnceLock::new(),
        }
    }

    fn rt(&self) -> &tokio::runtime::Runtime {
        self.runtime.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to create tokio runtime")
        })
    }

    fn base_url(&self) -> &str {
        self.base_url.get_or_init(|| {
            std::env::var("SOMA_ASSEMBLYAI_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string())
        })
    }

    fn api_key(&self) -> Res<&str> {
        self.api_key
            .get_or_init(|| std::env::var("SOMA_ASSEMBLYAI_API_KEY").unwrap_or_default());
        let key = self.api_key.get().unwrap();
        if key.is_empty() {
            return Err(PortError::AuthorizationDenied(
                "SOMA_ASSEMBLYAI_API_KEY not set".into(),
            ));
        }
        Ok(key.as_str())
    }

    fn client(&self) -> Res<reqwest::Client> {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| PortError::Internal(format!("failed to create HTTP client: {e}")))
    }

    // ------------------------------------------------------------------
    // Capability handlers
    // ------------------------------------------------------------------

    fn do_upload(&self, input: &serde_json::Value) -> Res<serde_json::Value> {
        let audio_data_url = input
            .get("audio_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PortError::Validation("missing 'audio_url' field".into()))?;

        let api_key = self.api_key()?;
        let client = self.client()?;
        let base = self.base_url();

        self.rt().block_on(async {
            let fetch_resp = client
                .get(audio_data_url)
                .send()
                .await
                .map_err(|e: reqwest::Error| PortError::TransportError(format!("failed to fetch audio: {e}")))?;
            let audio_bytes = fetch_resp
                .bytes()
                .await
                .map_err(|e: reqwest::Error| PortError::TransportError(format!("failed to read audio bytes: {e}")))?;

            send_and_parse(
                client
                    .post(format!("{base}/v2/upload"))
                    .header("Authorization", api_key)
                    .header("Content-Type", "application/octet-stream")
                    .body(audio_bytes),
            )
            .await
        })
    }

    fn do_transcribe(&self, input: &serde_json::Value) -> Res<serde_json::Value> {
        let api_key = self.api_key()?;
        let client = self.client()?;
        let base = self.base_url();

        if input.get("audio_url").and_then(|v| v.as_str()).is_none() {
            return Err(PortError::Validation("missing 'audio_url' field".into()));
        }

        self.rt().block_on(send_and_parse(
            client
                .post(format!("{base}/v2/transcript"))
                .header("Authorization", api_key)
                .json(input),
        ))
    }

    fn do_get_transcript(&self, input: &serde_json::Value) -> Res<serde_json::Value> {
        let id = require_str(input, "transcript_id")?;
        let api_key = self.api_key()?;
        let client = self.client()?;
        let base = self.base_url();

        self.rt().block_on(send_and_parse(
            client
                .get(format!("{base}/v2/transcript/{id}"))
                .header("Authorization", api_key),
        ))
    }

    fn do_get_sentences(&self, input: &serde_json::Value) -> Res<serde_json::Value> {
        let id = require_str(input, "transcript_id")?;
        let api_key = self.api_key()?;
        let client = self.client()?;
        let base = self.base_url();

        self.rt().block_on(send_and_parse(
            client
                .get(format!("{base}/v2/transcript/{id}/sentences"))
                .header("Authorization", api_key),
        ))
    }

    fn do_get_paragraphs(&self, input: &serde_json::Value) -> Res<serde_json::Value> {
        let id = require_str(input, "transcript_id")?;
        let api_key = self.api_key()?;
        let client = self.client()?;
        let base = self.base_url();

        self.rt().block_on(send_and_parse(
            client
                .get(format!("{base}/v2/transcript/{id}/paragraphs"))
                .header("Authorization", api_key),
        ))
    }

    fn do_get_subtitles(&self, input: &serde_json::Value) -> Res<serde_json::Value> {
        let id = require_str(input, "transcript_id")?;
        let sub_format = input
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("srt");

        if sub_format != "srt" && sub_format != "vtt" {
            return Err(PortError::Validation(
                "format must be 'srt' or 'vtt'".into(),
            ));
        }

        let api_key = self.api_key()?;
        let client = self.client()?;
        let base = self.base_url();

        self.rt().block_on(async {
            let resp = client
                .get(format!("{base}/v2/transcript/{id}/{sub_format}"))
                .header("Authorization", api_key)
                .send()
                .await
                .map_err(|e: reqwest::Error| PortError::TransportError(e.to_string()))?;

            let status = resp.status();
            let body = resp
                .text()
                .await
                .map_err(|e: reqwest::Error| PortError::TransportError(e.to_string()))?;

            if !status.is_success() {
                return Err(PortError::ExternalError(format!(
                    "AssemblyAI returned {status}: {body}"
                )));
            }

            Ok(serde_json::json!({
                "format": sub_format,
                "content": body,
            }))
        })
    }

    fn do_word_search(&self, input: &serde_json::Value) -> Res<serde_json::Value> {
        let id = require_str(input, "transcript_id")?;
        let words = input
            .get("words")
            .and_then(|v| v.as_array())
            .ok_or_else(|| PortError::Validation("missing 'words' array".into()))?;

        let words_csv: Vec<&str> = words.iter().filter_map(|v| v.as_str()).collect();
        if words_csv.is_empty() {
            return Err(PortError::Validation(
                "'words' must contain string values".into(),
            ));
        }

        let api_key = self.api_key()?;
        let client = self.client()?;
        let base = self.base_url();
        let query = words_csv.join(",");

        self.rt().block_on(send_and_parse(
            client
                .get(format!("{base}/v2/transcript/{id}/word-search"))
                .query(&[("words", query.as_str())])
                .header("Authorization", api_key),
        ))
    }

    fn do_list_transcripts(&self, input: &serde_json::Value) -> Res<serde_json::Value> {
        let api_key = self.api_key()?;
        let client = self.client()?;
        let base = self.base_url();

        let mut query_params: Vec<(&str, String)> = Vec::new();
        if let Some(limit) = input.get("limit").and_then(|v| v.as_u64()) {
            query_params.push(("limit", limit.to_string()));
        }
        if let Some(status) = input.get("status").and_then(|v| v.as_str()) {
            query_params.push(("status", status.to_string()));
        }
        if let Some(before_id) = input.get("before_id").and_then(|v| v.as_str()) {
            query_params.push(("before_id", before_id.to_string()));
        }
        if let Some(after_id) = input.get("after_id").and_then(|v| v.as_str()) {
            query_params.push(("after_id", after_id.to_string()));
        }

        self.rt().block_on(send_and_parse(
            client
                .get(format!("{base}/v2/transcript"))
                .query(&query_params)
                .header("Authorization", api_key),
        ))
    }

    fn do_delete_transcript(&self, input: &serde_json::Value) -> Res<serde_json::Value> {
        let id = require_str(input, "transcript_id")?;
        let api_key = self.api_key()?;
        let client = self.client()?;
        let base = self.base_url();

        self.rt().block_on(send_and_parse(
            client
                .delete(format!("{base}/v2/transcript/{id}"))
                .header("Authorization", api_key),
        ))
    }

    fn do_get_redacted_audio(&self, input: &serde_json::Value) -> Res<serde_json::Value> {
        let id = require_str(input, "transcript_id")?;
        let api_key = self.api_key()?;
        let client = self.client()?;
        let base = self.base_url();

        self.rt().block_on(send_and_parse(
            client
                .get(format!("{base}/v2/transcript/{id}/redacted-audio"))
                .header("Authorization", api_key),
        ))
    }

    // ------------------------------------------------------------------
    // Spec builder
    // ------------------------------------------------------------------

    fn build_spec() -> PortSpec {
        let api_latency = LatencyProfile {
            expected_latency_ms: 500,
            p95_latency_ms: 5_000,
            max_latency_ms: 120_000,
        };

        let net_cost = CostProfile {
            cpu_cost_class: CostClass::Low,
            memory_cost_class: CostClass::Low,
            io_cost_class: CostClass::Low,
            network_cost_class: CostClass::Medium,
            energy_cost_class: CostClass::Low,
        };

        let fast_latency = LatencyProfile {
            expected_latency_ms: 200,
            p95_latency_ms: 2_000,
            max_latency_ms: 30_000,
        };

        let upload_latency = LatencyProfile {
            expected_latency_ms: 1_000,
            p95_latency_ms: 10_000,
            max_latency_ms: 120_000,
        };

        let upload_cost = CostProfile {
            cpu_cost_class: CostClass::Low,
            memory_cost_class: CostClass::Medium,
            io_cost_class: CostClass::Medium,
            network_cost_class: CostClass::High,
            energy_cost_class: CostClass::Low,
        };

        let capabilities = vec![
            make_cap(
                "upload",
                "Upload a local audio file via URL and receive a temporary AssemblyAI-hosted URL",
                Cap {
                    effect: SideEffectClass::ExternalStateMutation,
                    rollback: RollbackSupport::Irreversible,
                    determinism: DeterminismClass::Deterministic,
                    idempotence: IdempotenceClass::NonIdempotent,
                    risk: RiskClass::Low,
                },
                &upload_latency,
                &upload_cost,
            ),
            make_cap(
                "transcribe",
                "Submit audio for transcription with optional audio intelligence features (speaker labels, sentiment, entities, topics, PII redaction, chapters, highlights, content safety)",
                Cap {
                    effect: SideEffectClass::ExternalStateMutation,
                    rollback: RollbackSupport::CompensatingAction,
                    determinism: DeterminismClass::Stochastic,
                    idempotence: IdempotenceClass::NonIdempotent,
                    risk: RiskClass::Low,
                },
                &api_latency,
                &net_cost,
            ),
            make_cap(
                "get_transcript",
                "Poll transcript status and retrieve results including all audio intelligence outputs",
                Cap {
                    effect: SideEffectClass::ReadOnly,
                    rollback: RollbackSupport::Irreversible,
                    determinism: DeterminismClass::PartiallyDeterministic,
                    idempotence: IdempotenceClass::Idempotent,
                    risk: RiskClass::Negligible,
                },
                &fast_latency,
                &net_cost,
            ),
            make_cap(
                "get_sentences",
                "Retrieve transcript segmented by sentences with timing data",
                Cap {
                    effect: SideEffectClass::ReadOnly,
                    rollback: RollbackSupport::Irreversible,
                    determinism: DeterminismClass::Deterministic,
                    idempotence: IdempotenceClass::Idempotent,
                    risk: RiskClass::Negligible,
                },
                &fast_latency,
                &net_cost,
            ),
            make_cap(
                "get_paragraphs",
                "Retrieve transcript segmented by paragraphs with timing data",
                Cap {
                    effect: SideEffectClass::ReadOnly,
                    rollback: RollbackSupport::Irreversible,
                    determinism: DeterminismClass::Deterministic,
                    idempotence: IdempotenceClass::Idempotent,
                    risk: RiskClass::Negligible,
                },
                &fast_latency,
                &net_cost,
            ),
            make_cap(
                "get_subtitles",
                "Export transcript as SRT or VTT subtitle format",
                Cap {
                    effect: SideEffectClass::ReadOnly,
                    rollback: RollbackSupport::Irreversible,
                    determinism: DeterminismClass::Deterministic,
                    idempotence: IdempotenceClass::Idempotent,
                    risk: RiskClass::Negligible,
                },
                &fast_latency,
                &net_cost,
            ),
            make_cap(
                "word_search",
                "Search transcript for specific words and get timestamps",
                Cap {
                    effect: SideEffectClass::ReadOnly,
                    rollback: RollbackSupport::Irreversible,
                    determinism: DeterminismClass::Deterministic,
                    idempotence: IdempotenceClass::Idempotent,
                    risk: RiskClass::Negligible,
                },
                &fast_latency,
                &net_cost,
            ),
            make_cap(
                "list_transcripts",
                "List recent transcripts with optional status and pagination filters",
                Cap {
                    effect: SideEffectClass::ReadOnly,
                    rollback: RollbackSupport::Irreversible,
                    determinism: DeterminismClass::PartiallyDeterministic,
                    idempotence: IdempotenceClass::Idempotent,
                    risk: RiskClass::Negligible,
                },
                &fast_latency,
                &net_cost,
            ),
            make_cap(
                "delete_transcript",
                "Permanently remove a transcript and its associated data",
                Cap {
                    effect: SideEffectClass::Destructive,
                    rollback: RollbackSupport::Irreversible,
                    determinism: DeterminismClass::Deterministic,
                    idempotence: IdempotenceClass::Idempotent,
                    risk: RiskClass::Medium,
                },
                &fast_latency,
                &net_cost,
            ),
            make_cap(
                "get_redacted_audio",
                "Retrieve PII-redacted audio URL for a transcript with redact_pii_audio enabled",
                Cap {
                    effect: SideEffectClass::ReadOnly,
                    rollback: RollbackSupport::Irreversible,
                    determinism: DeterminismClass::Deterministic,
                    idempotence: IdempotenceClass::Idempotent,
                    risk: RiskClass::Negligible,
                },
                &fast_latency,
                &net_cost,
            ),
        ];

        PortSpec {
            port_id: "soma.ports.assemblyai".to_string(),
            name: "assemblyai".to_string(),
            version: Version::new(0, 1, 0),
            kind: PortKind::Http,
            description: "AssemblyAI speech-to-text and audio intelligence: transcription, speaker diarization, sentiment analysis, entity detection, topic classification, PII redaction, content safety, subtitles".to_string(),
            namespace: "soma.ports.assemblyai".to_string(),
            trust_level: TrustLevel::Verified,
            capabilities,
            input_schema: SchemaRef { schema: serde_json::json!({"type": "object"}) },
            output_schema: SchemaRef { schema: serde_json::json!({"description": "any"}) },
            failure_modes: vec![
                PortFailureClass::ValidationError,
                PortFailureClass::AuthorizationDenied,
                PortFailureClass::DependencyUnavailable,
                PortFailureClass::TransportError,
                PortFailureClass::ExternalError,
                PortFailureClass::Timeout,
            ],
            side_effect_class: SideEffectClass::ExternalStateMutation,
            latency_profile: api_latency,
            cost_profile: net_cost,
            auth_requirements: AuthRequirements {
                methods: vec![AuthMethod::ApiKey],
                required: true,
            },
            sandbox_requirements: SandboxRequirements {
                filesystem_access: false,
                network_access: true,
                device_access: false,
                process_access: false,
                memory_limit_mb: None,
                cpu_limit_percent: None,
                time_limit_ms: Some(120_000),
                syscall_limit: None,
            },
            observable_fields: vec![
                "transcript_id".to_string(),
                "status".to_string(),
                "audio_duration".to_string(),
                "confidence".to_string(),
            ],
            validation_rules: vec![],
            remote_exposure: false,
        }
    }
}

// ------------------------------------------------------------------
// Helpers
// ------------------------------------------------------------------

fn require_str<'a>(input: &'a serde_json::Value, field: &str) -> Res<&'a str> {
    input
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| PortError::Validation(format!("missing '{field}' field")))
}

async fn send_and_parse(req: reqwest::RequestBuilder) -> Res<serde_json::Value> {
    let resp = req
        .send()
        .await
        .map_err(|e: reqwest::Error| PortError::TransportError(e.to_string()))?;
    parse_response(resp).await
}

async fn parse_response(resp: reqwest::Response) -> Res<serde_json::Value> {
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| PortError::TransportError(format!("failed to read response: {e}")))?;

    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(PortError::AuthorizationDenied(format!(
            "AssemblyAI auth failed ({status}): {body}"
        )));
    }

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(PortError::ExternalError(format!(
            "AssemblyAI rate limited: {body}"
        )));
    }

    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(PortError::NotFound(format!(
            "AssemblyAI resource not found: {body}"
        )));
    }

    if !status.is_success() {
        return Err(PortError::ExternalError(format!(
            "AssemblyAI returned {status}: {body}"
        )));
    }

    serde_json::from_str(&body)
        .map_err(|e| PortError::Internal(format!("failed to parse response JSON: {e}")))
}

fn make_cap(
    id: &str,
    purpose: &str,
    cap: Cap,
    latency: &LatencyProfile,
    cost: &CostProfile,
) -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: id.to_string(),
        name: id.to_string(),
        purpose: purpose.to_string(),
        input_schema: SchemaRef { schema: serde_json::json!({"type": "object"}) },
        output_schema: SchemaRef { schema: serde_json::json!({"description": "any"}) },
        effect_class: cap.effect,
        rollback_support: cap.rollback,
        determinism_class: cap.determinism,
        idempotence_class: cap.idempotence,
        risk_class: cap.risk,
        latency_profile: latency.clone(),
        cost_profile: cost.clone(),
        remote_exposable: false,
        auth_override: None,
    }
}

// ------------------------------------------------------------------
// Port trait
// ------------------------------------------------------------------

impl Port for AssemblyAiPort {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(&self, capability_id: &str, input: serde_json::Value) -> Res<PortCallRecord> {
        let start = Instant::now();
        let result = match capability_id {
            "upload" => self.do_upload(&input),
            "transcribe" => self.do_transcribe(&input),
            "get_transcript" => self.do_get_transcript(&input),
            "get_sentences" => self.do_get_sentences(&input),
            "get_paragraphs" => self.do_get_paragraphs(&input),
            "get_subtitles" => self.do_get_subtitles(&input),
            "word_search" => self.do_word_search(&input),
            "list_transcripts" => self.do_list_transcripts(&input),
            "delete_transcript" => self.do_delete_transcript(&input),
            "get_redacted_audio" => self.do_get_redacted_audio(&input),
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

    fn validate_input(&self, capability_id: &str, input: &serde_json::Value) -> Res<()> {
        if !input.is_object() {
            return Err(PortError::Validation("input must be a JSON object".into()));
        }
        match capability_id {
            "upload" => {
                require_str(input, "audio_url")?;
            }
            "transcribe" => {
                require_str(input, "audio_url")?;
            }
            "get_transcript" | "get_sentences" | "get_paragraphs" | "get_redacted_audio" => {
                require_str(input, "transcript_id")?;
            }
            "get_subtitles" => {
                require_str(input, "transcript_id")?;
                if let Some(fmt) = input.get("format").and_then(|v| v.as_str())
                    && fmt != "srt" && fmt != "vtt"
                {
                    return Err(PortError::Validation("format must be 'srt' or 'vtt'".into()));
                }
            }
            "word_search" => {
                require_str(input, "transcript_id")?;
                if input.get("words").and_then(|v| v.as_array()).is_none() {
                    return Err(PortError::Validation("missing 'words' array".into()));
                }
            }
            "list_transcripts" => {}
            "delete_transcript" => {
                require_str(input, "transcript_id")?;
            }
            other => {
                return Err(PortError::Validation(format!("unknown capability: {other}")));
            }
        }
        Ok(())
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        PortLifecycleState::Active
    }
}

// ------------------------------------------------------------------
// C ABI export
// ------------------------------------------------------------------

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    let port = AssemblyAiPort::new();
    Box::into_raw(Box::new(port))
}

// ------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec_valid() {
        let port = AssemblyAiPort::new();
        let spec = port.spec();

        assert_eq!(spec.port_id, "soma.ports.assemblyai");
        assert_eq!(spec.capabilities.len(), 10);
        assert!(!spec.failure_modes.is_empty());
        assert!(spec.latency_profile.expected_latency_ms <= spec.latency_profile.p95_latency_ms);
        assert!(spec.latency_profile.p95_latency_ms <= spec.latency_profile.max_latency_ms);

        let ids: Vec<&str> = spec.capabilities.iter().map(|c| c.capability_id.as_str()).collect();
        assert!(ids.contains(&"upload"));
        assert!(ids.contains(&"transcribe"));
        assert!(ids.contains(&"get_transcript"));
        assert!(ids.contains(&"get_sentences"));
        assert!(ids.contains(&"get_paragraphs"));
        assert!(ids.contains(&"get_subtitles"));
        assert!(ids.contains(&"word_search"));
        assert!(ids.contains(&"list_transcripts"));
        assert!(ids.contains(&"delete_transcript"));
        assert!(ids.contains(&"get_redacted_audio"));
    }

    #[test]
    fn test_capability_latency_invariant() {
        let port = AssemblyAiPort::new();
        for cap in &port.spec().capabilities {
            assert!(
                cap.latency_profile.expected_latency_ms <= cap.latency_profile.p95_latency_ms,
                "{}: expected <= p95", cap.capability_id
            );
            assert!(
                cap.latency_profile.p95_latency_ms <= cap.latency_profile.max_latency_ms,
                "{}: p95 <= max", cap.capability_id
            );
        }
    }

    #[test]
    fn test_unique_capability_ids() {
        let port = AssemblyAiPort::new();
        let ids: Vec<&str> = port.spec().capabilities.iter().map(|c| c.capability_id.as_str()).collect();
        let mut deduped = ids.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(ids.len(), deduped.len());
    }

    #[test]
    fn test_validate_input_rejects_missing_fields() {
        let port = AssemblyAiPort::new();
        let empty = serde_json::json!({});

        assert!(port.validate_input("upload", &empty).is_err());
        assert!(port.validate_input("transcribe", &empty).is_err());
        assert!(port.validate_input("get_transcript", &empty).is_err());
        assert!(port.validate_input("word_search", &empty).is_err());
        assert!(port.validate_input("delete_transcript", &empty).is_err());
        assert!(port.validate_input("list_transcripts", &empty).is_ok());
    }

    #[test]
    fn test_validate_input_unknown_capability() {
        let port = AssemblyAiPort::new();
        assert!(port.validate_input("nonexistent", &serde_json::json!({})).is_err());
    }

    #[test]
    fn test_destructive_cap_has_appropriate_risk() {
        let port = AssemblyAiPort::new();
        let delete_cap = port.spec().capabilities.iter().find(|c| c.capability_id == "delete_transcript").unwrap();
        assert_eq!(delete_cap.effect_class, SideEffectClass::Destructive);
        assert!(matches!(delete_cap.risk_class, RiskClass::Medium | RiskClass::High | RiskClass::Critical));
    }
}
