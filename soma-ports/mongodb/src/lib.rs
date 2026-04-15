//! SOMA MongoDB port pack — document database operations via the `mongodb` crate
//! with synchronous connections.
//!
//! Provides 7 capabilities:
//!
//! - **Read**: `find`, `find_one`, `count`
//! - **Write**: `insert_one`, `insert_many`
//! - **Mutate**: `update_one`, `delete_one`
//!
//! Each capability accepts JSON input and returns JSON output via the Port trait.
//! The connection string is read from `SOMA_MONGODB_URL` or `MONGODB_URL`.
//! The database name is read from `SOMA_MONGODB_DATABASE`.

use std::sync::OnceLock;
use std::time::Instant;

use chrono::Utc;
use mongodb::bson::{self, Bson, Document};
use mongodb::sync::{Client, Collection};
use semver::Version;
use soma_port_sdk::prelude::*;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Port struct
// ---------------------------------------------------------------------------

pub struct MongoDbPort {
    spec: PortSpec,
    client: OnceLock<Option<Client>>,
    database_name: OnceLock<String>,
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

impl Default for MongoDbPort {
    fn default() -> Self {
        Self::new()
    }
}

impl MongoDbPort {
    pub fn new() -> Self {
        let spec = Self::build_spec();
        Self {
            spec,
            client: OnceLock::new(),
            database_name: OnceLock::new(),
        }
    }

    fn db_name(&self) -> &str {
        self.database_name.get_or_init(|| {
            std::env::var("SOMA_MONGODB_DATABASE").unwrap_or_else(|_| "soma".to_string())
        })
    }

    fn get_client(&self) -> std::result::Result<&Client, PortError> {
        let client_opt = self.client.get_or_init(|| {
            let url = std::env::var("SOMA_MONGODB_URL")
                .or_else(|_| std::env::var("MONGODB_URL"))
                .ok()?;
            Client::with_uri_str(&url).ok()
        });
        client_opt.as_ref().ok_or_else(|| {
            PortError::DependencyUnavailable(
                "MongoDB connection not available. Set SOMA_MONGODB_URL or MONGODB_URL".to_string(),
            )
        })
    }

    fn collection(&self, name: &str) -> std::result::Result<Collection<Document>, PortError> {
        let client = self.get_client()?;
        let db = client.database(self.db_name());
        Ok(db.collection(name))
    }

    /// Convert a serde_json::Value to a BSON Document. If the value is an
    /// object, convert directly. Otherwise wrap in a single-key doc.
    fn json_to_doc(val: &serde_json::Value) -> std::result::Result<Document, String> {
        match val {
            serde_json::Value::Object(_) => {
                let bson_val = bson::to_bson(val).map_err(|e| format!("BSON conversion error: {e}"))?;
                match bson_val {
                    Bson::Document(doc) => Ok(doc),
                    _ => Err("expected BSON document".to_string()),
                }
            }
            _ => Err("expected a JSON object".to_string()),
        }
    }

    /// Convert a BSON Document to a serde_json::Value.
    fn doc_to_json(doc: &Document) -> serde_json::Value {
        bson::to_bson(doc)
            .ok()
            .and_then(|b| {
                let relaxed = b.into_relaxed_extjson();
                Some(relaxed)
            })
            .unwrap_or(serde_json::Value::Null)
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

    /// `find` -- find multiple documents with optional filter and limit.
    fn do_find(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let coll_name = input
            .get("collection")
            .and_then(|v| v.as_str())
            .ok_or("missing 'collection' field")?;

        let filter = match input.get("filter") {
            Some(f) if !f.is_null() => Some(Self::json_to_doc(f)?),
            _ => None,
        };

        let limit = input.get("limit").and_then(|v| v.as_i64());

        let coll = self.collection(coll_name).map_err(|e| e.to_string())?;

        let mut opts = mongodb::options::FindOptions::default();
        if let Some(l) = limit {
            opts.limit = Some(l);
        }

        let cursor = coll
            .find(filter.unwrap_or_default())
            .with_options(opts)
            .run()
            .map_err(|e| format!("MongoDB find error: {e}"))?;

        let mut docs = Vec::new();
        for result in cursor {
            let doc = result.map_err(|e| format!("MongoDB cursor error: {e}"))?;
            docs.push(Self::doc_to_json(&doc));
        }

        Ok(serde_json::json!({ "documents": docs, "count": docs.len() }))
    }

    /// `find_one` -- find a single document matching a filter.
    fn do_find_one(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let coll_name = input
            .get("collection")
            .and_then(|v| v.as_str())
            .ok_or("missing 'collection' field")?;

        let filter = input
            .get("filter")
            .ok_or("missing 'filter' field")
            .and_then(|f| Self::json_to_doc(f).map_err(|e| e.to_string().leak() as &str))?;

        let coll = self.collection(coll_name).map_err(|e| e.to_string())?;

        let result = coll
            .find_one(filter)
            .run()
            .map_err(|e| format!("MongoDB find_one error: {e}"))?;

        match result {
            Some(doc) => Ok(Self::doc_to_json(&doc)),
            None => Ok(serde_json::Value::Null),
        }
    }

    /// `insert_one` -- insert a single document.
    fn do_insert_one(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let coll_name = input
            .get("collection")
            .and_then(|v| v.as_str())
            .ok_or("missing 'collection' field")?;

        let document = input
            .get("document")
            .ok_or("missing 'document' field")?;
        let doc = Self::json_to_doc(document)?;

        let coll = self.collection(coll_name).map_err(|e| e.to_string())?;

        let result = coll
            .insert_one(doc)
            .run()
            .map_err(|e| format!("MongoDB insert_one error: {e}"))?;

        let id_str = format!("{}", result.inserted_id);
        Ok(serde_json::json!({ "inserted_id": id_str }))
    }

    /// `insert_many` -- insert multiple documents.
    fn do_insert_many(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let coll_name = input
            .get("collection")
            .and_then(|v| v.as_str())
            .ok_or("missing 'collection' field")?;

        let documents = input
            .get("documents")
            .and_then(|v| v.as_array())
            .ok_or("missing 'documents' array")?;

        let docs: Vec<Document> = documents
            .iter()
            .map(Self::json_to_doc)
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let coll = self.collection(coll_name).map_err(|e| e.to_string())?;

        let result = coll
            .insert_many(docs)
            .run()
            .map_err(|e| format!("MongoDB insert_many error: {e}"))?;

        let ids: Vec<String> = result
            .inserted_ids
            .values()
            .map(|id| format!("{id}"))
            .collect();
        Ok(serde_json::json!({ "inserted_ids": ids, "count": ids.len() }))
    }

    /// `update_one` -- update a single document.
    fn do_update_one(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let coll_name = input
            .get("collection")
            .and_then(|v| v.as_str())
            .ok_or("missing 'collection' field")?;

        let filter = input
            .get("filter")
            .ok_or("missing 'filter' field")?;
        let filter_doc = Self::json_to_doc(filter)?;

        let update = input
            .get("update")
            .ok_or("missing 'update' field")?;
        let update_doc = Self::json_to_doc(update)?;

        let coll = self.collection(coll_name).map_err(|e| e.to_string())?;

        let result = coll
            .update_one(filter_doc, update_doc)
            .run()
            .map_err(|e| format!("MongoDB update_one error: {e}"))?;

        Ok(serde_json::json!({
            "matched_count": result.matched_count,
            "modified_count": result.modified_count
        }))
    }

    /// `delete_one` -- delete a single document.
    fn do_delete_one(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let coll_name = input
            .get("collection")
            .and_then(|v| v.as_str())
            .ok_or("missing 'collection' field")?;

        let filter = input
            .get("filter")
            .ok_or("missing 'filter' field")?;
        let filter_doc = Self::json_to_doc(filter)?;

        let coll = self.collection(coll_name).map_err(|e| e.to_string())?;

        let result = coll
            .delete_one(filter_doc)
            .run()
            .map_err(|e| format!("MongoDB delete_one error: {e}"))?;

        Ok(serde_json::json!({ "deleted_count": result.deleted_count }))
    }

    /// `count` -- count documents matching a filter.
    fn do_count(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let coll_name = input
            .get("collection")
            .and_then(|v| v.as_str())
            .ok_or("missing 'collection' field")?;

        let filter = match input.get("filter") {
            Some(f) if !f.is_null() => Some(Self::json_to_doc(f)?),
            _ => None,
        };

        let coll = self.collection(coll_name).map_err(|e| e.to_string())?;

        let count = coll
            .count_documents(filter.unwrap_or_default())
            .run()
            .map_err(|e| format!("MongoDB count error: {e}"))?;

        Ok(serde_json::json!({ "count": count }))
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

        let db_latency = LatencyProfile {
            expected_latency_ms: 10,
            p95_latency_ms: 100,
            max_latency_ms: 30_000,
        };

        let capabilities = vec![
            Self::cap(
                "find",
                "Find documents matching an optional filter",
                CapabilityBehavior::new(
                    SideEffectClass::ReadOnly,
                    RollbackSupport::Irreversible,
                    DeterminismClass::PartiallyDeterministic,
                    IdempotenceClass::Idempotent,
                    RiskClass::Low,
                ),
                &db_latency,
                &low_cost,
            ),
            Self::cap(
                "find_one",
                "Find a single document matching a filter",
                CapabilityBehavior::new(
                    SideEffectClass::ReadOnly,
                    RollbackSupport::Irreversible,
                    DeterminismClass::PartiallyDeterministic,
                    IdempotenceClass::Idempotent,
                    RiskClass::Low,
                ),
                &db_latency,
                &low_cost,
            ),
            Self::cap(
                "insert_one",
                "Insert a single document",
                CapabilityBehavior::new(
                    SideEffectClass::ExternalStateMutation,
                    RollbackSupport::CompensatingAction,
                    DeterminismClass::PartiallyDeterministic,
                    IdempotenceClass::NonIdempotent,
                    RiskClass::Low,
                ),
                &db_latency,
                &low_cost,
            ),
            Self::cap(
                "insert_many",
                "Insert multiple documents",
                CapabilityBehavior::new(
                    SideEffectClass::ExternalStateMutation,
                    RollbackSupport::CompensatingAction,
                    DeterminismClass::PartiallyDeterministic,
                    IdempotenceClass::NonIdempotent,
                    RiskClass::Low,
                ),
                &db_latency,
                &low_cost,
            ),
            Self::cap(
                "update_one",
                "Update a single document matching a filter",
                CapabilityBehavior::new(
                    SideEffectClass::ExternalStateMutation,
                    RollbackSupport::CompensatingAction,
                    DeterminismClass::PartiallyDeterministic,
                    IdempotenceClass::ConditionallyIdempotent,
                    RiskClass::Medium,
                ),
                &db_latency,
                &low_cost,
            ),
            Self::cap(
                "delete_one",
                "Delete a single document matching a filter",
                CapabilityBehavior::new(
                    SideEffectClass::Destructive,
                    RollbackSupport::Irreversible,
                    DeterminismClass::PartiallyDeterministic,
                    IdempotenceClass::Idempotent,
                    RiskClass::High,
                ),
                &db_latency,
                &low_cost,
            ),
            Self::cap(
                "count",
                "Count documents matching an optional filter",
                CapabilityBehavior::new(
                    SideEffectClass::ReadOnly,
                    RollbackSupport::Irreversible,
                    DeterminismClass::PartiallyDeterministic,
                    IdempotenceClass::Idempotent,
                    RiskClass::Negligible,
                ),
                &db_latency,
                &low_cost,
            ),
        ];

        PortSpec {
            port_id: "soma.mongodb".to_string(),
            name: "mongodb".to_string(),
            version: Version::new(0, 1, 0),
            kind: PortKind::Database,
            description: "MongoDB document database operations: find, insert, update, delete, count"
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
            latency_profile: db_latency,
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

impl Port for MongoDbPort {
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
            "find" => self.do_find(&input),
            "find_one" => self.do_find_one(&input),
            "insert_one" => self.do_insert_one(&input),
            "insert_many" => self.do_insert_many(&input),
            "update_one" => self.do_update_one(&input),
            "delete_one" => self.do_delete_one(&input),
            "count" => self.do_count(&input),
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
                    "find" | "find_one" | "count" => "read_only",
                    "insert_one" | "insert_many" | "update_one" => "external_state_mutation",
                    "delete_one" => "destructive",
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
            "find" | "count" => {
                if input.get("collection").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'collection' field".into()));
                }
            }
            "find_one" => {
                if input.get("collection").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'collection' field".into()));
                }
                if input.get("filter").is_none() {
                    return Err(PortError::Validation("missing 'filter' field".into()));
                }
            }
            "insert_one" => {
                if input.get("collection").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'collection' field".into()));
                }
                if input.get("document").is_none() {
                    return Err(PortError::Validation("missing 'document' field".into()));
                }
            }
            "insert_many" => {
                if input.get("collection").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'collection' field".into()));
                }
                if input.get("documents").and_then(|v| v.as_array()).is_none() {
                    return Err(PortError::Validation("missing 'documents' array".into()));
                }
            }
            "update_one" => {
                if input.get("collection").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'collection' field".into()));
                }
                if input.get("filter").is_none() {
                    return Err(PortError::Validation("missing 'filter' field".into()));
                }
                if input.get("update").is_none() {
                    return Err(PortError::Validation("missing 'update' field".into()));
                }
            }
            "delete_one" => {
                if input.get("collection").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'collection' field".into()));
                }
                if input.get("filter").is_none() {
                    return Err(PortError::Validation("missing 'filter' field".into()));
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
    let port = MongoDbPort::new();
    Box::into_raw(Box::new(port))
}
