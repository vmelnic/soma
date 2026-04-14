# Building Projects

A **soma-project** is a self-contained directory that wires a soma-next binary to one or more port `.dylib` libraries via pack manifests. The MCP server exposes `invoke_port` and `list_ports` tools for LLMs and test clients.

## Quick Alternative: `--pack auto`

If you want to skip manifest writing entirely, use `--pack auto`. The runtime scans `SOMA_PORTS_PLUGIN_PATH` for port dylibs, loads every one it finds, and exposes all their capabilities through MCP — no manifest file needed:

```bash
SOMA_PORTS_PLUGIN_PATH=soma-ports/target/release soma-next/target/release/soma --mcp --pack auto
```

This is how `soma-project-terminal` runs in production: zero manifests, all ports auto-discovered. It works well for the LLM-driven path where the brain (LLM) decides which capabilities to invoke. For the autonomous path (goal-driven skill selection), you still need manifests because the runtime's selector needs skill metadata (input schemas, effect classes, preconditions) to make informed choices.

## Project Structure

```
soma-project-<name>/
  bin/soma                              # Runtime binary
  packs/<port>/manifest.json            # Pack manifest (PackSpec)
  packs/<port>/libsoma_port_<port>.dylib  # Port library
  .env                                  # Port config + runtime wiring
  .gitignore                            # Excludes bin/ and dylibs
  mcp-client.mjs                        # Node.js MCP test client
  scripts/run-mcp.sh                    # Launch SOMA in MCP mode
  scripts/list-skills.sh                # Verify port loaded
  scripts/test-all.sh                   # Smoke test
  samples/                              # Sample payloads
  README.md
```

## Step by Step

### 1. Create the directory

```bash
mkdir -p soma-project-foo/{bin,packs/foo,scripts,samples}
```

### 2. Copy the binary

```bash
cp soma-next/target/release/soma soma-project-foo/bin/soma
```

### 3. Copy the port library

```bash
cp soma-ports/target/release/libsoma_port_foo.dylib soma-project-foo/packs/foo/
```

### 4. Write the pack manifest

Create `packs/foo/manifest.json`. The manifest follows the PackSpec format. Minimal required fields:

```json
{
  "id": "soma.ports.foo",
  "name": "Foo",
  "version": "0.1.0",
  "runtime_compatibility": ">=0.1.0",
  "namespace": "soma.ports.foo",
  "description": "What this port does",
  "capabilities": [
    {
      "group_name": "ops",
      "scope": "local",
      "capabilities": ["do_thing"]
    }
  ],
  "dependencies": [],
  "resources": [],
  "schemas": [],
  "routines": [],
  "policies": [],
  "exposure": {
    "local_skills": ["soma.ports.foo.do_thing"],
    "remote_skills": [],
    "local_resources": [],
    "remote_resources": [],
    "default_deny_destructive": true
  },
  "observability": {
    "health_checks": ["foo_reachable"],
    "version_metadata": {"version": "0.1.0"},
    "dependency_status": [],
    "capability_inventory": ["do_thing"],
    "expected_latency_classes": ["medium"],
    "expected_failure_modes": ["validation_error", "external_error"],
    "trace_categories": ["foo"],
    "metric_names": ["foo_request_count"],
    "pack_load_state": "active"
  },
  "authors": [],
  "tags": ["foo"],
  "ports": [
    {
      "port_id": "foo",
      "name": "foo",
      "version": "0.1.0",
      "kind": "service",
      "description": "What this port does",
      "namespace": "soma.ports.foo",
      "trust_level": "verified",
      "capabilities": [
        {
          "capability_id": "do_thing",
          "name": "do_thing",
          "purpose": "Describe the capability",
          "input_schema": {"schema": {"type": "object", "required": ["x"], "properties": {"x": {"type": "string"}}}},
          "output_schema": {"schema": {"type": "object", "properties": {"ok": {"type": "boolean"}}}},
          "effect_class": "pure_computation",
          "rollback_support": "not_applicable",
          "determinism_class": "deterministic",
          "idempotence_class": "idempotent",
          "risk_class": "low",
          "latency_profile": {"expected_latency_ms": 50, "p95_latency_ms": 200, "max_latency_ms": 1000},
          "cost_profile": {"cpu_cost_class": "negligible", "memory_cost_class": "negligible", "io_cost_class": "negligible", "network_cost_class": "negligible", "energy_cost_class": "negligible"},
          "remote_exposable": false
        }
      ],
      "input_schema": {"schema": {"type": "object"}},
      "output_schema": {"schema": {"type": "object"}},
      "failure_modes": ["validation_error", "external_error"],
      "side_effect_class": "pure_computation",
      "latency_profile": {"expected_latency_ms": 50, "p95_latency_ms": 200, "max_latency_ms": 1000},
      "cost_profile": {"cpu_cost_class": "negligible", "memory_cost_class": "negligible", "io_cost_class": "negligible", "network_cost_class": "negligible", "energy_cost_class": "negligible"},
      "auth_requirements": {"methods": [], "required": false},
      "sandbox_requirements": {"filesystem_access": false, "network_access": false, "device_access": false, "process_access": false},
      "observable_fields": [],
      "validation_rules": [],
      "remote_exposure": false
    }
  ],
  "skills": [],
  "port_dependencies": []
}
```

### 5. Write .env

```bash
# Port-specific configuration.
SOMA_FOO_SETTING=value

# Runtime wiring.
SOMA_PORTS_PLUGIN_PATH=./packs/foo
SOMA_PORTS_REQUIRE_SIGNATURES=false
```

Port-specific env var conventions from existing projects:

| Project | Variables |
|---------|-----------|
| smtp | `SOMA_SMTP_HOST`, `SOMA_SMTP_PORT`, `SOMA_SMTP_USERNAME`, `SOMA_SMTP_PASSWORD`, `SOMA_SMTP_FROM` |
| s3 | `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `SOMA_S3_REGION`, `SOMA_S3_ENDPOINT`, `SOMA_S3_DEFAULT_BUCKET` |
| postgres | `SOMA_POSTGRES_URL` |

### 6. Write scripts

**scripts/run-mcp.sh** -- launches SOMA in MCP mode with the pack:

```bash
#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [ ! -x "$PROJECT_ROOT/bin/soma" ]; then
  printf 'Missing %s\n' "$PROJECT_ROOT/bin/soma" >&2
  exit 1
fi

if [ -f "$PROJECT_ROOT/.env" ]; then
  set -a
  . "$PROJECT_ROOT/.env"
  set +a
fi

export SOMA_PORTS_PLUGIN_PATH="$PROJECT_ROOT/packs/foo"
export SOMA_PORTS_REQUIRE_SIGNATURES="${SOMA_PORTS_REQUIRE_SIGNATURES:-false}"

exec "$PROJECT_ROOT/bin/soma" --mcp --pack "$PROJECT_ROOT/packs/foo/manifest.json"
```

**scripts/list-skills.sh** and **scripts/test-all.sh** delegate to the MCP client:

```bash
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
node "$SCRIPT_DIR/../mcp-client.mjs" skills   # list-skills.sh
node "$SCRIPT_DIR/../mcp-client.mjs" smoke     # test-all.sh
```

### 7. Write .gitignore

```
bin/
packs/*/*.dylib
packs/*/*.so
```

### 8. Write mcp-client.mjs

The MCP client spawns `scripts/run-mcp.sh` as a child process, communicates over stdio using JSON-RPC 2.0, and sends `invoke_port` / `list_ports` tool calls. Copy an existing project's `mcp-client.mjs` and adapt the command map and payloads for your port's capabilities.

## Manifest Rules

- `port_id` must match the library name: `libsoma_port_{port_id}.dylib`.
- `namespace` follows the pattern `soma.ports.{port_id}`.
- `observable_fields` should be `[]` or contain only fields from the output schema.
- For ports that return non-object values, use `{"schema": {"description": "any"}}` as the `output_schema`.
- `skills` array can be empty for `invoke_port`-only usage. Add skills when you want the port's capabilities to appear in `list_ports` with full metadata.
- Each capability needs `input_schema` and `output_schema` with a `schema` wrapper object.
- `effect_class` options: `pure_computation`, `local_state_mutation`, `external_state_mutation`.

## Testing

```bash
./scripts/list-skills.sh     # Verify the port loaded and skills registered
./scripts/test-all.sh        # Run smoke tests via mcp-client.mjs
```

The MCP client pattern: spawn `run-mcp.sh`, send a JSON-RPC `initialize` request, then call `tools/call` with `invoke_port` or `list_ports`, check results, and exit.

## Existing Projects

### Server-side (dylib-based)

| Project | Port | What it does |
|---------|------|-------------|
| `soma-project-smtp` | smtp | Email delivery via mailcatcher or real SMTP |
| `soma-project-s3` | s3 | AWS S3 object storage |
| `soma-project-postgres` | postgres | Database queries against HelperBook schema |
| `soma-project-llm` | postgres | Ollama local LLM generates SQL, SOMA executes via postgres port |
| `soma-project-mcp` | (any) | Claude Code MCP integration — SOMA registered as an MCP server |
| `soma-project-s2s` | filesystem | SOMA-to-SOMA delegation and routine transfer (42 tests) |
| `soma-project-multistep` | filesystem (reference pack) | End-to-end proof of multi-step autonomous routine learning |

### Embedded (compile-time ports, no dylib)

| Project | Target | What it does |
|---------|--------|-------------|
| `soma-project-esp32` | ESP32-S3, ESP32 LX6 | `no_std` leaf firmware with 12 hardware ports, runtime-configurable pins, mDNS auto-discovery, SSD1306 OLED display. Different structure from the dylib projects (see below). |

## Building an Embedded Project (`soma-project-esp32` pattern)

Embedded projects target microcontrollers, where dynamic library loading is impossible and the runtime deliberation logic is too heavy for the hardware. The solution is to deploy a **leaf** — a pure dispatcher with no control loop, no memory pipeline, and no policy engine — and drive it from a server SOMA over the distributed transport layer.

The directory structure is different from the dylib projects. It is a full Rust cargo workspace rather than a single directory with a pre-built binary:

```
soma-project-esp32/
  Cargo.toml                    # workspace root, 14 members
  rust-toolchain.toml           # pins the +esp Xtensa toolchain
  GETTING_STARTED.md            # full setup and build guide
  leaf/                         # no_std wire protocol library
  ports/                        # chip-agnostic hardware port crates
    gpio/ delay/ uart/ i2c/ spi/ adc/ pwm/ wifi/ storage/
    thermistor/ board/ display/
  firmware/                     # the flashable binary
    Cargo.toml                  # chip-esp32 / chip-esp32s3 cargo features
    chips/                      # per-chip cargo config overlays
      esp32s3.toml
      esp32.toml
    build.rs                    # custom linker fragment for ESP-IDF app descriptor
    src/
      main.rs                   # chip-agnostic: heap, dispatch loop, self-test
      chip/
        mod.rs                  # cfg-gated `pub use ... as active`
        esp32s3.rs              # S3 pin map + register_all_ports()
        esp32.rs                # ESP32 LX6 pin map + register_all_ports()
      mdns.rs                   # edge-mdns responder on smoltcp UDP
  scripts/
    setup.sh boards.sh build.sh flash.sh monitor.sh
    test.sh cycle.sh wire-test.py
    wifi-scan.sh wifi-connect.sh
    thermistor-to-display.py    # brain-side periodic sensor→display loop
  vendor/
    esp-alloc/                  # vendored + patched upstream crate
```

### Key differences from the dylib projects

1. **Ports are `rlib` crates, not `cdylib`.** Microcontrollers can't dynamically load shared libraries. Each port is a compile-time dependency with an `optional = true` feature flag in `firmware/Cargo.toml`. Adding a port to a build means flipping a cargo feature, not copying a file.

2. **Ports are chip-agnostic.** Port crates never depend on `esp-hal` or a specific chip feature. Hardware access happens through injected `Box<dyn FnMut(...)>` closures the firmware builds in the chip module. Adding a new chip is dropping one `chip/<chip>.rs` file — port crates are untouched.

3. **No pack manifests.** The leaf has no notion of packs, sandbox permissions, or policy rules. All skills are compiled in via feature flags. Safety enforcement lives on the server SOMA that drives the leaf.

4. **No `dump_state`.** The leaf has no episodic memory or belief state to dump. `ListCapabilities` returns the registered primitives + stored routines, which is the leaf's entire self-model.

5. **Runtime pin configuration.** Pin assignments are loaded from `FlashKvStore` at boot with per-chip `DEFAULT_*` fallbacks. `board.configure_pin` + `board.reboot` changes pins without a reflash.

6. **mDNS announcement.** The leaf announces `soma-<chip>-<mac>._soma._tcp.local.` after DHCP. A server SOMA running `--discover-lan` finds it automatically and `invoke_remote_skill` works immediately.

### Build cycle

Use the helper scripts — they're the single source of truth for chip→target/features/config mapping:

```bash
cd soma-project-esp32

# One-time toolchain install
./scripts/setup.sh

# Probe connected boards and get serial-port suggestions
./scripts/boards.sh

# Edit scripts/devices.env so each chip's *_PORT matches your /dev/cu.usbserial-*

# Build + flash + run the wire-protocol exerciser, all in one shot
./scripts/cycle.sh esp32s3           # S3 without wifi
./scripts/cycle.sh esp32s3 wifi      # S3 with wifi
./scripts/cycle.sh esp32 wifi        # WROOM-32D with wifi
```

The full getting-started guide is in `soma-project-esp32/GETTING_STARTED.md`. It covers every step: toolchain install, chip detection, flashing, wire protocol reference, adding a new port, adding a new chip, the display port + shared I²C bus architecture, and the troubleshooting matrix.

### Driving the leaf from an LLM

Once the leaf is flashed and on the LAN, any MCP-aware LLM client can reach it through the server SOMA:

```bash
soma --mcp --discover-lan --pack packs/reference/manifest.json
```

Then from the LLM side:

1. Call `list_peers` — the leaf shows up as `lan-soma-<chip>-<mac>`.
2. Call `invoke_remote_skill` with that peer_id to invoke any skill the leaf exposes.

Example brain-side loop reading a thermistor every 5 seconds and drawing the value on an OLED (see `scripts/thermistor-to-display.py` for a working Python implementation):

```python
for tick in range(N):
    temp = invoke_remote_skill(peer, "thermistor.read_temp", {"channel": 0})
    invoke_remote_skill(peer, "display.draw_text", {
        "line": 0,
        "text": f"Temp: {temp['observation']['temp_c']:.2f} C",
    })
    time.sleep(5)
```

The leaf has no concept of "every 5 seconds" and no concept of "read sensor, show on screen" — both are the brain's composition of two primitive invocations. Change the LLM, change the prompt, change the cadence, and the "application" changes without any code on the body.
