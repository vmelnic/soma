# soma-project-web

**SOMA runs in a browser tab.** A ~1.3 MB WebAssembly build of
`soma-next` loads as a cdylib, boots a full `Runtime` (session
controller, skill runtime, goal runtime, memory stores, metrics,
proprioception, policy engine, the 16-step control loop), registers
three in-tab ports (`dom`, `audio`, `voice`) that implement the same
`Port` trait every native proof project uses, and accepts natural-
language goals from JavaScript. An external brain (OpenAI `gpt-5-mini`
via a local proxy, a test fixture, or any other swappable endpoint)
composes multi-port plans that the runtime walks through
`DefaultPortRuntime::invoke` — the exact same code path soma-project-
postgres, soma-project-esp32, and soma-project-mcp-bridge drive on
native.

## Current status (phase 1a – 1g shipped)

| Phase | What works |
|---|---|
| 1a | WASM entry point, first DOM port call visible as `<h1>` |
| 1b | Widened DomPort + AudioPort (Web Speech API) + generic `soma_invoke_port` |
| 1c | VoicePort (SpeechRecognition) + three-port composition (voice → dom → audio) |
| 1d | Full `soma-next` `Runtime` bootstrapped in the tab via `bootstrap_from_specs` |
| 1e | Autonomous goal execution — `soma_run_goal` drives the real SessionController |
| 1f | Plan-following dispatch via `soma_inject_routine` |
| 1g | LLM brain over HTTP, real OpenAI proxy + Playwright mock for CI |

**18 Playwright tests, ~5 seconds end-to-end.** Every phase re-runs
on every build — no more "click the buttons" verification cycles.

## Architecture

```
┌─────────────────────────────┐  fetch POST /api/brain
│   browser tab               │◄───────────────────────►  brain proxy
│                             │   {prompt, port_catalog}   (scripts/brain-proxy.mjs)
│  ┌───────────────────────┐  │   {plan, explanation}          │
│  │ index.html + JS       │  │                                 ▼
│  │ (shell + harness)     │  │                          OpenAI gpt-5-mini
│  │                       │  │                          reasoning_effort: low
│  │   soma_invoke_port ──┐│  │
│  │   soma_run_goal     ─┤│  │
│  │   soma_inject_routine┤│  │
│  └──────────────────────┴┘  │
│             │               │
│             ▼               │
│  ┌───────────────────────┐  │
│  │ soma_next.wasm        │  │
│  │   Runtime             │  │
│  │   ├── SessionCtrl     │  │
│  │   ├── PortRuntime ────┼──┼──► web_sys::Document  (dom port)
│  │   │                   │  │
│  │   ├── SkillRuntime    │  │──► speechSynthesis    (audio port)
│  │   ├── GoalRuntime     │  │
│  │   ├── EpisodeStore    │  │◄── SpeechRecognition  (voice port)
│  │   ├── SchemaStore     │  │
│  │   └── RoutineStore    │  │
│  └───────────────────────┘  │
└─────────────────────────────┘
```

The runtime is the body. The brain is outside the tab. Changing
brains (OpenAI ↔ Claude ↔ local model ↔ test fixture) is one URL
change in localStorage — the wasm never knows or cares.

## Files

```
soma-project-web/
  index.html                    # shell + harness with all phase sections
  packs/hello/manifest.json     # minimal browser pack declaring one skill
  pkg/                          # wasm-bindgen output (gitignored, regenerated)
    soma_next_bg.wasm           #   ~1.3 MB core runtime
    soma_next.js                #   ~29 KB JS glue
  scripts/
    build.sh                    # cargo build + wasm-bindgen → pkg/
    serve.sh                    # python3 -m http.server on 8765
    brain-proxy.mjs             # Node HTTP proxy in front of OpenAI
    test-browser.sh             # wrapper around `npx playwright test`
  tests/
    phase1d.spec.js             # Runtime boot + dom/audio gate assertions
    phase1e.spec.js             # Autonomous goal execution
    phase1f.spec.js             # Plan-following via injected routine
    phase1g.spec.js             # LLM brain with page.route mocks (hermetic)
    phase1g-proxy.spec.js       # Real brain proxy round trip (fake mode)
  package.json                  # devDeps: @playwright/test, dep: openai
  playwright.config.js
```

## Quick start

**1. Build the wasm bundle.**

```bash
./scripts/build.sh
```

Requires `cargo`, the `wasm32-unknown-unknown` target, and
`wasm-bindgen-cli` pinned to the version the `soma-next/Cargo.lock`
uses:

```bash
rustup target add wasm32-unknown-unknown
cargo install -f wasm-bindgen-cli --version 0.2.117
```

**2. Run the test suite.**

```bash
npx playwright test
```

First run downloads Chromium (~150 MB, once). All 18 tests run in ~5
seconds. Zero external dependencies — tests use `page.route` and a
Node subprocess for the proxy case.

**3. Start the demo server.**

```bash
./scripts/serve.sh
```

Opens `http://localhost:8765/index.html`. You'll see grouped sections
for each subsystem: runtime info, autonomous goals, LLM brain, dom
port, audio port, voice port, composition. Every button dispatches
through the real runtime pipeline — the `PortCallRecord` pane shows
the full observation envelope after each click.

## Running the brain proxy against real OpenAI

```bash
export OPENAI_API_KEY=sk-...
node scripts/brain-proxy.mjs                  # listens on 127.0.0.1:8787
```

Then in the browser:

1. Paste `http://localhost:8787/api/brain` into the "Brain endpoint"
   field under the **LLM brain** section, press Tab (persists to
   `localStorage.soma.brain.endpoint`).
2. Type a natural-language prompt — e.g. *"say hello to marcu in
   three different ways"*.
3. Click **compose & run**.

The proxy posts your prompt + the current port catalog to
`gpt-5-mini` with `reasoning_effort: "low"` and
`response_format: { type: "json_object" }`, parses the returned
`{plan, explanation}`, and sends it back. The browser then walks the
plan by calling `soma_invoke_port` for each step; you see the DOM
mutate and hear the audio play.

Brain-proxy flags:

```
node scripts/brain-proxy.mjs [options]
  --port N        listen on localhost:N (default 8787)
  --model M       OpenAI model id (default gpt-5-mini)
  --fake          return a canned plan without calling OpenAI —
                  useful for testing the full browser → proxy →
                  browser HTTP round trip without an API key.
```

### Wire contract

The proxy (and any brain endpoint you want to plug in) must speak
this wire protocol. Nothing soma-specific about it — any service
that returns this shape is a valid brain.

**Request:**

```json
POST /api/brain
Content-Type: application/json
{
  "prompt": "say hello to marcu",
  "port_catalog": [
    {
      "port_id": "dom",
      "namespace": "soma.ports.dom",
      "kind": "Renderer",
      "capabilities": ["append_heading", "append_paragraph", "set_title", "clear_soma"]
    },
    {
      "port_id": "audio",
      "namespace": "soma.ports.audio",
      "kind": "Actuator",
      "capabilities": ["say_text"]
    },
    {
      "port_id": "voice",
      "namespace": "soma.ports.voice",
      "kind": "Sensor",
      "capabilities": ["start_listening", "stop_listening", "get_last_transcript", "get_all_transcripts", "clear_transcripts"]
    }
  ]
}
```

**Response:**

```json
200 OK
{
  "plan": [
    {
      "port_id": "dom",
      "capability_id": "append_heading",
      "input": { "text": "Hello marcu!", "level": 1 }
    },
    {
      "port_id": "audio",
      "capability_id": "say_text",
      "input": { "text": "Hello marcu!" }
    }
  ],
  "explanation": "Greet the user visually and aloud."
}
```

Swapping OpenAI for Claude, a local model, or any other provider is
just replacing the proxy's body with a different client call. The
wire contract is stable.

## Testing

```bash
# hermetic suite (no external services, uses page.route mocks)
npx playwright test                          # 18 tests, ~5s
npx playwright test tests/phase1d.spec.js    # runtime + ports only
npx playwright test tests/phase1f.spec.js    # plan-following only

# real HTTP round trip through the Node proxy in --fake mode
npx playwright test tests/phase1g-proxy.spec.js
```

Playwright auto-starts `python3 -m http.server 8765` on first run via
`webServer` config. `reuseExistingServer: true` means a developer who
left `./scripts/serve.sh` running in another terminal won't fight it.

## What's NOT in this proof yet

- **Organic routine compilation.** Phase 1f proves the plan-following
  dispatch path works by injecting a compiled routine. Whether the
  multistep learning pipeline fires on organic single-skill episodes
  from phase 1e is a separate question — it requires multi-step
  episodes which the single-skill hello pack doesn't naturally
  produce. Same limitation `soma-project-multistep` calls out on
  native. Phase 2 work.
- **Optimized bundle.** The 1.3 MB wasm is an unoptimized release
  build. `wasm-opt -Oz` + brotli would bring the wire size to ~150
  KB. Not worth doing until the runtime surface stabilizes.
- **WebGPU / local LLM.** The brain runs outside the tab over HTTP.
  Running a small LLM in-tab via WebGPU is a plausible future phase
  but not necessary to validate the architecture.
- **Pack manifest bootstrap from a real LLM.** Phase 1g composes
  per-request plans, not persistent routines. A richer integration
  would let the LLM design pack manifests that SOMA loads, not just
  one-shot invocations.

## Architectural significance

Every SOMA proof project demonstrates the brain/body split from a
different angle:

| Project | Body | How |
|---|---|---|
| `soma-project-postgres` | PostgreSQL | dylib port |
| `soma-project-esp32` | ESP32 + OLED | distributed wire protocol |
| `soma-project-mcp-bridge` | Python / Node / PHP MCP servers | stdio subprocess |
| `soma-project-web` | **Browser DOM + audio + voice** | **in-tab wasm** |

One runtime, one `Port` trait, four kinds of body. The browser is
not a special target — it's just the fifth one that made the cut.
