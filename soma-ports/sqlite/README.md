# soma-port-sqlite

`soma-port-sqlite` is a `cdylib` SOMA port for SQLite local database operations.

- Port ID: `sqlite`
- Kind: `Database`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: not required

## Capabilities

- Raw SQL: `query`, `execute`
- ORM-style: `find`, `find_many`, `count`, `aggregate`
- DDL: `create_table`, `drop_table`, `alter_table`
- DML: `insert`, `update`, `delete`
- Transactions: `begin_transaction`, `commit`, `rollback`

## Configuration

| Env var | Description |
|---|---|
| `SOMA_SQLITE_PATH` | Database file path (default: `soma.db`) |

## Build

```bash
cargo build
cargo test
```
