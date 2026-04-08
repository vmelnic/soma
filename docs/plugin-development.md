# Plugin Development Guide

## Overview

A SOMA plugin is a Rust shared library (`cdylib`) that implements the `SomaPlugin` trait. Plugins give the Mind its capabilities -- without plugins, a SOMA can think but cannot act.

A complete plugin consists of:

- **Compiled binary** -- `.dylib` (macOS) or `.so` (Linux), loaded at runtime via `dlopen`
- **Manifest** -- `manifest.json` declaring identity, conventions, and dependencies
- **Training data** -- `training/examples.json` with (intent, program) pairs that teach the Mind to use your conventions
- **LoRA weights** (optional) -- pre-trained adaptations so the Mind is proficient with your plugin immediately

The end result is a `.soma-plugin` archive installable via `soma plugin install`.

## Prerequisites

- Rust stable toolchain (edition 2024)
- `soma-plugin-sdk` as a dependency (provides the `SomaPlugin` trait, `Value` enum, `Convention`, and all supporting types)
- Familiarity with the convention and `Value` type system (see [docs/architecture.md](architecture.md))
- For LoRA training: Python 3.10+, PyTorch 2.0+, and the `soma-synthesizer` package

## Project Setup

### Directory Structure

```
my-plugin/
  Cargo.toml
  manifest.json
  src/lib.rs
  training/examples.json
```

### Cargo.toml

The crate type must be `cdylib` so that Rust produces a shared library. Depend on `soma-plugin-sdk` and any crates your conventions need.

```toml
[package]
name = "soma-plugin-my-plugin"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
soma-plugin-sdk = { path = "../sdk" }
serde_json = "1"    # if you need JSON parsing
```

If building inside the `soma-plugins/` workspace, add your crate to the workspace `Cargo.toml`:

```toml
[workspace]
members = [
    "sdk",
    "my-plugin",
    # ...
]
```

## Implement the Trait

Every plugin follows the same structure: a struct implementing `SomaPlugin`, plus a C ABI entry point for dynamic loading.

Here is a complete working plugin -- following the pattern used by `soma-plugins/geo/`:

```rust
use soma_plugin_sdk::prelude::*;
use std::collections::HashMap;

pub struct WeatherPlugin;

impl SomaPlugin for WeatherPlugin {
    fn name(&self) -> &str { "weather" }
    fn version(&self) -> &str { "0.1.0" }
    fn description(&self) -> &str { "Weather data: current conditions and forecasts" }
    fn trust_level(&self) -> TrustLevel { TrustLevel::Community }

    fn conventions(&self) -> Vec<Convention> {
        vec![
            Convention {
                id: 0,
                name: "current".into(),
                description: "Get current weather for a location".into(),
                call_pattern: "current(location)".into(),
                args: vec![ArgSpec {
                    name: "location".into(),
                    arg_type: ArgType::String,
                    required: true,
                    description: "City name or coordinates".into(),
                }],
                returns: ReturnSpec::Value("Map".into()),
                is_deterministic: false,
                estimated_latency_ms: 200,
                max_latency_ms: 5000,
                side_effects: vec![SideEffect("sends_network".into())],
                cleanup: None,
            },
            Convention {
                id: 1,
                name: "forecast".into(),
                description: "Get weather forecast for a location".into(),
                call_pattern: "forecast(location, days)".into(),
                args: vec![
                    ArgSpec { name: "location".into(), arg_type: ArgType::String,
                              required: true, description: "City name or coordinates".into() },
                    ArgSpec { name: "days".into(), arg_type: ArgType::Int,
                              required: true, description: "Number of days to forecast".into() },
                ],
                returns: ReturnSpec::Value("List".into()),
                is_deterministic: false,
                estimated_latency_ms: 300,
                max_latency_ms: 5000,
                side_effects: vec![SideEffect("sends_network".into())],
                cleanup: None,
            },
        ]
    }

    fn execute(&self, convention_id: u32, args: Vec<Value>) -> Result<Value, PluginError> {
        match convention_id {
            0 => self.current(args),
            1 => self.forecast(args),
            _ => Err(PluginError::NotFound(format!("unknown convention_id: {}", convention_id))),
        }
    }
}

impl WeatherPlugin {
    fn current(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let location = args.first()
            .ok_or_else(|| PluginError::InvalidArg("missing: location".into()))?
            .as_str()?;
        let mut map = HashMap::new();
        map.insert("location".into(), Value::String(location.to_string()));
        map.insert("temp_c".into(), Value::Float(22.0));
        map.insert("condition".into(), Value::String("partly cloudy".into()));
        Ok(Value::Map(map))
    }

    fn forecast(&self, args: Vec<Value>) -> Result<Value, PluginError> {
        let location = args.first()
            .ok_or_else(|| PluginError::InvalidArg("missing: location".into()))?
            .as_str()?;
        let days = args.get(1)
            .ok_or_else(|| PluginError::InvalidArg("missing: days".into()))?
            .as_int()?;
        let mut forecasts = Vec::new();
        for day in 0..days {
            let mut entry = HashMap::new();
            entry.insert("day".into(), Value::Int(day));
            entry.insert("location".into(), Value::String(location.to_string()));
            entry.insert("high_c".into(), Value::Float(24.0));
            entry.insert("low_c".into(), Value::Float(16.0));
            forecasts.push(Value::Map(entry));
        }
        Ok(Value::List(forecasts))
    }
}

// C ABI entry point -- required for dynamic loading
#[unsafe(no_mangle)]
pub extern "C" fn soma_plugin_init() -> *mut dyn SomaPlugin {
    Box::into_raw(Box::new(WeatherPlugin))
}
```

### Key points

- **`soma_plugin_init`** must be `#[unsafe(no_mangle)]` and return `*mut dyn SomaPlugin`. SOMA Core calls this to instantiate your plugin.
- **Convention IDs** are local to your plugin (0, 1, 2, ...). The PluginManager offsets them by `plugin_idx * 1000` at registration time to prevent routing conflicts across plugins.
- **Argument extraction** uses `Value::as_str()`, `Value::as_float()`, `Value::as_int()`, etc. These return `Result` with a descriptive `PluginError::InvalidArg` on type mismatch.
- **Lifecycle methods** `on_load` and `on_unload` have default no-op implementations. Override them if your plugin needs initialization (database connections, config reading) or cleanup.

## Define Conventions

Each convention is a `Convention` struct. The fields visible in the example above are:

| Field | Purpose |
|---|---|
| `id` | Unique within your plugin (0, 1, 2, ...) |
| `name` | Used in training data as `plugin_name.convention_name` |
| `description` | Human-readable, shown in MCP tool listings |
| `call_pattern` | Signature hint, e.g. `"distance(lat1, lon1, lat2, lon2)"` |
| `args` | `Vec<ArgSpec>` -- name, type (`String`, `Int`, `Float`, `Bool`, `Bytes`, `Handle`, `Any`), required, description |
| `returns` | `ReturnSpec` -- `Value("String")`, `Value("Map")`, `Handle`, `Void`, or `Stream("String")` |
| `is_deterministic` | Same input = same output? Helps Mind with caching decisions |
| `estimated_latency_ms` | Typical execution time -- Mind prefers faster alternatives |
| `max_latency_ms` | Timeout -- Core kills execution if exceeded |
| `side_effects` | Declared effects: `"writes_disk"`, `"sends_network"`, `"modifies_state"` |
| `cleanup` | `Option<CleanupSpec>` -- convention to call if a later step fails (rollback) |

### CleanupSpec

If a convention allocates resources (opens a transaction, creates a temp file), point `cleanup` to a rollback convention:

```rust
cleanup: Some(CleanupSpec {
    convention_id: 1,  // points to "rollback" convention
    pass_result_as: 0, // pass the transaction handle as arg 0
}),
```

## Write Training Data

Training data teaches the Mind when and how to invoke your conventions. Without it, the Mind cannot generate programs that use your plugin.

### File Format

Create `training/examples.json` in your plugin directory:

```json
{
  "schema_version": "1.0",
  "plugin": "weather",
  "plugin_version": "0.1.0",
  "examples": [
    {
      "id": "weather_current_001",
      "intents": [
        "what's the weather in {location}",
        "current weather for {location}",
        "how's the weather in {location}",
        "get weather conditions in {location}",
        "check weather at {location}",
        "show me the weather in {location}"
      ],
      "program": [
        {"convention": "weather.current", "args": [{"name": "location", "type": "span", "extract": "location"}]},
        {"convention": "EMIT", "args": [{"name": "data", "type": "ref", "step": 0}]},
        {"convention": "STOP"}
      ],
      "params": {
        "location": ["New York", "London", "Tokyo", "Paris", "downtown"]
      },
      "tags": ["weather", "current"]
    }
  ]
}
```

### Program Step Format

Each program step has:

- **`convention`** -- fully qualified name: `"plugin_name.convention_name"`, or the control opcodes `"EMIT"` and `"STOP"`
- **`args`** -- array of argument descriptors, each with a `type`:
  - `"span"` -- extracted from the user's intent text. The `extract` field names the parameter placeholder.
  - `"ref"` -- references the result of a previous step. The `step` field is the zero-based step index.
  - `"literal"` -- a hardcoded value. The `value` field contains the constant.

### Critical Rules

**Every program must end with EMIT and STOP.** EMIT sends the result to the caller. STOP terminates execution. If training examples omit these, the Mind learns to generate programs that run but never return results.

**Provide 50-200 expanded training pairs per convention.** Each intent template gets expanded by parameter pools. Under 30 expanded pairs leads to undertrained conventions; over 500 hits diminishing returns.

**Use at least 5 structurally different intent templates per convention.** Synonym variations alone ("list"/"show"/"display") are not enough -- you need structural variety:

```json
"intents": [
  "what's the weather in {location}",
  "get {location} weather conditions",
  "check if it's raining in {location}",
  "{location} weather right now",
  "tell me the temperature in {location}",
  "how cold is it in {location}",
  "weather report for {location}"
]
```

**Parameter pools must have 5+ diverse, realistic values:**

```json
"params": {
  "location": ["New York", "London", "Tokyo", "San Francisco", "downtown"]
}
```

Bad pool (too small): `["New York"]` -- the Mind only learns one city.
Bad pool (unrealistic): `["xyzzy", "test123"]` -- won't match real intents.

**Start simple, add complexity.** Begin with 1-2 step programs. Add multi-step programs (3+ steps) after simple patterns are covered. If all training examples are complex, the Mind overcomplicates simple intents.

**Include multi-step examples** for operations that naturally chain. See the Cross-Plugin Programs section in the Training Data Reference below for an example.

### Common Mistakes

| Mistake | Problem | Fix |
|---|---|---|
| Too few examples | Mind guesses wrong conventions | 50-200 expanded pairs per convention |
| All examples use same parameter values | Mind can't generalize | 5+ diverse values per parameter pool |
| Missing EMIT/STOP | Programs run but never return results | Every program must end with EMIT then STOP |
| Overly complex programs for simple intents | Mind adds unnecessary steps | Start with 1-2 step examples |
| Inconsistent convention naming | `"weather.current"` vs `"weath.cur"` | Names must exactly match manifest |
| No cross-plugin examples | Mind can't chain across plugins | Add examples combining your conventions with other plugins |

## Write the Manifest

Create `manifest.json` in your plugin root. SOMA Core reads it during dynamic loading to verify conventions match the binary, and the Synthesizer uses it during validation. Add a `config` section if your plugin needs `soma.toml` settings:

```json
{
  "plugin": {
    "name": "weather",
    "version": "0.1.0",
    "description": "Weather data: current conditions and forecasts"
  },
  "conventions": [
    {"name": "current", "description": "Get current weather for a location"},
    {"name": "forecast", "description": "Get weather forecast for a location"}
  ],
  "config": {
    "api_key": "Weather API key (or use api_key_env to reference an env var)",
    "units": "Temperature units: celsius or fahrenheit (default: celsius)"
  }
}
```

The corresponding `soma.toml` section:

```toml
[plugins.weather]
api_key_env = "WEATHER_API_KEY"
units = "celsius"
```

## Build

```bash
cargo build --release
# macOS: target/release/libsoma_plugin_my_plugin.dylib
# Linux: target/release/libsoma_plugin_my_plugin.so
```

If building as part of the `soma-plugins/` workspace, run `cargo build --release` from the workspace root to build all plugins at once.

## Test

### Unit Tests

Test convention logic directly:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_weather() {
        let plugin = WeatherPlugin;
        let result = plugin.execute(0, vec![Value::String("London".into())]).unwrap();
        if let Value::Map(map) = result {
            assert_eq!(map["location"], Value::String("London".into()));
        } else { panic!("expected Map"); }
    }

    #[test]
    fn test_forecast_day_count() {
        let plugin = WeatherPlugin;
        let result = plugin.execute(1, vec![Value::String("Tokyo".into()), Value::Int(3)]).unwrap();
        if let Value::List(items) = result { assert_eq!(items.len(), 3); }
        else { panic!("expected List"); }
    }

    #[test]
    fn test_unknown_convention() {
        assert!(WeatherPlugin.execute(99, vec![]).is_err());
    }

    #[test]
    fn test_missing_args() {
        assert!(WeatherPlugin.execute(0, vec![]).is_err());
    }
}
```

### Training Data Validation

```bash
soma-synthesize validate --plugins ./my-plugin
```

This verifies convention references, checks for conflicting examples, validates ref indices and span extractions, flags duplicates after expansion, and reports coverage imbalances.

### Integration Test with SOMA Core

Copy the library to a SOMA's plugin directory and run an intent:

```bash
cp target/release/libsoma_plugin_my_plugin.dylib /path/to/soma/plugins/
soma --config soma.toml --intent "what's the weather in London"
```

## Package

Bundle your plugin for distribution as a `.soma-plugin` archive containing the compiled binary, manifest, training data, and optional LoRA weights:

```bash
soma plugin package ./my-plugin
# Produces: weather-0.1.0.soma-plugin

# Others install from registry or local file:
soma plugin install weather
soma plugin install ./weather-0.1.0.soma-plugin
```

## LoRA Training (Optional)

### When to Train LoRA

LoRA (Low-Rank Adaptation) teaches the Mind to be proficient with your plugin's conventions without requiring full re-synthesis. A plugin WITHOUT LoRA still works -- the Mind learns through runtime experience -- but a plugin WITH LoRA works well immediately.

Train LoRA when:
- Your plugin has domain-specific patterns the base Mind doesn't know
- You want out-of-the-box accuracy for your conventions
- You're distributing the plugin to others who won't re-synthesize

### Process

The Synthesizer freezes the base Mind weights, attaches LoRA adapters to target layers, and trains on your plugin's examples only:

```bash
soma-synthesize train-lora \
  --plugin weather \
  --base-model ./models/server \
  --output ./lora
```

This produces `lora/weather.lora` and `lora/weather.lora.json` (metadata). Copy into your plugin's `lora/` directory.

### How It Works at Runtime

When SOMA loads a plugin with LoRA weights, the LoRA is attached to the Mind and becomes active immediately. Multiple plugin LoRAs compose at runtime -- a gating mechanism weights each plugin's LoRA based on the intent (e.g., a database query activates the Postgres LoRA strongly while the Redis LoRA stays dormant).

### LoRA Config

Defaults in `synthesis_config.toml` (override as needed):

```toml
[lora]
rank = 8
alpha = 16
target_modules = ["op_head", "gru", "a0t_head", "a1t_head"]
epochs = 50
learning_rate = 1e-3
```

## Training Data Reference

### Complete Example: Single-Convention Program

```json
{
  "id": "crypto_sha256_001",
  "intents": [
    "hash {data}",
    "compute SHA256 of {data}",
    "get the SHA-256 hash of {data}",
    "sha256 hash {data}",
    "calculate hash for {data}",
    "generate SHA256 digest of {data}"
  ],
  "program": [
    {"convention": "crypto.hash_sha256", "args": [{"name": "data", "type": "span", "extract": "data"}]},
    {"convention": "EMIT", "args": [{"name": "data", "type": "ref", "step": 0}]},
    {"convention": "STOP"}
  ],
  "params": {
    "data": ["hello", "password123", "test data", "secret message", "my file content"]
  },
  "tags": ["crypto", "hash", "sha256"]
}
```

Expansion: 6 intents x 5 param values = 30 training pairs from one example block. Add more intent templates or pool values to reach the 50-200 target.

### Multi-Step Program with Ref Arguments

```json
{
  "id": "geo_geocode_and_distance_001",
  "intents": [
    "how far is {address1} from {address2}",
    "distance between {address1} and {address2}",
    "calculate distance from {address1} to {address2}"
  ],
  "program": [
    {"convention": "geo.geocode", "args": [{"name": "address", "type": "span", "extract": "address1"}]},
    {"convention": "geo.geocode", "args": [{"name": "address", "type": "span", "extract": "address2"}]},
    {"convention": "EMIT", "args": [{"name": "data", "type": "ref", "step": 0}]},
    {"convention": "STOP"}
  ],
  "params": {
    "address1": ["Times Square", "Central Park", "Eiffel Tower"],
    "address2": ["Statue of Liberty", "Golden Gate Bridge", "Big Ben"]
  }
}
```

Step 0 geocodes the first address; step 1 geocodes the second. Each step's result is available via `"type": "ref"` to later steps.

### Cross-Plugin Programs

Programs that chain conventions across plugins require dedicated training examples. The Mind does not automatically combine single-plugin programs.

```json
{
  "id": "cross_query_and_cache_001",
  "intents": [
    "find contacts nearby and cache the results",
    "query contacts within {radius}km and store in redis",
    "get nearby contacts and cache them for {ttl} seconds"
  ],
  "program": [
    {"convention": "postgres.query", "args": [{"name": "sql", "type": "literal", "value": "SELECT * FROM contacts"}]},
    {"convention": "redis.set", "args": [
      {"name": "key", "type": "literal", "value": "contacts:nearby"},
      {"name": "value", "type": "ref", "step": 0}
    ]},
    {"convention": "EMIT", "args": [{"name": "data", "type": "ref", "step": 0}]},
    {"convention": "STOP"}
  ],
  "params": {
    "radius": ["5", "10", "25"],
    "ttl": ["60", "300", "3600"]
  }
}
```

### Parameter Pool Expansion

The Synthesizer substitutes each `{placeholder}` in every intent with values from the pool. With multiple pools, expansion is the Cartesian product: 5 intent templates x 3 pool values per parameter = wide coverage from compact definitions.

### Validation Checklist

Before running `soma-synthesize validate`, verify:

- [ ] Every convention in the manifest has at least one training example
- [ ] Every program ends with EMIT + STOP
- [ ] 5+ unique intent templates per convention
- [ ] Param pools have 5+ diverse values each
- [ ] All convention names in examples match `manifest.json` exactly (as `plugin_name.convention_name`)
- [ ] All `ref` indices point to valid previous steps (step N can only reference steps 0..N-1)
- [ ] Cross-plugin operations have dedicated examples
- [ ] Edge cases covered: empty results, error conditions, boundary values
