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
- [`assemblyai`](assemblyai): AssemblyAI transcription and audio intelligence
- [`auth`](auth): authentication flows
- [`calendar`](calendar): local iCalendar (.ics) file management
- [`crypto`](crypto): cryptographic primitives and token helpers
- [`deepgram`](deepgram): Deepgram speech-to-text and text analysis
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
- [`sqlite`](sqlite): SQLite local database operations (bundled, no external dependency)
- [`stripe`](stripe): Stripe payment processing
- [`timer`](timer): timer and scheduler primitives
- [`twilio`](twilio): Twilio SMS, WhatsApp, and voice calls
- [`youtube`](youtube): YouTube video/audio download and metadata

Workspace membership is defined in [`Cargo.toml`](Cargo.toml). `redis` is
intentionally excluded from the main workspace and carries its own lockfile, so
it must be built and tested separately.

## Port catalog

| Port | Crate | Output library | Notes |
| --- | --- | --- | --- |
| assemblyai | `soma-port-assemblyai` | `libsoma_port_assemblyai.*` | Transcription, paragraphs, sentences, word search, audio intelligence; uses `SOMA_ASSEMBLYAI_API_KEY` |
| auth | `soma-port-auth` | `libsoma_port_auth.*` | OTP, sessions, TOTP, bearer tokens; uses in-memory stores |
| calendar | `soma-port-calendar` | `libsoma_port_calendar.*` | Local iCalendar (.ics) file management; uses `SOMA_CALENDAR_DIR` |
| crypto | `soma-port-crypto` | `libsoma_port_crypto.*` | Hashing, HMAC, bcrypt, AES-GCM, RSA, JWT, randomness |
| deepgram | `soma-port-deepgram` | `libsoma_port_deepgram.*` | Speech-to-text, text analysis, text-to-speech; uses `SOMA_DEEPGRAM_API_KEY` |
| elasticsearch | `soma-port-elasticsearch` | `libsoma_port_elasticsearch.*` | Search, document CRUD, index management; uses `SOMA_ELASTICSEARCH_URL` |
| geo | `soma-port-geo` | `libsoma_port_geo.*` | Distance, radius filter, bounds check, geocode stubs |
| google-calendar | `soma-port-google-calendar` | `libsoma_port_google_calendar.*` | Google Calendar events: list, create, get, delete; uses `SOMA_GOOGLE_ACCESS_TOKEN` |
| google-drive | `soma-port-google-drive` | `libsoma_port_google_drive.*` | Google Drive: list, get, upload, delete files, create folders; uses `SOMA_GOOGLE_ACCESS_TOKEN` |
| google-mail | `soma-port-google-mail` | `libsoma_port_google_mail.*` | Gmail: send email, list/get messages, list labels; uses `SOMA_GOOGLE_ACCESS_TOKEN` |
| image | `soma-port-image` | `libsoma_port_image.*` | Thumbnail, resize, crop, format conversion, EXIF strip |
| mongodb | `soma-port-mongodb` | `libsoma_port_mongodb.*` | Find, insert, update, delete, count documents; uses `SOMA_MONGODB_URL` |
| mysql | `soma-port-mysql` | `libsoma_port_mysql.*` | Raw SQL, ORM-style CRUD, DDL, transactions; uses `SOMA_MYSQL_URL` |
| pdf | `soma-port-pdf` | `libsoma_port_pdf.*` | Create documents, add pages, text-to-PDF via `printpdf` |
| postgres | `soma-port-postgres` | `libsoma_port_postgres.*` | Raw SQL, CRUD, DDL, transactions; uses `SOMA_POSTGRES_URL` |
| push | `soma-port-push` | `libsoma_port_push.*` | FCM, WebPush, device registration; in-memory registry |
| redis | `soma-port-redis` | `libsoma_port_redis.*` | Strings, hashes, lists, pub/sub; uses `SOMA_REDIS_URL` |
| s3 | `soma-port-s3` | `libsoma_port_s3.*` | Put/get/delete/presign/list via AWS SDK |
| slack | `soma-port-slack` | `libsoma_port_slack.*` | Send messages, list channels, upload files, add reactions; uses `SOMA_SLACK_BOT_TOKEN` |
| smtp | `soma-port-smtp` | `libsoma_port_smtp.*` | Plain, HTML, attachment email via `lettre` |
| sqlite | `soma-port-sqlite` | `libsoma_port_sqlite.*` | Raw SQL, CRUD, DDL, transactions; bundled `rusqlite`, no external dependency |
| stripe | `soma-port-stripe` | `libsoma_port_stripe.*` | Charges, customers, payment intents, balance; uses `SOMA_STRIPE_SECRET_KEY` |
| timer | `soma-port-timer` | `libsoma_port_timer.*` | Timeouts, intervals, cancellation, listing; in-memory state |
| twilio | `soma-port-twilio` | `libsoma_port_twilio.*` | SMS, WhatsApp, voice calls; uses `SOMA_TWILIO_ACCOUNT_SID` |
| youtube | `soma-port-youtube` | `libsoma_port_youtube.*` | Video/audio download, format listing, metadata; uses `yt-dlp` |

Notes:

- `image`, `timer`, `pdf`, `calendar`, `sqlite`, and most of `geo` are pure
  local logic with no service dependency.
- `postgres`, `mysql`, and `sqlite` create a fresh connection per call.
- `mongodb` uses a lazily-initialized sync client.
- `redis`, `s3`, and `smtp` bridge async clients into the synchronous `Port`
  trait with internal Tokio runtimes.
- `assemblyai`, `deepgram`, `stripe`, `twilio`, `slack`, `elasticsearch`,
  `google-calendar`, `google-drive`, and `google-mail` use
  `reqwest::blocking::Client` for HTTP.
- `google-calendar`, `google-drive`, and `google-mail` share the same OAuth2
  token env var (`SOMA_GOOGLE_ACCESS_TOKEN`).
- `auth`, `push`, and `timer` keep volatile in-memory state and are not durable
  across process restarts.
- `youtube` shells out to `yt-dlp` for media extraction.

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

Redis builds to its own target directory: `redis/target/release/libsoma_port_redis.dylib`.
Use `ls -la target/release/libsoma_port_*` for current library sizes.

### Deploying to project directories

After a release build, copy the relevant `.dylib` files to each project's pack
directory:

```bash
# Postgres port → soma-project-postgres, soma-project-llm, soma-project-helperbook
cp target/release/libsoma_port_postgres.dylib ../soma-project-postgres/packs/postgres/
cp target/release/libsoma_port_postgres.dylib ../soma-project-llm/packs/postgres/
cp target/release/libsoma_port_postgres.dylib ../soma-project-helperbook/packs/postgres/

# Redis port → soma-project-helperbook
cp redis/target/release/libsoma_port_redis.dylib ../soma-project-helperbook/packs/redis/

# SMTP port → soma-project-smtp
cp target/release/libsoma_port_smtp.dylib ../soma-project-smtp/packs/smtp/

# S3 port → soma-project-s3
cp target/release/libsoma_port_s3.dylib ../soma-project-s3/packs/s3/

# Auth port → soma-project-helperbook
cp target/release/libsoma_port_auth.dylib ../soma-project-helperbook/packs/auth/
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
- `SOMA_MYSQL_URL` / `MYSQL_URL` for [`mysql`](mysql)
- `SOMA_MONGODB_URL` / `MONGODB_URL` for [`mongodb`](mongodb);
  `SOMA_MONGODB_DATABASE` for database name (default: `soma`)
- `SOMA_ELASTICSEARCH_URL` / `ELASTICSEARCH_URL` for
  [`elasticsearch`](elasticsearch)
- [`s3`](s3): `SOMA_S3_DEFAULT_BUCKET` (default: `soma-uploads`),
  `SOMA_S3_REGION` (falls back to `AWS_REGION`, `AWS_DEFAULT_REGION`),
  `SOMA_S3_ENDPOINT` (falls back to `AWS_ENDPOINT_URL_S3`)
- [`smtp`](smtp): `SOMA_SMTP_HOST` / `SMTP_HOST`,
  `SOMA_SMTP_FROM` / `SMTP_FROM`,
  `SOMA_SMTP_PORT` / `SMTP_PORT` (default: 587),
  `SOMA_SMTP_USERNAME` / `SMTP_USERNAME`,
  `SOMA_SMTP_PASSWORD` / `SMTP_PASSWORD`,
  `SOMA_SMTP_STARTTLS` / `SMTP_STARTTLS` (default: true)
- `SOMA_STRIPE_SECRET_KEY` / `STRIPE_SECRET_KEY` for [`stripe`](stripe)
- [`twilio`](twilio): `SOMA_TWILIO_ACCOUNT_SID` / `TWILIO_ACCOUNT_SID`,
  `SOMA_TWILIO_AUTH_TOKEN` / `TWILIO_AUTH_TOKEN`,
  `SOMA_TWILIO_FROM_NUMBER` / `TWILIO_FROM_NUMBER`
- `SOMA_SLACK_BOT_TOKEN` / `SLACK_BOT_TOKEN` for [`slack`](slack)
- `SOMA_GOOGLE_ACCESS_TOKEN` / `GOOGLE_ACCESS_TOKEN` for
  [`google-calendar`](google-calendar), [`google-drive`](google-drive),
  [`google-mail`](google-mail)
- `SOMA_CALENDAR_DIR` for [`calendar`](calendar) (default:
  `~/.soma/calendars/`)

- `SOMA_ASSEMBLYAI_API_KEY` / `ASSEMBLYAI_API_KEY` for [`assemblyai`](assemblyai)
- `SOMA_DEEPGRAM_API_KEY` / `DEEPGRAM_API_KEY` for [`deepgram`](deepgram)

`pdf`, `image`, `timer`, `sqlite`, `youtube`, and most of `geo` require no
environment variables (`youtube` shells out to `yt-dlp` which must be on PATH).

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
- [`assemblyai/src/lib.rs`](assemblyai/src/lib.rs): assemblyai adapter
- [`auth/src/lib.rs`](auth/src/lib.rs): auth adapter
- [`calendar/src/lib.rs`](calendar/src/lib.rs): calendar adapter
- [`crypto/src/lib.rs`](crypto/src/lib.rs): crypto adapter
- [`deepgram/src/lib.rs`](deepgram/src/lib.rs): deepgram adapter
- [`elasticsearch/src/lib.rs`](elasticsearch/src/lib.rs): elasticsearch adapter
- [`geo/src/lib.rs`](geo/src/lib.rs): geo adapter
- [`google-calendar/src/lib.rs`](google-calendar/src/lib.rs): google-calendar adapter
- [`google-drive/src/lib.rs`](google-drive/src/lib.rs): google-drive adapter
- [`google-mail/src/lib.rs`](google-mail/src/lib.rs): google-mail adapter
- [`image/src/lib.rs`](image/src/lib.rs): image adapter
- [`mongodb/src/lib.rs`](mongodb/src/lib.rs): mongodb adapter
- [`mysql/src/lib.rs`](mysql/src/lib.rs): mysql adapter
- [`pdf/src/lib.rs`](pdf/src/lib.rs): pdf adapter
- [`postgres/src/lib.rs`](postgres/src/lib.rs): postgres adapter
- [`push/src/lib.rs`](push/src/lib.rs): push adapter
- [`redis/src/lib.rs`](redis/src/lib.rs): redis adapter
- [`s3/src/lib.rs`](s3/src/lib.rs): s3 adapter
- [`slack/src/lib.rs`](slack/src/lib.rs): slack adapter
- [`smtp/src/lib.rs`](smtp/src/lib.rs): smtp adapter
- [`sqlite/src/lib.rs`](sqlite/src/lib.rs): sqlite adapter
- [`stripe/src/lib.rs`](stripe/src/lib.rs): stripe adapter
- [`timer/src/lib.rs`](timer/src/lib.rs): timer adapter
- [`twilio/src/lib.rs`](twilio/src/lib.rs): twilio adapter
- [`youtube/src/lib.rs`](youtube/src/lib.rs): youtube adapter
