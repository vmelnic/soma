# soma-project-web

Phase 1a proof that **soma-next runs in the browser**. A ~132 KB wasm
build of the core runtime loads in a browser tab, registers a `dom`
port that implements the same `Port` trait as every other SOMA port
(filesystem, HTTP, ESP32 hardware, Python MCP server, …), and renders
an `<h1>` into `document.body` via `web-sys` when a JS handler calls
the wasm-exported entry point.

This is the smallest honest end-to-end demonstration that:

1. `soma-next` compiles cleanly for `wasm32-unknown-unknown` (phase 0)
2. A `Port` trait impl inside the wasm tab can reach through `web-sys`
   and mutate the actual browser DOM
3. The `PortCallRecord` that comes back has exactly the same shape a
   native port would produce (success, latency, structured_result,
   observation_id, side_effect_summary, …)

Every downstream phase-1 step — voice input, audio output, keyboard
input, and eventually an LLM brain composing routines — plugs into
the same path this proof establishes.

## Files

```
soma-project-web/
  index.html                # HTML shell + JS harness that loads the wasm
  pkg/                      # wasm-bindgen output (gitignored, regenerated)
    soma_next.js            #   JS glue (~15 KB)
    soma_next.d.ts
    soma_next_bg.wasm       #   wasm bundle (~132 KB)
    soma_next_bg.wasm.d.ts
  scripts/
    build.sh                # cargo build + wasm-bindgen → pkg/
    serve.sh                # python3 -m http.server on localhost:8080
```

## Building and running

**Prerequisites.** Rust with the `wasm32-unknown-unknown` target and
`wasm-bindgen-cli` pinned to the version the `soma-next` Cargo.toml uses.
The build script enforces matching versions — if `wasm-bindgen-cli` is
a different version, it will produce an empty `pkg/` with no error.

```bash
rustup target add wasm32-unknown-unknown
cargo install -f wasm-bindgen-cli --version 0.2.117
```

**Build and serve:**

```bash
./scripts/build.sh       # compiles soma-next as a cdylib + runs wasm-bindgen
./scripts/serve.sh       # local HTTP server on http://localhost:8080
```

Then open `http://localhost:8080/index.html` in a browser. You should see:

- The page shell (traditional HTML)
- An input field with `hello marcu` pre-filled
- An "Invoke dom.append_heading" button
- A `PortCallRecord returned by the runtime` code block reading
  `soma-next wasm booted. Click the button to invoke the dom port.`
- Open the browser dev console: look for
  `[soma-next wasm] boot — soma body loaded in the browser`

**Click the button.** Below the `<hr>` an `<h1>hello marcu</h1>` appears,
rendered by `DomPort::append_heading` via `web_sys::Document::create_element`.
Above it the `PortCallRecord` code block shows the full observation:

```json
{
  "observation_id": "…",
  "port_id": "dom",
  "capability_id": "append_heading",
  "success": true,
  "failure_class": null,
  "structured_result": {
    "rendered": true,
    "tag": "h1",
    "text": "hello marcu"
  },
  "side_effect_summary": "dom_append",
  "latency_ms": 0,
  "retry_safe": true,
  ...
}
```

That's the same `PortCallRecord` shape the filesystem port produces on
native, the postgres port produces against a real database, and the
ESP32 `display.draw_text` port produces on the WROOM-32D. Identical
schema, different body.

## What's NOT in this proof yet

- **No LLM brain.** The test harness calls `soma_demo_render_heading`
  directly from JavaScript. Phase 1b+ will bring an LLM composer so
  the tab can respond to higher-level intent instead of explicit
  function calls.
- **No pack manifest bootstrap.** The DomPort is instantiated directly
  in the wasm entry point, not loaded from a `packs/` manifest via
  `bootstrap()`. That wiring comes after the core in-browser port
  catalog is fleshed out.
- **Only one capability** (`append_heading`). The full `dom` port will
  add `create_element`, `set_text`, `set_attr`, `on_event`, etc.
- **No other ports.** `audio`, `voice`, `keyboard` are all phase 1b+.
- **No learned routines.** The multistep episode → schema → routine
  pipeline runs inside the core runtime already, but this proof doesn't
  exercise it. The point of phase 1a is "wasm works end-to-end"; phase
  1c will add a routine demo where the 10th button-click skips the
  brain entirely.

## The architectural significance

Every SOMA proof project so far has demonstrated the brain/body split
from a different angle:

- `soma-project-postgres` — body = PostgreSQL via dylib port
- `soma-project-esp32` — body = ESP32 hardware via distributed wire protocol
- `soma-project-mcp-bridge` — body = Python / Node / PHP MCP servers
- **`soma-project-web`** — body = browser DOM, inside the tab itself

One runtime, one port trait, five different kinds of body. The browser
is not a special target — it's just the fifth one that made the cut.
