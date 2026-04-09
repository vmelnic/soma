# soma-port-redis

`soma-port-redis` is a `cdylib` SOMA database port for Redis strings, hashes, lists, publish, and key-pattern lookup.

- Port ID: `redis`
- Kind: `Database`
- Trust level: `Verified`
- Remote exposure: `true`
- Network access: required

## Configuration

- Environment variable: `SOMA_REDIS_URL`
- Default when unset: `redis://localhost:6379/0`

## Capabilities

The current spec exposes 13 capabilities:

- Strings: `get`, `set`, `del`
- Hashes: `hget`, `hset`, `hdel`, `hgetall`
- Lists: `lpush`, `lpop`, `lrange`
- Pub/Sub: `publish`, `subscribe`
- Keys: `keys`

## Runtime Behavior

- The port establishes a Redis `ConnectionManager` during construction and bridges async Redis calls through a private Tokio runtime.
- Lifecycle state is `Active` when the connection manager exists and `Degraded` when Redis is unavailable.
- `set` supports optional `ttl` in seconds and uses `SETEX` when provided.
- `publish` returns the receiver count reported by Redis.

## Important Caveats

- `subscribe` is declared in the port spec but is not supported by the synchronous invocation model. It always returns an error explaining that streaming subscribe is unsupported here.
- If Redis is unreachable during construction, the port still loads but later calls fail with `DependencyUnavailable`.

## Production Notes

- This port is usable today for request/response Redis work.
- If you need subscriptions, blocking consumers, or streaming event delivery, you need a different runtime contract than the current synchronous port API.

## Build

```bash
cargo build
cargo test
```
