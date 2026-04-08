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

## Build

```bash
cargo build --release
```

Produces .dylib (macOS) or .so (Linux) files in target/release/.

## Training Data

Each plugin has training/examples.json for Mind synthesis.

## Adding a Plugin

1. Create a new crate directory
2. Implement SomaPlugin trait from sdk
3. Export soma_plugin_init()
4. Add manifest.json + training/examples.json
5. Add to workspace Cargo.toml members
