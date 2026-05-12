use std::sync::Mutex;
use std::time::Instant;

use chrono::Utc;
use postgres::{Client, NoTls};
use serde_json::{json, Value};
use uuid::Uuid;

use soma_next::errors::{Result, SomaError};
use soma_next::runtime::port::Port;
use soma_next::types::common::{
    AuthRequirements, CostClass, CostProfile, DeterminismClass,
    IdempotenceClass, LatencyProfile, RiskClass,
    RollbackSupport, SandboxRequirements, SchemaRef, SideEffectClass, TrustLevel,
};
use soma_next::types::observation::PortCallRecord;
use soma_next::types::port::{PortBackend, PortCapabilitySpec, PortKind, PortLifecycleState, PortSpec};

const PORT_ID: &str = "knowledge";

fn cap(id: &str, name: &str, purpose: &str) -> PortCapabilitySpec {
    PortCapabilitySpec {
        capability_id: id.to_string(),
        name: name.to_string(),
        purpose: purpose.to_string(),
        input_schema: SchemaRef { schema: json!({}) },
        output_schema: SchemaRef { schema: json!({}) },
        effect_class: SideEffectClass::ReadOnly,
        rollback_support: RollbackSupport::Irreversible,
        determinism_class: DeterminismClass::PartiallyDeterministic,
        idempotence_class: IdempotenceClass::Idempotent,
        risk_class: RiskClass::Negligible,
        latency_profile: LatencyProfile {
            expected_latency_ms: 20,
            p95_latency_ms: 100,
            max_latency_ms: 5000,
        },
        cost_profile: CostProfile {
            cpu_cost_class: CostClass::Low,
            memory_cost_class: CostClass::Low,
            io_cost_class: CostClass::Low,
            network_cost_class: CostClass::Low,
            energy_cost_class: CostClass::Negligible,
        },
        remote_exposable: false,
        auth_override: None,
    }
}

fn make_record(capability_id: &str, result: Value, latency_ms: u64) -> PortCallRecord {
    PortCallRecord {
        observation_id: Uuid::new_v4(),
        port_id: PORT_ID.to_string(),
        capability_id: capability_id.to_string(),
        invocation_id: Uuid::new_v4(),
        success: true,
        failure_class: None,
        raw_result: result.clone(),
        structured_result: result,
        effect_patch: None,
        side_effect_summary: None,
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

fn build_spec() -> PortSpec {
    PortSpec {
        port_id: PORT_ID.to_string(),
        name: "knowledge".to_string(),
        version: semver::Version::new(0, 1, 0),
        kind: PortKind::Database,
        description: "pgvector knowledge store with vector and keyword search".to_string(),
        namespace: "soma.ports.knowledge".to_string(),
        trust_level: TrustLevel::Verified,
        capabilities: vec![
            cap("vector_search", "Vector Search", "Semantic similarity search via pgvector"),
            cap("keyword_search", "Keyword Search", "Full-text search via tsvector"),
            cap("category_filter", "Category Filter", "Filter documents by category"),
            cap("get_document", "Get Document", "Retrieve document by ID"),
        ],
        input_schema: SchemaRef { schema: json!({}) },
        output_schema: SchemaRef { schema: json!({}) },
        failure_modes: vec![],
        side_effect_class: SideEffectClass::ReadOnly,
        latency_profile: LatencyProfile {
            expected_latency_ms: 20,
            p95_latency_ms: 100,
            max_latency_ms: 5000,
        },
        cost_profile: CostProfile {
            cpu_cost_class: CostClass::Low,
            memory_cost_class: CostClass::Low,
            io_cost_class: CostClass::Low,
            network_cost_class: CostClass::Low,
            energy_cost_class: CostClass::Negligible,
        },
        auth_requirements: AuthRequirements {
            methods: vec![],
            required: false,
        },
        sandbox_requirements: SandboxRequirements {
            network_access: true,
            filesystem_access: false,
            device_access: false,
            process_access: false,
            memory_limit_mb: None,
            cpu_limit_percent: None,
            time_limit_ms: None,
            syscall_limit: None,
        },
        observable_fields: vec![],
        validation_rules: vec![],
        remote_exposure: false,
        backend: PortBackend::default(),
    }
}

pub struct KnowledgePort {
    spec: PortSpec,
    conn: Mutex<Option<Client>>,
    conn_string: String,
}

impl KnowledgePort {
    pub fn new(conn_string: &str) -> Self {
        Self {
            spec: build_spec(),
            conn: Mutex::new(None),
            conn_string: conn_string.to_string(),
        }
    }

    fn get_conn(&self) -> Result<std::sync::MutexGuard<'_, Option<Client>>> {
        let mut guard = self
            .conn
            .lock()
            .map_err(|e| SomaError::Port(format!("lock poisoned: {e}")))?;
        if guard.is_none() {
            let client = Client::connect(&self.conn_string, NoTls)
                .map_err(|e| SomaError::Port(format!("pgvector connect failed: {e}")))?;
            *guard = Some(client);
        }
        Ok(guard)
    }

    fn vector_search(&self, input: &Value) -> Result<PortCallRecord> {
        let t = Instant::now();
        let query_text = input["query"].as_str().unwrap_or("");
        let limit = input["limit"].as_u64().unwrap_or(5) as i64;

        let embedding = crate::embed::hash_embed(query_text);
        let embedding_str = format!(
            "[{}]",
            embedding.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")
        );

        let mut guard = self.get_conn()?;
        let client = guard.as_mut().unwrap();
        let stmt = format!(
            "SELECT id, title, content, category,
                    1 - (embedding <=> '{embedding_str}'::vector) as similarity
             FROM documents
             WHERE embedding IS NOT NULL
             ORDER BY embedding <=> '{embedding_str}'::vector
             LIMIT {limit}"
        );
        let rows = client
            .query(&stmt[..], &[])
            .map_err(|e| SomaError::Port(format!("vector_search: {e}")))?;

        let docs: Vec<Value> = rows
            .iter()
            .map(|row| {
                json!({
                    "id": row.get::<_, i32>("id"),
                    "title": row.get::<_, String>("title"),
                    "content": row.get::<_, String>("content"),
                    "category": row.get::<_, String>("category"),
                    "similarity": row.get::<_, f64>("similarity"),
                })
            })
            .collect();

        let ms = t.elapsed().as_millis() as u64;
        Ok(make_record(
            "vector_search",
            json!({ "documents": docs, "match_count": docs.len(), "strategy": "vector" }),
            ms,
        ))
    }

    fn keyword_search(&self, input: &Value) -> Result<PortCallRecord> {
        let t = Instant::now();
        let query_text = input["query"].as_str().unwrap_or("");
        let limit = input["limit"].as_u64().unwrap_or(5) as i64;

        let mut guard = self.get_conn()?;
        let client = guard.as_mut().unwrap();
        let rows = client
            .query(
                "SELECT id, title, content, category,
                        ts_rank(to_tsvector('english', title || ' ' || content),
                                plainto_tsquery('english', $1)) as rank
                 FROM documents
                 WHERE to_tsvector('english', title || ' ' || content)
                       @@ plainto_tsquery('english', $1)
                 ORDER BY rank DESC
                 LIMIT $2",
                &[&query_text, &limit],
            )
            .map_err(|e| SomaError::Port(format!("keyword_search: {e}")))?;

        let docs: Vec<Value> = rows
            .iter()
            .map(|row| {
                json!({
                    "id": row.get::<_, i32>("id"),
                    "title": row.get::<_, String>("title"),
                    "content": row.get::<_, String>("content"),
                    "category": row.get::<_, String>("category"),
                    "rank": row.get::<_, f32>("rank"),
                })
            })
            .collect();

        let ms = t.elapsed().as_millis() as u64;
        Ok(make_record(
            "keyword_search",
            json!({ "documents": docs, "match_count": docs.len(), "strategy": "keyword" }),
            ms,
        ))
    }

    fn category_filter(&self, input: &Value) -> Result<PortCallRecord> {
        let t = Instant::now();
        let category = input["category"].as_str().unwrap_or("");
        let limit = input["limit"].as_u64().unwrap_or(10) as i64;

        let mut guard = self.get_conn()?;
        let client = guard.as_mut().unwrap();
        let rows = client
            .query(
                "SELECT id, title, content, category
                 FROM documents WHERE category = $1
                 ORDER BY created_at DESC LIMIT $2",
                &[&category, &limit],
            )
            .map_err(|e| SomaError::Port(format!("category_filter: {e}")))?;

        let docs: Vec<Value> = rows
            .iter()
            .map(|row| {
                json!({
                    "id": row.get::<_, i32>("id"),
                    "title": row.get::<_, String>("title"),
                    "content": row.get::<_, String>("content"),
                    "category": row.get::<_, String>("category"),
                })
            })
            .collect();

        let ms = t.elapsed().as_millis() as u64;
        Ok(make_record(
            "category_filter",
            json!({ "documents": docs, "match_count": docs.len(), "strategy": "category" }),
            ms,
        ))
    }

    fn get_document(&self, input: &Value) -> Result<PortCallRecord> {
        let t = Instant::now();
        let doc_id = input["id"].as_i64().unwrap_or(0) as i32;

        let mut guard = self.get_conn()?;
        let client = guard.as_mut().unwrap();
        let rows = client
            .query(
                "SELECT id, title, content, category FROM documents WHERE id = $1",
                &[&doc_id],
            )
            .map_err(|e| SomaError::Port(format!("get_document: {e}")))?;

        let doc = rows.first().map(|row| {
            json!({
                "id": row.get::<_, i32>("id"),
                "title": row.get::<_, String>("title"),
                "content": row.get::<_, String>("content"),
                "category": row.get::<_, String>("category"),
            })
        });

        let ms = t.elapsed().as_millis() as u64;
        Ok(make_record(
            "get_document",
            json!({ "document": doc, "found": doc.is_some() }),
            ms,
        ))
    }
}

impl Port for KnowledgePort {
    fn spec(&self) -> &PortSpec {
        &self.spec
    }

    fn invoke(&self, capability_id: &str, input: Value) -> Result<PortCallRecord> {
        match capability_id {
            "vector_search" => self.vector_search(&input),
            "keyword_search" => self.keyword_search(&input),
            "category_filter" => self.category_filter(&input),
            "get_document" => self.get_document(&input),
            other => Err(SomaError::Port(format!("unknown capability: {other}"))),
        }
    }

    fn validate_input(&self, _capability_id: &str, _input: &Value) -> Result<()> {
        Ok(())
    }

    fn lifecycle_state(&self) -> PortLifecycleState {
        if self.conn.lock().map(|g| g.is_some()).unwrap_or(false) {
            PortLifecycleState::Active
        } else {
            PortLifecycleState::Validated
        }
    }
}
