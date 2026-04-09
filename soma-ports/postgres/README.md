# soma-port-postgres

`soma-port-postgres` is a `cdylib` SOMA database port that exposes raw SQL, CRUD-style helpers, DDL, and atomic multi-statement execution on PostgreSQL.

- Port ID: `soma.ports.postgres`
- Kind: `Database`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: required

## Capabilities

- Raw SQL: `query`, `execute`
- ORM-style reads: `find`, `find_many`, `count`, `aggregate`
- Row mutation: `insert`, `update`, `delete`
- Schema operations: `create_table`, `drop_table`, `alter_table`
- Transaction surface: `begin_transaction`, `commit`, `rollback`

## Configuration

- Environment variable: `SOMA_POSTGRES_URL`
- Default when unset: `host=localhost dbname=soma`

## Runtime Behavior

- Each invocation opens a fresh PostgreSQL connection through `tokio-postgres` and bridges async execution through a private Tokio runtime.
- There is no connection pool and there are no long-lived transaction handles across calls.
- `begin_transaction` executes a list of SQL statements atomically inside a single invocation.
- `commit` and `rollback` exist for API completeness only. They are compatibility no-ops and do not resume or control a transaction opened by an earlier call.
- `delete` requires a `where` clause and intentionally rejects full-table deletes.

## Query Model

- `query` and `execute` accept `sql` plus optional `params`.
- JSON scalar params are stringified before binding. If you need exact server-side types, use explicit SQL casts or adjust the implementation.
- The higher-level `find` and `find_many` helpers validate identifiers and build SQL from JSON.
- Supported `where` operators are equality, `>`, `>=`, `<`, `<=`, `!=`, `<>`, `like`, `in`, and `not`.
- `aggregate` supports `SUM`, `AVG`, `MIN`, `MAX`, and `COUNT`.

## Production Notes

- The raw SQL surface is the most flexible and the safest place for advanced PostgreSQL features.
- The CRUD helpers are intentionally narrow and do not try to be a full ORM.
- Because every call creates a new connection, high-throughput deployments will likely want pooling before using this crate as-is.

## Build

```bash
cargo build
cargo test
```
