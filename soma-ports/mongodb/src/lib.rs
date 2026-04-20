use std::sync::Mutex;
use std::time::Instant;

use chrono::Utc;
use futures_util::TryStreamExt;
use mongodb::bson::{self, Bson, Document};
use mongodb::Client;
use semver::Version;
use soma_port_sdk::prelude::*;
use uuid::Uuid;

pub struct MongoDbPort {
    spec: PortSpec,
    url: String,
    client: Mutex<Option<Client>>,
    rt: Mutex<Option<tokio::runtime::Runtime>>,
}

impl Default for MongoDbPort {
    fn default() -> Self {
        Self::new()
    }
}

impl MongoDbPort {
    pub fn new() -> Self {
        let url = std::env::var("SOMA_MONGODB_URL")
            .or_else(|_| std::env::var("MONGODB_URL"))
            .unwrap_or_else(|_| "mongodb://localhost:27017".to_string());
        Self {
            spec: build_spec(),
            url,
            client: Mutex::new(None),
            rt: Mutex::new(None),
        }
    }

    fn ensure_runtime(&self) -> soma_port_sdk::Result<()> {
        let mut guard = self.rt.lock()
            .map_err(|e| PortError::Internal(format!("rt lock poisoned: {e}")))?;
        if guard.is_none() {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| PortError::Internal(format!("tokio runtime: {e}")))?;
            *guard = Some(rt);
        }
        Ok(())
    }

    fn get_client(&self) -> soma_port_sdk::Result<Client> {
        let mut guard = self.client.lock()
            .map_err(|e| PortError::Internal(format!("lock poisoned: {e}")))?;
        if let Some(ref client) = *guard {
            return Ok(client.clone());
        }
        let rt_guard = self.rt.lock()
            .map_err(|e| PortError::Internal(format!("rt lock poisoned: {e}")))?;
        let rt = rt_guard.as_ref()
            .ok_or_else(|| PortError::Internal("runtime not initialized".into()))?;
        let url = self.url.clone();
        let client = rt.block_on(async {
            let opts = mongodb::options::ClientOptions::parse(&url).await
                .map_err(|e| PortError::DependencyUnavailable(format!("MongoDB parse URI: {e}")))?;
            Ok::<Client, PortError>(Client::with_options(opts)
                .map_err(|e| PortError::DependencyUnavailable(format!("MongoDB client: {e}")))?)
        })?;
        *guard = Some(client.clone());
        Ok(client)
    }

    fn db_name(&self) -> String {
        std::env::var("SOMA_MONGODB_DATABASE").unwrap_or_else(|_| "soma".to_string())
    }

    fn run<F, T>(&self, f: F) -> soma_port_sdk::Result<T>
    where
        F: std::future::Future<Output = soma_port_sdk::Result<T>>,
    {
        let rt_guard = self.rt.lock()
            .map_err(|e| PortError::Internal(format!("rt lock poisoned: {e}")))?;
        let rt = rt_guard.as_ref()
            .ok_or_else(|| PortError::Internal("runtime not initialized".into()))?;
        rt.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(30), f)
                .await
                .map_err(|_| PortError::Timeout("MongoDB operation timed out (30s)".into()))?
        })
    }

    fn json_to_doc(val: &serde_json::Value) -> std::result::Result<Document, String> {
        match val {
            serde_json::Value::Object(_) => {
                let bson_val = bson::to_bson(val).map_err(|e| format!("BSON conversion: {e}"))?;
                match bson_val {
                    Bson::Document(doc) => Ok(doc),
                    _ => Err("expected BSON document".to_string()),
                }
            }
            _ => Err("expected a JSON object".to_string()),
        }
    }

    fn doc_to_json(doc: &Document) -> serde_json::Value {
        bson::to_bson(doc)
            .ok()
            .map(|b| b.into_relaxed_extjson())
            .unwrap_or(serde_json::Value::Null)
    }

    fn do_find(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let coll_name = input.get("collection").and_then(|v| v.as_str())
            .ok_or_else(|| PortError::Validation("missing 'collection'".into()))?;
        let filter = match input.get("filter") {
            Some(f) if !f.is_null() => Self::json_to_doc(f).map_err(|e| PortError::Validation(e))?,
            _ => Document::new(),
        };
        let limit = input.get("limit").and_then(|v| v.as_i64());
        let client = self.get_client()?;
        let db = client.database(&self.db_name());
        let coll = db.collection::<Document>(coll_name);

        self.run(async move {
            let mut opts = mongodb::options::FindOptions::default();
            if let Some(l) = limit {
                opts.limit = Some(l);
            }
            let mut cursor = coll.find(filter).with_options(opts).await
                .map_err(|e| PortError::ExternalError(format!("find: {e}")))?;
            let mut docs = Vec::new();
            while let Some(doc) = cursor.try_next().await
                .map_err(|e| PortError::ExternalError(format!("cursor: {e}")))? {
                docs.push(Self::doc_to_json(&doc));
            }
            Ok(serde_json::json!({ "documents": docs, "count": docs.len() }))
        })
    }

    fn do_find_one(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let coll_name = input.get("collection").and_then(|v| v.as_str())
            .ok_or_else(|| PortError::Validation("missing 'collection'".into()))?;
        let filter = input.get("filter")
            .ok_or_else(|| PortError::Validation("missing 'filter'".into()))
            .and_then(|f| Self::json_to_doc(f).map_err(|e| PortError::Validation(e)))?;
        let client = self.get_client()?;
        let db = client.database(&self.db_name());
        let coll = db.collection::<Document>(coll_name);

        self.run(async move {
            let result = coll.find_one(filter).await
                .map_err(|e| PortError::ExternalError(format!("find_one: {e}")))?;
            Ok(match result {
                Some(doc) => Self::doc_to_json(&doc),
                None => serde_json::Value::Null,
            })
        })
    }

    fn do_insert_one(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let coll_name = input.get("collection").and_then(|v| v.as_str())
            .ok_or_else(|| PortError::Validation("missing 'collection'".into()))?;
        let doc = input.get("document")
            .ok_or_else(|| PortError::Validation("missing 'document'".into()))
            .and_then(|d| Self::json_to_doc(d).map_err(|e| PortError::Validation(e)))?;
        let client = self.get_client()?;
        let db = client.database(&self.db_name());
        let coll = db.collection::<Document>(coll_name);

        self.run(async move {
            let result = coll.insert_one(doc).await
                .map_err(|e| PortError::ExternalError(format!("insert_one: {e}")))?;
            Ok(serde_json::json!({ "inserted_id": format!("{}", result.inserted_id) }))
        })
    }

    fn do_insert_many(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let coll_name = input.get("collection").and_then(|v| v.as_str())
            .ok_or_else(|| PortError::Validation("missing 'collection'".into()))?;
        let documents = input.get("documents").and_then(|v| v.as_array())
            .ok_or_else(|| PortError::Validation("missing 'documents' array".into()))?;
        let docs: Vec<Document> = documents.iter()
            .map(Self::json_to_doc)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| PortError::Validation(e))?;
        let client = self.get_client()?;
        let db = client.database(&self.db_name());
        let coll = db.collection::<Document>(coll_name);

        self.run(async move {
            let result = coll.insert_many(docs).await
                .map_err(|e| PortError::ExternalError(format!("insert_many: {e}")))?;
            let ids: Vec<String> = result.inserted_ids.values().map(|id| format!("{id}")).collect();
            Ok(serde_json::json!({ "inserted_ids": ids, "count": ids.len() }))
        })
    }

    fn do_update_one(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let coll_name = input.get("collection").and_then(|v| v.as_str())
            .ok_or_else(|| PortError::Validation("missing 'collection'".into()))?;
        let filter = input.get("filter")
            .ok_or_else(|| PortError::Validation("missing 'filter'".into()))
            .and_then(|f| Self::json_to_doc(f).map_err(|e| PortError::Validation(e)))?;
        let update = input.get("update")
            .ok_or_else(|| PortError::Validation("missing 'update'".into()))
            .and_then(|u| Self::json_to_doc(u).map_err(|e| PortError::Validation(e)))?;
        let client = self.get_client()?;
        let db = client.database(&self.db_name());
        let coll = db.collection::<Document>(coll_name);

        self.run(async move {
            let result = coll.update_one(filter, update).await
                .map_err(|e| PortError::ExternalError(format!("update_one: {e}")))?;
            Ok(serde_json::json!({
                "matched_count": result.matched_count,
                "modified_count": result.modified_count
            }))
        })
    }

    fn do_delete_one(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let coll_name = input.get("collection").and_then(|v| v.as_str())
            .ok_or_else(|| PortError::Validation("missing 'collection'".into()))?;
        let filter = input.get("filter")
            .ok_or_else(|| PortError::Validation("missing 'filter'".into()))
            .and_then(|f| Self::json_to_doc(f).map_err(|e| PortError::Validation(e)))?;
        let client = self.get_client()?;
        let db = client.database(&self.db_name());
        let coll = db.collection::<Document>(coll_name);

        self.run(async move {
            let result = coll.delete_one(filter).await
                .map_err(|e| PortError::ExternalError(format!("delete_one: {e}")))?;
            Ok(serde_json::json!({ "deleted_count": result.deleted_count }))
        })
    }

    fn do_count(&self, input: &serde_json::Value) -> soma_port_sdk::Result<serde_json::Value> {
        let coll_name = input.get("collection").and_then(|v| v.as_str())
            .ok_or_else(|| PortError::Validation("missing 'collection'".into()))?;
        let filter = match input.get("filter") {
            Some(f) if !f.is_null() => Self::json_to_doc(f).map_err(|e| PortError::Validation(e))?,
            _ => Document::new(),
        };
        let client = self.get_client()?;
        let db = client.database(&self.db_name());
        let coll = db.collection::<Document>(coll_name);

        self.run(async move {
            let count = coll.count_documents(filter).await
                .map_err(|e| PortError::ExternalError(format!("count: {e}")))?;
            Ok(serde_json::json!({ "count": count }))
        })
    }
}

impl Port for MongoDbPort {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(
        &self,
        capability_id: &str,
        input: serde_json::Value,
    ) -> soma_port_sdk::Result<PortCallRecord> {
        self.ensure_runtime()?;
        let start = Instant::now();

        let result = match capability_id {
            "find" => self.do_find(&input),
            "find_one" => self.do_find_one(&input),
            "insert_one" => self.do_insert_one(&input),
            "insert_many" => self.do_insert_many(&input),
            "update_one" => self.do_update_one(&input),
            "delete_one" => self.do_delete_one(&input),
            "count" => self.do_count(&input),
            _ => return Ok(failure_record(&self.spec, capability_id, PortFailureClass::ValidationError,
                &format!("unknown capability: {capability_id}"), start.elapsed().as_millis() as u64)),
        };

        let latency_ms = start.elapsed().as_millis() as u64;
        match result {
            Ok(value) => {
                let effect = match capability_id {
                    "find" | "find_one" | "count" => "read_only",
                    "insert_one" | "insert_many" | "update_one" => "external_state_mutation",
                    "delete_one" => "destructive",
                    _ => "unknown",
                };
                Ok(success_record(&self.spec, capability_id, value, effect, latency_ms))
            }
            Err(e) => {
                let class = e.failure_class();
                Ok(failure_record(&self.spec, capability_id, class, &e.to_string(), latency_ms))
            }
        }
    }

    fn validate_input(&self, capability_id: &str, input: &serde_json::Value) -> soma_port_sdk::Result<()> {
        if !input.is_object() {
            return Err(PortError::Validation("input must be a JSON object".into()));
        }
        match capability_id {
            "find" | "count" => {
                if input.get("collection").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'collection'".into()));
                }
            }
            "find_one" => {
                if input.get("collection").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'collection'".into()));
                }
                if input.get("filter").is_none() {
                    return Err(PortError::Validation("missing 'filter'".into()));
                }
            }
            "insert_one" => {
                if input.get("collection").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'collection'".into()));
                }
                if input.get("document").is_none() {
                    return Err(PortError::Validation("missing 'document'".into()));
                }
            }
            "insert_many" => {
                if input.get("collection").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'collection'".into()));
                }
                if input.get("documents").and_then(|v| v.as_array()).is_none() {
                    return Err(PortError::Validation("missing 'documents' array".into()));
                }
            }
            "update_one" => {
                if input.get("collection").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'collection'".into()));
                }
                if input.get("filter").is_none() {
                    return Err(PortError::Validation("missing 'filter'".into()));
                }
                if input.get("update").is_none() {
                    return Err(PortError::Validation("missing 'update'".into()));
                }
            }
            "delete_one" => {
                if input.get("collection").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'collection'".into()));
                }
                if input.get("filter").is_none() {
                    return Err(PortError::Validation("missing 'filter'".into()));
                }
            }
            _ => return Err(PortError::Validation(format!("unknown capability: {capability_id}"))),
        }
        Ok(())
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        PortLifecycleState::Active
    }
}

fn success_record(spec: &PortSpec, capability_id: &str, result: serde_json::Value, effect: &str, latency_ms: u64) -> PortCallRecord {
    PortCallRecord {
        observation_id: Uuid::new_v4(),
        port_id: spec.port_id.clone(),
        capability_id: capability_id.to_string(),
        invocation_id: Uuid::new_v4(),
        success: true,
        failure_class: None,
        raw_result: result.clone(),
        structured_result: result,
        effect_patch: None,
        side_effect_summary: Some(effect.to_string()),
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

fn failure_record(spec: &PortSpec, capability_id: &str, failure_class: PortFailureClass, msg: &str, latency_ms: u64) -> PortCallRecord {
    PortCallRecord {
        observation_id: Uuid::new_v4(),
        port_id: spec.port_id.clone(),
        capability_id: capability_id.to_string(),
        invocation_id: Uuid::new_v4(),
        success: false,
        failure_class: Some(failure_class),
        raw_result: serde_json::Value::Null,
        structured_result: serde_json::json!({ "error": msg }),
        effect_patch: None,
        side_effect_summary: Some("none".to_string()),
        latency_ms,
        resource_cost: 0.0,
        confidence: 0.0,
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

fn build_spec() -> PortSpec {
    let any_schema = SchemaRef { schema: serde_json::json!({"type": "object"}) };
    let low_cost = CostProfile {
        cpu_cost_class: CostClass::Low,
        memory_cost_class: CostClass::Low,
        io_cost_class: CostClass::Low,
        network_cost_class: CostClass::Low,
        energy_cost_class: CostClass::Negligible,
    };
    let db_latency = LatencyProfile { expected_latency_ms: 10, p95_latency_ms: 100, max_latency_ms: 30_000 };

    let cap = |name: &str, purpose: &str, effect: SideEffectClass, risk: RiskClass| -> PortCapabilitySpec {
        PortCapabilitySpec {
            capability_id: name.to_string(),
            name: name.to_string(),
            purpose: purpose.to_string(),
            input_schema: any_schema.clone(),
            output_schema: any_schema.clone(),
            effect_class: effect,
            rollback_support: RollbackSupport::Irreversible,
            determinism_class: DeterminismClass::PartiallyDeterministic,
            idempotence_class: if matches!(effect, SideEffectClass::ReadOnly) { IdempotenceClass::Idempotent } else { IdempotenceClass::NonIdempotent },
            risk_class: risk,
            latency_profile: db_latency.clone(),
            cost_profile: low_cost.clone(),
            remote_exposable: false,
            auth_override: None,
        }
    };

    PortSpec {
        port_id: "soma.mongodb".to_string(),
        name: "mongodb".to_string(),
        version: Version::new(0, 1, 0),
        kind: PortKind::Database,
        description: "MongoDB document database operations".to_string(),
        namespace: "soma.ports".to_string(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![
            cap("find", "Find documents matching a filter", SideEffectClass::ReadOnly, RiskClass::Low),
            cap("find_one", "Find a single document", SideEffectClass::ReadOnly, RiskClass::Low),
            cap("insert_one", "Insert a single document", SideEffectClass::ExternalStateMutation, RiskClass::Low),
            cap("insert_many", "Insert multiple documents", SideEffectClass::ExternalStateMutation, RiskClass::Low),
            cap("update_one", "Update a single document", SideEffectClass::ExternalStateMutation, RiskClass::Medium),
            cap("delete_one", "Delete a single document", SideEffectClass::Destructive, RiskClass::High),
            cap("count", "Count documents matching a filter", SideEffectClass::ReadOnly, RiskClass::Negligible),
        ],
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
        latency_profile: db_latency,
        cost_profile: low_cost,
        auth_requirements: AuthRequirements { methods: vec![AuthMethod::LocalProcessTrust], required: false },
        sandbox_requirements: SandboxRequirements {
            filesystem_access: false, network_access: true, device_access: false, process_access: false,
            memory_limit_mb: None, cpu_limit_percent: None, time_limit_ms: Some(30_000), syscall_limit: None,
        },
        observable_fields: vec![],
        validation_rules: vec![],
        remote_exposure: false,
    }
}

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    Box::into_raw(Box::new(MongoDbPort::new()))
}
