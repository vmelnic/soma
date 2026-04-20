//! SOMA MySQL port pack — database operations via the `mysql` crate with
//! synchronous connections.
//!
//! Provides 15 capabilities mirroring the postgres port:
//!
//! - **Raw SQL**: `query`, `execute`
//! - **ORM-style**: `find`, `find_many`, `count`, `aggregate`
//! - **Row-level CRUD**: `insert`, `update`, `delete`
//! - **DDL**: `create_table`, `drop_table`, `alter_table`
//! - **Transactions**: `begin_transaction`, `commit`, `rollback`
//!
//! Each capability accepts JSON input and returns JSON output via the Port trait.
//! Connection string is read from `SOMA_MYSQL_URL` or `MYSQL_URL`.

use std::fmt::Write as _;
use std::sync::OnceLock;
use std::time::Instant;

use chrono::Utc;
use mysql::prelude::Queryable;
use semver::Version;
use soma_port_sdk::prelude::*;
use uuid::Uuid;

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
        Self { effect_class, rollback_support, determinism_class, idempotence_class, risk_class }
    }
}

impl Default for MysqlPort {
    fn default() -> Self { Self::new() }
}

impl MysqlPort {
    pub fn new() -> Self {
        Self { spec: Self::build_spec(), conn_url: OnceLock::new() }
    }

    fn conn_url(&self) -> Option<&str> {
        self.conn_url
            .get_or_init(|| {
                std::env::var("SOMA_MYSQL_URL")
                    .or_else(|_| std::env::var("MYSQL_URL"))
                    .ok()
            })
            .as_deref()
    }

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

    fn extract_params(input: &serde_json::Value) -> Vec<mysql::Value> {
        match input.get("params") {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .map(|v| match v {
                    serde_json::Value::String(s) => mysql::Value::Bytes(s.as_bytes().to_vec()),
                    serde_json::Value::Number(n) => {
                        if let Some(i) = n.as_i64() { mysql::Value::Int(i) }
                        else if let Some(f) = n.as_f64() { mysql::Value::Double(f) }
                        else { mysql::Value::Bytes(n.to_string().into_bytes()) }
                    }
                    serde_json::Value::Bool(b) => mysql::Value::Int(i64::from(*b)),
                    serde_json::Value::Null => mysql::Value::NULL,
                    other => mysql::Value::Bytes(other.to_string().into_bytes()),
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn mysql_value_to_json(val: &mysql::Value) -> serde_json::Value {
        match val {
            mysql::Value::NULL => serde_json::Value::Null,
            mysql::Value::Int(i) => serde_json::json!(i),
            mysql::Value::UInt(u) => serde_json::json!(u),
            mysql::Value::Float(f) => serde_json::json!(f),
            mysql::Value::Double(d) => serde_json::json!(d),
            mysql::Value::Bytes(b) => match String::from_utf8(b.clone()) {
                Ok(s) => serde_json::Value::String(s),
                Err(_) => serde_json::Value::String(format!("<binary {} bytes>", b.len())),
            },
            mysql::Value::Date(y, m, d, h, min, s, _us) => {
                serde_json::Value::String(format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}"))
            }
            mysql::Value::Time(neg, d, h, min, s, _us) => {
                let sign = if *neg { "-" } else { "" };
                let total_h = *d * 24 + (*h as u32);
                serde_json::Value::String(format!("{sign}{total_h}:{min:02}:{s:02}"))
            }
        }
    }

    fn row_to_json(row: mysql::Row) -> serde_json::Value {
        let columns: Vec<String> =
            row.columns_ref().iter().map(|c| c.name_str().to_string()).collect();
        let mut map = serde_json::Map::new();
        let values: Vec<mysql::Value> = row.unwrap();
        for (i, col_name) in columns.iter().enumerate() {
            let val =
                values.get(i).map_or(serde_json::Value::Null, Self::mysql_value_to_json);
            map.insert(col_name.clone(), val);
        }
        serde_json::Value::Object(map)
    }

    fn json_to_mysql_value(val: &serde_json::Value) -> mysql::Value {
        match val {
            serde_json::Value::Null => mysql::Value::NULL,
            serde_json::Value::Bool(b) => mysql::Value::Int(i64::from(*b)),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() { mysql::Value::Int(i) }
                else if let Some(f) = n.as_f64() { mysql::Value::Double(f) }
                else { mysql::Value::Bytes(n.to_string().into_bytes()) }
            }
            serde_json::Value::String(s) => mysql::Value::Bytes(s.as_bytes().to_vec()),
            other => mysql::Value::Bytes(other.to_string().into_bytes()),
        }
    }

    fn validate_identifier(name: &str) -> std::result::Result<(), String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("empty identifier".into());
        }
        for ch in trimmed.chars() {
            if ch.is_alphanumeric() || ch == '_' || ch == '.' {
                continue;
            }
            return Err(format!("invalid character '{ch}' in identifier '{trimmed}'"));
        }
        Ok(())
    }

    fn format_sql_value(val: &serde_json::Value) -> String {
        match val {
            serde_json::Value::Null => "NULL".to_string(),
            serde_json::Value::Bool(b) => if *b { "1".to_string() } else { "0".to_string() },
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => {
                let escaped = s.replace('\\', "\\\\").replace('\'', "\\'");
                format!("'{escaped}'")
            }
            other => {
                let escaped = other.to_string().replace('\\', "\\\\").replace('\'', "\\'");
                format!("'{escaped}'")
            }
        }
    }

    fn build_where_clauses(
        obj: &serde_json::Map<String, serde_json::Value>,
    ) -> std::result::Result<Vec<String>, String> {
        let mut clauses = Vec::new();
        for (field, condition) in obj {
            Self::validate_identifier(field)?;
            match condition {
                serde_json::Value::Null => clauses.push(format!("`{field}` IS NULL")),
                serde_json::Value::Bool(b) => {
                    clauses.push(format!("`{field}` = {}", if *b { 1 } else { 0 }));
                }
                serde_json::Value::Number(n) => clauses.push(format!("`{field}` = {n}")),
                serde_json::Value::String(s) => {
                    let escaped = s.replace('\\', "\\\\").replace('\'', "\\'");
                    clauses.push(format!("`{field}` = '{escaped}'"));
                }
                serde_json::Value::Object(ops) => {
                    for (op, val) in ops {
                        let op_lower = op.to_lowercase();
                        match op_lower.as_str() {
                            ">" | ">=" | "<" | "<=" | "!=" | "<>" => {
                                clauses.push(format!("`{field}` {op} {}", Self::format_sql_value(val)));
                            }
                            "like" => {
                                let s = val.as_str().ok_or("LIKE value must be a string")?;
                                let escaped = s.replace('\\', "\\\\").replace('\'', "\\'");
                                clauses.push(format!("`{field}` LIKE '{escaped}'"));
                            }
                            "in" => {
                                let arr = val.as_array().ok_or("IN value must be an array")?;
                                let items: Vec<String> =
                                    arr.iter().map(Self::format_sql_value).collect();
                                clauses.push(format!("`{field}` IN ({})", items.join(", ")));
                            }
                            "not" => {
                                if val.is_null() {
                                    clauses.push(format!("`{field}` IS NOT NULL"));
                                } else {
                                    clauses.push(format!("`{field}` != {}", Self::format_sql_value(val)));
                                }
                            }
                            _ => return Err(format!("unsupported operator '{op}'")),
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

    fn build_order_by(arr: &[serde_json::Value]) -> std::result::Result<Vec<String>, String> {
        let mut orders = Vec::new();
        for item in arr {
            if let Some(s) = item.as_str() {
                Self::validate_identifier(s)?;
                orders.push(format!("`{s}` ASC"));
            } else if let Some(obj) = item.as_object() {
                let field = obj.get("field").and_then(|v| v.as_str())
                    .ok_or("order_by item missing 'field'")?;
                Self::validate_identifier(field)?;
                let dir = obj.get("dir").and_then(|v| v.as_str()).unwrap_or("asc").to_uppercase();
                if dir != "ASC" && dir != "DESC" {
                    return Err(format!("invalid order direction '{dir}'"));
                }
                orders.push(format!("`{field}` {dir}"));
            }
        }
        Ok(orders)
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

    fn do_query(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let sql = input.get("sql").and_then(|v| v.as_str()).ok_or("missing 'sql' field")?;
        let params = Self::extract_params(input);
        let mut conn = self.connect().map_err(|e| e.to_string())?;
        let stmt = conn.prep(sql).map_err(|e| format!("MySQL prepare error: {e}"))?;
        let rows: Vec<mysql::Row> =
            conn.exec(stmt, params).map_err(|e| format!("MySQL query error: {e}"))?;
        let values: Vec<serde_json::Value> = rows.into_iter().map(Self::row_to_json).collect();
        let count = values.len();
        Ok(serde_json::json!({ "rows": values, "count": count }))
    }

    fn do_execute(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let sql = input.get("sql").and_then(|v| v.as_str()).ok_or("missing 'sql' field")?;
        let params = Self::extract_params(input);
        let mut conn = self.connect().map_err(|e| e.to_string())?;
        let stmt = conn.prep(sql).map_err(|e| format!("MySQL prepare error: {e}"))?;
        conn.exec_drop(stmt, &params).map_err(|e| format!("MySQL execute error: {e}"))?;
        let affected = conn.affected_rows();
        Ok(serde_json::json!({ "rows_affected": affected }))
    }

    fn do_find(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let table = input.get("table").and_then(|v| v.as_str()).ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;
        let id = input.get("id").ok_or("missing 'id' field")?;
        let id_val = Self::json_to_mysql_value(id);
        let id_column = input.get("id_column").and_then(|v| v.as_str()).unwrap_or("id");
        Self::validate_identifier(id_column)?;
        let sql = format!("SELECT * FROM `{table}` WHERE `{id_column}` = ? LIMIT 1");
        let mut conn = self.connect().map_err(|e| e.to_string())?;
        let stmt = conn.prep(&sql).map_err(|e| format!("MySQL prepare error: {e}"))?;
        let rows: Vec<mysql::Row> =
            conn.exec(stmt, vec![id_val]).map_err(|e| format!("MySQL find error: {e}"))?;
        Ok(rows.into_iter().next().map_or(serde_json::Value::Null, Self::row_to_json))
    }

    fn do_find_many(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let table = input.get("table").and_then(|v| v.as_str()).ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;
        let mut sql = format!("SELECT * FROM `{table}`");
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
        let mut conn = self.connect().map_err(|e| e.to_string())?;
        let rows: Vec<mysql::Row> =
            conn.query(sql).map_err(|e| format!("MySQL find_many error: {e}"))?;
        let values: Vec<serde_json::Value> = rows.into_iter().map(Self::row_to_json).collect();
        let count = values.len();
        Ok(serde_json::json!({ "rows": values, "count": count }))
    }

    fn do_count(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let table = input.get("table").and_then(|v| v.as_str()).ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;
        let mut sql = format!("SELECT COUNT(*) FROM `{table}`");
        if let Some(where_obj) = input.get("where").and_then(|v| v.as_object()) {
            let clauses = Self::build_where_clauses(where_obj)?;
            if !clauses.is_empty() {
                let _ = write!(sql, " WHERE {}", clauses.join(" AND "));
            }
        }
        let mut conn = self.connect().map_err(|e| e.to_string())?;
        let count: Option<i64> =
            conn.query_first(sql).map_err(|e| format!("MySQL count error: {e}"))?;
        Ok(serde_json::json!({ "count": count.unwrap_or(0) }))
    }

    fn do_aggregate(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let table = input.get("table").and_then(|v| v.as_str()).ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;
        let function = input.get("function").and_then(|v| v.as_str())
            .ok_or("missing 'function' field")?.to_uppercase();
        if !["SUM", "AVG", "MIN", "MAX", "COUNT"].contains(&function.as_str()) {
            return Err(format!("unsupported aggregate function '{function}'"));
        }
        let column = input.get("column").and_then(|v| v.as_str()).ok_or("missing 'column' field")?;
        Self::validate_identifier(column)?;
        let mut select_parts = vec![format!("{function}(`{column}`) AS result")];
        let mut group_by_fields: Vec<String> = Vec::new();
        if let Some(gb_arr) = input.get("group_by").and_then(|v| v.as_array()) {
            for item in gb_arr {
                let field = item.as_str().ok_or("group_by items must be strings")?;
                Self::validate_identifier(field)?;
                select_parts.push(format!("`{field}`"));
                group_by_fields.push(format!("`{field}`"));
            }
        }
        let mut sql = format!("SELECT {} FROM `{table}`", select_parts.join(", "));
        if let Some(where_obj) = input.get("where").and_then(|v| v.as_object()) {
            let clauses = Self::build_where_clauses(where_obj)?;
            if !clauses.is_empty() {
                let _ = write!(sql, " WHERE {}", clauses.join(" AND "));
            }
        }
        if !group_by_fields.is_empty() {
            let _ = write!(sql, " GROUP BY {}", group_by_fields.join(", "));
        }
        let mut conn = self.connect().map_err(|e| e.to_string())?;
        let rows: Vec<mysql::Row> =
            conn.query(sql).map_err(|e| format!("MySQL aggregate error: {e}"))?;
        let values: Vec<serde_json::Value> = rows.into_iter().map(Self::row_to_json).collect();
        Ok(serde_json::json!({ "rows": values }))
    }

    fn do_create_table(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let table = input.get("table").and_then(|v| v.as_str()).ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;
        let columns = input.get("columns").and_then(|v| v.as_str())
            .ok_or("missing 'columns' field")?;
        let sql = format!("CREATE TABLE IF NOT EXISTS `{table}` ({columns})");
        let mut conn = self.connect().map_err(|e| e.to_string())?;
        conn.query_drop(sql).map_err(|e| format!("MySQL create_table error: {e}"))?;
        Ok(serde_json::json!({ "created": table }))
    }

    fn do_drop_table(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let table = input.get("table").and_then(|v| v.as_str()).ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;
        let sql = format!("DROP TABLE IF EXISTS `{table}`");
        let mut conn = self.connect().map_err(|e| e.to_string())?;
        conn.query_drop(sql).map_err(|e| format!("MySQL drop_table error: {e}"))?;
        Ok(serde_json::json!({ "dropped": table }))
    }

    fn do_alter_table(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let table = input.get("table").and_then(|v| v.as_str()).ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;
        let changes = input.get("changes").and_then(|v| v.as_str())
            .ok_or("missing 'changes' field")?;
        let sql = format!("ALTER TABLE `{table}` {changes}");
        let mut conn = self.connect().map_err(|e| e.to_string())?;
        conn.query_drop(sql).map_err(|e| format!("MySQL alter_table error: {e}"))?;
        Ok(serde_json::json!({ "altered": table }))
    }

    fn do_insert(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let table = input.get("table").and_then(|v| v.as_str()).ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;
        let data = input.get("data").and_then(|v| v.as_object())
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
        let stmt = conn.prep(&sql).map_err(|e| format!("MySQL prepare error: {e}"))?;
        conn.exec_drop(stmt, param_values).map_err(|e| format!("MySQL insert error: {e}"))?;
        let last_id = conn.last_insert_id();
        let affected = conn.affected_rows();
        Ok(serde_json::json!({ "inserted": true, "last_insert_id": last_id, "rows_affected": affected }))
    }

    fn do_update(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let table = input.get("table").and_then(|v| v.as_str()).ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;
        let data = input.get("data").and_then(|v| v.as_object())
            .ok_or("missing or invalid 'data' object")?;
        if data.is_empty() {
            return Err("'data' must contain at least one column".into());
        }
        let where_clause = input.get("where_clause").and_then(|v| v.as_str())
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
        let stmt = conn.prep(&sql).map_err(|e| format!("MySQL prepare error: {e}"))?;
        conn.exec_drop(stmt, param_values).map_err(|e| format!("MySQL update error: {e}"))?;
        let affected = conn.affected_rows();
        Ok(serde_json::json!({ "rows_affected": affected }))
    }

    fn do_delete(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let table = input.get("table").and_then(|v| v.as_str()).ok_or("missing 'table' field")?;
        Self::validate_identifier(table)?;
        let where_clause = input.get("where_clause").and_then(|v| v.as_str())
            .ok_or("missing 'where_clause' (required to prevent full-table deletion)")?;
        let sql = format!("DELETE FROM `{table}` WHERE {where_clause}");
        let mut conn = self.connect().map_err(|e| e.to_string())?;
        let stmt = conn.prep(&sql).map_err(|e| format!("MySQL prepare error: {e}"))?;
        conn.exec_drop(stmt, ()).map_err(|e| format!("MySQL delete error: {e}"))?;
        let affected = conn.affected_rows();
        Ok(serde_json::json!({ "rows_affected": affected }))
    }

    fn do_begin_transaction(&self, input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        let statements = input.get("statements").and_then(|v| v.as_array())
            .ok_or("missing 'statements' array")?;
        let mut conn = self.connect().map_err(|e| e.to_string())?;
        conn.query_drop("START TRANSACTION").map_err(|e| format!("START TRANSACTION error: {e}"))?;
        let mut results = Vec::new();
        for stmt_val in statements {
            let sql = stmt_val.get("sql").and_then(|v| v.as_str())
                .ok_or_else(|| "each statement must have a 'sql' field".to_string())?;
            let params: Vec<mysql::Value> =
                if let Some(arr) = stmt_val.get("params").and_then(|v| v.as_array()) {
                    arr.iter().map(|v| match v {
                        serde_json::Value::String(s) => mysql::Value::Bytes(s.as_bytes().to_vec()),
                        serde_json::Value::Number(n) => {
                            if let Some(i) = n.as_i64() { mysql::Value::Int(i) }
                            else if let Some(f) = n.as_f64() { mysql::Value::Double(f) }
                            else { mysql::Value::Bytes(n.to_string().into_bytes()) }
                        }
                        serde_json::Value::Bool(b) => mysql::Value::Int(i64::from(*b)),
                        _ => mysql::Value::NULL,
                    }).collect()
                } else {
                    Vec::new()
                };
            let prepped = match conn.prep(sql) {
                Ok(p) => p,
                Err(e) => {
                    let _ = conn.query_drop("ROLLBACK");
                    return Err(format!("transaction rolled back: prepare error {e} (failed on: {sql})"));
                }
            };
            match conn.exec_drop(prepped, params) {
                Ok(()) => {
                    let affected = conn.affected_rows();
                    results.push(serde_json::json!({ "sql": sql, "rows_affected": affected }));
                }
                Err(e) => {
                    let _ = conn.query_drop("ROLLBACK");
                    return Err(format!("transaction rolled back: {e} (failed on: {sql})"));
                }
            }
        }
        conn.query_drop("COMMIT").map_err(|e| format!("COMMIT error: {e}"))?;
        Ok(serde_json::json!({ "committed": true, "results": results }))
    }

    fn do_commit(&self, _input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        Ok(serde_json::json!({ "committed": true, "note": "use begin_transaction for atomic multi-statement execution" }))
    }

    fn do_rollback(&self, _input: &serde_json::Value) -> std::result::Result<serde_json::Value, String> {
        Ok(serde_json::json!({ "rolled_back": true, "note": "use begin_transaction for atomic multi-statement execution" }))
    }

    // -----------------------------------------------------------------------
    // PortSpec builder
    // -----------------------------------------------------------------------

    fn build_spec() -> PortSpec {
        let any_schema = SchemaRef { schema: serde_json::json!({ "type": "object" }) };
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
            Self::cap("query", "Execute a SELECT query and return rows",
                CapabilityBehavior::new(SideEffectClass::ReadOnly, RollbackSupport::Irreversible,
                    DeterminismClass::PartiallyDeterministic, IdempotenceClass::Idempotent, RiskClass::Low),
                &db_latency, &low_cost),
            Self::cap("execute", "Execute an INSERT/UPDATE/DELETE and return rows affected",
                CapabilityBehavior::new(SideEffectClass::ExternalStateMutation, RollbackSupport::CompensatingAction,
                    DeterminismClass::PartiallyDeterministic, IdempotenceClass::NonIdempotent, RiskClass::Medium),
                &db_latency, &low_cost),
            Self::cap("find", "Find a single row by ID (ORM-style)",
                CapabilityBehavior::new(SideEffectClass::ReadOnly, RollbackSupport::Irreversible,
                    DeterminismClass::PartiallyDeterministic, IdempotenceClass::Idempotent, RiskClass::Low),
                &db_latency, &low_cost),
            Self::cap("find_many", "Find multiple rows with structured filter",
                CapabilityBehavior::new(SideEffectClass::ReadOnly, RollbackSupport::Irreversible,
                    DeterminismClass::PartiallyDeterministic, IdempotenceClass::Idempotent, RiskClass::Low),
                &db_latency, &low_cost),
            Self::cap("count", "Count rows matching a filter",
                CapabilityBehavior::new(SideEffectClass::ReadOnly, RollbackSupport::Irreversible,
                    DeterminismClass::PartiallyDeterministic, IdempotenceClass::Idempotent, RiskClass::Negligible),
                &db_latency, &low_cost),
            Self::cap("aggregate", "Run an aggregate function (SUM/AVG/MIN/MAX) with optional grouping",
                CapabilityBehavior::new(SideEffectClass::ReadOnly, RollbackSupport::Irreversible,
                    DeterminismClass::PartiallyDeterministic, IdempotenceClass::Idempotent, RiskClass::Low),
                &db_latency, &low_cost),
            Self::cap("create_table", "Create a table (DDL CREATE TABLE IF NOT EXISTS)",
                CapabilityBehavior::new(SideEffectClass::ExternalStateMutation, RollbackSupport::CompensatingAction,
                    DeterminismClass::Deterministic, IdempotenceClass::Idempotent, RiskClass::High),
                &db_latency, &low_cost),
            Self::cap("drop_table", "Drop a table (DDL DROP TABLE IF EXISTS)",
                CapabilityBehavior::new(SideEffectClass::Destructive, RollbackSupport::Irreversible,
                    DeterminismClass::Deterministic, IdempotenceClass::Idempotent, RiskClass::Critical),
                &db_latency, &low_cost),
            Self::cap("alter_table", "Alter a table (DDL ALTER TABLE)",
                CapabilityBehavior::new(SideEffectClass::ExternalStateMutation, RollbackSupport::CompensatingAction,
                    DeterminismClass::Deterministic, IdempotenceClass::NonIdempotent, RiskClass::High),
                &db_latency, &low_cost),
            Self::cap("insert", "Insert a row from a JSON data object",
                CapabilityBehavior::new(SideEffectClass::ExternalStateMutation, RollbackSupport::CompensatingAction,
                    DeterminismClass::PartiallyDeterministic, IdempotenceClass::NonIdempotent, RiskClass::Low),
                &db_latency, &low_cost),
            Self::cap("update", "Update rows matching a WHERE clause",
                CapabilityBehavior::new(SideEffectClass::ExternalStateMutation, RollbackSupport::CompensatingAction,
                    DeterminismClass::PartiallyDeterministic, IdempotenceClass::ConditionallyIdempotent, RiskClass::Medium),
                &db_latency, &low_cost),
            Self::cap("delete", "Delete rows matching a WHERE clause",
                CapabilityBehavior::new(SideEffectClass::Destructive, RollbackSupport::Irreversible,
                    DeterminismClass::PartiallyDeterministic, IdempotenceClass::Idempotent, RiskClass::High),
                &db_latency, &low_cost),
            Self::cap("begin_transaction", "Execute multiple statements atomically within a transaction",
                CapabilityBehavior::new(SideEffectClass::ExternalStateMutation, RollbackSupport::FullReversal,
                    DeterminismClass::PartiallyDeterministic, IdempotenceClass::NonIdempotent, RiskClass::Medium),
                &db_latency, &low_cost),
            Self::cap("commit", "Commit a transaction (provided for API completeness)",
                CapabilityBehavior::new(SideEffectClass::ExternalStateMutation, RollbackSupport::Irreversible,
                    DeterminismClass::Deterministic, IdempotenceClass::Idempotent, RiskClass::Low),
                &db_latency, &CostProfile::default()),
            Self::cap("rollback", "Rollback a transaction (provided for API completeness)",
                CapabilityBehavior::new(SideEffectClass::None, RollbackSupport::FullReversal,
                    DeterminismClass::Deterministic, IdempotenceClass::Idempotent, RiskClass::Negligible),
                &db_latency, &CostProfile::default()),
        ];

        PortSpec {
            port_id: "soma.ports.mysql".to_string(),
            name: "mysql".to_string(),
            version: Version::new(0, 1, 0),
            kind: PortKind::Database,
            description: "MySQL database operations: queries, DDL, ORM-style CRUD, transactions"
                .to_string(),
            namespace: "soma.ports.mysql".to_string(),
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
        let any_schema = SchemaRef { schema: serde_json::json!({ "type": "object" }) };
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
    fn spec(&self) -> &PortSpec { &self.spec }

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

    fn lifecycle_state(&self) -> PortLifecycleState { PortLifecycleState::Active }
}

// ---------------------------------------------------------------------------
// C ABI entry point
// ---------------------------------------------------------------------------

#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port {
    Box::into_raw(Box::new(MysqlPort::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spec_valid() {
        let port = MysqlPort::new();
        let spec = port.spec();
        assert!(!spec.capabilities.is_empty());
        assert!(!spec.failure_modes.is_empty());
        assert!(spec.latency_profile.expected_latency_ms <= spec.latency_profile.p95_latency_ms);
        assert!(spec.latency_profile.p95_latency_ms <= spec.latency_profile.max_latency_ms);
    }

    #[test]
    fn test_all_capabilities_present() {
        let port = MysqlPort::new();
        let ids: Vec<&str> = port
            .spec()
            .capabilities
            .iter()
            .map(|c| c.capability_id.as_str())
            .collect();
        for cap in &[
            "query", "execute", "find", "find_many", "count", "aggregate",
            "create_table", "drop_table", "alter_table", "insert", "update",
            "delete", "begin_transaction", "commit", "rollback",
        ] {
            assert!(ids.contains(cap), "missing capability: {cap}");
        }
    }

    #[test]
    fn test_validate_input_unknown_capability() {
        let port = MysqlPort::new();
        assert!(port.validate_input("nonexistent", &serde_json::json!({})).is_err());
    }

    #[test]
    fn test_validate_input_query_missing_sql() {
        let port = MysqlPort::new();
        assert!(port.validate_input("query", &serde_json::json!({})).is_err());
    }
}
