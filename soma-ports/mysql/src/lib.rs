//! SOMA MySQL port pack — database operations via the `mysql` crate with
//! synchronous connections.
//!
//! Provides 5 capabilities mirroring the postgres port pattern:
//!
//! - **Raw SQL**: `query`, `execute`
//! - **Row-level CRUD**: `insert`, `update`, `delete`
//!
//! Each capability accepts JSON input and returns JSON output via the Port trait.
//! The connection string is read from `SOMA_MYSQL_URL` or `MYSQL_URL`.

use std::sync::OnceLock;
use std::time::Instant;

use chrono::Utc;
use mysql::prelude::Queryable;
use semver::Version;
use soma_port_sdk::prelude::*;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Port struct
// ---------------------------------------------------------------------------

/// The SOMA MySQL port adapter.
///
/// Holds a lazily-initialized connection URL. Each capability invocation
/// creates a fresh connection.
pub struct MysqlPort {
    spec: PortSpec,
    conn_url: OnceLock<Option<String>>,
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

impl Default for MysqlPort {
    fn default() -> Self {
        Self::new()
    }
}

impl MysqlPort {
    pub fn new() -> Self {
        let spec = Self::build_spec();
        Self {
            spec,
            conn_url: OnceLock::new(),
        }
    }

    /// Resolve the connection URL from environment. Returns None if not set.
    fn conn_url(&self) -> Option<&str> {
        self.conn_url
            .get_or_init(|| {
                std::env::var("SOMA_MYSQL_URL")
                    .or_else(|_| std::env::var("MYSQL_URL"))
                    .ok()
            })
            .as_deref()
    }

    /// Open a fresh connection to MySQL.
    fn connect(&self) -> std::result::Result<mysql::PooledConn, PortError> {
        let url = self.conn_url().ok_or_else(|| {
            PortError::DependencyUnavailable(
                "MySQL connection URL not set. Set SOMA_MYSQL_URL or MYSQL_URL".to_string(),
            )
        })?;
        let pool = mysql::Pool::new(url)
            .map_err(|e| PortError::DependencyUnavailable(format!("MySQL pool error: {e}")))?;
        pool.get_conn()
            .map_err(|e| PortError::DependencyUnavailable(format!("MySQL connection failed: {e}")))
    }

    /// Extract query parameters from the input JSON.
    fn extract_params(input: &serde_json::Value) -> Vec<mysql::Value> {
        match input.get("params") {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .map(|v| match v {
                    serde_json::Value::String(s) => mysql::Value::Bytes(s.as_bytes().to_vec()),
                    serde_json::Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            mysql::Value::Int(i)
                        } else if let Some(f) = n.as_f64() {
                            mysql::Value::Double(f)
                        } else {
                            mysql::Value::Bytes(n.to_string().into_bytes())
                        }
                    }
                    serde_json::Value::Bool(b) => mysql::Value::Int(i64::from(*b)),
                    serde_json::Value::Null => mysql::Value::NULL,
                    other => mysql::Value::Bytes(other.to_string().into_bytes()),
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Convert a mysql::Value to a serde_json::Value.
    fn mysql_value_to_json(val: &mysql::Value) -> serde_json::Value {
        match val {
            mysql::Value::NULL => serde_json::Value::Null,
            mysql::Value::Int(i) => serde_json::json!(i),
            mysql::Value::UInt(u) => serde_json::json!(u),
            mysql::Value::Float(f) => serde_json::json!(f),
            mysql::Value::Double(d) => serde_json::json!(d),
            mysql::Value::Bytes(b) => {
                match String::from_utf8(b.clone()) {
                    Ok(s) => serde_json::Value::String(s),
                    Err(_) => serde_json::Value::String(format!("<binary {} bytes>", b.len())),
                }
            }
            mysql::Value::Date(y, m, d, h, min, s, _us) => {
                serde_json::Value::String(format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}"))
            }
            mysql::Value::Time(neg, d, h, min, s, _us) => {
                let sign = if *neg { "-" } else { "" };
                let total_h = (*d as u32) * 24 + (*h as u32);
                serde_json::Value::String(format!("{sign}{total_h}:{min:02}:{s:02}"))
            }
        }
    }

    /// Convert a mysql Row to a JSON object.
    fn row_to_json(row: mysql::Row) -> serde_json::Value {
        let columns: Vec<String> = row
            .columns_ref()
            .iter()
            .map(|c| c.name_str().to_string())
            .collect();
        let mut map = serde_json::Map::new();
        let values: Vec<mysql::Value> = row.unwrap();
        for (i, col_name) in columns.iter().enumerate() {
            let val = values.get(i).map_or(serde_json::Value::Null, Self::mysql_value_to_json);
            map.insert(col_name.clone(), val);
        }
        serde_json::Value::Object(map)
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

    /// `query` -- execute a SELECT and return all rows as a JSON array.
    fn do_query(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let sql = input
            .get("sql")
            .and_then(|v| v.as_str())
            .ok_or("missing 'sql' field")?;
        let params = Self::extract_params(input);
        let mut conn = self.connect().map_err(|e| e.to_string())?;

        let stmt = conn
            .prep(sql)
            .map_err(|e| format!("MySQL prepare error: {e}"))?;
        let rows: Vec<mysql::Row> = conn
            .exec(stmt, params)
            .map_err(|e| format!("MySQL query error: {e}"))?;

        let values: Vec<serde_json::Value> = rows.into_iter().map(Self::row_to_json).collect();
        Ok(serde_json::json!({ "rows": values, "count": values.len() }))
    }

    /// `execute` -- run INSERT/UPDATE/DELETE, return rows affected.
    fn do_execute(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let sql = input
            .get("sql")
            .and_then(|v| v.as_str())
            .ok_or("missing 'sql' field")?;
        let params = Self::extract_params(input);
        let mut conn = self.connect().map_err(|e| e.to_string())?;

        let stmt = conn
            .prep(sql)
            .map_err(|e| format!("MySQL prepare error: {e}"))?;
        conn.exec_drop(stmt, &params)
            .map_err(|e| format!("MySQL execute error: {e}"))?;

        let affected = conn.affected_rows();
        Ok(serde_json::json!({ "rows_affected": affected }))
    }

    /// `insert` -- build and execute INSERT from JSON object.
    fn do_insert(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let table = input
            .get("table")
            .and_then(|v| v.as_str())
            .ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;

        let data = input
            .get("data")
            .and_then(|v| v.as_object())
            .ok_or("missing or invalid 'data' object")?;

        if data.is_empty() {
            return Err("'data' must contain at least one column".into());
        }

        let mut columns = Vec::new();
        let mut placeholders = Vec::new();
        let mut param_values: Vec<mysql::Value> = Vec::new();

        for (col, val) in data {
            Self::validate_identifier(col)?;
            columns.push(format!("`{col}`"));
            placeholders.push("?".to_string());
            param_values.push(Self::json_to_mysql_value(val));
        }

        let sql = format!(
            "INSERT INTO `{table}` ({}) VALUES ({})",
            columns.join(", "),
            placeholders.join(", ")
        );

        let mut conn = self.connect().map_err(|e| e.to_string())?;
        let stmt = conn
            .prep(&sql)
            .map_err(|e| format!("MySQL prepare error: {e}"))?;
        conn.exec_drop(stmt, param_values)
            .map_err(|e| format!("MySQL insert error: {e}"))?;

        let last_id = conn.last_insert_id();
        let affected = conn.affected_rows();
        Ok(serde_json::json!({ "inserted": true, "last_insert_id": last_id, "rows_affected": affected }))
    }

    /// `update` -- build and execute UPDATE from JSON data + where_clause.
    fn do_update(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let table = input
            .get("table")
            .and_then(|v| v.as_str())
            .ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;

        let data = input
            .get("data")
            .and_then(|v| v.as_object())
            .ok_or("missing or invalid 'data' object")?;

        if data.is_empty() {
            return Err("'data' must contain at least one column".into());
        }

        let where_clause = input
            .get("where_clause")
            .and_then(|v| v.as_str())
            .ok_or("missing 'where_clause' field")?;

        let mut set_clauses = Vec::new();
        let mut param_values: Vec<mysql::Value> = Vec::new();

        for (col, val) in data {
            Self::validate_identifier(col)?;
            set_clauses.push(format!("`{col}` = ?"));
            param_values.push(Self::json_to_mysql_value(val));
        }

        let sql = format!(
            "UPDATE `{table}` SET {} WHERE {where_clause}",
            set_clauses.join(", ")
        );

        let mut conn = self.connect().map_err(|e| e.to_string())?;
        let stmt = conn
            .prep(&sql)
            .map_err(|e| format!("MySQL prepare error: {e}"))?;
        conn.exec_drop(stmt, param_values)
            .map_err(|e| format!("MySQL update error: {e}"))?;

        let affected = conn.affected_rows();
        Ok(serde_json::json!({ "rows_affected": affected }))
    }

    /// `delete` -- DELETE with where_clause.
    fn do_delete(
        &self,
        input: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let table = input
            .get("table")
            .and_then(|v| v.as_str())
            .ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;

        let where_clause = input
            .get("where_clause")
            .and_then(|v| v.as_str())
            .ok_or("missing 'where_clause' field (required to prevent accidental full-table deletion)")?;

        let sql = format!("DELETE FROM `{table}` WHERE {where_clause}");

        let mut conn = self.connect().map_err(|e| e.to_string())?;
        let stmt = conn
            .prep(&sql)
            .map_err(|e| format!("MySQL prepare error: {e}"))?;
        conn.exec_drop(stmt, ())
            .map_err(|e| format!("MySQL delete error: {e}"))?;

        let affected = conn.affected_rows();
        Ok(serde_json::json!({ "rows_affected": affected }))
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn validate_identifier(name: &str) -> std::result::Result<(), String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("empty identifier".into());
        }
        for ch in trimmed.chars() {
            if ch.is_alphanumeric() || ch == '_' || ch == '.' {
                continue;
            }
            return Err(format!(
                "invalid character '{ch}' in identifier '{trimmed}'"
            ));
        }
        Ok(())
    }

    fn json_to_mysql_value(val: &serde_json::Value) -> mysql::Value {
        match val {
            serde_json::Value::Null => mysql::Value::NULL,
            serde_json::Value::Bool(b) => mysql::Value::Int(i64::from(*b)),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    mysql::Value::Int(i)
                } else if let Some(f) = n.as_f64() {
                    mysql::Value::Double(f)
                } else {
                    mysql::Value::Bytes(n.to_string().into_bytes())
                }
            }
            serde_json::Value::String(s) => mysql::Value::Bytes(s.as_bytes().to_vec()),
            other => mysql::Value::Bytes(other.to_string().into_bytes()),
        }
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
            expected_latency_ms: 20,
            p95_latency_ms: 200,
            max_latency_ms: 30_000,
        };

        let capabilities = vec![
            Self::cap(
                "query",
                "Execute a SELECT query and return rows",
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
                "execute",
                "Execute an INSERT/UPDATE/DELETE and return rows affected",
                CapabilityBehavior::new(
                    SideEffectClass::ExternalStateMutation,
                    RollbackSupport::CompensatingAction,
                    DeterminismClass::PartiallyDeterministic,
                    IdempotenceClass::NonIdempotent,
                    RiskClass::Medium,
                ),
                &db_latency,
                &low_cost,
            ),
            Self::cap(
                "insert",
                "Insert a row from a JSON data object",
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
                "update",
                "Update rows matching a WHERE clause",
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
                "delete",
                "Delete rows matching a WHERE clause",
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
        ];

        PortSpec {
            port_id: "soma.mysql".to_string(),
            name: "mysql".to_string(),
            version: Version::new(0, 1, 0),
            kind: PortKind::Database,
            description: "MySQL database operations: raw SQL queries, INSERT/UPDATE/DELETE, ORM-style CRUD"
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

impl Port for MysqlPort {
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
            "query" => self.do_query(&input),
            "execute" => self.do_execute(&input),
            "insert" => self.do_insert(&input),
            "update" => self.do_update(&input),
            "delete" => self.do_delete(&input),
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
                    "query" => "read_only",
                    "execute" | "insert" | "update" => "external_state_mutation",
                    "delete" => "destructive",
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
            "query" | "execute" => {
                if input.get("sql").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'sql' field".into()));
                }
            }
            "insert" => {
                if input.get("table").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'table' field".into()));
                }
                if input.get("data").and_then(|v| v.as_object()).is_none() {
                    return Err(PortError::Validation("missing 'data' object".into()));
                }
            }
            "update" => {
                if input.get("table").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'table' field".into()));
                }
                if input.get("data").and_then(|v| v.as_object()).is_none() {
                    return Err(PortError::Validation("missing 'data' object".into()));
                }
                if input.get("where_clause").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'where_clause' field".into()));
                }
            }
            "delete" => {
                if input.get("table").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'table' field".into()));
                }
                if input.get("where_clause").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'where_clause' field".into()));
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
    let port = MysqlPort::new();
    Box::into_raw(Box::new(port))
}
