//! SOMA Elasticsearch port pack — search and indexing operations via `reqwest`
//! HTTP client against the Elasticsearch REST API.
//!
//! Provides 6 capabilities:
//!
//! - **Search**: `search`
//! - **Document CRUD**: `index_document`, `get_document`, `delete_document`
//! - **Index management**: `create_index`, `delete_index`
//!
//! Each capability accepts JSON input and returns JSON output via the Port trait.
//! The base URL is read from `SOMA_ELASTICSEARCH_URL` or `ELASTICSEARCH_URL`
//! (e.g., `http://localhost:9200`).

use std::sync::OnceLock;
use std::time::Instant;

use chrono::Utc;
use semver::Version;
use soma_port_sdk::prelude::*;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Port struct
// ---------------------------------------------------------------------------

pub struct ElasticsearchPort {
    spec: PortSpec,
    base_url: OnceLock<Option<String>>,
    client: OnceLock<reqwest::blocking::Client>,
}

#[derive(Clone, Copy)]
struct CapabilityBehavior {
    effect_class: SideEffectClass,
    rollback_support: RollbackSupport,
    determinism_class: DeterminismClass,
    idempotence_class: IdempotenceClass,
    risk_class: RiskClass,
}

impl CapabilityBehavior {
    fn new(
        effect_class: SideEffectClass,
        rollback_support: RollbackSupport,
        determinism_class: DeterminismClass,
        idempotence_class: IdempotenceClass,
        risk_class: RiskClass,
    ) -> Self {
        Self {
            effect_class,
            rollback_support,
            determinism_class,
            idempotence_class,
            risk_class,
        }
    }
}

impl Default for ElasticsearchPort {
    fn default() -> Self {
        Self::new()
    }
}

impl ElasticsearchPort {
    pub fn new() -> Self {
        let spec = Self::build_spec();
        Self {
            spec,
            base_url: OnceLock::new(),
            client: OnceLock::new(),
        }
    }

    fn base_url(&self) -> Option<&str> {
        self.base_url
            .get_or_init(|| {
                std::env::var("SOMA_ELASTICSEARCH_URL")
                    .or_else(|_| std::env::var("ELASTICSEARCH_URL"))
                    .ok()
            })
            .as_deref()
    }

    fn client(&self) -> &reqwest::blocking::Client {
        self.client.get_or_init(|| {
            reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("failed to create HTTP client for elasticsearch port")
        })
    }

    fn require_url(&self) -> std::result::Result<&str, String> {
        self.base_url().ok_or_else(|| {
            "Elasticsearch URL not set. Set SOMA_ELASTICSEARCH_URL or ELASTICSEARCH_URL".to_string()
        })
    }

    /// Parse an Elasticsearch response body, returning the JSON or an error string.
    fn parse_response(
        resp: reqwest::blocking::Response,
    ) -> std::result::Result<serde_json::Value, String> {
        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| format!("failed to parse Elasticsearch response: {e}"))?;

        if !status.is_success() {
            let error_msg = body
                .get("error")
                .map(|e| e.to_string())
                .unwrap_or_else(|| format!("HTTP {status}"));
            return Err(format!("Elasticsearch error: {error_msg}"));
        }

        Ok(body)
    }

    // -----------------------------------------------------------------------
    // Observation helpers
    // -----------------------------------------------------------------------

    fn success_record(
        &self,
        capability_id: &str,
        result: serde_json::Value,
        effect_summary: &str,
        latency_ms: u64,
    ) -> PortCallRecord {
        PortCallRecord {
            observation_id: Uuid::new_v4(),
            port_id: self.spec.port_id.clone(),
            capability_id: capability_id.to_string(),
            invocation_id: Uuid::new_v4(),
            success: true,
            failure_class: None,
            raw_result: result.clone(),
            structured_result: result,
            effect_patch: None,
            side_effect_summary: Some(effect_summary.to_string()),
            latency_ms,
            resource_cost: 0.0,
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
        }
    }

    fn failure_record(
        &self,
        capability_id: &str,
        failure_class: PortFailureClass,
        message: &str,
        latency_ms: u64,
    ) -> PortCallRecord {
        let retry_safe = matches!(
            failure_class,
            PortFailureClass::Timeout
                | PortFailureClass::DependencyUnavailable
                | PortFailureClass::TransportError
                | PortFailureClass::ExternalError
                | PortFailureClass::Unknown
        );
        PortCallRecord {
            observation_id: Uuid::new_v4(),
            port_id: self.spec.port_id.clone(),
            capability_id: capability_id.to_string(),
            invocation_id: Uuid::new_v4(),
            success: false,
            failure_class: Some(failure_class),
            raw_result: serde_json::Value::Null,
            structured_result: serde_json::json!({ "error": message }),
            effect_patch: None,
            side_effect_summary: Some("none".to_string()),
            latency_ms,
            resource_cost: 0.0,
            confidence: 0.0,
            timestamp: Utc::now(),
            retry_safe,
            input_hash: None,
            session_id: None,
            goal_id: None,
            caller_identity: None,
            auth_result: None,
            policy_result: None,
            sandbox_result: None,
        }
    }

    // -----------------------------------------------------------------------
    // Capability implementations
    // -----------------------------------------------------------------------

    /// `search` -- POST /{index}/_search
    fn do_search(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let base = self.require_url()?;
        let index = input
            .get("index")
            .and_then(|v| v.as_str())
            .ok_or("missing 'index' field")?;

        let query = input
            .get("query")
            .cloned()
            .unwrap_or(serde_json::json!({ "match_all": {} }));

        let mut body = serde_json::json!({ "query": query });

        if let Some(size) = input.get("size").and_then(|v| v.as_i64()) {
            body["size"] = serde_json::json!(size);
        }
        if let Some(from) = input.get("from").and_then(|v| v.as_i64()) {
            body["from"] = serde_json::json!(from);
        }

        let url = format!("{base}/{index}/_search");
        let resp = self
            .client()
            .post(&url)
            .json(&body)
            .send()
            .map_err(|e| format!("Elasticsearch request failed: {e}"))?;

        let result = Self::parse_response(resp)?;

        let hits = result
            .get("hits")
            .and_then(|h| h.get("hits"))
            .and_then(|h| h.as_array())
            .cloned()
            .unwrap_or_default();
        let total = result
            .get("hits")
            .and_then(|h| h.get("total"))
            .and_then(|t| t.get("value"))
            .and_then(|v| v.as_i64())
            .unwrap_or(hits.len() as i64);

        Ok(serde_json::json!({ "hits": hits, "total": total }))
    }

    /// `index_document` -- PUT/POST /{index}/_doc/{id?}
    fn do_index_document(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let base = self.require_url()?;
        let index = input
            .get("index")
            .and_then(|v| v.as_str())
            .ok_or("missing 'index' field")?;

        let document = input
            .get("document")
            .ok_or("missing 'document' field")?;

        let resp = if let Some(id) = input.get("id").and_then(|v| v.as_str()) {
            let url = format!("{base}/{index}/_doc/{id}");
            self.client()
                .put(&url)
                .json(document)
                .send()
                .map_err(|e| format!("Elasticsearch request failed: {e}"))?
        } else {
            let url = format!("{base}/{index}/_doc");
            self.client()
                .post(&url)
                .json(document)
                .send()
                .map_err(|e| format!("Elasticsearch request failed: {e}"))?
        };

        let result = Self::parse_response(resp)?;
        Ok(result)
    }

    /// `get_document` -- GET /{index}/_doc/{id}
    fn do_get_document(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let base = self.require_url()?;
        let index = input
            .get("index")
            .and_then(|v| v.as_str())
            .ok_or("missing 'index' field")?;
        let id = input
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("missing 'id' field")?;

        let url = format!("{base}/{index}/_doc/{id}");
        let resp = self
            .client()
            .get(&url)
            .send()
            .map_err(|e| format!("Elasticsearch request failed: {e}"))?;

        let result = Self::parse_response(resp)?;
        Ok(result)
    }

    /// `delete_document` -- DELETE /{index}/_doc/{id}
    fn do_delete_document(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let base = self.require_url()?;
        let index = input
            .get("index")
            .and_then(|v| v.as_str())
            .ok_or("missing 'index' field")?;
        let id = input
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("missing 'id' field")?;

        let url = format!("{base}/{index}/_doc/{id}");
        let resp = self
            .client()
            .delete(&url)
            .send()
            .map_err(|e| format!("Elasticsearch request failed: {e}"))?;

        let result = Self::parse_response(resp)?;
        Ok(result)
    }

    /// `create_index` -- PUT /{index}
    fn do_create_index(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let base = self.require_url()?;
        let index = input
            .get("index")
            .and_then(|v| v.as_str())
            .ok_or("missing 'index' field")?;

        let url = format!("{base}/{index}");

        let body = if let Some(mappings) = input.get("mappings") {
            serde_json::json!({ "mappings": mappings })
        } else {
            serde_json::json!({})
        };

        let resp = self
            .client()
            .put(&url)
            .json(&body)
            .send()
            .map_err(|e| format!("Elasticsearch request failed: {e}"))?;

        let result = Self::parse_response(resp)?;
        Ok(result)
    }

    /// `delete_index` -- DELETE /{index}
    fn do_delete_index(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let base = self.require_url()?;
        let index = input
            .get("index")
            .and_then(|v| v.as_str())
            .ok_or("missing 'index' field")?;

        let url = format!("{base}/{index}");
        let resp = self
            .client()
            .delete(&url)
            .send()
            .map_err(|e| format!("Elasticsearch request failed: {e}"))?;

        let result = Self::parse_response(resp)?;
        Ok(result)
    }

    // -----------------------------------------------------------------------
    // PortSpec builder
    // -----------------------------------------------------------------------

    fn build_spec() -> PortSpec {
        let any_schema = SchemaRef {
            schema: serde_json::json!({ "type": "object" }),
        };

        let low_cost = CostProfile {
            cpu_cost_class: CostClass::Low,
            memory_cost_class: CostClass::Low,
            io_cost_class: CostClass::Low,
            network_cost_class: CostClass::Low,
            energy_cost_class: CostClass::Negligible,
        };

        let search_latency = LatencyProfile {
            expected_latency_ms: 50,
            p95_latency_ms: 500,
            max_latency_ms: 30_000,
        };

        let capabilities = vec![
            Self::cap(
                "search",
                "Search documents in an index",
                CapabilityBehavior::new(
                    SideEffectClass::ReadOnly,
                    RollbackSupport::Irreversible,
                    DeterminismClass::PartiallyDeterministic,
                    IdempotenceClass::Idempotent,
                    RiskClass::Low,
                ),
                &search_latency,
                &low_cost,
            ),
            Self::cap(
                "index_document",
                "Index (insert or replace) a document",
                CapabilityBehavior::new(
                    SideEffectClass::ExternalStateMutation,
                    RollbackSupport::CompensatingAction,
                    DeterminismClass::PartiallyDeterministic,
                    IdempotenceClass::ConditionallyIdempotent,
                    RiskClass::Low,
                ),
                &search_latency,
                &low_cost,
            ),
            Self::cap(
                "get_document",
                "Retrieve a document by ID",
                CapabilityBehavior::new(
                    SideEffectClass::ReadOnly,
                    RollbackSupport::Irreversible,
                    DeterminismClass::PartiallyDeterministic,
                    IdempotenceClass::Idempotent,
                    RiskClass::Low,
                ),
                &search_latency,
                &low_cost,
            ),
            Self::cap(
                "delete_document",
                "Delete a document by ID",
                CapabilityBehavior::new(
                    SideEffectClass::Destructive,
                    RollbackSupport::Irreversible,
                    DeterminismClass::PartiallyDeterministic,
                    IdempotenceClass::Idempotent,
                    RiskClass::High,
                ),
                &search_latency,
                &low_cost,
            ),
            Self::cap(
                "create_index",
                "Create an index with optional mappings",
                CapabilityBehavior::new(
                    SideEffectClass::ExternalStateMutation,
                    RollbackSupport::CompensatingAction,
                    DeterminismClass::Deterministic,
                    IdempotenceClass::NonIdempotent,
                    RiskClass::Medium,
                ),
                &search_latency,
                &low_cost,
            ),
            Self::cap(
                "delete_index",
                "Delete an entire index",
                CapabilityBehavior::new(
                    SideEffectClass::Destructive,
                    RollbackSupport::Irreversible,
                    DeterminismClass::Deterministic,
                    IdempotenceClass::Idempotent,
                    RiskClass::Critical,
                ),
                &search_latency,
                &low_cost,
            ),
        ];

        PortSpec {
            port_id: "soma.elasticsearch".to_string(),
            name: "elasticsearch".to_string(),
            version: Version::new(0, 1, 0),
            kind: PortKind::Database,
            description:
                "Elasticsearch search and indexing: search, document CRUD, index management"
                    .to_string(),
            namespace: "soma.ports".to_string(),
            trust_level: TrustLevel::Verified,
            capabilities,
            input_schema: any_schema.clone(),
            output_schema: any_schema,
            failure_modes: vec![
                PortFailureClass::ValidationError,
                PortFailureClass::DependencyUnavailable,
                PortFailureClass::TransportError,
                PortFailureClass::ExternalError,
                PortFailureClass::Timeout,
            ],
            side_effect_class: SideEffectClass::ExternalStateMutation,
            latency_profile: search_latency,
            cost_profile: low_cost,
            auth_requirements: AuthRequirements {
                methods: vec![AuthMethod::LocalProcessTrust],
                required: false,
            },
            sandbox_requirements: SandboxRequirements {
                filesystem_access: false,
                network_access: true,
                device_access: false,
                process_access: false,
                memory_limit_mb: None,
                cpu_limit_percent: None,
                time_limit_ms: Some(30_000),
                syscall_limit: None,
            },
            observable_fields: vec![],
            validation_rules: vec![],
            remote_exposure: false,
        }
    }

    fn cap(
        name: &str,
        purpose: &str,
        behavior: CapabilityBehavior,
        latency_profile: &LatencyProfile,
        cost_profile: &CostProfile,
    ) -> PortCapabilitySpec {
        let any_schema = SchemaRef {
            schema: serde_json::json!({ "type": "object" }),
        };
        PortCapabilitySpec {
            capability_id: name.to_string(),
            name: name.to_string(),
            purpose: purpose.to_string(),
            input_schema: any_schema.clone(),
            output_schema: any_schema,
            effect_class: behavior.effect_class,
            rollback_support: behavior.rollback_support,
            determinism_class: behavior.determinism_class,
            idempotence_class: behavior.idempotence_class,
            risk_class: behavior.risk_class,
            latency_profile: latency_profile.clone(),
            cost_profile: cost_profile.clone(),
            remote_exposable: false,
            auth_override: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Port trait implementation
// ---------------------------------------------------------------------------

impl Port for ElasticsearchPort {
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
            "search" => self.do_search(&input),
            "index_document" => self.do_index_document(&input),
            "get_document" => self.do_get_document(&input),
            "delete_document" => self.do_delete_document(&input),
            "create_index" => self.do_create_index(&input),
            "delete_index" => self.do_delete_index(&input),
            _ => {
                let latency_ms = start.elapsed().as_millis() as u64;
                return Ok(self.failure_record(
                    capability_id,
                    PortFailureClass::ValidationError,
                    &format!("unknown capability: {capability_id}"),
                    latency_ms,
                ));
            }
        };

        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(value) => {
                let effect = match capability_id {
                    "search" | "get_document" => "read_only",
                    "index_document" | "create_index" => "external_state_mutation",
                    "delete_document" | "delete_index" => "destructive",
                    _ => "unknown",
                };
                Ok(self.success_record(capability_id, value, effect, latency_ms))
            }
            Err(msg) => Ok(self.failure_record(
                capability_id,
                PortFailureClass::ExternalError,
                &msg,
                latency_ms,
            )),
        }
    }

    fn validate_input(
        &self,
        capability_id: &str,
        input: &serde_json::Value,
    ) -> soma_port_sdk::Result<()> {
        if !input.is_object() {
            return Err(PortError::Validation("input must be a JSON object".into()));
        }

        match capability_id {
            "search" | "create_index" | "delete_index" => {
                if input.get("index").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'index' field".into()));
                }
            }
            "index_document" => {
                if input.get("index").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'index' field".into()));
                }
                if input.get("document").is_none() {
                    return Err(PortError::Validation("missing 'document' field".into()));
                }
            }
            "get_document" | "delete_document" => {
                if input.get("index").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'index' field".into()));
                }
                if input.get("id").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'id' field".into()));
                }
            }
            _ => {
                return Err(PortError::Validation(format!(
                    "unknown capability: {capability_id}"
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
// C ABI entry point
// ---------------------------------------------------------------------------

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    let port = ElasticsearchPort::new();
    Box::into_raw(Box::new(port))
}
