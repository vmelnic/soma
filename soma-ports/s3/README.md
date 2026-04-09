# soma-port-s3

`soma-port-s3` is a `cdylib` SOMA port that declares object-storage operations for S3-compatible backends.

- Port ID: `soma.s3`
- Kind: `Database`
- Trust level: `Verified`
- Remote exposure: `true`
- Network access: required

## Declared Capabilities

- `put_object`
- `get_object`
- `delete_object`
- `presign_url`
- `list_objects`

## Intended Behavior

- `bucket` is optional. When omitted, the port uses the default bucket `soma-uploads`.
- `put_object` expects `key` and `data`, with optional `content_type`.
- `get_object`, `delete_object`, and `presign_url` require `key`.
- `list_objects` accepts optional `prefix`.

## Current Limitation

- The current implementation never initializes its AWS SDK client or Tokio runtime.
- Because of that, the lifecycle state stays `Loaded`, not `Active`.
- Invocations reach the port surface but fail with `DependencyUnavailable` because `client` and `runtime` are unset.

## Data Format Caveat

- `put_object` currently treats `data` as a UTF-8 string and uploads its raw bytes.
- `get_object` returns downloaded object bytes as base64 in `data`.
- That asymmetry is important if you intend to store arbitrary binary objects.

## Production Notes

- The declared API is useful, but this crate is not fully wired for production use yet.
- Before relying on it, add a real initialization path for AWS config and align the object payload format you want to support.

## Build

```bash
cargo build
cargo test
```
