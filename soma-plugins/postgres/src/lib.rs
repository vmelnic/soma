//! SOMA PostgreSQL Plugin — database operations via synchronous postgres crate.
//!
//! Provides 11 conventions for PostgreSQL interaction: query, execute,
//! query_one, begin/commit/rollback (MVP: unsupported), create_table,
//! alter_table, table_exists, list_tables, table_schema.
//!
//! Uses the synchronous `postgres` crate for blocking database access.
//! No tokio runtime required — avoids reactor TLS issues in cdylib plugins.

use soma_plugin_sdk::prelude::*;
use std::collections::HashMap;
use std::sync::OnceLock;

use postgres::types::Type;

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

pub struct PostgresPlugin {
    conn_string: OnceLock<String>,
}

impl PostgresPlugin {
    pub fn new() -> Self {
        Self {
            conn_string: OnceLock::new(),
        }
    }

    /// Get a fresh database connection.
    /// Each call creates a new connection — simple but sufficient for SOMA's
    /// request-per-intent model.
    fn connect(&self) -> Result<postgres::Client, PluginError> {
        let conn_str = self.conn_string.get()
            .ok_or_else(|| PluginError::Failed("postgres not configured — call on_load first".into()))?;
        let client = postgres::Client::connect(conn_str, postgres::NoTls)
            .map_err(|e| PluginError::ConnectionRefused(format!("PostgreSQL: {}", e)))?;
        Ok(client)
    }

    /// Convert a `postgres::Row` into `Value::Map`.
    fn row_to_value(row: &postgres::Row) -> Value {
        let mut map = HashMap::new();
        for (i, col) in row.columns().iter().enumerate() {
            let name = col.name().to_string();
            let val = Self::column_value(row, i, col.type_());
            map.insert(name, val);
        }
        Value::Map(map)
    }

    /// Extract a single column value, mapping PG types to `Value`.
    fn column_value(row: &postgres::Row, idx: usize, ty: &Type) -> Value {
        match *ty {
            Type::BOOL => match row.try_get::<_, Option<bool>>(idx) {
                Ok(Some(v)) => Value::Bool(v),
                Ok(None) => Value::Null,
                Err(_) => Value::Null,
            },
            Type::INT2 => match row.try_get::<_, Option<i16>>(idx) {
                Ok(Some(v)) => Value::Int(v as i64),
                Ok(None) => Value::Null,
                Err(_) => Value::Null,
            },
            Type::INT4 => match row.try_get::<_, Option<i32>>(idx) {
                Ok(Some(v)) => Value::Int(v as i64),
                Ok(None) => Value::Null,
                Err(_) => Value::Null,
            },
            Type::INT8 => match row.try_get::<_, Option<i64>>(idx) {
                Ok(Some(v)) => Value::Int(v),
                Ok(None) => Value::Null,
                Err(_) => Value::Null,
            },
            Type::FLOAT4 => match row.try_get::<_, Option<f32>>(idx) {
                Ok(Some(v)) => Value::Float(v as f64),
                Ok(None) => Value::Null,
                Err(_) => Value::Null,
            },
            Type::FLOAT8 => match row.try_get::<_, Option<f64>>(idx) {
                Ok(Some(v)) => Value::Float(v),
                Ok(None) => Value::Null,
                Err(_) => Value::Null,
            },
            Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => {
                match row.try_get::<_, Option<String>>(idx) {
                    Ok(Some(v)) => Value::String(v),
                    Ok(None) => Value::Null,
                    Err(_) => Value::Null,
                }
            }
            Type::JSON | Type::JSONB => {
                match row.try_get::<_, Option<serde_json::Value>>(idx) {
                    Ok(Some(v)) => Value::String(v.to_string()),
                    Ok(None) => Value::Null,
                    Err(_) => Value::Null,
                }
            }
            // Fallback: try to get as string
            _ => match row.try_get::<_, Option<String>>(idx) {
                Ok(Some(v)) => Value::String(v),
                Ok(None) => Value::Null,
                Err(_) => Value::String(format!("<unsupported type: {}>", ty)),
            },
        }
    }

    /// Extract optional params list from args. Returns empty vec if not provided.
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
                    other => format!("{}", other),
                })
                .collect(),
            // Single value treated as one-element list
            Value::String(s) => vec![s.clone()],
            Value::Int(n) => vec![n.to_string()],
            other => vec![format!("{}", other)],
        }
    }
}

// ---------------------------------------------------------------------------
// Synchronous implementation helpers
// ---------------------------------------------------------------------------

impl PostgresPlugin {
    fn do_query(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let sql = args.first()
            .ok_or_else(|| PluginError::InvalidArg("missing sql argument".into()))?
            .as_str()?;
        let params = Self::extract_params(&args);
        let param_refs: Vec<&(dyn postgres::types::ToSql + Sync)> =
            params.iter().map(|s| s as &(dyn postgres::types::ToSql + Sync)).collect();

        let mut client = self.connect()?;

        let rows = client
            .query(sql, &param_refs)
            .map_err(|e| PluginError::Failed(e.to_string()))?;

        let values: Vec<Value> = rows.iter().map(Self::row_to_value).collect();
        Ok(Value::List(values))
    }

    fn do_execute(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let sql = args.first()
            .ok_or_else(|| PluginError::InvalidArg("missing sql argument".into()))?
            .as_str()?;
        let params = Self::extract_params(&args);
        let param_refs: Vec<&(dyn postgres::types::ToSql + Sync)> =
            params.iter().map(|s| s as &(dyn postgres::types::ToSql + Sync)).collect();

        let mut client = self.connect()?;

        let count = client
            .execute(sql, &param_refs)
            .map_err(|e| PluginError::Failed(e.to_string()))?;

        Ok(Value::Int(count as i64))
    }

    fn do_query_one(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let sql = args.first()
            .ok_or_else(|| PluginError::InvalidArg("missing sql argument".into()))?
            .as_str()?;
        let params = Self::extract_params(&args);
        let param_refs: Vec<&(dyn postgres::types::ToSql + Sync)> =
            params.iter().map(|s| s as &(dyn postgres::types::ToSql + Sync)).collect();

        let mut client = self.connect()?;

        let row_opt = client
            .query_opt(sql, &param_refs)
            .map_err(|e| PluginError::Failed(e.to_string()))?;

        match row_opt {
            Some(row) => Ok(Self::row_to_value(&row)),
            None => Ok(Value::Null),
        }
    }

    fn do_create_table(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let name = args.first()
            .ok_or_else(|| PluginError::InvalidArg("missing table name".into()))?
            .as_str()?;
        let columns = args.get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing columns definition".into()))?
            .as_str()?;

        // Basic SQL injection guard on table name
        if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(PluginError::InvalidArg("table name must be alphanumeric/underscore".into()));
        }

        let sql = format!("CREATE TABLE IF NOT EXISTS {} ({})", name, columns);
        let mut client = self.connect()?;

        client
            .execute(&*sql, &[])
            .map_err(|e| PluginError::Failed(e.to_string()))?;

        Ok(Value::Null)
    }

    fn do_alter_table(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let name = args.first()
            .ok_or_else(|| PluginError::InvalidArg("missing table name".into()))?
            .as_str()?;
        let changes = args.get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing changes definition".into()))?
            .as_str()?;

        if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(PluginError::InvalidArg("table name must be alphanumeric/underscore".into()));
        }

        let sql = format!("ALTER TABLE {} {}", name, changes);
        let mut client = self.connect()?;

        client
            .execute(&*sql, &[])
            .map_err(|e| PluginError::Failed(e.to_string()))?;

        Ok(Value::Null)
    }

    fn do_table_exists(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let name = args.first()
            .ok_or_else(|| PluginError::InvalidArg("missing table name".into()))?
            .as_str()?;

        let sql = "SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = $1)";
        let mut client = self.connect()?;

        let row = client
            .query_one(sql, &[&name])
            .map_err(|e| PluginError::Failed(e.to_string()))?;

        let exists: bool = row.get(0);
        Ok(Value::Bool(exists))
    }

    fn do_list_tables(&self) -> Result<Value, PluginError> {
        let sql = "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' ORDER BY table_name";
        let mut client = self.connect()?;

        let rows = client
            .query(sql, &[])
            .map_err(|e| PluginError::Failed(e.to_string()))?;

        let tables: Vec<Value> = rows
            .iter()
            .map(|r| {
                let name: String = r.get(0);
                Value::String(name)
            })
            .collect();

        Ok(Value::List(tables))
    }

    fn do_table_schema(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let name = args.first()
            .ok_or_else(|| PluginError::InvalidArg("missing table name".into()))?
            .as_str()?;

        let sql = r#"
            SELECT column_name, data_type, is_nullable, column_default,
                   character_maximum_length, numeric_precision
            FROM information_schema.columns
            WHERE table_schema = 'public' AND table_name = $1
            ORDER BY ordinal_position
        "#;

        let mut client = self.connect()?;

        let rows = client
            .query(sql, &[&name])
            .map_err(|e| PluginError::Failed(e.to_string()))?;

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
                    map.insert("max_length".to_string(), Value::Int(len as i64));
                }
                if let Ok(Some(prec)) = r.try_get::<_, Option<i32>>(5) {
                    map.insert("precision".to_string(), Value::Int(prec as i64));
                }
                Value::Map(map)
            })
            .collect();

        Ok(Value::List(columns))
    }
}

// ---------------------------------------------------------------------------
// SomaPlugin trait implementation
// ---------------------------------------------------------------------------

impl SomaPlugin for PostgresPlugin {
    fn name(&self) -> &str {
        "postgres"
    }

    fn version(&self) -> &str {
        "0.1.0"
    }

    fn description(&self) -> &str {
        "PostgreSQL database operations: queries, DDL, schema inspection"
    }

    fn trust_level(&self) -> TrustLevel {
        TrustLevel::Community
    }

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
        let port = config.get_int("port").unwrap_or(5432) as u16;
        let database = config
            .get_str("database")
            .unwrap_or("soma")
            .to_string();
        let username = config
            .get_str("username")
            .unwrap_or("soma")
            .to_string();

        // Read password from env var named in password_env config field
        let password = if let Some(env_name) = config.get_str("password_env") {
            std::env::var(env_name).ok()
        } else {
            // Fall back to SOMA_POSTGRES_PASSWORD
            std::env::var("SOMA_POSTGRES_PASSWORD").ok()
        };

        let conn_string = format!(
            "host={} port={} dbname={} user={} password={}",
            host, port, database, username, password.as_deref().unwrap_or("")
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
// Dispatch — routes convention_id to the right sync method
// ---------------------------------------------------------------------------

impl PostgresPlugin {
    fn dispatch(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        match convention_id {
            0 => self.do_query(args),
            1 => self.do_execute(args),
            2 => self.do_query_one(args),
            3 | 4 | 5 => Err(PluginError::Failed(
                "transactions (begin/commit/rollback) are not yet supported in this MVP; \
                 use execute(\"BEGIN; ...; COMMIT\") for multi-statement transactions"
                    .into(),
            )),
            6 => self.do_create_table(args),
            7 => self.do_alter_table(args),
            8 => self.do_table_exists(args),
            9 => self.do_list_tables(),
            10 => self.do_table_schema(args),
            _ => Err(PluginError::NotFound(format!(
                "unknown convention id: {}",
                convention_id
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// C ABI export
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn soma_plugin_init() -> *mut dyn SomaPlugin {
    Box::into_raw(Box::new(PostgresPlugin::new()))
}
