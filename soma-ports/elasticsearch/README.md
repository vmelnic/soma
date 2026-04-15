# soma-port-elasticsearch

`soma-port-elasticsearch` is a `cdylib` SOMA port that provides search and indexing operations via `reqwest` HTTP client against the Elasticsearch REST API.

- Port ID: `soma.elasticsearch`
- Kind: `Database`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: required

## Capabilities

- `search`: `index`, `query`, `size`, `from`
- `index_document`: `index`, `document`, `id`
- `get_document`: `index`, `id`
- `delete_document`: `index`, `id`
- `create_index`: `index`, `mappings`
- `delete_index`: `index`

## Configuration

| Env var | Description |
|---|---|
| `SOMA_ELASTICSEARCH_URL` | Elasticsearch base URL, e.g. `http://localhost:9200` (primary) |
| `ELASTICSEARCH_URL` | Elasticsearch base URL (fallback) |

## Build

```bash
cargo build
cargo test
```
