# soma-ports

`soma-ports` is the external port workspace for `soma-next`.

Each port crate compiles to a shared library (`cdylib`) that implements the
`soma-port-sdk` `Port` trait and exports `soma_port_init`. These libraries are
loaded by `soma-next` at runtime from directories listed in
`[ports].plugin_path`.

This directory contains the external adapters. It is not the runtime itself.

## Workspace layout

Current layout:

- [`sdk`](sdk): shared SDK used by all external ports
- [`auth`](auth): authentication flows
- [`crypto`](crypto): cryptographic primitives and token helpers
- [`geo`](geo): geospatial math and geocoding stubs
- [`image`](image): image processing
- [`postgres`](postgres): PostgreSQL access
- [`push`](push): push notifications
- [`redis`](redis): Redis access
- [`s3`](s3): S3-compatible object storage
- [`smtp`](smtp): SMTP email delivery
- [`timer`](timer): timer and scheduler primitives

Workspace membership is defined in [`Cargo.toml`](Cargo.toml). `redis` is
intentionally excluded from the main workspace and carries its own lockfile, so
it must be built and tested separately.

## Port catalog

| Port | Crate | Output library | Capabilities | Notes |
| --- | --- | --- | ---: | --- |
| auth | `soma-port-auth` | `libsoma_port_auth.*` | 10 | OTP, sessions, TOTP, bearer tokens; uses in-memory stores |
| crypto | `soma-port-crypto` | `libsoma_port_crypto.*` | 13 | Hashing, HMAC, bcrypt, AES-GCM, RSA, JWT, randomness |
| geo | `soma-port-geo` | `libsoma_port_geo.*` | 5 | Distance, radius filter, bounds check, geocode stubs |
| image | `soma-port-image` | `libsoma_port_image.*` | 5 | Thumbnail, resize, crop, format conversion, EXIF strip |
| postgres | `soma-port-postgres` | `libsoma_port_postgres.*` | 15 | Raw SQL, CRUD, DDL, transactions; uses `SOMA_POSTGRES_URL` |
| push | `soma-port-push` | `libsoma_port_push.*` | 4 | FCM, WebPush, device registration; in-memory registry |
| redis | `soma-port-redis` | `libsoma_port_redis.*` | 14 | Strings, hashes, lists, pub/sub; uses `SOMA_REDIS_URL` |
| s3 | `soma-port-s3` | `libsoma_port_s3.*` | 5 | Put/get/delete/presign/list via AWS SDK |
| smtp | `soma-port-smtp` | `libsoma_port_smtp.*` | 3 | Plain, HTML, attachment email via `lettre` |
| timer | `soma-port-timer` | `libsoma_port_timer.*` | 4 | Timeouts, intervals, cancellation, listing; in-memory state |

Notes:

- `image`, `timer`, and most of `geo` are pure local logic with no service
  dependency.
- `postgres` creates a fresh connection per call.
- `redis`, `s3`, and `smtp` bridge async clients into the synchronous `Port`
  trait with internal Tokio runtimes.
- `auth`, `push`, and `timer` keep volatile in-memory state and are not durable
  across process restarts.

## Build, test, and lint

Build the main workspace:

```bash
cd soma-ports

cargo build --workspace
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets --all-features
```

Build `redis` separately:

```bash
cd soma-ports

cargo build --manifest-path redis/Cargo.toml --all-targets
cargo test --manifest-path redis/Cargo.toml --all-targets
cargo clippy --manifest-path redis/Cargo.toml --all-targets --all-features
```

Debug libraries are emitted under `target/debug`, for example:

- `target/debug/libsoma_port_postgres.dylib`
- `target/debug/libsoma_port_auth.dylib`

On Linux the extension is `.so`. On Windows the loader expects `.dll`.

## How `soma-next` loads these ports

At runtime, `soma-next`:

1. reads a full pack manifest
2. sees a declared port with some `port_id`
3. resolves a library named `libsoma_port_<port_id>.<ext>`
4. loads the exported `soma_port_init` symbol
5. keeps the shared library handle alive for the lifetime of the adapter

In concrete terms, a pack `port_id` of `postgres` means the runtime looks for:

- `libsoma_port_postgres.dylib` on macOS
- `libsoma_port_postgres.so` on Linux
- `libsoma_port_postgres.dll` on Windows

The search roots come from `[ports].plugin_path` in `soma-next`'s
`soma.toml`.

If `[ports].require_signatures = true`, the runtime also requires `.sig` and
`.pub` sidecar files for each port library.

## SDK contract

The SDK in [`sdk`](sdk) provides the runtime-facing contract:

- `Port`
- `PortSpec`
- `PortCapabilitySpec`
- `PortCallRecord`
- common enums such as `PortKind`, `RiskClass`, `SideEffectClass`,
  `RollbackSupport`, `TrustLevel`, and `PortFailureClass`

Every external port crate:

- depends on `soma-port-sdk`
- implements `Port`
- builds as `cdylib`
- exports `soma_port_init`

Minimal export shape:

```rust
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn soma_port_sdk::Port {
    Box::into_raw(Box::new(MyPort::new()))
}
```

## Compatibility rules

Treat `soma-next` and `soma-ports` as one deployable unit.

Do this:

- build the runtime and these port libraries from the same repository revision
- ship the matching set together
- point `plugin_path` at the exact build output you intend to load

Do not do this:

- mix runtime binaries and port libraries from unrelated revisions
- assume semver-level ABI stability across dynamic boundaries
- rename a library without also updating the pack `port_id`

When adding a new port, keep these identifiers aligned:

- the pack-facing `port_id`
- the shared library filename suffix
- the adapter's runtime-facing `PortSpec.port_id`
- any local metadata you keep in `manifest.json`

## About the local `manifest.json` files

Each port directory includes a `manifest.json` with descriptive metadata about
that port package.

Those files are useful as local inventory, but they are not, by themselves, the
full pack manifests that `soma-next` bootstraps with `--pack`. To expose one of
these ports in the runtime, you still need a full pack manifest that declares:

- the port spec the runtime should register
- the skills that call that port
- exposure, observability, and dependency metadata

## Environment variables used by ports

Only two ports currently read dedicated connection variables directly:

- `SOMA_POSTGRES_URL` for [`postgres`](postgres)
- `SOMA_REDIS_URL` for [`redis`](redis)

Everything else is configured through call inputs or library-default client
behavior.

## Adding a new port

Recommended checklist:

1. Create a new crate under `soma-ports/<name>`.
2. Set `[lib] crate-type = ["cdylib"]`.
3. Depend on [`sdk`](sdk).
4. Implement the `Port` trait and return complete `PortCallRecord`s.
5. Export `soma_port_init`.
6. Add tests for validation and at least one successful or expected-failure
   invocation path.
7. Add the crate to the workspace, or exclude it intentionally like `redis`.
8. Build the library and load it through a matching `soma-next` build.

## Source map

- [`Cargo.toml`](Cargo.toml): workspace membership
- [`sdk/src/lib.rs`](sdk/src/lib.rs): SDK types and trait
- [`auth/src/lib.rs`](auth/src/lib.rs): auth adapter
- [`crypto/src/lib.rs`](crypto/src/lib.rs): crypto adapter
- [`geo/src/lib.rs`](geo/src/lib.rs): geo adapter
- [`image/src/lib.rs`](image/src/lib.rs): image adapter
- [`postgres/src/lib.rs`](postgres/src/lib.rs): postgres adapter
- [`push/src/lib.rs`](push/src/lib.rs): push adapter
- [`redis/src/lib.rs`](redis/src/lib.rs): redis adapter
- [`s3/src/lib.rs`](s3/src/lib.rs): s3 adapter
- [`smtp/src/lib.rs`](smtp/src/lib.rs): smtp adapter
- [`timer/src/lib.rs`](timer/src/lib.rs): timer adapter
