# soma-ports — Port Development Guide

## Structure

Each port is a cdylib crate that depends on `soma-port-sdk` and exports `soma_port_init`. One manifest.json per port.

## Build

```bash
cargo build --workspace --release
cargo build --release --manifest-path redis/Cargo.toml
```

After rebuilding, re-copy the `.dylib` to any `soma-project-*/packs/` that uses it.

## Pack manifests

Full `PackSpec`; template in `packs/reference/manifest.json`. `port_id` must match library name. Skills need all fields.
