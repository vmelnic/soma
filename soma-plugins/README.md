# SOMA Plugins

Shared reusable plugins for SOMA. Each plugin is a Rust cdylib that implements the SomaPlugin trait from the SDK.

## Plugins

| Plugin | Conventions | Description |
|--------|-------------|-------------|
| sdk | - | Plugin interface types (SomaPlugin trait, Value, Convention) |
| crypto | 13 | Hash, sign, encrypt, JWT, random generation |
| postgres | 15 | Query, execute, ORM-style find/count/aggregate |
| redis | 14 | Strings, hashes, lists, pub/sub, keys |
| auth | 10 | OTP verification, session management, TOTP |
| geo | 5 | Distance, radius filter, geocoding |
| http-bridge | 5 | HTTP client (GET/POST/PUT/DELETE) |
| image | 5 | Thumbnail, resize, crop, format convert, EXIF strip |
| s3 | 5 | S3-compatible object storage (put, get, delete, presign, list) |
| push | 4 | FCM, WebPush, device registration |
| timer | 4 | Timeout, interval, cancel, list active |
| smtp | 3 | Email send (plain, HTML, attachments) |

**Total: 11 plugins, 83 conventions.**

## Build

```bash
cargo build --release
```

Produces .dylib (macOS) or .so (Linux) files in target/release/.

## Training Data

Each plugin has `training/examples.json` for Mind synthesis. See [Plugin Development Guide](../docs/plugin-development.md) for training data format and best practices.

## Adding a Plugin

1. Create a new crate directory with `src/lib.rs` and `Cargo.toml` (`crate-type = ["cdylib"]`)
2. Implement `SomaPlugin` trait from sdk
3. Export `soma_plugin_init()` C ABI entry point
4. Add `manifest.json` + `training/examples.json`
5. Add to workspace `Cargo.toml` members

See [Plugin Development Guide](../docs/plugin-development.md) for the full tutorial.

## Documentation

- [Plugin System](../docs/plugin-system.md) — Architecture, trait, conventions, Value type
- [Plugin Catalog](../docs/plugin-catalog.md) — All conventions reference
- [Plugin Development](../docs/plugin-development.md) — Build a plugin from scratch
