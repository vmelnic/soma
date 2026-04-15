# soma-port-mysql

`soma-port-mysql` is a `cdylib` SOMA port that provides MySQL database operations via the `mysql` crate with synchronous connections.

- Port ID: `soma.mysql`
- Kind: `Database`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: required

## Capabilities

- `query`: `sql`, `params`
- `execute`: `sql`, `params`
- `insert`: `table`, `data`
- `update`: `table`, `data`, `where_clause`
- `delete`: `table`, `where_clause`

## Configuration

| Env var | Description |
|---|---|
| `SOMA_MYSQL_URL` | MySQL connection URL (primary) |
| `MYSQL_URL` | MySQL connection URL (fallback) |

## Build

```bash
cargo build
cargo test
```
