# Future HCI: Research Notes and Architectural Framing

**Status.** Research notes plus an architectural thesis and a proposed phase
plan. Belongs in the same category as `memory-fusion.md` and `semantic-memory.md`:
a design direction, not a commitment. The industry-context section is a
time-bound snapshot of where the field stood when this was written; the
architectural framing is meant to outlast it.

**Context.** The question this document answers: should SOMA pursue the
frontend / future-web-interaction direction, and if so what does the
architecture look like? The short answer: the industry is independently
converging on a pattern SOMA is already shaped like, the web is only one of
several converging surfaces, and the right framing is **SOMA as a universal
runtime for intent-driven human-computer interaction** — with the web as one
output modality among several.

## Industry context

Four convergent trends plus one open problem.

### 1. Generative UI has shipped

Generative UI (GenUI) — where parts of the interface are composed by an AI
agent at runtime rather than hard-coded — has moved from experimental demos
to production. Three reference points from the research:

- **Google A2UI** — agents describe the UI they need (forms, tables,
  multi-step flows) as structured JSONL; the host framework renders it.
- **Vercel `json-render`** — open-source Apache 2.0, lets LLMs construct UIs
  from natural language by emitting against a developer-defined component
  catalog. Supports multiple frontend frameworks.
- **CopilotKit** productized the pattern with React-side tooling.

All three converged on the same architectural decision: **do not let the
model emit raw HTML**. Structured JSON against a pre-approved component
library, no free-form generation. Design systems didn't die; they became
essential because they're the only thing the LLM is allowed to compose
against.

Evaluations where generation speed is held constant show strong user
preference for generative UI output over static LLM-generated equivalents.

### 2. Zero UI / ambient computing is entering consumer hardware

Screenless ambient AI gadgets are beginning to ship, forcing a shift to
high-intent voice interaction, explicitly designed around moving away from
scrolling and tapping. The design thesis:

> **"Interaction → intention."** Not "press button to do thing," but
> "state what you want and let the system figure out the steps."

Interfaces drawn on demand, dismissed when done, no persistent navigation.
"Zero UI" as a term covers voice, gesture, gaze, and sensor-driven flows
that replace explicit UI elements entirely.

### 3. Adaptive UI from user patterns

LLMs observe user behavior, predict intent from recent actions and context,
and **generate bespoke micro-interfaces featuring only the relevant details
for the current task**. Production reports cite double-digit engagement lifts
and measurable latency reductions when predictive personalization loops are
integrated into front-end adaptation. Key idea: every user's UI should be
different because every user's task is different.

### 4. Input modalities are fragmenting

- **Eye tracking** is production. Apple Vision Pro uses gaze + pinch as its
  primary paradigm, with a deliberate privacy rule that gaze data is only
  exposed to apps when the user intentionally confirms with a gesture.
- **Voice + text** is the hinge modality — usable alongside screens, bridge
  toward the screenless future.
- **BCI** is clinical, not consumer. Neuralink and Synchron have early
  human implant recipients; analyst consensus puts consumer-grade BCI
  launch roughly a half-decade out.

None of these displace the others. The future is multi-modal: the same
"application" must route the same intent through voice, gaze, text, touch,
and eventually neural input.

### The open problem: accessibility for generative UIs

WCAG was written for static semantic HTML. When a GenUI composes new DOM on
every interaction, screen-reader semantics, focus management, and keyboard
navigation all degrade. A draft AAG v0.1 ("Accessibility Guidelines for AI
Interfaces") acknowledges this and tries to adapt WCAG principles, but it's
early, and every framework shipping GenUI today is papering over the gap
with post-hoc ARIA injection — brittle and incomplete. This is a real
problem the industry has not solved.

## How SOMA maps onto each trend

Direct correspondence, not approximation:

| Industry 2026 trend | SOMA primitive that already matches it |
|---|---|
| GenUI framework: LLM emits structured JSON against a component library | Pack manifest: LLM emits structured JSON against a port catalog |
| Rendering at runtime from AI-generated config | `DefaultPortRuntime::invoke` is the renderer |
| Design system = approved components the AI composes | `PortSpec.capabilities` + typed input/output schemas |
| Intent in, execution out | Literal SOMA tagline |
| Adaptive UI from user behavior | Episode memory → schema induction → routine compilation (multistep proven) |
| Bespoke micro-interfaces per task | Working memory + goal scope — one runtime serves different compositions |
| Cross-modal input (voice, gaze, text) | Ports are I/O-symmetric: filesystem, HTTP, SMTP, display, thermistor all go through the same trait |

The last row is the strongest one. The soma-project-esp32 thermistor →
display loop is, in retrospect, direct evidence for the HCI thesis: the
brain composed `thermistor.read_temp → display.draw_text` without knowing
or caring that one port was a sensor and the other was a screen. **That is
the same pattern as `microphone.transcribe → dom.render` or
`gaze.focus → display.overlay`.** The runtime does not distinguish sensors
from renderers because both are just ports with a capability contract.
The "human" in the ESP32 proof happened to be a thermistor.

## Where SOMA is ahead of the current GenUI frameworks

Three things the surveyed frameworks do not have:

### 1. No episode memory, no routine compilation

Every GenUI framework reviewed is **stateless**. Each interaction is a
fresh LLM composition. First click pays the LLM cost; the thousandth click
still pays the LLM cost. No episode store, no schema induction, no compiled
routine, no plan-following fast path.

SOMA's multistep pipeline is *exactly* the fix. First use: LLM deliberates.
Once a schema crystallizes: compile to a routine. Every subsequent use:
plan-following walks the compiled path at runtime speed, no LLM roundtrip.

**Claim: a SOMA UI gets faster the more it is used. A Vercel / CopilotKit
GenUI stays bottlenecked by model latency forever.**

### 2. No modality-agnostic abstraction

Each framework is bound to its host:

- Vercel AI SDK / json-render: ships React.
- CopilotKit: React-centric.
- Google A2UI: Google's internal runtime.

None can run the same pack against a browser AND a voice-only ambient
gadget AND a Vision Pro overlay AND (future) a BCI decoder. SOMA is already
modality-agnostic at the architecture level because **ports are the
abstraction**. Same manifest, different output ports, different renderings.

### 3. No universal contract

Every framework invents its own JSON schema. SOMA consumes MCP
(via `McpClientPort`), which is becoming the de facto universal
brain↔body contract.

**Strategic position: don't invent a UI schema, make MCP the UI schema.**
Any MCP-speaking brain (Claude, GPT, local model, future agents) can drive
any MCP-speaking body (browser, voice gadget, AR glasses, BCI decoder).

## The accessibility angle

The GenUI accessibility problem — ad-hoc ARIA injection over LLM-composed
DOM — is solvable inside SOMA's architecture in a way the industry frameworks
are not set up for:

**The pack manifest is the canonical semantic layer.**

A screen reader does not need to parse the composed DOM. It can read the
manifest via `dump_state` — which already declares intent, capabilities,
observable fields, and expected outcomes — and render it as audio or braille.
**The audio rendering is just another port** (say `audio.say_text`). The
LLM composes the same intent through it that it composed through `dom.render`
for sighted users. Multiple rendering ports, one intent composition, per-user
fanout.

This is a cleaner accessibility story than WCAG-over-React, and it emerges
from the architecture for free. You do not have to design for it — it falls
out of the brain/body split.

## Architectural thesis

> **SOMA is not "a Web 4 frontend framework." SOMA is the universal
> runtime for intent-driven human-computer interaction across every
> current and future input and output modality — and the web is one of
> its surfaces.**

Four pieces of existing evidence:

1. The **ESP32 thermistor → display loop** — sensors and renderers are
   indistinguishable to the brain.
2. The **`McpClient` port backend** — any MCP server in any language is a
   port; MCP is a universal contract.
3. The **multistep routine pipeline** — repeated interactions compile to
   plans that walk without the LLM.
4. **soma-next running in a browser tab** (`soma-project-web`) — the core
   runtime compiles to `wasm32-unknown-unknown` via a feature-flag
   restructure, registers in-tab `dom` / `audio` / `voice` ports through the
   same `DefaultPortRuntime` pipeline native proof projects use, executes
   autonomous goals through `SessionController::run_step`, follows injected
   routines through the plan-following dispatch path, and accepts LLM-composed
   plans over a mockable HTTP brain protocol.

Each one is saying the same thing: the body part and the brain part are
separable, the body is modality-agnostic, the brain can be any LLM, and
intent is the contract between them.

## Phase plan

Earlier drafts of this plan were web-centric. The research pushed toward a
wider scoping where the browser is one of several surfaces. For authoritative
status of each phase, read the code in `soma-project-web/` and neighbouring
proof projects; the summary below describes what each phase IS, not when it
landed.

### Phase 1 — The browser as a first-class SOMA body

`soma-next` compiles cleanly for `wasm32-unknown-unknown` via a feature-flag
restructure: `default = ["native"]` keeps every native build equivalent, and
every wasm-incompatible dep (`reqwest`, `libloading`, `tokio::net`, `mdns-sd`,
`hostname`, `libc`, `rustls`) sits behind `native`, `distributed`,
`dylib-ports`, `native-http`, `native-hostname`, `native-filesystem`
features. `web-time` replaces `std::time::Instant`, which aborts on wasm.

On that foundation, the browser gains:

- **In-tab ports** — `DomPort` (`append_heading`, `append_paragraph`,
  `set_title`, `clear_soma`), `AudioPort` (`say_text` via `speechSynthesis`),
  `VoicePort` (Web Speech API `SpeechRecognition`, transcripts drained from
  an async event buffer through synchronous `Port::invoke`).
- **Full `Runtime` booted in-tab** — a `bootstrap_from_specs` entry point
  wires the real `DefaultPortRuntime` pipeline (lifecycle, policy, auth,
  sandbox, input-schema) instead of a hand-rolled registry.
  `soma_invoke_port(port_id, capability_id, input_json)` is the single JS
  entry; every `PortCallRecord` carries the full auth / policy / sandbox
  result.
- **Autonomous goal execution** — `soma_run_goal(objective)` runs the same
  selection → prediction → criticism → learning loop used natively, against
  a minimal in-tab pack.
- **Plan-following via injected routine** — `soma_inject_routine(...)`
  registers a routine whose `match_conditions` target a goal's
  `goal_fingerprint`; the follow-up run trips the plan-following fast path.
- **LLM brain via fetch** — the brain lives outside the tab. JavaScript
  POSTs the prompt plus the runtime's port catalog to a configurable
  endpoint; the endpoint returns `{plan, explanation}` and the harness walks
  the plan through `soma_invoke_port`. A tiny Node proxy forwards prompts to
  whichever LLM is configured; swapping providers is a proxy body swap,
  the wire contract stays fixed.

### Phase 2 — Showcases across other bodies

Phase 1 proves the browser surface. Phase 2 points the same runtime and
brain protocol at other SOMA bodies to demonstrate the modality-agnostic
thesis end to end.

- **Cross-device HCI.** Browser SOMA as brain, ESP32 SOMA as body. A voice
  command into a laptop mic becomes an `invoke_remote_skill` against the
  leaf's `display.draw_text`. The `soma-project-esp32` wire protocol is
  already compatible; the remaining piece is routing the browser's brain
  call path to a transport that speaks the distributed message format.
- **Native mobile surfaces.** Wire `soma-project-android` /
  `soma-project-ios` as additional bodies. The `aarch64-linux-android` and
  `aarch64-apple-ios` cross-compiles already work; the missing piece is the
  platform-specific UI shell (Kotlin / Swift) that hosts the runtime and
  exposes its own in-process ports (mic, camera, haptics, sensors).
- **Real-brain proxies.** Same wire contract, different proxy body: Claude,
  a local-LLM proxy (Ollama / vLLM / llama.cpp), the Claude Agent SDK.

### Phase 3 — Organic multi-step learning (deferred)

Phase 1 proves the plan-following DISPATCH path by injecting a routine.
Phase 3 proves the LEARNING path can produce that routine from organic
episodes — a gap `soma-project-multistep` already calls out natively. Needs
multi-step skills that the selector / critic walks across multiple steps of
one session, and possibly lower PrefixSpan min-support thresholds. Not
blocking phase 2.

### Phase 4 — Manifest as canonical semantic layer (deferred)

Add rendering ports that take current belief / manifest and emit audio
narration and braille cells. Prove the same runtime serves sighted, blind,
and deaf users from one manifest. The wasm side is already most of the way
there — `audio.say_text` exists, a `braille.render` port is a small
web-sys + Unicode braille job.

### Deferred (not a phase)

- **BCI input.** When consumer SDKs appear it is just another input port.
  The runtime does not need to change.
- **Gaze input.** Same shape as BCI — port.
- **Server-side pre-render for SEO.** Symmetric with phase 1: the server
  runs the same SOMA runtime and feeds `dom` output into a string instead
  of a `document`.

The test of whether the architecture is right: **if adding a future input
modality later means "only writing a port," the architecture passed.**

## What this does NOT commit to

- Not a React-alternative framework.
- Not a component system layer (pack manifests are already the declaration
  format — fix them if unergonomic; do not add a new layer on top).
- Not SSR before a live client works.
- Not a JSX equivalent.
- Not a rewrite of anything. HelperBook's existing Express frontend stays
  while any new surface ships as an additive SOMA pack.

## Sources

Industry state and framework landscape:
- [The Developer's Guide to Generative UI in 2026 — CopilotKit](https://www.copilotkit.ai/blog/the-developer-s-guide-to-generative-ui-in-2026)
- [Vercel Releases JSON-Render: a Generative UI Framework for AI-Driven Interface Composition — InfoQ](https://www.infoq.com/news/2026/03/vercel-json-render/)
- [Generative UI: A rich, custom, visual interactive user experience for any prompt — Google Research](https://research.google/blog/generative-ui-a-rich-custom-visual-interactive-user-experience-for-any-prompt/)
- [UX/UI Trends 2026: Generative UI, AI Personalization & Modern Product Design — Stan Vision](https://www.stan.vision/journal/ux-ui-trends-shaping-digital-products)
- [The Future of UI Design Past 2026: Adaptive, Agentic, and Ambient](https://www.basantasapkota026.com.np/2026/03/the-future-of-ui-design-past-2026.html)

Zero UI and ambient computing:
- [Zero UI in 2026: Voice, AI & Screenless Interface Design Trends — Algoworks](https://www.algoworks.com/blog/zero-ui-designing-screenless-interfaces-in-2025/)
- [OpenAI's "Ambient" Ambitions: The Screenless AI Gadget Set to Redefine Computing in Fall 2026](https://markets.financialcontent.com/wral/article/tokenring-2026-1-5-openais-ambient-ambitions-the-screenless-ai-gadget-set-to-redefine-computing-in-fall-2026)
- [Ambient AI in UX: Interfaces That Work Without Buttons — Raw.Studio](https://raw.studio/blog/ambient-ai-in-ux-interfaces-that-work-without-buttons/)

Adaptive UI and user behavior prediction:
- [18 Predictions for 2026 — Jakob Nielsen on UX](https://jakobnielsenphd.substack.com/p/2026-predictions)
- [Predictive Analytics in UX: 2026 Guide to Adaptive UI — Parallel HQ](https://www.parallelhq.com/blog/predictive-analytics-in-ux-design)
- [Intelligent Front-End Personalization: AI-Driven UI Adaptation (arXiv)](https://arxiv.org/pdf/2602.03154)
- [Adaptive User Interface Generation Through LLMs (arXiv)](https://arxiv.org/pdf/2412.16837)

Input modalities (eye tracking, BCI):
- [Exploring Apple Vision Pro's Revolutionary Eye Tracking System — TechInsights](https://www.techinsights.com/blog/exploring-apple-vision-pros-revolutionary-eye-tracking-system)
- [Eyes — Apple Developer Human Interface Guidelines](https://developer.apple.com/design/human-interface-guidelines/eyes)
- [Neuralink's big vision collides with reality of brain implants — STAT](https://www.statnews.com/2026/01/05/neuralink-brain-computer-interface-medical-device-vs-transhumanism/)
- [Synchron — The brain-computer interface device](https://synchron.com/)
- [Brain-computer interfaces face a critical test — MIT Technology Review](https://www.technologyreview.com/2025/04/01/1114009/brain-computer-interfaces-10-breakthrough-technologies-2025/)

Accessibility for AI-generated interfaces:
- [Accessible AI: Ensuring WCAG Compliance in Chatbots, Generative UIs, and Assistive Tech — A11Y Pros](https://a11ypros.com/blog/accessible-ai)
- [AAG v0.1 — Accessibility Guidelines for AI Interfaces (inspired by WCAG)](https://medium.com/@anky18milestone/aag-v0-1-accessibility-guidelines-for-ai-interfaces-inspired-by-wcag-40ab4e8badc2)
- [How WCAG Guidelines Apply to AI-Generated Content — AudioEye](https://www.audioeye.com/post/wcag-guidelines-ai-generated-content/)
