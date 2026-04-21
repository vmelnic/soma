# soma-ports

External port workspace for `soma-next`. Each port crate compiles to a shared
library (`cdylib`) that implements the `soma-port-sdk` `Port` trait and exports
`soma_port_init`. These libraries are loaded by `soma-next` at runtime from
directories listed in `[ports].plugin_path`.

## Workspace layout

Run `cargo metadata --no-deps --format-version 1 | jq '.packages[].name'` for
the current member list. `redis` is excluded from the main workspace and carries
its own lockfile — build and test it separately.

Ports fall into three categories:

- **Service-backed** (postgres, mysql, mongodb, redis, elasticsearch, s3, smtp,
  slack, twilio, stripe, google-calendar, google-drive, google-mail, assemblyai,
  deepgram) — require credentials via `SOMA_*` env vars.
- **Local logic** (filesystem, http, image, timer, pdf, calendar, sqlite, crypto,
  auth, geo, push) — no external service dependency.
- **Shell-out** (youtube) — requires `yt-dlp` on PATH.

Each port directory includes a `manifest.json` with descriptive metadata. Those
are local inventory, not the full pack manifests that `soma-next` bootstraps
with — see the Packs section in `soma-next/README.md`.

## Build and test

```bash
cd soma-ports

cargo build --workspace                                # debug libs → target/debug/
cargo test --workspace --all-targets                   # run all tests
cargo clippy --workspace --all-targets --all-features  # must be zero warnings

# redis (separate workspace)
cargo build --manifest-path redis/Cargo.toml --all-targets
cargo test --manifest-path redis/Cargo.toml --all-targets

# single port
cargo build --release -p soma-port-postgres
```

### Deploying to project directories

After a release build, copy the relevant `.dylib`/`.so` files to each project's
pack directory. Each `soma-project-*/` has its own `packs/<port>/` layout — check
the project's manifest to see which ports it needs.

On macOS, fix quarantine after copying:

```bash
xattr -d com.apple.quarantine packs/<port>/libsoma_port_<port>.dylib
codesign -fs - packs/<port>/libsoma_port_<port>.dylib
```

## How `soma-next` loads these ports

1. Reads a full pack manifest (`--pack <manifest.json>`)
2. Sees a declared port with some `port_id`
3. Resolves `libsoma_port_<port_id>.{dylib,so,dll}` from `[ports].plugin_path`
4. Loads the exported `soma_port_init` symbol
5. Keeps the shared library handle alive for the adapter's lifetime

With `--pack auto`, step 1-2 are skipped — the runtime scans `SOMA_PORTS_PLUGIN_PATH`
directly and registers every library it finds.

If `[ports].require_signatures = true`, the runtime also requires `.sig` and
`.pub` sidecar files for each port library.

## SDK contract

The SDK in [`sdk`](sdk) provides the runtime-facing contract: `Port`,
`PortSpec`, `PortCapabilitySpec`, `PortCallRecord`, and enums for risk class,
side effects, rollback support, trust level, and failure class.

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

Treat `soma-next` and `soma-ports` as one deployable unit. Build from the same
repository revision. Do not mix runtime binaries and port libraries from
unrelated revisions — there is no semver-level ABI stability across the dynamic
boundary.

When adding a new port, keep these identifiers aligned: the pack-facing
`port_id`, the shared library filename suffix, the adapter's `PortSpec.port_id`,
and any `manifest.json` metadata.

## Adding a new port

1. Create `soma-ports/<name>/` with `[lib] crate-type = ["cdylib"]`
2. Depend on [`sdk`](sdk), implement `Port`, export `soma_port_init`
3. Return complete `PortCallRecord`s from every invocation
4. Add tests for validation and at least one invocation path
5. Add to the workspace (or exclude intentionally like `redis`)
6. Build and load through a matching `soma-next` build

## Environment variables

Each service-backed port reads credentials from `SOMA_*` env vars. Check the
port's source (`<port>/src/lib.rs`) or its `manifest.json` for the exact
variable names. Most ports also accept unprefixed fallbacks (e.g.
`POSTGRES_URL` alongside `SOMA_POSTGRES_URL`).

Google ports (calendar, drive, mail) share `SOMA_GOOGLE_ACCESS_TOKEN`.
