# Building Projects

A **soma-project** is a self-contained directory that wires a soma-next binary to one or more port `.dylib` libraries via pack manifests. The MCP server exposes `invoke_port` and `list_ports` tools for LLMs and test clients.

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

| Project | Port | What it does |
|---------|------|-------------|
| `soma-project-smtp` | smtp | Email delivery via mailcatcher or real SMTP |
| `soma-project-s3` | s3 | AWS S3 object storage |
| `soma-project-postgres` | postgres | Database queries against HelperBook schema |
