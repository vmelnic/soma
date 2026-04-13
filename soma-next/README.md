# soma-next

`soma-next` is the current SOMA runtime. It is both a Rust library and a `soma`
binary that:

- bootstraps a runtime from JSON pack manifests or auto-discovers ports (`--pack auto`)
- runs goal-driven control sessions
- executes skills through built-in and dynamically loaded ports
- persists episodes, schemas, routines, and session checkpoints
- exposes CLI and MCP interfaces
- supports peer transport over TCP, TLS, WebSocket, and Unix sockets

## What the runtime contains

At a high level, `soma-next` is organized into six layers:

- Runtime logic in [`src/runtime`](src/runtime): goal parsing, belief/resource
  handling, session control, selection, prediction, criticism, policy, trace,
  metrics, dynamic port loading, and pack registration.
- Adapter layer in [`src/adapters.rs`](src/adapters.rs): wiring between the
  runtime traits and the concrete stores/executors used by bootstrap.
- Memory stores in [`src/memory`](src/memory): in-memory and disk-backed
  episodes, schemas, routines, world state, and session checkpoints.
- Interfaces in [`src/interfaces`](src/interfaces): CLI, MCP, and supporting
  request/response glue.
- Distributed transport in [`src/distributed`](src/distributed): peer
  addressing, remote execution, routing, streaming, auth, queueing, and rate
  limiting.
- Ports in [`src/ports`](src/ports): built-in `filesystem` and `http` ports.

Bootstrap happens in [`src/bootstrap.rs`](src/bootstrap.rs). The assembled
runtime includes a session controller, goal runtime, skill runtime, port
runtime, stores, loaded pack specs, and shared metrics.

## Built-in capabilities

Two ports are built into the binary:

- `filesystem`: `readdir`, `readfile`, `writefile`, `stat`, `mkdir`, `rmdir`,
  `rm`
- `http`: `get`, `post`, `put`, `delete`

Everything else is expected to come from pack manifests, external shared
libraries, or auto-discovery (`--pack auto`).

When the default reference pack is loaded, the runtime also exposes these local
primitive skills:

- `soma.ports.reference.readdir`
- `soma.ports.reference.readfile`
- `soma.ports.reference.writefile`
- `soma.ports.reference.stat`
- `soma.ports.reference.mkdir`
- `soma.ports.reference.rmdir`
- `soma.ports.reference.rm`

## Build and test

The crate exposes a library target from [`src/lib.rs`](src/lib.rs) and a binary
target named `soma` from [`src/main.rs`](src/main.rs).

### Development build

```bash
cd soma-next

cargo build                              # debug binary → target/debug/soma (~32 MB)
cargo test                               # 1198+ tests, all must pass
cargo clippy --all-targets --all-features # must be zero warnings
```

Debug builds include full debug info and are ~3x larger than release. Use them
for day-to-day development — they compile faster and produce better backtraces.

### Production release

```bash
cd soma-next

cargo build --release                    # optimized binary → target/release/soma (~11 MB)
cargo test --release                     # run tests against the release binary
```

The release binary is what gets deployed to project directories:

```bash
cp target/release/soma ../soma-project-postgres/bin/soma
cp target/release/soma ../soma-helperbook/bin/soma
cp target/release/soma ../soma-project-llm/bin/soma
```

### macOS post-copy

Copied binaries on macOS may be quarantined by Gatekeeper. Fix with:

```bash
xattr -d com.apple.quarantine bin/soma
codesign -fs - bin/soma
```

### Useful cargo commands

```bash
# Run tests matching a pattern.
cargo test session              # all tests with "session" in the name

# Run a single test with output.
cargo test --release test_name -- --nocapture

# Check without building (fast feedback).
cargo check --all-targets

# Build and show warnings without stopping on first error.
cargo build 2>&1 | head -50
```

## Running the binary

The binary accepts `--pack <manifest.json>` zero or more times, or `--pack auto`
to skip manifests entirely and auto-discover all port libraries from the search
path. If no pack is provided, startup falls back to
[`packs/reference/manifest.json`](packs/reference/manifest.json) when that file
exists. Otherwise it starts with an empty runtime.

### Auto-discovery mode

`--pack auto` scans every directory in `SOMA_PORTS_PLUGIN_PATH` for files
matching `libsoma_port_*.{dylib,so,dll}`, loads each one, and registers the port
using the library's own `spec()`. No manifest is needed. Built-in ports
(filesystem, http) are always included.

```bash
SOMA_PORTS_PLUGIN_PATH=../soma-ports/target/release cargo run --release -- --mcp --pack auto
```

This is the fastest path for the LLM-driven mode: the LLM calls `list_ports` to
discover capabilities and `invoke_port` to use them. No skills, schemas,
routines, or policies are registered — the LLM drives port invocations directly.

### Common commands

```bash
# List loaded packs
cargo run -- packs

# List registered skills
cargo run -- skills

# Run a goal that maps cleanly to the bundled filesystem skills
cargo run -- run "list files in /tmp"

# Equivalent goal form
cargo run -- --goal "list files in /tmp"

# Dump selected runtime sections
cargo run -- dump --ports --skills --packs

# Show metrics
cargo run -- metrics --format json

# Restore a checkpointed session
cargo run -- restore <session_id>
```

`run` is heuristic goal routing, not a stable direct skill invocation surface.
For bundled filesystem skills, automatic `path` binding currently only happens
when the goal text contains an explicit absolute path such as `/tmp`. A goal
like `read the current directory` is recognized as filesystem-related, but it
stops in `WaitingForInput` because no `path` binding is inferred.

Supported CLI commands:

- `run <goal text>`
- `inspect <session_id>`
- `restore <session_id>`
- `sessions`
- `packs`
- `skills`
- `metrics [--format text|json|prometheus]`
- `verify-port <path>`
- `dump [--full|--belief|--episodes|--schemas|--routines|--sessions|--skills|--ports|--packs|--metrics]`
- `repl`

`verify-port` checks Ed25519 sidecar signatures for a port library. It does not
prove that the library can be loaded and invoked successfully by the runtime.

## MCP mode

MCP mode runs JSON-RPC 2.0 over stdin/stdout:

```bash
# With a manifest
cargo run -- --mcp --pack packs/reference/manifest.json

# With auto-discovery (no manifest)
SOMA_PORTS_PLUGIN_PATH=../soma-ports/target/release cargo run -- --mcp --pack auto
```

The server exposes 24 runtime tools:

**16 core tools:**

- `create_goal`, `inspect_session`, `inspect_belief`, `inspect_resources`
- `inspect_packs`, `inspect_skills`, `inspect_trace`
- `pause_session`, `resume_session`, `abort_session`, `list_sessions`
- `query_metrics`, `query_policy`, `dump_state`
- `invoke_port`, `list_ports`

**3 scheduler tools:**

- `schedule` — one-shot (`delay_ms`), recurring (`interval_ms`), message-only or port-call, optional `max_fires` and `brain` routing
- `list_schedules` — list active schedules
- `cancel_schedule` — cancel by UUID

**3 distributed peer tools:**

- `list_peers`, `invoke_remote_skill`, `transfer_routine`

**3 memory and learning tools:**

- `execute_routine` — run a compiled routine by ID with pre-loaded plan
- `trigger_consolidation` — manually trigger the episode → schema → routine pipeline

Implementation lives in [`src/interfaces/mcp.rs`](src/interfaces/mcp.rs).

## Peer and listener modes

The runtime can listen for and connect to peers:

```bash
# TCP listener
cargo run -- --listen 127.0.0.1:9100 repl

# WebSocket listener
cargo run -- --ws-listen 127.0.0.1:9200 repl

# Register a remote TCP peer
cargo run -- --peer 127.0.0.1:9101 packs

# Unix socket listener (Unix only)
cargo run -- --unix-listen /tmp/soma.sock repl
```

Relevant flags:

- `--listen <addr>`
- `--ws-listen <addr>`
- `--peer <addr>`
- `--unix-listen <path>`
- `--unix-peer <path>`

If `tls_cert` and `tls_key` are configured, outbound peer connections and TCP
listeners use TLS. Rate limiting and blacklist behavior come from the
`[distributed]` config section.

## Configuration

Configuration is loaded from `soma.toml`. The effective precedence is:

1. compiled defaults
2. `soma.toml`
3. `SOMA_*` environment overrides
4. CLI flags for mode and pack/listener selection

Use [`soma.toml.example`](soma.toml.example) as a starting point. The fields
that matter most in the current entrypoint are:

```toml
[soma]
log_level = "info"
data_dir = "~/.soma/data"

[runtime]
max_steps = 100
default_risk_budget = 0.5
default_latency_budget_ms = 30000
default_resource_budget = 100.0

[ports]
plugin_path = ["../soma-ports/target/debug"]
require_signatures = false

[distributed]
bind = "0.0.0.0:9100"
rate_limit_rps = 100
burst_limit = 20
blacklist_threshold = 50
rate_limit_enabled = true
blacklist_enabled = true
# tls_cert = "/path/to/cert.pem"
# tls_key = "/path/to/key.pem"
# tls_ca = "/path/to/ca.pem"
```

Operational notes:

- Persistent episodes, schemas, and routines are stored under `data_dir`.
- Session checkpoints are stored under `data_dir/sessions`.
- `plugin_path` is the search path for dynamically loaded external ports.
- `require_signatures = true` rejects unsigned port libraries.

## Packs and external ports

`soma-next` expects full pack manifests in the shape modeled by
[`src/types/pack.rs`](src/types/pack.rs). A pack can register:

- ports
- skills
- schemas
- routines
- policies
- exposure metadata
- dependency metadata

When bootstrap encounters a non-built-in port kind, it asks the dynamic loader
to resolve a shared library from `[ports].plugin_path` using this naming rule:

- macOS/Linux: `libsoma_port_<port_id>.<dylib|so>`
- Windows: `libsoma_port_<port_id>.dll`

The library must export `soma_port_init`.

Important compatibility rule: treat `soma-next` and external ports as a matched
build set. The loader and the external port SDK share mirrored Rust types and a
trait-object based boundary. In practice that means:

- build `soma-next` and `soma-ports` from the same repository revision
- do not assume cross-version ABI compatibility
- do not mix independently versioned runtime binaries and port libraries

## Using the library

If you embed the runtime instead of shelling out to the binary, bootstrap from
code:

```rust
use std::path::Path;

use soma_next::bootstrap;
use soma_next::config::SomaConfig;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = SomaConfig::load(Path::new("soma.toml"))?;
    let packs = vec!["packs/reference/manifest.json".to_string()];
    let runtime = bootstrap::bootstrap(&config, &packs)?;
    let _ = runtime.self_model();
    Ok(())
}
```

## Source map

- [`src/main.rs`](src/main.rs): CLI entrypoint, MCP mode, listener startup
- [`src/bootstrap.rs`](src/bootstrap.rs): runtime assembly from config and pack manifests
- [`src/runtime`](src/runtime): runtime subsystems
- [`src/interfaces`](src/interfaces): CLI and MCP surface
- [`src/distributed`](src/distributed): peer transport and remote execution
- [`src/memory`](src/memory): persistent and in-memory stores
- [`src/ports`](src/ports): built-in ports
- [`packs/reference/manifest.json`](packs/reference/manifest.json): reference pack
- [`soma.toml.example`](soma.toml.example): example config
