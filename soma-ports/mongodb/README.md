# soma-port-mongodb

`soma-port-mongodb` is a `cdylib` SOMA port that provides MongoDB document database operations via the `mongodb` crate with synchronous connections.

- Port ID: `soma.mongodb`
- Kind: `Database`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: required

## Capabilities

- `find`: `collection`, `filter`, `limit`
- `find_one`: `collection`, `filter`
- `insert_one`: `collection`, `document`
- `insert_many`: `collection`, `documents`
- `update_one`: `collection`, `filter`, `update`
- `delete_one`: `collection`, `filter`
- `count`: `collection`, `filter`

## Configuration

| Env var | Description |
|---|---|
| `SOMA_MONGODB_URL` | MongoDB connection URL (primary) |
| `MONGODB_URL` | MongoDB connection URL (fallback) |
| `SOMA_MONGODB_DATABASE` | Database name (default: `soma`) |

## Build

```bash
cargo build
cargo test
```
