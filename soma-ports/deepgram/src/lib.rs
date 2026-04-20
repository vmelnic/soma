use std::sync::OnceLock;
use std::time::Instant;

use base64::Engine;
use semver::Version;
use soma_port_sdk::prelude::*;

type Res<T> = soma_port_sdk::Result<T>;

const DEFAULT_BASE_URL: &str = "https://api.deepgram.com";

pub struct DeepgramPort {
    spec: PortSpec,
    runtime: OnceLock<tokio::runtime::Runtime>,
    base_url: OnceLock<String>,
    api_key: OnceLock<String>,
}

impl Default for DeepgramPort {
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

impl DeepgramPort {
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
            std::env::var("SOMA_DEEPGRAM_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string())
        })
    }

    fn api_key(&self) -> Res<&str> {
        self.api_key
            .get_or_init(|| std::env::var("SOMA_DEEPGRAM_API_KEY").unwrap_or_default());
        let key = self.api_key.get().unwrap();
        if key.is_empty() {
            return Err(PortError::AuthorizationDenied(
                "SOMA_DEEPGRAM_API_KEY not set".into(),
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
    // Capability: transcribe (POST /v1/listen)
    // ------------------------------------------------------------------

    fn do_transcribe(&self, input: &serde_json::Value) -> Res<serde_json::Value> {
        let url = require_str(input, "url")?;
        let api_key = self.api_key()?;
        let client = self.client()?;
        let base = self.base_url();

        let mut query_params: Vec<(&str, String)> = Vec::new();

        if let Some(model) = input.get("model").and_then(|v| v.as_str()) {
            query_params.push(("model", model.to_string()));
        }
        if let Some(language) = input.get("language").and_then(|v| v.as_str()) {
            query_params.push(("language", language.to_string()));
        }
        if let Some(v) = input.get("punctuate").and_then(|v| v.as_bool()) {
            query_params.push(("punctuate", v.to_string()));
        }
        if let Some(v) = input.get("diarize").and_then(|v| v.as_bool()) {
            query_params.push(("diarize", v.to_string()));
        }
        if let Some(v) = input.get("smart_format").and_then(|v| v.as_bool()) {
            query_params.push(("smart_format", v.to_string()));
        }
        if let Some(v) = input.get("utterances").and_then(|v| v.as_bool()) {
            query_params.push(("utterances", v.to_string()));
        }
        if let Some(v) = input.get("paragraphs").and_then(|v| v.as_bool()) {
            query_params.push(("paragraphs", v.to_string()));
        }
        if let Some(v) = input.get("summarize").and_then(|v| v.as_str()) {
            query_params.push(("summarize", v.to_string()));
        } else if let Some(v) = input.get("summarize").and_then(|v| v.as_bool()) {
            query_params.push(("summarize", v.to_string()));
        }
        if let Some(v) = input.get("topics").and_then(|v| v.as_bool()) {
            query_params.push(("topics", v.to_string()));
        }
        if let Some(v) = input.get("intents").and_then(|v| v.as_bool()) {
            query_params.push(("intents", v.to_string()));
        }
        if let Some(v) = input.get("sentiment").and_then(|v| v.as_bool()) {
            query_params.push(("sentiment", v.to_string()));
        }
        if let Some(v) = input.get("detect_language").and_then(|v| v.as_bool()) {
            query_params.push(("detect_language", v.to_string()));
        }
        if let Some(v) = input.get("redact").and_then(|v| v.as_array()) {
            for item in v {
                if let Some(s) = item.as_str() {
                    query_params.push(("redact", s.to_string()));
                }
            }
        }
        if let Some(v) = input.get("search").and_then(|v| v.as_array()) {
            for item in v {
                if let Some(s) = item.as_str() {
                    query_params.push(("search", s.to_string()));
                }
            }
        }
        if let Some(v) = input.get("keywords").and_then(|v| v.as_array()) {
            for item in v {
                if let Some(s) = item.as_str() {
                    query_params.push(("keywords", s.to_string()));
                }
            }
        }
        if let Some(v) = input.get("multichannel").and_then(|v| v.as_bool()) {
            query_params.push(("multichannel", v.to_string()));
        }
        if let Some(v) = input.get("alternatives").and_then(|v| v.as_u64()) {
            query_params.push(("alternatives", v.to_string()));
        }
        if let Some(v) = input.get("numerals").and_then(|v| v.as_bool()) {
            query_params.push(("numerals", v.to_string()));
        }
        if let Some(v) = input.get("profanity_filter").and_then(|v| v.as_bool()) {
            query_params.push(("profanity_filter", v.to_string()));
        }
        if let Some(v) = input.get("tag").and_then(|v| v.as_str()) {
            query_params.push(("tag", v.to_string()));
        }

        let body = serde_json::json!({ "url": url });

        self.rt().block_on(send_and_parse(
            client
                .post(format!("{base}/v1/listen"))
                .header("Authorization", format!("Token {api_key}"))
                .query(&query_params)
                .json(&body),
        ))
    }

    // ------------------------------------------------------------------
    // Capability: speak (POST /v1/speak)
    // ------------------------------------------------------------------

    fn do_speak(&self, input: &serde_json::Value) -> Res<serde_json::Value> {
        let text = require_str(input, "text")?;
        if text.len() > 2000 {
            return Err(PortError::Validation(
                "text exceeds 2000 character limit".into(),
            ));
        }

        let api_key = self.api_key()?;
        let client = self.client()?;
        let base = self.base_url();

        let mut query_params: Vec<(&str, String)> = Vec::new();
        if let Some(model) = input.get("model").and_then(|v| v.as_str()) {
            query_params.push(("model", model.to_string()));
        }
        if let Some(encoding) = input.get("encoding").and_then(|v| v.as_str()) {
            query_params.push(("encoding", encoding.to_string()));
        }
        if let Some(container) = input.get("container").and_then(|v| v.as_str()) {
            query_params.push(("container", container.to_string()));
        }
        if let Some(sample_rate) = input.get("sample_rate").and_then(|v| v.as_u64()) {
            query_params.push(("sample_rate", sample_rate.to_string()));
        }
        if let Some(bit_rate) = input.get("bit_rate").and_then(|v| v.as_u64()) {
            query_params.push(("bit_rate", bit_rate.to_string()));
        }

        let body = serde_json::json!({ "text": text });

        self.rt().block_on(async {
            let resp = client
                .post(format!("{base}/v1/speak"))
                .header("Authorization", format!("Token {api_key}"))
                .query(&query_params)
                .json(&body)
                .send()
                .await
                .map_err(|e: reqwest::Error| PortError::TransportError(e.to_string()))?;

            let status = resp.status();

            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                let body_text = resp
                    .text()
                    .await
                    .unwrap_or_else(|_| "unknown".to_string());
                return Err(PortError::AuthorizationDenied(format!(
                    "Deepgram auth failed ({status}): {body_text}"
                )));
            }

            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let body_text = resp
                    .text()
                    .await
                    .unwrap_or_else(|_| "unknown".to_string());
                return Err(PortError::ExternalError(format!(
                    "Deepgram rate limited: {body_text}"
                )));
            }

            if !status.is_success() {
                let body_text = resp
                    .text()
                    .await
                    .unwrap_or_else(|_| "unknown".to_string());
                return Err(PortError::ExternalError(format!(
                    "Deepgram returned {status}: {body_text}"
                )));
            }

            let model_name = resp
                .headers()
                .get("dg-model-name")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let char_count = resp
                .headers()
                .get("dg-char-count")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let request_id = resp
                .headers()
                .get("dg-request-id")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("audio/mpeg")
                .to_string();

            let audio_bytes = resp
                .bytes()
                .await
                .map_err(|e: reqwest::Error| PortError::TransportError(e.to_string()))?;

            let audio_base64 =
                base64::engine::general_purpose::STANDARD.encode(&audio_bytes);

            Ok(serde_json::json!({
                "audio_base64": audio_base64,
                "audio_size_bytes": audio_bytes.len(),
                "content_type": content_type,
                "model_name": model_name,
                "char_count": char_count,
                "request_id": request_id,
            }))
        })
    }

    // ------------------------------------------------------------------
    // Capability: analyze_text (POST /v1/read)
    // ------------------------------------------------------------------

    fn do_analyze_text(&self, input: &serde_json::Value) -> Res<serde_json::Value> {
        let has_text = input.get("text").and_then(|v| v.as_str()).is_some();
        let has_url = input.get("url").and_then(|v| v.as_str()).is_some();
        if !has_text && !has_url {
            return Err(PortError::Validation(
                "must provide 'text' or 'url' field".into(),
            ));
        }

        let api_key = self.api_key()?;
        let client = self.client()?;
        let base = self.base_url();

        let mut query_params: Vec<(&str, String)> = Vec::new();
        if let Some(v) = input.get("sentiment").and_then(|v| v.as_bool()) {
            query_params.push(("sentiment", v.to_string()));
        }
        if let Some(v) = input.get("summarize").and_then(|v| v.as_str()) {
            query_params.push(("summarize", v.to_string()));
        } else if let Some(v) = input.get("summarize").and_then(|v| v.as_bool()) {
            query_params.push(("summarize", v.to_string()));
        }
        if let Some(v) = input.get("topics").and_then(|v| v.as_bool()) {
            query_params.push(("topics", v.to_string()));
        }
        if let Some(v) = input.get("intents").and_then(|v| v.as_bool()) {
            query_params.push(("intents", v.to_string()));
        }
        if let Some(v) = input.get("language").and_then(|v| v.as_str()) {
            query_params.push(("language", v.to_string()));
        }
        if let Some(v) = input.get("custom_topic").and_then(|v| v.as_array()) {
            for item in v {
                if let Some(s) = item.as_str() {
                    query_params.push(("custom_topic", s.to_string()));
                }
            }
        }
        if let Some(v) = input.get("custom_topic_mode").and_then(|v| v.as_str()) {
            query_params.push(("custom_topic_mode", v.to_string()));
        }
        if let Some(v) = input.get("custom_intent").and_then(|v| v.as_array()) {
            for item in v {
                if let Some(s) = item.as_str() {
                    query_params.push(("custom_intent", s.to_string()));
                }
            }
        }
        if let Some(v) = input.get("custom_intent_mode").and_then(|v| v.as_str()) {
            query_params.push(("custom_intent_mode", v.to_string()));
        }

        let mut body = serde_json::Map::new();
        if let Some(text) = input.get("text").and_then(|v| v.as_str()) {
            body.insert("text".to_string(), serde_json::Value::String(text.to_string()));
        } else if let Some(url) = input.get("url").and_then(|v| v.as_str()) {
            body.insert("url".to_string(), serde_json::Value::String(url.to_string()));
        }

        self.rt().block_on(send_and_parse(
            client
                .post(format!("{base}/v1/read"))
                .header("Authorization", format!("Token {api_key}"))
                .query(&query_params)
                .json(&serde_json::Value::Object(body)),
        ))
    }

    // ------------------------------------------------------------------
    // Spec builder
    // ------------------------------------------------------------------

    fn build_spec() -> PortSpec {
        let stt_latency = LatencyProfile {
            expected_latency_ms: 500,
            p95_latency_ms: 5_000,
            max_latency_ms: 120_000,
        };

        let tts_latency = LatencyProfile {
            expected_latency_ms: 200,
            p95_latency_ms: 2_000,
            max_latency_ms: 30_000,
        };

        let read_latency = LatencyProfile {
            expected_latency_ms: 300,
            p95_latency_ms: 3_000,
            max_latency_ms: 60_000,
        };

        let net_cost = CostProfile {
            cpu_cost_class: CostClass::Low,
            memory_cost_class: CostClass::Low,
            io_cost_class: CostClass::Low,
            network_cost_class: CostClass::Medium,
            energy_cost_class: CostClass::Low,
        };

        let tts_cost = CostProfile {
            cpu_cost_class: CostClass::Low,
            memory_cost_class: CostClass::Medium,
            io_cost_class: CostClass::Low,
            network_cost_class: CostClass::Medium,
            energy_cost_class: CostClass::Low,
        };

        let capabilities = vec![
            make_cap(
                "transcribe",
                "Transcribe pre-recorded audio from URL using Deepgram STT (nova-3, etc.) with optional punctuation, diarization, smart formatting, summarization, topics, intents, sentiment, language detection, redaction, search, and keywords",
                Cap {
                    effect: SideEffectClass::ReadOnly,
                    rollback: RollbackSupport::Irreversible,
                    determinism: DeterminismClass::Stochastic,
                    idempotence: IdempotenceClass::Idempotent,
                    risk: RiskClass::Low,
                },
                &stt_latency,
                &net_cost,
            ),
            make_cap(
                "speak",
                "Convert text to speech audio using Deepgram TTS (Aura models), returns base64-encoded audio with configurable model, encoding, sample rate, and bit rate",
                Cap {
                    effect: SideEffectClass::ReadOnly,
                    rollback: RollbackSupport::Irreversible,
                    determinism: DeterminismClass::Stochastic,
                    idempotence: IdempotenceClass::Idempotent,
                    risk: RiskClass::Negligible,
                },
                &tts_latency,
                &tts_cost,
            ),
            make_cap(
                "analyze_text",
                "Analyze text or URL content using Deepgram text intelligence for sentiment analysis, summarization, topic detection, and intent recognition",
                Cap {
                    effect: SideEffectClass::ReadOnly,
                    rollback: RollbackSupport::Irreversible,
                    determinism: DeterminismClass::Stochastic,
                    idempotence: IdempotenceClass::Idempotent,
                    risk: RiskClass::Negligible,
                },
                &read_latency,
                &net_cost,
            ),
        ];

        PortSpec {
            port_id: "soma.ports.deepgram".to_string(),
            name: "deepgram".to_string(),
            version: Version::new(0, 1, 0),
            kind: PortKind::Http,
            description: "Deepgram speech-to-text, text-to-speech, and text intelligence: transcription with diarization/smart formatting/summarization/topics/intents/sentiment/redaction, TTS with Aura models, text analysis with sentiment/summary/topics/intents".to_string(),
            namespace: "soma.ports.deepgram".to_string(),
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
            side_effect_class: SideEffectClass::ReadOnly,
            latency_profile: stt_latency,
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
                "request_id".to_string(),
                "model".to_string(),
                "duration".to_string(),
                "channels".to_string(),
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
            "Deepgram auth failed ({status}): {body}"
        )));
    }

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(PortError::ExternalError(format!(
            "Deepgram rate limited: {body}"
        )));
    }

    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(PortError::NotFound(format!(
            "Deepgram resource not found: {body}"
        )));
    }

    if !status.is_success() {
        return Err(PortError::ExternalError(format!(
            "Deepgram returned {status}: {body}"
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

impl Port for DeepgramPort {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(&self, capability_id: &str, input: serde_json::Value) -> Res<PortCallRecord> {
        let start = Instant::now();
        let result = match capability_id {
            "transcribe" => self.do_transcribe(&input),
            "speak" => self.do_speak(&input),
            "analyze_text" => self.do_analyze_text(&input),
            other => return Err(PortError::Validation(format!("unknown capability: {other}"))),
        };
        let latency_ms = start.elapsed().as_millis() as u64;
        Ok(match result {
            Ok(v) => PortCallRecord::success(&self.spec.port_id, capability_id, v, latency_ms),
            Err(e) => {
                let fc = e.failure_class();
                PortCallRecord::failure(
                    &self.spec.port_id,
                    capability_id,
                    fc,
                    &e.to_string(),
                    latency_ms,
                )
            }
        })
    }

    fn validate_input(&self, capability_id: &str, input: &serde_json::Value) -> Res<()> {
        if !input.is_object() {
            return Err(PortError::Validation("input must be a JSON object".into()));
        }
        match capability_id {
            "transcribe" => {
                require_str(input, "url")?;
            }
            "speak" => {
                let text = require_str(input, "text")?;
                if text.len() > 2000 {
                    return Err(PortError::Validation(
                        "text exceeds 2000 character limit".into(),
                    ));
                }
            }
            "analyze_text" => {
                let has_text = input.get("text").and_then(|v| v.as_str()).is_some();
                let has_url = input.get("url").and_then(|v| v.as_str()).is_some();
                if !has_text && !has_url {
                    return Err(PortError::Validation(
                        "must provide 'text' or 'url' field".into(),
                    ));
                }
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

// ------------------------------------------------------------------
// C ABI export
// ------------------------------------------------------------------

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    let port = DeepgramPort::new();
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
        let port = DeepgramPort::new();
        let spec = port.spec();

        assert_eq!(spec.port_id, "soma.ports.deepgram");
        assert_eq!(spec.capabilities.len(), 3);
        assert!(!spec.failure_modes.is_empty());
        assert!(spec.latency_profile.expected_latency_ms <= spec.latency_profile.p95_latency_ms);
        assert!(spec.latency_profile.p95_latency_ms <= spec.latency_profile.max_latency_ms);

        let ids: Vec<&str> = spec
            .capabilities
            .iter()
            .map(|c| c.capability_id.as_str())
            .collect();
        assert!(ids.contains(&"transcribe"));
        assert!(ids.contains(&"speak"));
        assert!(ids.contains(&"analyze_text"));
    }

    #[test]
    fn test_capability_latency_invariant() {
        let port = DeepgramPort::new();
        for cap in &port.spec().capabilities {
            assert!(
                cap.latency_profile.expected_latency_ms <= cap.latency_profile.p95_latency_ms,
                "{}: expected <= p95",
                cap.capability_id
            );
            assert!(
                cap.latency_profile.p95_latency_ms <= cap.latency_profile.max_latency_ms,
                "{}: p95 <= max",
                cap.capability_id
            );
        }
    }

    #[test]
    fn test_unique_capability_ids() {
        let port = DeepgramPort::new();
        let ids: Vec<&str> = port
            .spec()
            .capabilities
            .iter()
            .map(|c| c.capability_id.as_str())
            .collect();
        let mut deduped = ids.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(ids.len(), deduped.len());
    }

    #[test]
    fn test_validate_input_rejects_missing_fields() {
        let port = DeepgramPort::new();
        let empty = serde_json::json!({});

        assert!(port.validate_input("transcribe", &empty).is_err());
        assert!(port.validate_input("speak", &empty).is_err());
        assert!(port.validate_input("analyze_text", &empty).is_err());
    }

    #[test]
    fn test_validate_input_accepts_valid() {
        let port = DeepgramPort::new();

        assert!(port
            .validate_input("transcribe", &serde_json::json!({"url": "https://example.com/a.wav"}))
            .is_ok());
        assert!(port
            .validate_input("speak", &serde_json::json!({"text": "Hello world"}))
            .is_ok());
        assert!(port
            .validate_input("analyze_text", &serde_json::json!({"text": "Some text"}))
            .is_ok());
        assert!(port
            .validate_input(
                "analyze_text",
                &serde_json::json!({"url": "https://example.com/doc"})
            )
            .is_ok());
    }

    #[test]
    fn test_validate_input_unknown_capability() {
        let port = DeepgramPort::new();
        assert!(port
            .validate_input("nonexistent", &serde_json::json!({}))
            .is_err());
    }

    #[test]
    fn test_speak_rejects_too_long_text() {
        let port = DeepgramPort::new();
        let long_text = "a".repeat(2001);
        assert!(port
            .validate_input("speak", &serde_json::json!({"text": long_text}))
            .is_err());
    }

    #[test]
    fn test_all_capabilities_read_only() {
        let port = DeepgramPort::new();
        for cap in &port.spec().capabilities {
            assert_eq!(
                cap.effect_class,
                SideEffectClass::ReadOnly,
                "{} should be read_only",
                cap.capability_id
            );
        }
    }
}
