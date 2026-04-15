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
- [`calendar`](calendar): local iCalendar (.ics) file management
- [`crypto`](crypto): cryptographic primitives and token helpers
- [`elasticsearch`](elasticsearch): Elasticsearch search and indexing
- [`geo`](geo): geospatial math and geocoding stubs
- [`google-calendar`](google-calendar): Google Calendar event management
- [`google-drive`](google-drive): Google Drive file and folder management
- [`google-mail`](google-mail): Gmail send and read
- [`image`](image): image processing
- [`mongodb`](mongodb): MongoDB document database operations
- [`mysql`](mysql): MySQL database operations
- [`pdf`](pdf): PDF document generation
- [`postgres`](postgres): PostgreSQL access
- [`push`](push): push notifications
- [`redis`](redis): Redis access
- [`s3`](s3): S3-compatible object storage
- [`slack`](slack): Slack messaging
- [`smtp`](smtp): SMTP email delivery
- [`stripe`](stripe): Stripe payment processing
- [`timer`](timer): timer and scheduler primitives
- [`twilio`](twilio): Twilio SMS, WhatsApp, and voice calls

Workspace membership is defined in [`Cargo.toml`](Cargo.toml). `redis` is
intentionally excluded from the main workspace and carries its own lockfile, so
it must be built and tested separately.

## Port catalog

| Port | Crate | Output library | Capabilities | Notes |
| --- | --- | --- | ---: | --- |
| auth | `soma-port-auth` | `libsoma_port_auth.*` | 10 | OTP, sessions, TOTP, bearer tokens; uses in-memory stores |
| calendar | `soma-port-calendar` | `libsoma_port_calendar.*` | 4 | Local iCalendar (.ics) file management; uses `SOMA_CALENDAR_DIR` |
| crypto | `soma-port-crypto` | `libsoma_port_crypto.*` | 13 | Hashing, HMAC, bcrypt, AES-GCM, RSA, JWT, randomness |
| elasticsearch | `soma-port-elasticsearch` | `libsoma_port_elasticsearch.*` | 6 | Search, document CRUD, index management; uses `SOMA_ELASTICSEARCH_URL` |
| geo | `soma-port-geo` | `libsoma_port_geo.*` | 5 | Distance, radius filter, bounds check, geocode stubs |
| google-calendar | `soma-port-google-calendar` | `libsoma_port_google_calendar.*` | 4 | Google Calendar events: list, create, get, delete; uses `SOMA_GOOGLE_ACCESS_TOKEN` |
| google-drive | `soma-port-google-drive` | `libsoma_port_google_drive.*` | 5 | Google Drive: list, get, upload, delete files, create folders; uses `SOMA_GOOGLE_ACCESS_TOKEN` |
| google-mail | `soma-port-google-mail` | `libsoma_port_google_mail.*` | 4 | Gmail: send email, list/get messages, list labels; uses `SOMA_GOOGLE_ACCESS_TOKEN` |
| image | `soma-port-image` | `libsoma_port_image.*` | 5 | Thumbnail, resize, crop, format conversion, EXIF strip |
| mongodb | `soma-port-mongodb` | `libsoma_port_mongodb.*` | 7 | Find, insert, update, delete, count documents; uses `SOMA_MONGODB_URL` |
| mysql | `soma-port-mysql` | `libsoma_port_mysql.*` | 5 | Raw SQL, ORM-style CRUD; uses `SOMA_MYSQL_URL` |
| pdf | `soma-port-pdf` | `libsoma_port_pdf.*` | 3 | Create documents, add pages, text-to-PDF via `printpdf` |
| postgres | `soma-port-postgres` | `libsoma_port_postgres.*` | 15 | Raw SQL, CRUD, DDL, transactions; uses `SOMA_POSTGRES_URL` |
| push | `soma-port-push` | `libsoma_port_push.*` | 4 | FCM, WebPush, device registration; in-memory registry |
| redis | `soma-port-redis` | `libsoma_port_redis.*` | 13 | Strings, hashes, lists, pub/sub; uses `SOMA_REDIS_URL` |
| s3 | `soma-port-s3` | `libsoma_port_s3.*` | 5 | Put/get/delete/presign/list via AWS SDK |
| slack | `soma-port-slack` | `libsoma_port_slack.*` | 4 | Send messages, list channels, upload files, add reactions; uses `SOMA_SLACK_BOT_TOKEN` |
| smtp | `soma-port-smtp` | `libsoma_port_smtp.*` | 3 | Plain, HTML, attachment email via `lettre` |
| stripe | `soma-port-stripe` | `libsoma_port_stripe.*` | 5 | Charges, customers, payment intents, balance; uses `SOMA_STRIPE_SECRET_KEY` |
| timer | `soma-port-timer` | `libsoma_port_timer.*` | 4 | Timeouts, intervals, cancellation, listing; in-memory state |
| twilio | `soma-port-twilio` | `libsoma_port_twilio.*` | 4 | SMS, WhatsApp, voice calls; uses `SOMA_TWILIO_ACCOUNT_SID` |

Notes:

- `image`, `timer`, `pdf`, `calendar`, and most of `geo` are pure local logic
  with no service dependency.
- `postgres` and `mysql` create a fresh connection per call.
- `mongodb` uses a lazily-initialized sync client.
- `redis`, `s3`, and `smtp` bridge async clients into the synchronous `Port`
  trait with internal Tokio runtimes.
- `stripe`, `twilio`, `slack`, `elasticsearch`, `google-calendar`,
  `google-drive`, and `google-mail` use `reqwest::blocking::Client` for HTTP.
- `google-calendar`, `google-drive`, and `google-mail` share the same OAuth2
  token env var (`SOMA_GOOGLE_ACCESS_TOKEN`).
- `auth`, `push`, and `timer` keep volatile in-memory state and are not durable
  across process restarts.

## Build, test, and lint

### Development build

```bash
cd soma-ports

cargo build --workspace                                # debug libs → target/debug/
cargo test --workspace --all-targets                   # run all tests
cargo clippy --workspace --all-targets --all-features  # must be zero warnings
```

Build `redis` separately (excluded from the workspace):

```bash
cargo build --manifest-path redis/Cargo.toml --all-targets
cargo test --manifest-path redis/Cargo.toml --all-targets
cargo clippy --manifest-path redis/Cargo.toml --all-targets --all-features
```

Debug libraries land under `target/debug/`:

- `target/debug/libsoma_port_postgres.dylib`
- `target/debug/libsoma_port_auth.dylib`

On Linux the extension is `.so`. On Windows the loader expects `.dll`.

### Production release

```bash
cd soma-ports

cargo build --workspace --release                      # optimized libs → target/release/
cargo build --release --manifest-path redis/Cargo.toml # redis separately
```

Release library sizes (macOS arm64):

| Port | Release size |
| --- | ---: |
| auth | 548 KB |
| crypto | 1.4 MB |
| geo | 496 KB |
| image | 6.2 MB |
| postgres | 2.0 MB |
| push | 3.4 MB |
| redis | 1.7 MB |
| s3 | 17 MB |
| smtp | 3.4 MB |
| timer | 458 KB |

Redis builds to its own target directory: `redis/target/release/libsoma_port_redis.dylib`.

### Deploying to project directories

After a release build, copy the relevant `.dylib` files to each project's pack
directory:

```bash
# Postgres port → soma-project-postgres, soma-project-llm, soma-helperbook
cp target/release/libsoma_port_postgres.dylib ../soma-project-postgres/packs/postgres/
cp target/release/libsoma_port_postgres.dylib ../soma-project-llm/packs/postgres/
cp target/release/libsoma_port_postgres.dylib ../soma-helperbook/packs/postgres/

# Redis port → soma-helperbook
cp redis/target/release/libsoma_port_redis.dylib ../soma-helperbook/packs/redis/

# SMTP port → soma-project-smtp
cp target/release/libsoma_port_smtp.dylib ../soma-project-smtp/packs/smtp/

# S3 port → soma-project-s3
cp target/release/libsoma_port_s3.dylib ../soma-project-s3/packs/s3/

# Auth port → soma-helperbook
cp target/release/libsoma_port_auth.dylib ../soma-helperbook/packs/auth/
```

### macOS post-copy

Copied libraries on macOS may be quarantined by Gatekeeper. Fix per library:

```bash
xattr -d com.apple.quarantine packs/postgres/libsoma_port_postgres.dylib
codesign -fs - packs/postgres/libsoma_port_postgres.dylib
```

### Building a single port

To rebuild one port without recompiling the whole workspace:

```bash
cargo build --release -p soma-port-postgres   # just the postgres port
cargo test -p soma-port-postgres              # just its tests
```

### Useful cargo commands

```bash
# Check all without building (fast feedback).
cargo check --workspace --all-targets

# Run tests for one port with output.
cargo test -p soma-port-crypto -- --nocapture

# List all workspace members.
cargo metadata --no-deps --format-version 1 | jq '.packages[].name'
```

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

Several ports read configuration from environment variables:

- `SOMA_POSTGRES_URL` for [`postgres`](postgres)
- `SOMA_REDIS_URL` for [`redis`](redis)
- [`s3`](s3): `SOMA_S3_DEFAULT_BUCKET` (default: `soma-uploads`),
  `SOMA_S3_REGION` (falls back to `AWS_REGION`, `AWS_DEFAULT_REGION`),
  `SOMA_S3_ENDPOINT` (falls back to `AWS_ENDPOINT_URL_S3`)
- [`smtp`](smtp): `SOMA_SMTP_HOST` / `SMTP_HOST`,
  `SOMA_SMTP_FROM` / `SMTP_FROM`,
  `SOMA_SMTP_PORT` / `SMTP_PORT` (default: 587),
  `SOMA_SMTP_USERNAME` / `SMTP_USERNAME`,
  `SOMA_SMTP_PASSWORD` / `SMTP_PASSWORD`,
  `SOMA_SMTP_STARTTLS` / `SMTP_STARTTLS` (default: true)

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
