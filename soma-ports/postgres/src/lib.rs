//! SOMA PostgreSQL port pack — database operations via `tokio-postgres` with
//! synchronous `block_on()` bridging.
//!
//! Provides 15 capabilities matching the soma-core postgres plugin conventions:
//!
//! - **Raw SQL**: `query`, `execute`
//! - **ORM-style**: `find`, `find_many`, `count`, `aggregate`
//! - **Row-level CRUD**: `insert`, `update`, `delete`
//! - **DDL**: `create_table`, `drop_table`, `alter_table`
//! - **Transactions**: `begin_transaction`, `commit`, `rollback`
//!
//! Each capability accepts JSON input and returns JSON output via the Port trait.
//! The connection string is read from `SOMA_POSTGRES_URL` or defaults to
//! `host=localhost dbname=soma`.

use std::fmt::Write as _;
use std::sync::OnceLock;
use std::time::Instant;

use chrono::Utc;
use semver::Version;
use soma_port_sdk::prelude::*;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Port struct
// ---------------------------------------------------------------------------

/// The SOMA PostgreSQL port adapter.
///
/// Holds a lazily-initialized Tokio runtime and connection string. Each
/// capability invocation creates a fresh connection through `block_on()`.
pub struct PostgresPort {
    spec: PortSpec,
    conn_string: OnceLock<String>,
    runtime: OnceLock<tokio::runtime::Runtime>,
}

impl PostgresPort {
    pub fn new() -> Self {
        let spec = Self::build_spec();
        Self {
            spec,
            conn_string: OnceLock::new(),
            runtime: OnceLock::new(),
        }
    }

    /// Get or create the Tokio runtime used for async postgres operations.
    fn rt(&self) -> &tokio::runtime::Runtime {
        self.runtime.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to create tokio runtime for postgres port")
        })
    }

    /// Resolve the connection string from environment or default.
    fn conn_str(&self) -> &str {
        self.conn_string.get_or_init(|| {
            std::env::var("SOMA_POSTGRES_URL")
                .unwrap_or_else(|_| "host=localhost dbname=soma".to_string())
        })
    }

    /// Open a fresh connection to PostgreSQL using `block_on()`.
    fn connect(&self) -> std::result::Result<tokio_postgres::Client, PortError> {
        let conn_str = self.conn_str();
        self.rt().block_on(async {
            let (client, connection) =
                tokio_postgres::connect(conn_str, tokio_postgres::NoTls)
                    .await
                    .map_err(|e| PortError::DependencyUnavailable(format!("PostgreSQL connection failed: {e}")))?;

            // Spawn the connection handler so it processes messages in the background.
            tokio::spawn(async move {
                if let Err(e) = connection.await {
                    eprintln!("postgres connection error: {e}");
                }
            });

            Ok(client)
        })
    }

    /// Format a `tokio_postgres::Error` with database-level detail when available.
    fn format_pg_error(e: &tokio_postgres::Error) -> String {
        e.as_db_error().map_or_else(
            || e.to_string(),
            |db_err| {
                format!(
                    "{}: {} ({})",
                    db_err.severity(),
                    db_err.message(),
                    db_err.code().code()
                )
            },
        )
    }

    /// Convert a `tokio_postgres::Row` into a `serde_json::Value` object.
    fn row_to_json(row: &tokio_postgres::Row) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (i, col) in row.columns().iter().enumerate() {
            let name = col.name().to_string();
            let val = Self::column_value(row, i, col.type_());
            map.insert(name, val);
        }
        serde_json::Value::Object(map)
    }

    /// Extract a single column value, mapping PostgreSQL types to JSON.
    fn column_value(
        row: &tokio_postgres::Row,
        idx: usize,
        ty: &tokio_postgres::types::Type,
    ) -> serde_json::Value {
        use tokio_postgres::types::Type;

        match *ty {
            Type::BOOL => match row.try_get::<_, Option<bool>>(idx) {
                Ok(Some(v)) => serde_json::Value::Bool(v),
                _ => serde_json::Value::Null,
            },
            Type::INT2 => match row.try_get::<_, Option<i16>>(idx) {
                Ok(Some(v)) => serde_json::json!(v),
                _ => serde_json::Value::Null,
            },
            Type::INT4 => match row.try_get::<_, Option<i32>>(idx) {
                Ok(Some(v)) => serde_json::json!(v),
                _ => serde_json::Value::Null,
            },
            Type::INT8 => match row.try_get::<_, Option<i64>>(idx) {
                Ok(Some(v)) => serde_json::json!(v),
                _ => serde_json::Value::Null,
            },
            Type::FLOAT4 => match row.try_get::<_, Option<f32>>(idx) {
                Ok(Some(v)) => serde_json::json!(v),
                _ => serde_json::Value::Null,
            },
            Type::FLOAT8 => match row.try_get::<_, Option<f64>>(idx) {
                Ok(Some(v)) => serde_json::json!(v),
                _ => serde_json::Value::Null,
            },
            Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => {
                match row.try_get::<_, Option<String>>(idx) {
                    Ok(Some(v)) => serde_json::Value::String(v),
                    _ => serde_json::Value::Null,
                }
            }
            Type::JSON | Type::JSONB => {
                match row.try_get::<_, Option<serde_json::Value>>(idx) {
                    Ok(Some(v)) => v,
                    _ => serde_json::Value::Null,
                }
            }
            Type::UUID => match row.try_get::<_, Option<uuid::Uuid>>(idx) {
                Ok(Some(v)) => serde_json::Value::String(v.to_string()),
                _ => serde_json::Value::Null,
            },
            Type::TIMESTAMP | Type::TIMESTAMPTZ => {
                match row.try_get::<_, Option<chrono::NaiveDateTime>>(idx) {
                    Ok(Some(v)) => {
                        serde_json::Value::String(v.format("%Y-%m-%dT%H:%M:%S").to_string())
                    }
                    Ok(None) => serde_json::Value::Null,
                    Err(_) => match row.try_get::<_, Option<String>>(idx) {
                        Ok(Some(v)) => serde_json::Value::String(v),
                        _ => serde_json::Value::Null,
                    },
                }
            }
            _ => match row.try_get::<_, Option<String>>(idx) {
                Ok(Some(v)) => serde_json::Value::String(v),
                Ok(None) => serde_json::Value::Null,
                Err(_) => serde_json::Value::String(format!("<unsupported type: {ty}>")),
            },
        }
    }

    /// Extract query parameters from the input JSON. Supports `params` as an
    /// array of strings, or omitted/null for no parameters.
    fn extract_params(input: &serde_json::Value) -> Vec<String> {
        match input.get("params") {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    serde_json::Value::Null => String::new(),
                    other => other.to_string(),
                })
                .collect(),
            Some(serde_json::Value::String(s)) => vec![s.clone()],
            _ => Vec::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Observation helpers
    // -----------------------------------------------------------------------

    /// Build a success `PortCallRecord`.
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

    /// Build a failure `PortCallRecord`.
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
    ///
    /// Input: `{ "sql": "SELECT ...", "params": ["val1", "val2"] }`
    fn do_query(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let raw_sql = input
            .get("sql")
            .and_then(|v| v.as_str())
            .ok_or("missing 'sql' field")?;

        // Auto-wrap bare table names into SELECT * FROM <table>.
        let sql = if !raw_sql.trim().is_empty()
            && !raw_sql.trim_start().to_uppercase().starts_with("SELECT")
            && !raw_sql.trim_start().to_uppercase().starts_with("WITH")
            && !raw_sql.contains(' ')
            && raw_sql.chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            format!("SELECT * FROM {raw_sql}")
        } else {
            raw_sql.to_string()
        };

        let params = Self::extract_params(input);
        let client = self.connect().map_err(|e| e.to_string())?;

        self.rt().block_on(async {
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
                .iter()
                .map(|s| s as &(dyn tokio_postgres::types::ToSql + Sync))
                .collect();

            let rows = client
                .query(&*sql, &param_refs)
                .await
                .map_err(|e| Self::format_pg_error(&e))?;

            let values: Vec<serde_json::Value> = rows.iter().map(Self::row_to_json).collect();
            Ok(serde_json::json!({ "rows": values, "count": values.len() }))
        })
    }

    /// `execute` -- run INSERT/UPDATE/DELETE, return rows affected.
    ///
    /// Input: `{ "sql": "INSERT INTO ...", "params": [...] }`
    fn do_execute(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let sql = input
            .get("sql")
            .and_then(|v| v.as_str())
            .ok_or("missing 'sql' field")?;
        let params = Self::extract_params(input);
        let client = self.connect().map_err(|e| e.to_string())?;

        self.rt().block_on(async {
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
                .iter()
                .map(|s| s as &(dyn tokio_postgres::types::ToSql + Sync))
                .collect();

            let count = client
                .execute(sql, &param_refs)
                .await
                .map_err(|e| Self::format_pg_error(&e))?;

            Ok(serde_json::json!({ "rows_affected": count }))
        })
    }

    /// `find` -- ORM-style single row by ID.
    ///
    /// Input: `{ "table": "users", "id": "some-uuid-or-int", "id_column": "id" }`
    fn do_find(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let table = input
            .get("table")
            .and_then(|v| v.as_str())
            .ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;

        let id = input
            .get("id")
            .ok_or("missing 'id' field")?;
        let id_str = match id {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            _ => return Err("'id' must be a string or number".into()),
        };

        let id_column = input
            .get("id_column")
            .and_then(|v| v.as_str())
            .unwrap_or("id");
        Self::validate_identifier(id_column)?;

        let sql = format!("SELECT * FROM {table} WHERE {id_column} = $1 LIMIT 1");
        let client = self.connect().map_err(|e| e.to_string())?;

        self.rt().block_on(async {
            let row_opt = client
                .query_opt(&*sql, &[&id_str as &(dyn tokio_postgres::types::ToSql + Sync)])
                .await
                .map_err(|e| Self::format_pg_error(&e))?;

            Ok(row_opt.map_or(serde_json::Value::Null, |row| Self::row_to_json(&row)))
        })
    }

    /// `find_many` -- ORM-style multi-row query with structured filter.
    ///
    /// Input: `{ "table": "users", "where": {"active": true}, "order_by": [...], "limit": 10, "offset": 0 }`
    fn do_find_many(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let table = input
            .get("table")
            .and_then(|v| v.as_str())
            .ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;

        let mut sql = format!("SELECT * FROM {table}");

        if let Some(where_obj) = input.get("where").and_then(|v| v.as_object()) {
            let clauses = Self::build_where_clauses(where_obj)?;
            if !clauses.is_empty() {
                let _ = write!(sql, " WHERE {}", clauses.join(" AND "));
            }
        }

        if let Some(order_arr) = input.get("order_by").and_then(|v| v.as_array()) {
            let orders = Self::build_order_by(order_arr)?;
            if !orders.is_empty() {
                let _ = write!(sql, " ORDER BY {}", orders.join(", "));
            }
        }

        if let Some(limit) = input.get("limit").and_then(|v| v.as_i64()) {
            let _ = write!(sql, " LIMIT {limit}");
        }

        if let Some(offset) = input.get("offset").and_then(|v| v.as_i64()) {
            let _ = write!(sql, " OFFSET {offset}");
        }

        let client = self.connect().map_err(|e| e.to_string())?;

        self.rt().block_on(async {
            let rows = client
                .query(&*sql, &[])
                .await
                .map_err(|e| Self::format_pg_error(&e))?;

            let values: Vec<serde_json::Value> = rows.iter().map(Self::row_to_json).collect();
            Ok(serde_json::json!({ "rows": values, "count": values.len() }))
        })
    }

    /// `count` -- COUNT query with optional filter.
    ///
    /// Input: `{ "table": "users", "where": {"active": true} }`
    fn do_count(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let table = input
            .get("table")
            .and_then(|v| v.as_str())
            .ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;

        let mut sql = format!("SELECT COUNT(*) FROM {table}");

        if let Some(where_obj) = input.get("where").and_then(|v| v.as_object()) {
            let clauses = Self::build_where_clauses(where_obj)?;
            if !clauses.is_empty() {
                let _ = write!(sql, " WHERE {}", clauses.join(" AND "));
            }
        }

        let client = self.connect().map_err(|e| e.to_string())?;

        self.rt().block_on(async {
            let row = client
                .query_one(&*sql, &[])
                .await
                .map_err(|e| Self::format_pg_error(&e))?;

            let count: i64 = row.get(0);
            Ok(serde_json::json!({ "count": count }))
        })
    }

    /// `aggregate` -- SUM/AVG/MIN/MAX aggregation query.
    ///
    /// Input: `{ "table": "orders", "function": "SUM", "column": "amount", "where": {...}, "group_by": ["status"] }`
    fn do_aggregate(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let table = input
            .get("table")
            .and_then(|v| v.as_str())
            .ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;

        let function = input
            .get("function")
            .and_then(|v| v.as_str())
            .ok_or("missing 'function' field")?
            .to_uppercase();
        if !["SUM", "AVG", "MIN", "MAX", "COUNT"].contains(&function.as_str()) {
            return Err(format!("unsupported aggregate function '{function}'"));
        }

        let column = input
            .get("column")
            .and_then(|v| v.as_str())
            .ok_or("missing 'column' field")?;
        Self::validate_identifier(column)?;

        let mut select_parts = vec![format!("{function}({column}) AS result")];
        let mut group_by_fields: Vec<String> = Vec::new();

        if let Some(gb_arr) = input.get("group_by").and_then(|v| v.as_array()) {
            for item in gb_arr {
                let field = item.as_str().ok_or("group_by items must be strings")?;
                Self::validate_identifier(field)?;
                select_parts.push(field.to_string());
                group_by_fields.push(field.to_string());
            }
        }

        let mut sql = format!("SELECT {} FROM {table}", select_parts.join(", "));

        if let Some(where_obj) = input.get("where").and_then(|v| v.as_object()) {
            let clauses = Self::build_where_clauses(where_obj)?;
            if !clauses.is_empty() {
                let _ = write!(sql, " WHERE {}", clauses.join(" AND "));
            }
        }

        if !group_by_fields.is_empty() {
            let _ = write!(sql, " GROUP BY {}", group_by_fields.join(", "));
        }

        let client = self.connect().map_err(|e| e.to_string())?;

        self.rt().block_on(async {
            let rows = client
                .query(&*sql, &[])
                .await
                .map_err(|e| Self::format_pg_error(&e))?;

            let values: Vec<serde_json::Value> = rows.iter().map(Self::row_to_json).collect();
            Ok(serde_json::json!({ "rows": values }))
        })
    }

    /// `create_table` -- DDL CREATE TABLE IF NOT EXISTS.
    ///
    /// Input: `{ "table": "my_table", "columns": "id SERIAL PRIMARY KEY, name TEXT NOT NULL" }`
    fn do_create_table(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let table = input
            .get("table")
            .and_then(|v| v.as_str())
            .ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;

        let columns = input
            .get("columns")
            .and_then(|v| v.as_str())
            .ok_or("missing 'columns' field")?;

        let sql = format!("CREATE TABLE IF NOT EXISTS {table} ({columns})");
        let client = self.connect().map_err(|e| e.to_string())?;

        self.rt().block_on(async {
            client
                .execute(&*sql, &[])
                .await
                .map_err(|e| Self::format_pg_error(&e))?;
            Ok(serde_json::json!({ "created": table }))
        })
    }

    /// `drop_table` -- DDL DROP TABLE IF EXISTS.
    ///
    /// Input: `{ "table": "my_table", "cascade": false }`
    fn do_drop_table(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let table = input
            .get("table")
            .and_then(|v| v.as_str())
            .ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;

        let cascade = input
            .get("cascade")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let suffix = if cascade { " CASCADE" } else { "" };

        let sql = format!("DROP TABLE IF EXISTS {table}{suffix}");
        let client = self.connect().map_err(|e| e.to_string())?;

        self.rt().block_on(async {
            client
                .execute(&*sql, &[])
                .await
                .map_err(|e| Self::format_pg_error(&e))?;
            Ok(serde_json::json!({ "dropped": table }))
        })
    }

    /// `alter_table` -- DDL ALTER TABLE.
    ///
    /// Input: `{ "table": "my_table", "changes": "ADD COLUMN email TEXT" }`
    fn do_alter_table(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let table = input
            .get("table")
            .and_then(|v| v.as_str())
            .ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;

        let changes = input
            .get("changes")
            .and_then(|v| v.as_str())
            .ok_or("missing 'changes' field")?;

        let sql = format!("ALTER TABLE {table} {changes}");
        let client = self.connect().map_err(|e| e.to_string())?;

        self.rt().block_on(async {
            client
                .execute(&*sql, &[])
                .await
                .map_err(|e| Self::format_pg_error(&e))?;
            Ok(serde_json::json!({ "altered": table }))
        })
    }

    /// `insert` -- row-level INSERT with column/value pairs.
    ///
    /// Input: `{ "table": "users", "values": {"name": "Alice", "email": "alice@example.com"}, "returning": "*" }`
    fn do_insert(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let table = input
            .get("table")
            .and_then(|v| v.as_str())
            .ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;

        let values = input
            .get("values")
            .and_then(|v| v.as_object())
            .ok_or("missing or invalid 'values' object")?;

        if values.is_empty() {
            return Err("'values' must contain at least one column".into());
        }

        let mut columns = Vec::new();
        let mut placeholders = Vec::new();
        let mut param_values: Vec<String> = Vec::new();

        for (i, (col, val)) in values.iter().enumerate() {
            Self::validate_identifier(col)?;
            columns.push(col.as_str());
            placeholders.push(format!("${}", i + 1));
            param_values.push(Self::json_to_sql_string(val));
        }

        let returning = input
            .get("returning")
            .and_then(|v| v.as_str())
            .unwrap_or("*");

        let sql = format!(
            "INSERT INTO {table} ({}) VALUES ({}) RETURNING {returning}",
            columns.join(", "),
            placeholders.join(", ")
        );

        let client = self.connect().map_err(|e| e.to_string())?;

        self.rt().block_on(async {
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = param_values
                .iter()
                .map(|s| s as &(dyn tokio_postgres::types::ToSql + Sync))
                .collect();

            let row_opt = client
                .query_opt(&*sql, &param_refs)
                .await
                .map_err(|e| Self::format_pg_error(&e))?;

            Ok(row_opt.map_or(
                serde_json::json!({ "inserted": true }),
                |row| Self::row_to_json(&row),
            ))
        })
    }

    /// `update` -- row-level UPDATE with SET values and WHERE filter.
    ///
    /// Input: `{ "table": "users", "set": {"name": "Bob"}, "where": {"id": 1} }`
    fn do_update(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let table = input
            .get("table")
            .and_then(|v| v.as_str())
            .ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;

        let set_values = input
            .get("set")
            .and_then(|v| v.as_object())
            .ok_or("missing or invalid 'set' object")?;

        if set_values.is_empty() {
            return Err("'set' must contain at least one column".into());
        }

        let mut set_clauses = Vec::new();
        let mut param_values: Vec<String> = Vec::new();
        let mut param_idx = 1;

        for (col, val) in set_values {
            Self::validate_identifier(col)?;
            set_clauses.push(format!("{col} = ${param_idx}"));
            param_values.push(Self::json_to_sql_string(val));
            param_idx += 1;
        }

        let mut sql = format!("UPDATE {table} SET {}", set_clauses.join(", "));

        if let Some(where_obj) = input.get("where").and_then(|v| v.as_object()) {
            let clauses = Self::build_where_clauses(where_obj)?;
            if !clauses.is_empty() {
                let _ = write!(sql, " WHERE {}", clauses.join(" AND "));
            }
        }

        let client = self.connect().map_err(|e| e.to_string())?;

        self.rt().block_on(async {
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = param_values
                .iter()
                .map(|s| s as &(dyn tokio_postgres::types::ToSql + Sync))
                .collect();

            let count = client
                .execute(&*sql, &param_refs)
                .await
                .map_err(|e| Self::format_pg_error(&e))?;

            Ok(serde_json::json!({ "rows_affected": count }))
        })
    }

    /// `delete` -- row-level DELETE with WHERE filter.
    ///
    /// Input: `{ "table": "users", "where": {"id": 1} }`
    fn do_delete(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let table = input
            .get("table")
            .and_then(|v| v.as_str())
            .ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;

        let mut sql = format!("DELETE FROM {table}");

        if let Some(where_obj) = input.get("where").and_then(|v| v.as_object()) {
            let clauses = Self::build_where_clauses(where_obj)?;
            if !clauses.is_empty() {
                let _ = write!(sql, " WHERE {}", clauses.join(" AND "));
            }
        } else {
            return Err("'where' clause is required for delete to prevent accidental full-table deletion".into());
        }

        let client = self.connect().map_err(|e| e.to_string())?;

        self.rt().block_on(async {
            let count = client
                .execute(&*sql, &[])
                .await
                .map_err(|e| Self::format_pg_error(&e))?;

            Ok(serde_json::json!({ "rows_affected": count }))
        })
    }

    /// `begin_transaction` -- start a transaction and execute statements within it.
    ///
    /// Input: `{ "statements": [{"sql": "INSERT ...", "params": [...]}] }`
    ///
    /// Wraps all statements in BEGIN/COMMIT with automatic ROLLBACK on failure.
    /// True transaction handles across invocations are not possible with
    /// per-invocation connections, so this runs all statements atomically.
    fn do_begin_transaction(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let statements = input
            .get("statements")
            .and_then(|v| v.as_array())
            .ok_or("missing 'statements' array")?;

        let client = self.connect().map_err(|e| e.to_string())?;

        self.rt().block_on(async {
            client
                .execute("BEGIN", &[])
                .await
                .map_err(|e| Self::format_pg_error(&e))?;

            let mut results = Vec::new();

            for stmt in statements {
                let sql = stmt
                    .get("sql")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "each statement must have a 'sql' field".to_string())?;

                let params: Vec<String> = if let Some(arr) = stmt.get("params").and_then(|v| v.as_array()) {
                    arr.iter()
                        .map(|v| match v {
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Number(n) => n.to_string(),
                            serde_json::Value::Bool(b) => b.to_string(),
                            _ => v.to_string(),
                        })
                        .collect()
                } else {
                    Vec::new()
                };

                let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
                    .iter()
                    .map(|s| s as &(dyn tokio_postgres::types::ToSql + Sync))
                    .collect();

                match client.execute(sql, &param_refs).await {
                    Ok(count) => {
                        results.push(serde_json::json!({ "sql": sql, "rows_affected": count }));
                    }
                    Err(e) => {
                        let _ = client.execute("ROLLBACK", &[]).await;
                        return Err(format!(
                            "transaction rolled back: {} (failed on: {})",
                            Self::format_pg_error(&e),
                            sql
                        ));
                    }
                }
            }

            client
                .execute("COMMIT", &[])
                .await
                .map_err(|e| Self::format_pg_error(&e))?;

            Ok(serde_json::json!({ "committed": true, "results": results }))
        })
    }

    /// `commit` -- explicit commit (no-op in the atomic transaction model,
    /// provided for API completeness).
    fn do_commit(&self, _input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        Ok(serde_json::json!({ "committed": true, "note": "use begin_transaction for atomic multi-statement execution" }))
    }

    /// `rollback` -- explicit rollback (no-op in the atomic transaction model,
    /// provided for API completeness).
    fn do_rollback(&self, _input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        Ok(serde_json::json!({ "rolled_back": true, "note": "use begin_transaction for atomic multi-statement execution" }))
    }

    // -----------------------------------------------------------------------
    // SQL building helpers
    // -----------------------------------------------------------------------

    /// Validate that a SQL identifier contains only safe characters.
    fn validate_identifier(name: &str) -> std::result::Result<(), String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("empty identifier".into());
        }
        for ch in trimmed.chars() {
            if ch.is_alphanumeric()
                || ch == '_'
                || ch == '.'
                || ch == '('
                || ch == ')'
                || ch == '*'
                || ch == ','
                || ch == ' '
            {
                continue;
            }
            return Err(format!("invalid character '{ch}' in identifier '{trimmed}'"));
        }
        Ok(())
    }

    /// Convert a JSON value to a SQL-safe string literal for embedding in queries.
    fn format_sql_value(val: &serde_json::Value) -> String {
        match val {
            serde_json::Value::Null => "NULL".to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => {
                let escaped = s.replace('\'', "''");
                format!("'{escaped}'")
            }
            other => {
                let escaped = other.to_string().replace('\'', "''");
                format!("'{escaped}'")
            }
        }
    }

    /// Convert a JSON value to a string suitable for use as a parameterized value.
    fn json_to_sql_string(val: &serde_json::Value) -> String {
        match val {
            serde_json::Value::Null => String::new(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    }

    /// Build WHERE clauses from a JSON object.
    ///
    /// Supports: `{"field": value}`, `{"field": {">": 5}}`, `{"field": {"like": "%x%"}}`,
    /// `{"field": {"in": [1,2]}}`, `{"field": null}`, `{"field": {"not": null}}`.
    fn build_where_clauses(
        obj: &serde_json::Map<String, serde_json::Value>,
    ) -> std::result::Result<Vec<String>, String> {
        let mut clauses = Vec::new();
        for (field, condition) in obj {
            Self::validate_identifier(field)?;
            match condition {
                serde_json::Value::Null => {
                    clauses.push(format!("{field} IS NULL"));
                }
                serde_json::Value::Bool(b) => {
                    clauses.push(format!("{field} = {b}"));
                }
                serde_json::Value::Number(n) => {
                    clauses.push(format!("{field} = {n}"));
                }
                serde_json::Value::String(s) => {
                    let escaped = s.replace('\'', "''");
                    clauses.push(format!("{field} = '{escaped}'"));
                }
                serde_json::Value::Object(ops) => {
                    for (op, val) in ops {
                        let op_lower = op.to_lowercase();
                        match op_lower.as_str() {
                            ">" | ">=" | "<" | "<=" | "!=" | "<>" => {
                                let formatted = Self::format_sql_value(val);
                                clauses.push(format!("{field} {op} {formatted}"));
                            }
                            "like" => {
                                let s = val.as_str().ok_or("LIKE value must be a string")?;
                                let escaped = s.replace('\'', "''");
                                clauses.push(format!("{field} LIKE '{escaped}'"));
                            }
                            "in" => {
                                let arr = val.as_array().ok_or("IN value must be an array")?;
                                let items: Vec<String> =
                                    arr.iter().map(Self::format_sql_value).collect();
                                clauses.push(format!("{field} IN ({})", items.join(", ")));
                            }
                            "not" => {
                                if val.is_null() {
                                    clauses.push(format!("{field} IS NOT NULL"));
                                } else {
                                    let formatted = Self::format_sql_value(val);
                                    clauses.push(format!("{field} != {formatted}"));
                                }
                            }
                            _ => {
                                return Err(format!("unsupported operator '{op}'"));
                            }
                        }
                    }
                }
                serde_json::Value::Array(_) => {
                    return Err(format!(
                        "where value for '{field}' cannot be a bare array; use {{\"in\": [...]}}"
                    ));
                }
            }
        }
        Ok(clauses)
    }

    /// Build ORDER BY clause from a JSON array.
    fn build_order_by(
        arr: &[serde_json::Value],
    ) -> std::result::Result<Vec<String>, String> {
        let mut orders = Vec::new();
        for item in arr {
            if let Some(s) = item.as_str() {
                Self::validate_identifier(s)?;
                orders.push(format!("{s} ASC"));
            } else if let Some(obj) = item.as_object() {
                let field = obj
                    .get("field")
                    .and_then(|v| v.as_str())
                    .ok_or("order_by item missing 'field'")?;
                Self::validate_identifier(field)?;
                let dir = obj
                    .get("dir")
                    .and_then(|v| v.as_str())
                    .unwrap_or("asc")
                    .to_uppercase();
                if dir != "ASC" && dir != "DESC" {
                    return Err(format!("invalid order direction '{dir}'"));
                }
                orders.push(format!("{field} {dir}"));
            }
        }
        Ok(orders)
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
                SideEffectClass::ReadOnly,
                RollbackSupport::Irreversible,
                DeterminismClass::PartiallyDeterministic,
                IdempotenceClass::Idempotent,
                RiskClass::Low,
                &db_latency,
                &low_cost,
            ),
            Self::cap(
                "execute",
                "Execute an INSERT/UPDATE/DELETE and return rows affected",
                SideEffectClass::ExternalStateMutation,
                RollbackSupport::CompensatingAction,
                DeterminismClass::PartiallyDeterministic,
                IdempotenceClass::NonIdempotent,
                RiskClass::Medium,
                &db_latency,
                &low_cost,
            ),
            Self::cap(
                "find",
                "Find a single row by ID (ORM-style)",
                SideEffectClass::ReadOnly,
                RollbackSupport::Irreversible,
                DeterminismClass::PartiallyDeterministic,
                IdempotenceClass::Idempotent,
                RiskClass::Low,
                &db_latency,
                &low_cost,
            ),
            Self::cap(
                "find_many",
                "Find multiple rows with structured filter",
                SideEffectClass::ReadOnly,
                RollbackSupport::Irreversible,
                DeterminismClass::PartiallyDeterministic,
                IdempotenceClass::Idempotent,
                RiskClass::Low,
                &db_latency,
                &low_cost,
            ),
            Self::cap(
                "count",
                "Count rows matching a filter",
                SideEffectClass::ReadOnly,
                RollbackSupport::Irreversible,
                DeterminismClass::PartiallyDeterministic,
                IdempotenceClass::Idempotent,
                RiskClass::Negligible,
                &db_latency,
                &low_cost,
            ),
            Self::cap(
                "aggregate",
                "Run an aggregate function (SUM/AVG/MIN/MAX) with optional grouping",
                SideEffectClass::ReadOnly,
                RollbackSupport::Irreversible,
                DeterminismClass::PartiallyDeterministic,
                IdempotenceClass::Idempotent,
                RiskClass::Low,
                &db_latency,
                &low_cost,
            ),
            Self::cap(
                "create_table",
                "Create a table (DDL CREATE TABLE IF NOT EXISTS)",
                SideEffectClass::ExternalStateMutation,
                RollbackSupport::CompensatingAction,
                DeterminismClass::Deterministic,
                IdempotenceClass::Idempotent,
                RiskClass::High,
                &db_latency,
                &low_cost,
            ),
            Self::cap(
                "drop_table",
                "Drop a table (DDL DROP TABLE IF EXISTS)",
                SideEffectClass::Destructive,
                RollbackSupport::Irreversible,
                DeterminismClass::Deterministic,
                IdempotenceClass::Idempotent,
                RiskClass::Critical,
                &db_latency,
                &low_cost,
            ),
            Self::cap(
                "alter_table",
                "Alter a table (DDL ALTER TABLE)",
                SideEffectClass::ExternalStateMutation,
                RollbackSupport::CompensatingAction,
                DeterminismClass::Deterministic,
                IdempotenceClass::NonIdempotent,
                RiskClass::High,
                &db_latency,
                &low_cost,
            ),
            Self::cap(
                "insert",
                "Insert a row with column/value pairs",
                SideEffectClass::ExternalStateMutation,
                RollbackSupport::CompensatingAction,
                DeterminismClass::PartiallyDeterministic,
                IdempotenceClass::NonIdempotent,
                RiskClass::Low,
                &db_latency,
                &low_cost,
            ),
            Self::cap(
                "update",
                "Update rows matching a WHERE filter",
                SideEffectClass::ExternalStateMutation,
                RollbackSupport::CompensatingAction,
                DeterminismClass::PartiallyDeterministic,
                IdempotenceClass::ConditionallyIdempotent,
                RiskClass::Medium,
                &db_latency,
                &low_cost,
            ),
            Self::cap(
                "delete",
                "Delete rows matching a WHERE filter",
                SideEffectClass::Destructive,
                RollbackSupport::Irreversible,
                DeterminismClass::PartiallyDeterministic,
                IdempotenceClass::Idempotent,
                RiskClass::High,
                &db_latency,
                &low_cost,
            ),
            Self::cap(
                "begin_transaction",
                "Execute multiple statements atomically within a transaction",
                SideEffectClass::ExternalStateMutation,
                RollbackSupport::FullReversal,
                DeterminismClass::PartiallyDeterministic,
                IdempotenceClass::NonIdempotent,
                RiskClass::Medium,
                &db_latency,
                &low_cost,
            ),
            Self::cap(
                "commit",
                "Commit a transaction (provided for API completeness)",
                SideEffectClass::ExternalStateMutation,
                RollbackSupport::Irreversible,
                DeterminismClass::Deterministic,
                IdempotenceClass::Idempotent,
                RiskClass::Low,
                &db_latency,
                &CostProfile::default(),
            ),
            Self::cap(
                "rollback",
                "Rollback a transaction (provided for API completeness)",
                SideEffectClass::None,
                RollbackSupport::FullReversal,
                DeterminismClass::Deterministic,
                IdempotenceClass::Idempotent,
                RiskClass::Negligible,
                &db_latency,
                &CostProfile::default(),
            ),
        ];

        PortSpec {
            port_id: "soma.ports.postgres".to_string(),
            name: "postgres".to_string(),
            version: Version::new(0, 1, 0),
            kind: PortKind::Database,
            description: "PostgreSQL database operations: queries, DDL, ORM-style CRUD, transactions".to_string(),
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

    /// Helper to build a `PortCapabilitySpec` with common defaults.
    fn cap(
        name: &str,
        purpose: &str,
        effect_class: SideEffectClass,
        rollback_support: RollbackSupport,
        determinism_class: DeterminismClass,
        idempotence_class: IdempotenceClass,
        risk_class: RiskClass,
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
            effect_class,
            rollback_support,
            determinism_class,
            idempotence_class,
            risk_class,
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

impl Port for PostgresPort {
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
            "find" => self.do_find(&input),
            "find_many" => self.do_find_many(&input),
            "count" => self.do_count(&input),
            "aggregate" => self.do_aggregate(&input),
            "create_table" => self.do_create_table(&input),
            "drop_table" => self.do_drop_table(&input),
            "alter_table" => self.do_alter_table(&input),
            "insert" => self.do_insert(&input),
            "update" => self.do_update(&input),
            "delete" => self.do_delete(&input),
            "begin_transaction" => self.do_begin_transaction(&input),
            "commit" => self.do_commit(&input),
            "rollback" => self.do_rollback(&input),
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
                    "query" | "find" | "find_many" | "count" | "aggregate" => "read_only",
                    "execute" | "insert" | "update" | "create_table" | "alter_table"
                    | "begin_transaction" | "commit" => "external_state_mutation",
                    "delete" | "drop_table" => "destructive",
                    "rollback" => "none",
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
            "find" => {
                if input.get("table").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'table' field".into()));
                }
                if input.get("id").is_none() {
                    return Err(PortError::Validation("missing 'id' field".into()));
                }
            }
            "find_many" | "count" => {
                if input.get("table").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'table' field".into()));
                }
            }
            "aggregate" => {
                if input.get("table").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'table' field".into()));
                }
                if input.get("function").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'function' field".into()));
                }
                if input.get("column").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'column' field".into()));
                }
            }
            "create_table" => {
                if input.get("table").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'table' field".into()));
                }
                if input.get("columns").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'columns' field".into()));
                }
            }
            "drop_table" | "alter_table" => {
                if input.get("table").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'table' field".into()));
                }
            }
            "insert" => {
                if input.get("table").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'table' field".into()));
                }
                if input.get("values").and_then(|v| v.as_object()).is_none() {
                    return Err(PortError::Validation("missing 'values' object".into()));
                }
            }
            "update" => {
                if input.get("table").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'table' field".into()));
                }
                if input.get("set").and_then(|v| v.as_object()).is_none() {
                    return Err(PortError::Validation("missing 'set' object".into()));
                }
            }
            "delete" => {
                if input.get("table").and_then(|v| v.as_str()).is_none() {
                    return Err(PortError::Validation("missing 'table' field".into()));
                }
                if input.get("where").and_then(|v| v.as_object()).is_none() {
                    return Err(PortError::Validation("missing 'where' clause".into()));
                }
            }
            "begin_transaction" => {
                if input.get("statements").and_then(|v| v.as_array()).is_none() {
                    return Err(PortError::Validation("missing 'statements' array".into()));
                }
            }
            "commit" | "rollback" => {}
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

#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    let port = PostgresPort::new();
    Box::into_raw(Box::new(port))
}
