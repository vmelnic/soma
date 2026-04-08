//! SOMA `PostgreSQL` plugin -- database operations via the synchronous `postgres` crate.
//!
//! # Overview
//!
//! Provides 15 conventions for `PostgreSQL` interaction:
//!
//! - **Raw SQL** (0-2): `query`, `execute`, `query_one`
//! - **Transactions** (3-5): `begin`, `commit`, `rollback` (MVP: stub, not yet wired)
//! - **DDL** (6-7): `create_table`, `alter_table`
//! - **Schema introspection** (8-10): `table_exists`, `list_tables`, `table_schema`
//! - **ORM-style query builder** (11-14): `find`, `find_one`, `count`, `aggregate`
//!
//! Conventions 11-14 accept a JSON spec instead of raw SQL. The Mind generates
//! a structured JSON object (table, select, where, join, `group_by`, having,
//! `order_by`, limit, offset) and the plugin builds safe SQL from it -- similar to
//! Prisma, Eloquent, or Doctrine query builders.
//!
//! # Why synchronous `postgres` instead of `tokio-postgres`?
//!
//! This crate compiles as a `cdylib` plugin loaded at runtime by the SOMA core
//! binary. The core's tokio reactor lives in the host process; a `cdylib` cannot
//! reliably share that reactor due to TLS (thread-local storage) boundary issues.
//! The synchronous `postgres` crate sidesteps this entirely -- each call opens a
//! blocking connection, executes, and returns. This is sufficient for SOMA's
//! request-per-intent execution model where database calls are infrequent and
//! latency is dominated by network round-trips, not connection setup.

use soma_plugin_sdk::prelude::*;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::OnceLock;

use postgres::types::Type;

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

/// The SOMA `PostgreSQL` plugin.
///
/// Holds a lazily-initialized connection string set during [`SomaPlugin::on_load`].
/// Each convention call creates a fresh [`postgres::Client`] from this string.
pub struct PostgresPlugin {
    conn_string: OnceLock<String>,
}

impl Default for PostgresPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl PostgresPlugin {
    /// Create a new unconfigured plugin instance.
    ///
    /// The connection string is not set until [`SomaPlugin::on_load`] is called
    /// by the plugin manager with the appropriate `PluginConfig`.
    pub const fn new() -> Self {
        Self {
            conn_string: OnceLock::new(),
        }
    }

    /// Establish a fresh database connection using the stored connection string.
    ///
    /// Each call creates a new TCP connection to `PostgreSQL`. This is intentionally
    /// simple -- SOMA's execution model is one-intent-at-a-time, so connection
    /// pooling adds complexity without meaningful benefit.
    fn connect(&self) -> Result<postgres::Client, PluginError> {
        let conn_str = self
            .conn_string
            .get()
            .ok_or_else(|| PluginError::Failed("postgres not configured -- call on_load first".into()))?;
        let client = postgres::Client::connect(conn_str, postgres::NoTls)
            .map_err(|e| PluginError::ConnectionRefused(format!("PostgreSQL: {e}")))?;
        Ok(client)
    }

    /// Format a `postgres::Error` with full detail from the database server.
    ///
    /// When the error originates from the database (as opposed to a connection
    /// failure), this extracts severity, message, and SQLSTATE code. Otherwise
    /// falls back to the generic `Display` output.
    fn format_pg_error(e: &postgres::Error) -> String {
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

    /// Convert a [`postgres::Row`] into a [`Value::Map`].
    ///
    /// Iterates over every column in the row, mapping each `PostgreSQL` type to
    /// the corresponding [`Value`] variant via [`Self::column_value`].
    fn row_to_value(row: &postgres::Row) -> Value {
        let mut map = HashMap::new();
        for (i, col) in row.columns().iter().enumerate() {
            let name = col.name().to_string();
            let val = Self::column_value(row, i, col.type_());
            map.insert(name, val);
        }
        Value::Map(map)
    }

    /// Extract a single column value from a row, mapping `PostgreSQL` types to [`Value`].
    ///
    /// Supported type mappings:
    /// - `BOOL` -> `Value::Bool`
    /// - `INT2`, `INT4`, `INT8` -> `Value::Int` (widened via `From`)
    /// - `FLOAT4`, `FLOAT8` -> `Value::Float` (widened via `From`)
    /// - `TEXT`, `VARCHAR`, `BPCHAR`, `NAME` -> `Value::String`
    /// - `JSON`, `JSONB` -> `Value::String` (serialized JSON text)
    /// - `UUID` -> `Value::String` (hyphenated form)
    /// - `TIMESTAMP`, `TIMESTAMPTZ` -> `Value::String` (ISO 8601)
    /// - `NUMERIC` -> `Value::String` (exact decimal representation)
    /// - Everything else -> attempted as `String`, with a fallback placeholder
    #[allow(clippy::too_many_lines)]
    fn column_value(row: &postgres::Row, idx: usize, ty: &Type) -> Value {
        match *ty {
            Type::BOOL => match row.try_get::<_, Option<bool>>(idx) {
                Ok(Some(v)) => Value::Bool(v),
                Ok(None) | Err(_) => Value::Null,
            },
            Type::INT2 => match row.try_get::<_, Option<i16>>(idx) {
                Ok(Some(v)) => Value::Int(i64::from(v)),
                Ok(None) | Err(_) => Value::Null,
            },
            Type::INT4 => match row.try_get::<_, Option<i32>>(idx) {
                Ok(Some(v)) => Value::Int(i64::from(v)),
                Ok(None) | Err(_) => Value::Null,
            },
            Type::INT8 => match row.try_get::<_, Option<i64>>(idx) {
                Ok(Some(v)) => Value::Int(v),
                Ok(None) | Err(_) => Value::Null,
            },
            Type::FLOAT4 => match row.try_get::<_, Option<f32>>(idx) {
                Ok(Some(v)) => Value::Float(f64::from(v)),
                Ok(None) | Err(_) => Value::Null,
            },
            Type::FLOAT8 => match row.try_get::<_, Option<f64>>(idx) {
                Ok(Some(v)) => Value::Float(v),
                Ok(None) | Err(_) => Value::Null,
            },
            Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => {
                match row.try_get::<_, Option<String>>(idx) {
                    Ok(Some(v)) => Value::String(v),
                    Ok(None) | Err(_) => Value::Null,
                }
            }
            Type::JSON | Type::JSONB => {
                match row.try_get::<_, Option<serde_json::Value>>(idx) {
                    Ok(Some(v)) => Value::String(v.to_string()),
                    Ok(None) | Err(_) => Value::Null,
                }
            }
            Type::UUID => match row.try_get::<_, Option<uuid::Uuid>>(idx) {
                Ok(Some(v)) => Value::String(v.to_string()),
                Ok(None) | Err(_) => Value::Null,
            },
            Type::TIMESTAMP | Type::TIMESTAMPTZ => {
                match row.try_get::<_, Option<chrono::NaiveDateTime>>(idx) {
                    Ok(Some(v)) => Value::String(v.format("%Y-%m-%dT%H:%M:%S").to_string()),
                    Ok(None) => Value::Null,
                    Err(_) => match row.try_get::<_, Option<String>>(idx) {
                        Ok(Some(v)) => Value::String(v),
                        _ => Value::Null,
                    },
                }
            }
            Type::NUMERIC => match row.try_get::<_, Option<String>>(idx) {
                Ok(Some(v)) => Value::String(v),
                Ok(None) | Err(_) => Value::Null,
            },
            _ => match row.try_get::<_, Option<String>>(idx) {
                Ok(Some(v)) => Value::String(v),
                Ok(None) => Value::Null,
                Err(_) => Value::String(format!("<unsupported type: {ty}>")),
            },
        }
    }

    /// Extract an optional parameter list from convention arguments.
    ///
    /// If `args[1]` is a `Value::List`, each element is converted to its string
    /// representation. A single scalar at `args[1]` is treated as a one-element
    /// list. Returns an empty `Vec` when no parameters are provided.
    fn extract_params(args: &[Value]) -> Vec<String> {
        if args.len() < 2 {
            return Vec::new();
        }
        match &args[1] {
            Value::Null => Vec::new(),
            Value::List(list) => list
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Int(n) => n.to_string(),
                    Value::Float(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    Value::Null => String::new(),
                    other => format!("{other}"),
                })
                .collect(),
            Value::String(s) => vec![s.clone()],
            Value::Int(n) => vec![n.to_string()],
            other => vec![format!("{other}")],
        }
    }
}

// ---------------------------------------------------------------------------
// QueryBuilder -- ORM-style structured query building from JSON specs
// ---------------------------------------------------------------------------

/// A single JOIN clause parsed from the JSON query spec.
struct JoinClause {
    table: String,
    on: String,
    /// One of `"LEFT"`, `"RIGHT"`, or `"INNER"`.
    join_type: String,
}

/// Builds SQL statements from a parsed JSON query specification.
///
/// The JSON spec format accepted by [`Self::from_json`]:
///
/// ```json
/// {
///   "table": "users",                          // required
///   "select": ["id", "name"],                  // optional, default ["*"]
///   "where": {"active": true, "age": {">": 18}},
///   "join": [{"table": "orders", "on": "users.id = orders.user_id", "type": "LEFT"}],
///   "group_by": ["status"],
///   "having": {"count": {">": 5}},
///   "order_by": [{"field": "name", "dir": "asc"}],
///   "limit": 10,
///   "offset": 20
/// }
/// ```
///
/// WHERE clause operators: `=`, `>`, `>=`, `<`, `<=`, `!=`, `<>`, `like`, `in`, `not`.
/// Setting a field to `null` produces `IS NULL`; `{"not": null}` produces `IS NOT NULL`.
struct QueryBuilder {
    table: String,
    select: Vec<String>,
    joins: Vec<JoinClause>,
    where_clauses: Vec<String>,
    group_by: Vec<String>,
    having_clauses: Vec<String>,
    order_by: Vec<(String, String)>,
    limit: Option<i64>,
    offset: Option<i64>,
}

impl QueryBuilder {
    /// Validate that a SQL identifier contains only safe characters.
    ///
    /// Allows alphanumeric, underscore, dot (qualified names like `users.id`),
    /// parentheses and comma (aggregate expressions like `COUNT(*)`), and spaces
    /// (for `AS` aliases). Rejects semicolons, quotes, and other SQL injection
    /// vectors.
    fn validate_identifier(name: &str) -> Result<(), PluginError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(PluginError::InvalidArg("empty identifier".into()));
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
            return Err(PluginError::InvalidArg(format!(
                "invalid character '{ch}' in identifier '{trimmed}'"
            )));
        }
        Ok(())
    }

    /// Parse a JSON spec object into a `QueryBuilder`.
    ///
    /// See the struct-level documentation for the full spec format.
    #[allow(clippy::too_many_lines)]
    fn from_json(spec: &serde_json::Value) -> Result<Self, PluginError> {
        let obj = spec.as_object().ok_or_else(|| {
            PluginError::InvalidArg("query spec must be a JSON object".into())
        })?;

        // table (required)
        let table = obj
            .get("table")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::InvalidArg("missing 'table' in query spec".into()))?
            .to_string();
        Self::validate_identifier(&table)?;

        // select (optional, default ["*"])
        let select = if let Some(sel) = obj.get("select") {
            let arr = sel.as_array().ok_or_else(|| {
                PluginError::InvalidArg("'select' must be an array of strings".into())
            })?;
            let mut fields = Vec::with_capacity(arr.len());
            for item in arr {
                let s = item.as_str().ok_or_else(|| {
                    PluginError::InvalidArg("select items must be strings".into())
                })?;
                Self::validate_identifier(s)?;
                fields.push(s.to_string());
            }
            fields
        } else {
            vec!["*".to_string()]
        };

        // join (optional)
        let joins = if let Some(join_val) = obj.get("join") {
            let arr = join_val.as_array().ok_or_else(|| {
                PluginError::InvalidArg("'join' must be an array".into())
            })?;
            let mut joins = Vec::with_capacity(arr.len());
            for j in arr {
                let j_obj = j.as_object().ok_or_else(|| {
                    PluginError::InvalidArg("join entry must be an object".into())
                })?;
                let j_table = j_obj
                    .get("table")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        PluginError::InvalidArg("join missing 'table'".into())
                    })?;
                Self::validate_identifier(j_table)?;
                let j_on = j_obj
                    .get("on")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        PluginError::InvalidArg("join missing 'on'".into())
                    })?;
                // Validate the ON clause -- allow = sign in addition to identifiers
                for part in j_on.split('=') {
                    Self::validate_identifier(part.trim())?;
                }
                let j_type = j_obj
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("LEFT")
                    .to_uppercase();
                if !["LEFT", "RIGHT", "INNER"].contains(&j_type.as_str()) {
                    return Err(PluginError::InvalidArg(format!(
                        "invalid join type '{j_type}', must be LEFT, RIGHT, or INNER"
                    )));
                }
                joins.push(JoinClause {
                    table: j_table.to_string(),
                    on: j_on.to_string(),
                    join_type: j_type,
                });
            }
            joins
        } else {
            Vec::new()
        };

        // where (optional)
        let where_clauses = if let Some(w) = obj.get("where") {
            Self::parse_where_clause(w)?
        } else {
            Vec::new()
        };

        // group_by (optional)
        let group_by = if let Some(gb) = obj.get("group_by") {
            let arr = gb.as_array().ok_or_else(|| {
                PluginError::InvalidArg("'group_by' must be an array".into())
            })?;
            let mut fields = Vec::with_capacity(arr.len());
            for item in arr {
                let s = item.as_str().ok_or_else(|| {
                    PluginError::InvalidArg("group_by items must be strings".into())
                })?;
                Self::validate_identifier(s)?;
                fields.push(s.to_string());
            }
            fields
        } else {
            Vec::new()
        };

        // having (optional) -- same structure as where
        let having_clauses = if let Some(h) = obj.get("having") {
            Self::parse_where_clause(h)?
        } else {
            Vec::new()
        };

        // order_by (optional)
        let order_by = if let Some(ob) = obj.get("order_by") {
            let arr = ob.as_array().ok_or_else(|| {
                PluginError::InvalidArg("'order_by' must be an array".into())
            })?;
            let mut orders = Vec::with_capacity(arr.len());
            for item in arr {
                let o = item.as_object().ok_or_else(|| {
                    PluginError::InvalidArg("order_by items must be objects".into())
                })?;
                let field = o
                    .get("field")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        PluginError::InvalidArg("order_by missing 'field'".into())
                    })?;
                Self::validate_identifier(field)?;
                let dir = o
                    .get("dir")
                    .and_then(|v| v.as_str())
                    .unwrap_or("asc")
                    .to_uppercase();
                if dir != "ASC" && dir != "DESC" {
                    return Err(PluginError::InvalidArg(format!(
                        "invalid order direction '{dir}', must be ASC or DESC"
                    )));
                }
                orders.push((field.to_string(), dir));
            }
            orders
        } else {
            Vec::new()
        };

        // limit (optional)
        let limit = obj.get("limit").and_then(serde_json::Value::as_i64);

        // offset (optional)
        let offset = obj.get("offset").and_then(serde_json::Value::as_i64);

        Ok(Self {
            table,
            select,
            joins,
            where_clauses,
            group_by,
            having_clauses,
            order_by,
            limit,
            offset,
        })
    }

    /// Parse a WHERE or HAVING clause from a JSON object.
    ///
    /// Supported patterns:
    /// - `{"field": "value"}` -> `field = 'value'`
    /// - `{"field": {">": 5}}` -> `field > 5`
    /// - `{"field": {"like": "%x%"}}` -> `field LIKE '%x%'`
    /// - `{"field": {"in": [1,2,3]}}` -> `field IN (1, 2, 3)`
    /// - `{"field": null}` -> `field IS NULL`
    /// - `{"field": {"not": null}}` -> `field IS NOT NULL`
    fn parse_where_clause(
        val: &serde_json::Value,
    ) -> Result<Vec<String>, PluginError> {
        let obj = val.as_object().ok_or_else(|| {
            PluginError::InvalidArg("where/having clause must be a JSON object".into())
        })?;
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
                                let formatted = Self::format_value(val);
                                clauses.push(format!("{field} {op} {formatted}"));
                            }
                            "like" => {
                                let s = val.as_str().ok_or_else(|| {
                                    PluginError::InvalidArg(
                                        "LIKE value must be a string".into(),
                                    )
                                })?;
                                let escaped = s.replace('\'', "''");
                                clauses.push(format!("{field} LIKE '{escaped}'"));
                            }
                            "in" => {
                                let arr = val.as_array().ok_or_else(|| {
                                    PluginError::InvalidArg(
                                        "IN value must be an array".into(),
                                    )
                                })?;
                                let items: Vec<String> =
                                    arr.iter().map(Self::format_value).collect();
                                clauses.push(format!(
                                    "{field} IN ({})",
                                    items.join(", ")
                                ));
                            }
                            "not" => {
                                if val.is_null() {
                                    clauses.push(format!("{field} IS NOT NULL"));
                                } else {
                                    let formatted = Self::format_value(val);
                                    clauses.push(format!("{field} != {formatted}"));
                                }
                            }
                            _ => {
                                return Err(PluginError::InvalidArg(format!(
                                    "unsupported operator '{op}'"
                                )));
                            }
                        }
                    }
                }
                serde_json::Value::Array(_) => {
                    return Err(PluginError::InvalidArg(format!(
                        "where value for '{field}' cannot be a bare array; use {{\"in\": [...]}}"
                    )));
                }
            }
        }
        Ok(clauses)
    }

    /// Format a JSON value as a SQL literal for embedding in generated queries.
    fn format_value(val: &serde_json::Value) -> String {
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

    /// Build a `SELECT` SQL statement from the parsed spec.
    fn build_select(&self) -> String {
        let mut sql = format!("SELECT {} FROM {}", self.select.join(", "), self.table);

        for join in &self.joins {
            let _ = write!(
                sql,
                " {} JOIN {} ON {}",
                join.join_type, join.table, join.on
            );
        }

        if !self.where_clauses.is_empty() {
            let _ = write!(sql, " WHERE {}", self.where_clauses.join(" AND "));
        }

        if !self.group_by.is_empty() {
            let _ = write!(sql, " GROUP BY {}", self.group_by.join(", "));
        }

        if !self.having_clauses.is_empty() {
            let _ = write!(sql, " HAVING {}", self.having_clauses.join(" AND "));
        }

        if !self.order_by.is_empty() {
            let orders: Vec<String> = self
                .order_by
                .iter()
                .map(|(f, d)| format!("{f} {d}"))
                .collect();
            let _ = write!(sql, " ORDER BY {}", orders.join(", "));
        }

        if let Some(limit) = self.limit {
            let _ = write!(sql, " LIMIT {limit}");
        }

        if let Some(offset) = self.offset {
            let _ = write!(sql, " OFFSET {offset}");
        }

        sql
    }

    /// Build a `SELECT COUNT(*)` SQL statement from the parsed spec.
    fn build_count(&self) -> String {
        let mut sql = format!("SELECT COUNT(*) FROM {}", self.table);

        for join in &self.joins {
            let _ = write!(
                sql,
                " {} JOIN {} ON {}",
                join.join_type, join.table, join.on
            );
        }

        if !self.where_clauses.is_empty() {
            let _ = write!(sql, " WHERE {}", self.where_clauses.join(" AND "));
        }

        if !self.group_by.is_empty() {
            let _ = write!(sql, " GROUP BY {}", self.group_by.join(", "));
        }

        if !self.having_clauses.is_empty() {
            let _ = write!(sql, " HAVING {}", self.having_clauses.join(" AND "));
        }

        sql
    }
}

// ---------------------------------------------------------------------------
// Convention implementation helpers
// ---------------------------------------------------------------------------

impl PostgresPlugin {
    /// Convention 0: `query` -- execute a SELECT and return all rows.
    ///
    /// If the argument looks like a bare table name (no spaces, no SQL keywords),
    /// it is auto-wrapped in `SELECT * FROM <name>`. This handles the common case
    /// where the Mind extracts just the table name from an intent like "find all users".
    fn do_query(&self, args: &[Value]) -> Result<Value, PluginError> {
        let raw_sql = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing sql argument".into()))?
            .as_str()?;

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

        let params = Self::extract_params(args);
        let param_refs: Vec<&(dyn postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|s| s as &(dyn postgres::types::ToSql + Sync))
            .collect();

        let mut client = self.connect()?;

        let rows = client
            .query(&*sql, &param_refs)
            .map_err(|e| PluginError::Failed(Self::format_pg_error(&e)))?;

        let values: Vec<Value> = rows.iter().map(Self::row_to_value).collect();
        Ok(Value::List(values))
    }

    /// Convention 1: `execute` -- run an INSERT/UPDATE/DELETE and return rows affected.
    fn do_execute(&self, args: &[Value]) -> Result<Value, PluginError> {
        let sql = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing sql argument".into()))?
            .as_str()?;
        let params = Self::extract_params(args);
        let param_refs: Vec<&(dyn postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|s| s as &(dyn postgres::types::ToSql + Sync))
            .collect();

        let mut client = self.connect()?;

        let count = client
            .execute(sql, &param_refs)
            .map_err(|e| PluginError::Failed(Self::format_pg_error(&e)))?;

        // Row count from execute() is u64; Value::Int is i64.
        // Wrapping is acceptable -- row counts exceeding i64::MAX are not realistic.
        #[allow(clippy::cast_possible_wrap)]
        Ok(Value::Int(count as i64))
    }

    /// Convention 2: `query_one` -- execute a query expecting zero or one row.
    fn do_query_one(&self, args: &[Value]) -> Result<Value, PluginError> {
        let sql = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing sql argument".into()))?
            .as_str()?;
        let params = Self::extract_params(args);
        let param_refs: Vec<&(dyn postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|s| s as &(dyn postgres::types::ToSql + Sync))
            .collect();

        let mut client = self.connect()?;

        let row_opt = client
            .query_opt(sql, &param_refs)
            .map_err(|e| PluginError::Failed(Self::format_pg_error(&e)))?;

        Ok(row_opt.map_or(Value::Null, |row| Self::row_to_value(&row)))
    }

    /// Convention 6: `create_table` -- DDL CREATE TABLE IF NOT EXISTS.
    fn do_create_table(&self, args: &[Value]) -> Result<Value, PluginError> {
        let name = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing table name".into()))?
            .as_str()?;
        let columns = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing columns definition".into()))?
            .as_str()?;

        if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(PluginError::InvalidArg(
                "table name must be alphanumeric/underscore".into(),
            ));
        }

        let sql = format!("CREATE TABLE IF NOT EXISTS {name} ({columns})");
        let mut client = self.connect()?;

        client
            .execute(&*sql, &[])
            .map_err(|e| PluginError::Failed(Self::format_pg_error(&e)))?;

        Ok(Value::Null)
    }

    /// Convention 7: `alter_table` -- DDL ALTER TABLE.
    fn do_alter_table(&self, args: &[Value]) -> Result<Value, PluginError> {
        let name = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing table name".into()))?
            .as_str()?;
        let changes = args
            .get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing changes definition".into()))?
            .as_str()?;

        if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(PluginError::InvalidArg(
                "table name must be alphanumeric/underscore".into(),
            ));
        }

        let sql = format!("ALTER TABLE {name} {changes}");
        let mut client = self.connect()?;

        client
            .execute(&*sql, &[])
            .map_err(|e| PluginError::Failed(Self::format_pg_error(&e)))?;

        Ok(Value::Null)
    }

    /// Convention 8: `table_exists` -- check whether a table exists in the public schema.
    fn do_table_exists(&self, args: &[Value]) -> Result<Value, PluginError> {
        let name = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing table name".into()))?
            .as_str()?;

        let sql = "SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = $1)";
        let mut client = self.connect()?;

        let row = client
            .query_one(sql, &[&name])
            .map_err(|e| PluginError::Failed(Self::format_pg_error(&e)))?;

        let exists: bool = row.get(0);
        Ok(Value::Bool(exists))
    }

    /// Convention 9: `list_tables` -- list all tables in the public schema.
    fn do_list_tables(&self) -> Result<Value, PluginError> {
        let sql = "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' ORDER BY table_name";
        let mut client = self.connect()?;

        let rows = client
            .query(sql, &[])
            .map_err(|e| PluginError::Failed(Self::format_pg_error(&e)))?;

        let tables: Vec<Value> = rows
            .iter()
            .map(|r| {
                let name: String = r.get(0);
                Value::String(name)
            })
            .collect();

        Ok(Value::List(tables))
    }

    /// Convention 10: `table_schema` -- return column metadata for a table.
    ///
    /// Queries `information_schema.columns` and returns each column as a map with
    /// keys: `name`, `type`, `nullable`, and optionally `default`, `max_length`,
    /// `precision`.
    fn do_table_schema(&self, args: &[Value]) -> Result<Value, PluginError> {
        let name = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing table name".into()))?
            .as_str()?;

        let sql = r"
            SELECT column_name, data_type, is_nullable, column_default,
                   character_maximum_length, numeric_precision
            FROM information_schema.columns
            WHERE table_schema = 'public' AND table_name = $1
            ORDER BY ordinal_position
        ";

        let mut client = self.connect()?;

        let rows = client
            .query(sql, &[&name])
            .map_err(|e| PluginError::Failed(Self::format_pg_error(&e)))?;

        let columns: Vec<Value> = rows
            .iter()
            .map(|r| {
                let mut map = HashMap::new();
                let col_name: String = r.get(0);
                let data_type: String = r.get(1);
                let nullable: String = r.get(2);
                map.insert("name".to_string(), Value::String(col_name));
                map.insert("type".to_string(), Value::String(data_type));
                map.insert("nullable".to_string(), Value::Bool(nullable == "YES"));
                if let Ok(Some(def)) = r.try_get::<_, Option<String>>(3) {
                    map.insert("default".to_string(), Value::String(def));
                }
                if let Ok(Some(len)) = r.try_get::<_, Option<i32>>(4) {
                    map.insert("max_length".to_string(), Value::Int(i64::from(len)));
                }
                if let Ok(Some(prec)) = r.try_get::<_, Option<i32>>(5) {
                    map.insert("precision".to_string(), Value::Int(i64::from(prec)));
                }
                Value::Map(map)
            })
            .collect();

        Ok(Value::List(columns))
    }

    /// Convention 11: `find` -- structured SELECT via JSON spec.
    fn do_find(&self, args: &[Value]) -> Result<Value, PluginError> {
        let spec_str = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing spec argument".into()))?
            .as_str()?;
        let spec: serde_json::Value = serde_json::from_str(spec_str)
            .map_err(|e| PluginError::InvalidArg(format!("invalid JSON spec: {e}")))?;
        let builder = QueryBuilder::from_json(&spec)?;
        let sql = builder.build_select();

        let mut client = self.connect()?;
        let rows = client
            .query(&*sql, &[])
            .map_err(|e| PluginError::Failed(format!("{e} -- SQL: {sql}")))?;

        let values: Vec<Value> = rows.iter().map(Self::row_to_value).collect();
        Ok(Value::List(values))
    }

    /// Convention 12: `find_one` -- structured single-row query (forces LIMIT 1).
    fn do_find_one(&self, args: &[Value]) -> Result<Value, PluginError> {
        let spec_str = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing spec argument".into()))?
            .as_str()?;
        let mut spec: serde_json::Value = serde_json::from_str(spec_str)
            .map_err(|e| PluginError::InvalidArg(format!("invalid JSON spec: {e}")))?;
        // Force LIMIT 1 for find_one
        spec.as_object_mut()
            .ok_or_else(|| PluginError::InvalidArg("spec must be a JSON object".into()))?
            .insert("limit".to_string(), serde_json::Value::from(1));
        let builder = QueryBuilder::from_json(&spec)?;
        let sql = builder.build_select();

        let mut client = self.connect()?;
        let row_opt = client
            .query_opt(&*sql, &[])
            .map_err(|e| PluginError::Failed(format!("{e} -- SQL: {sql}")))?;

        Ok(row_opt.map_or(Value::Null, |row| Self::row_to_value(&row)))
    }

    /// Convention 13: `count` -- count rows matching a JSON spec.
    fn do_count(&self, args: &[Value]) -> Result<Value, PluginError> {
        let spec_str = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing spec argument".into()))?
            .as_str()?;
        let spec: serde_json::Value = serde_json::from_str(spec_str)
            .map_err(|e| PluginError::InvalidArg(format!("invalid JSON spec: {e}")))?;
        let builder = QueryBuilder::from_json(&spec)?;
        let sql = builder.build_count();

        let mut client = self.connect()?;
        let row = client
            .query_one(&*sql, &[])
            .map_err(|e| PluginError::Failed(format!("{e} -- SQL: {sql}")))?;

        let count: i64 = row.get(0);
        Ok(Value::Int(count))
    }

    /// Convention 14: `aggregate` -- aggregation query with GROUP BY via JSON spec.
    fn do_aggregate(&self, args: &[Value]) -> Result<Value, PluginError> {
        let spec_str = args
            .first()
            .ok_or_else(|| PluginError::InvalidArg("missing spec argument".into()))?
            .as_str()?;
        let spec: serde_json::Value = serde_json::from_str(spec_str)
            .map_err(|e| PluginError::InvalidArg(format!("invalid JSON spec: {e}")))?;
        let builder = QueryBuilder::from_json(&spec)?;
        let sql = builder.build_select();

        let mut client = self.connect()?;
        let rows = client
            .query(&*sql, &[])
            .map_err(|e| PluginError::Failed(format!("{e} -- SQL: {sql}")))?;

        let values: Vec<Value> = rows.iter().map(Self::row_to_value).collect();
        Ok(Value::List(values))
    }
}

// ---------------------------------------------------------------------------
// SomaPlugin trait implementation
// ---------------------------------------------------------------------------

#[allow(clippy::unnecessary_literal_bound)]
impl SomaPlugin for PostgresPlugin {
    fn name(&self) -> &str {
        "postgres"
    }

    fn version(&self) -> &str {
        "0.1.0"
    }

    fn description(&self) -> &str {
        "PostgreSQL database operations: queries, DDL, schema inspection, ORM-style query building"
    }

    fn trust_level(&self) -> TrustLevel {
        TrustLevel::Community
    }

    #[allow(clippy::too_many_lines)]
    fn conventions(&self) -> Vec<Convention> {
        vec![
            Convention {
                id: 0,
                name: "query".into(),
                description: "Execute SELECT query, return rows as list of maps".into(),
                call_pattern: "query(sql, params?)".into(),
                args: vec![
                    ArgSpec {
                        name: "sql".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "SQL SELECT statement (use $1, $2, ... for parameters)".into(),
                    },
                    ArgSpec {
                        name: "params".into(),
                        arg_type: ArgType::Any,
                        required: false,
                        description: "Optional list of query parameters".into(),
                    },
                ],
                returns: ReturnSpec::Value("List<Map>".into()),
                is_deterministic: false,
                estimated_latency_ms: 50,
                max_latency_ms: 30_000,
                side_effects: vec![],
                cleanup: None,
            },
            Convention {
                id: 1,
                name: "execute".into(),
                description: "Execute INSERT/UPDATE/DELETE, return rows affected".into(),
                call_pattern: "execute(sql, params?)".into(),
                args: vec![
                    ArgSpec {
                        name: "sql".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "SQL statement (use $1, $2, ... for parameters)".into(),
                    },
                    ArgSpec {
                        name: "params".into(),
                        arg_type: ArgType::Any,
                        required: false,
                        description: "Optional list of query parameters".into(),
                    },
                ],
                returns: ReturnSpec::Value("Int".into()),
                is_deterministic: false,
                estimated_latency_ms: 50,
                max_latency_ms: 30_000,
                side_effects: vec![SideEffect("database_write".into())],
                cleanup: None,
            },
            Convention {
                id: 2,
                name: "query_one".into(),
                description: "Execute query returning a single row or null".into(),
                call_pattern: "query_one(sql, params?)".into(),
                args: vec![
                    ArgSpec {
                        name: "sql".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "SQL query expected to return 0 or 1 row".into(),
                    },
                    ArgSpec {
                        name: "params".into(),
                        arg_type: ArgType::Any,
                        required: false,
                        description: "Optional list of query parameters".into(),
                    },
                ],
                returns: ReturnSpec::Value("Map | Null".into()),
                is_deterministic: false,
                estimated_latency_ms: 20,
                max_latency_ms: 30_000,
                side_effects: vec![],
                cleanup: None,
            },
            Convention {
                id: 3,
                name: "begin".into(),
                description: "Begin a transaction (returns handle)".into(),
                call_pattern: "begin()".into(),
                args: vec![],
                returns: ReturnSpec::Handle,
                is_deterministic: false,
                estimated_latency_ms: 10,
                max_latency_ms: 5_000,
                side_effects: vec![SideEffect("transaction_start".into())],
                cleanup: Some(CleanupSpec {
                    convention_id: 5, // rollback
                    pass_result_as: 0,
                }),
            },
            Convention {
                id: 4,
                name: "commit".into(),
                description: "Commit a transaction".into(),
                call_pattern: "commit(txn)".into(),
                args: vec![ArgSpec {
                    name: "txn".into(),
                    arg_type: ArgType::Handle,
                    required: true,
                    description: "Transaction handle from begin()".into(),
                }],
                returns: ReturnSpec::Void,
                is_deterministic: false,
                estimated_latency_ms: 10,
                max_latency_ms: 5_000,
                side_effects: vec![SideEffect("transaction_commit".into())],
                cleanup: None,
            },
            Convention {
                id: 5,
                name: "rollback".into(),
                description: "Rollback a transaction".into(),
                call_pattern: "rollback(txn)".into(),
                args: vec![ArgSpec {
                    name: "txn".into(),
                    arg_type: ArgType::Handle,
                    required: true,
                    description: "Transaction handle from begin()".into(),
                }],
                returns: ReturnSpec::Void,
                is_deterministic: false,
                estimated_latency_ms: 10,
                max_latency_ms: 5_000,
                side_effects: vec![SideEffect("transaction_rollback".into())],
                cleanup: None,
            },
            Convention {
                id: 6,
                name: "create_table".into(),
                description: "Create a new table with specified columns".into(),
                call_pattern: "create_table(name, columns)".into(),
                args: vec![
                    ArgSpec {
                        name: "name".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Table name".into(),
                    },
                    ArgSpec {
                        name: "columns".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Column definitions, e.g. 'id SERIAL PRIMARY KEY, name TEXT NOT NULL'".into(),
                    },
                ],
                returns: ReturnSpec::Void,
                is_deterministic: false,
                estimated_latency_ms: 100,
                max_latency_ms: 10_000,
                side_effects: vec![SideEffect("ddl_create".into())],
                cleanup: None,
            },
            Convention {
                id: 7,
                name: "alter_table".into(),
                description: "Alter an existing table structure".into(),
                call_pattern: "alter_table(name, changes)".into(),
                args: vec![
                    ArgSpec {
                        name: "name".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "Table name".into(),
                    },
                    ArgSpec {
                        name: "changes".into(),
                        arg_type: ArgType::String,
                        required: true,
                        description: "ALTER clause, e.g. 'ADD COLUMN bio TEXT'".into(),
                    },
                ],
                returns: ReturnSpec::Void,
                is_deterministic: false,
                estimated_latency_ms: 100,
                max_latency_ms: 10_000,
                side_effects: vec![SideEffect("ddl_alter".into())],
                cleanup: None,
            },
            Convention {
                id: 8,
                name: "table_exists".into(),
                description: "Check if a table exists".into(),
                call_pattern: "table_exists(name)".into(),
                args: vec![ArgSpec {
                    name: "name".into(),
                    arg_type: ArgType::String,
                    required: true,
                    description: "Table name to check".into(),
                }],
                returns: ReturnSpec::Value("Bool".into()),
                is_deterministic: false,
                estimated_latency_ms: 10,
                max_latency_ms: 5_000,
                side_effects: vec![],
                cleanup: None,
            },
            Convention {
                id: 9,
                name: "list_tables".into(),
                description: "List all tables in the database".into(),
                call_pattern: "list_tables()".into(),
                args: vec![],
                returns: ReturnSpec::Value("List<String>".into()),
                is_deterministic: false,
                estimated_latency_ms: 20,
                max_latency_ms: 5_000,
                side_effects: vec![],
                cleanup: None,
            },
            Convention {
                id: 10,
                name: "table_schema".into(),
                description: "Get column information for a table".into(),
                call_pattern: "table_schema(name)".into(),
                args: vec![ArgSpec {
                    name: "name".into(),
                    arg_type: ArgType::String,
                    required: true,
                    description: "Table name".into(),
                }],
                returns: ReturnSpec::Value("List<Map>".into()),
                is_deterministic: false,
                estimated_latency_ms: 20,
                max_latency_ms: 5_000,
                side_effects: vec![],
                cleanup: None,
            },
            // ORM-style query builder conventions (11-14)
            Convention {
                id: 11,
                name: "find".into(),
                description: "Structured SELECT query builder. Accepts a JSON spec with table, select, where, join, group_by, having, order_by, limit, offset fields. Returns matching rows as list of maps.".into(),
                call_pattern: "find(spec)".into(),
                args: vec![ArgSpec {
                    name: "spec".into(),
                    arg_type: ArgType::String,
                    required: true,
                    description: "JSON query spec: {\"table\":\"...\", \"select\":[...], \"where\":{...}, \"join\":[...], \"group_by\":[...], \"having\":{...}, \"order_by\":[{\"field\":\"...\",\"dir\":\"asc\"}], \"limit\":N, \"offset\":N}".into(),
                }],
                returns: ReturnSpec::Value("List<Map>".into()),
                is_deterministic: false,
                estimated_latency_ms: 50,
                max_latency_ms: 30_000,
                side_effects: vec![],
                cleanup: None,
            },
            Convention {
                id: 12,
                name: "find_one".into(),
                description: "Structured single-row query. Same JSON spec as find but forces LIMIT 1 and returns a single Map or Null.".into(),
                call_pattern: "find_one(spec)".into(),
                args: vec![ArgSpec {
                    name: "spec".into(),
                    arg_type: ArgType::String,
                    required: true,
                    description: "JSON query spec (same as find). LIMIT 1 is applied automatically.".into(),
                }],
                returns: ReturnSpec::Value("Map | Null".into()),
                is_deterministic: false,
                estimated_latency_ms: 20,
                max_latency_ms: 30_000,
                side_effects: vec![],
                cleanup: None,
            },
            Convention {
                id: 13,
                name: "count".into(),
                description: "Count rows matching conditions. Accepts a JSON spec with table and optional where clause. Returns integer count.".into(),
                call_pattern: "count(spec)".into(),
                args: vec![ArgSpec {
                    name: "spec".into(),
                    arg_type: ArgType::String,
                    required: true,
                    description: "JSON spec: {\"table\":\"...\", \"where\":{...}}".into(),
                }],
                returns: ReturnSpec::Value("Int".into()),
                is_deterministic: false,
                estimated_latency_ms: 20,
                max_latency_ms: 30_000,
                side_effects: vec![],
                cleanup: None,
            },
            Convention {
                id: 14,
                name: "aggregate".into(),
                description: "Aggregation query with GROUP BY. Accepts a JSON spec with table, select (aggregate expressions), group_by, having, order_by, limit. Returns rows as list of maps.".into(),
                call_pattern: "aggregate(spec)".into(),
                args: vec![ArgSpec {
                    name: "spec".into(),
                    arg_type: ArgType::String,
                    required: true,
                    description: "JSON spec: {\"table\":\"...\", \"select\":[\"AVG(rating) as avg_rating\", ...], \"group_by\":[...], \"having\":{...}, \"order_by\":[...], \"limit\":N}".into(),
                }],
                returns: ReturnSpec::Value("List<Map>".into()),
                is_deterministic: false,
                estimated_latency_ms: 50,
                max_latency_ms: 30_000,
                side_effects: vec![],
                cleanup: None,
            },
        ]
    }

    fn execute(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        self.dispatch(convention_id, args)
    }

    fn on_load(&mut self, config: &PluginConfig) -> Result<(), PluginError> {
        let host = config
            .get_str("host")
            .unwrap_or("localhost")
            .to_string();

        // Port config is i64 from JSON; truncation to u16 is safe for valid port numbers.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let port = config.get_int("port").unwrap_or(5432) as u16;

        let database = config
            .get_str("database")
            .unwrap_or("soma")
            .to_string();
        let username = config
            .get_str("username")
            .unwrap_or("soma")
            .to_string();

        // Read password from the env var named in the password_env config field,
        // falling back to SOMA_POSTGRES_PASSWORD if not specified.
        let password = config.get_str("password_env").map_or_else(
            || std::env::var("SOMA_POSTGRES_PASSWORD").ok(),
            |env_name| std::env::var(env_name).ok(),
        );

        let conn_string = format!(
            "host={host} port={port} dbname={database} user={username} password={}",
            password.as_deref().unwrap_or("")
        );

        self.conn_string
            .set(conn_string)
            .map_err(|_| PluginError::Failed("postgres already configured".into()))?;

        Ok(())
    }

    fn permissions(&self) -> PluginPermissions {
        PluginPermissions {
            filesystem: vec![],
            network: vec!["tcp:*:5432".into()],
            env_vars: vec!["SOMA_POSTGRES_*".into()],
            process_spawn: false,
        }
    }

    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "host":         {"type": "string", "default": "localhost"},
                "port":         {"type": "integer", "default": 5432},
                "database":     {"type": "string", "default": "soma"},
                "username":     {"type": "string", "default": "soma"},
                "password_env": {"type": "string", "description": "Env var containing the password"}
            },
            "required": ["database"]
        }))
    }

    fn training_data(&self) -> Option<serde_json::Value> {
        let data = include_str!("../training/examples.json");
        serde_json::from_str(data).ok()
    }
}

// ---------------------------------------------------------------------------
// Dispatch -- routes convention_id to the appropriate handler
// ---------------------------------------------------------------------------

impl PostgresPlugin {
    /// Route a convention call to the corresponding implementation method.
    ///
    /// Takes `Vec<Value>` by value to match the `SomaPlugin::execute` trait signature.
    #[allow(clippy::needless_pass_by_value)]
    fn dispatch(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        match convention_id {
            0 => self.do_query(&args),
            1 => self.do_execute(&args),
            2 => self.do_query_one(&args),
            3..=5 => Err(PluginError::Failed(
                "transactions (begin/commit/rollback) are not yet supported in this MVP; \
                 use execute(\"BEGIN; ...; COMMIT\") for multi-statement transactions"
                    .into(),
            )),
            6 => self.do_create_table(&args),
            7 => self.do_alter_table(&args),
            8 => self.do_table_exists(&args),
            9 => self.do_list_tables(),
            10 => self.do_table_schema(&args),
            11 => self.do_find(&args),
            12 => self.do_find_one(&args),
            13 => self.do_count(&args),
            14 => self.do_aggregate(&args),
            _ => Err(PluginError::NotFound(format!(
                "unknown convention id: {convention_id}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// C ABI export
// ---------------------------------------------------------------------------

/// FFI entry point called by the SOMA plugin manager to instantiate this plugin.
///
/// Returns a heap-allocated trait object. The caller (plugin manager) takes
/// ownership and is responsible for dropping it.
#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn soma_plugin_init() -> *mut dyn SomaPlugin {
    Box::into_raw(Box::new(PostgresPlugin::new()))
}
