# Future HCI: Research Notes and Architectural Framing

**Status.** Research notes plus an architectural thesis and a proposed phase
plan. Not implemented. Snapshot dated 2026-04-11. Belongs in the same
category as `memory-fusion.md` and `semantic-memory.md`: a design direction,
not a commitment. The "industry state" section will go stale — the framing
section should age better.

**Context.** The question this document answers: should SOMA pursue the
frontend / future-web-interaction direction, and if so what does the
architecture look like? The short answer: the industry is independently
converging on a pattern SOMA is already shaped like, the web is only one of
several converging surfaces, and the right framing is **SOMA as a universal
runtime for intent-driven human-computer interaction** — with the web as one
output modality among several.

## Industry state (Q2 2026)

Four convergent trends plus one open problem.

### 1. Generative UI has shipped

Generative UI (GenUI) — where parts of the interface are composed by an AI
agent at runtime rather than hard-coded — has moved from experimental demos
to production in 2026. Three reference points from the research:

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

OpenAI is shipping a screenless ambient AI gadget in Fall 2026 — the entire
device forces a shift to high-intent voice interaction, explicitly designed
around moving away from scrolling and tapping. The design thesis everyone's
writing about:

> **"Interaction → intention."** Not "press button to do thing," but
> "state what you want and let the system figure out the steps."

Interfaces drawn on demand, dismissed when done, no persistent navigation.
"Zero UI" as a term covers voice, gesture, gaze, and sensor-driven flows
that replace explicit UI elements entirely.

### 3. Adaptive UI from user patterns

Jakob Nielsen's 2026 predictions call this out as the major shift. LLMs
observe user behavior, predict intent from recent actions and context, and
**generate bespoke micro-interfaces featuring only the relevant details for
the current task**. Production reports cite ~30% engagement lift and
measurable latency reductions when predictive personalization loops are
integrated into front-end adaptation. Key idea: every user's UI should be
different because every user's task is different.

### 4. Input modalities are fragmenting

- **Eye tracking** is production today. Apple Vision Pro uses gaze + pinch
  as its primary paradigm, ~1.1° gaze accuracy, with a deliberate privacy
  rule that gaze data is only exposed to apps when the user intentionally
  confirms with a gesture.
- **Voice + text** is the hinge modality — production today, usable
  alongside screens, bridge toward the screenless future.
- **BCI** is clinical, not consumer. Neuralink has three N1 implant
  recipients (quadriplegic / ALS). Synchron demoed iPad control in
  August 2025. Analyst consensus puts consumer-grade BCI launch around
  2030.

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

Every framework invents its own JSON schema. SOMA now consumes MCP
(via `McpClientPort`, see the 2026-04-11 commit), which is becoming the
de facto universal brain↔body contract.

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

Three pieces of existing evidence:

1. The **ESP32 thermistor → display loop** (proven, 2026-03) — sensors and
   renderers are indistinguishable to the brain.
2. The **`McpClient` port backend** (proven, 2026-04-11) — any MCP server
   in any language is a port; MCP is a universal contract.
3. The **multistep routine pipeline** (proven, 2026-04) — repeated
   interactions compile to plans that walk without the LLM.

Each one is saying the same thing: the body part and the brain part are
separable, the body is modality-agnostic, the brain can be any LLM, and
intent is the contract between them.

## Proposed phase plan

Earlier drafts of the phase plan were web-centric. The research pushes
toward a wider scoping where the browser is phase 1 but not the endpoint.

### Phase 0 — `soma-next` in WASM

- `cargo build --target wasm32-unknown-unknown --no-default-features`
  with feature flags disabling native-thread-dependent code.
- Swap tokio for a single-threaded browser executor.
- Proof: `create_goal` + `dump_state` round-trip in a browser tab.
- Output: ~1-2 MB gzipped WASM blob.

### Phase 1 — Input and output ports for the browser surface

Reuse the `McpClient` backend we already shipped. Each port is an in-tab
MCP server (JavaScript) talking to the WASM SOMA via a browser-side transport
variant.

- `dom` port (output) — `create_element`, `set_text`, `set_attr`, `on_event`.
- `audio` port (output) — `say_text` via Web Speech API.
- `voice` port (input) — microphone → transcript via Web Speech API or a
  local model.
- `keyboard` port (input) — text entry.

Proof: a pack where the user says *"hello"* via microphone, the LLM composes
`dom.create_element("h1", "hello marcu")`, the screen shows it AND the
speaker says it. One pack, dual output surfaces, voice-in.

### Phase 2 — Learned routines across modalities

Widen the multistep proof from filesystem to HCI. A pack learns:
*"when user says 'show my appointments'"* → query port → dom render → audio
announce. First utterance: LLM deliberates. Tenth utterance: compiled
routine walks without LLM.

Proof: a voice+screen flow where the Nth invocation is measurably faster
than the first.

### Phase 3 — One real multi-modal surface

Pick a HelperBook view (probably the appointment list) and render it
simultaneously through `dom`, `audio`, and `keyboard` ports. Same pack,
same intent, three output modalities.

This is the accessibility demo and the ambient computing demo at the same
time.

### Phase 4 — Manifest as canonical semantic layer

Add rendering ports that take current belief/manifest and emit:
- `audio.narrate` (manifest → structured speech)
- `braille.render` (manifest → braille cell grid)

Prove the same runtime serves sighted, blind, and deaf users from one
manifest with no per-modality authoring.

### Deferred (not a phase)

- **BCI input.** When consumer SDKs appear (~2030 estimate) it is just
  another input port. The runtime does not need to change.
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
