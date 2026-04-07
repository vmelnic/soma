# Interface SOMA — Specification

**Status:** Future Research  
**Depends on:** SOMA Core, Synaptic Protocol, Plugin System  
**Priority:** After core product works with simple renderer

---

## 0. Pragmatic vs Neural Rendering

**For Milestone 7 (getting HelperBook to users), build a simple JS/TS renderer.** It connects to the Backend SOMA via WebSocket (Synaptic Protocol), receives semantic JSON signals, and renders them with a component library styled by pencil.dev design tokens as CSS variables. No Mind. No neural inference. No WASM. Just a thin client. This works today, is fast, and is well-understood.

**This spec describes the future neural Interface SOMA** — a SOMA instance that uses its Mind to compose visual output from semantic signals. This adds value when:

- **Adaptive rendering** — the Mind makes layout decisions based on proprioception (screen size, input methods, accessibility) without explicit breakpoints or media queries
- **Design absorption** — the Mind learns a design language from .pen files as LoRA knowledge, not just CSS variables
- **Cross-platform from one model** — same Mind renders to DOM, UIKit, or Compose via different renderer plugins
- **Self-improving** — the Mind adapts rendering quality from experience

Until these capabilities are proven to outperform a well-built React app, the simple renderer is the pragmatic choice. This spec exists so the research direction is documented.

---

## 1. What the Interface SOMA Is

The Interface SOMA is a SOMA instance that runs on the user's device. Its body is the display, input methods, and device sensors. Its purpose: receive semantic signals from Backend SOMAs and render adaptive interfaces. It sends user events (taps, input, gestures) back as Synaptic signals.

It is NOT a frontend framework. It is NOT a template engine. It is NOT a conversational partner. It is a neural mind that composes visual output from semantic data, using its device as its body. Humans talk to an LLM, not to the Interface SOMA. The LLM talks to Backend SOMAs via MCP. Backend SOMAs send semantic signals to the Interface SOMA. The Interface SOMA renders.

---

## 2. Architecture

```
┌──────────────────────────────────────────────┐
│  Interface SOMA                               │
│                                               │
│  ┌────────────┐  ┌─────────────────────────┐ │
│  │ Mind Engine │  │ Synaptic Protocol       │ │
│  │ (ONNX or   │  │ Client                  │ │
│  │  WASM-opt)  │  │ (to Backend SOMAs)      │ │
│  └──────┬─────┘  └────────────┬────────────┘ │
│         │                      │              │
│  ┌──────┴──────────────────────┴────────────┐ │
│  │ Plugin Manager                            │ │
│  │ Loaded plugins:                           │ │
│  │   dom-renderer (or uikit, compose)        │ │
│  │   design (pencil.dev / Figma knowledge)   │ │
│  │   offline (local cache + queue)           │ │
│  │   crypto (session tokens)                 │ │
│  │   timer (intervals, animations)           │ │
│  └──────┬───────────────────────────────────┘ │
│         │                                     │
│  ┌──────┴──────┐  ┌────────────────────────┐ │
│  │ Memory      │  │ Proprioception         │ │
│  │ (LoRA +     │  │ (screen size, input    │ │
│  │ checkpoint)  │  │  methods, user prefs,  │ │
│  │             │  │  connection quality)    │ │
│  └─────────────┘  └────────────────────────┘ │
└──────────────────────────────────────────────┘
         │
    [Device Body]
    Screen / Speakers / Touch / Keyboard / Camera / GPS
```

---

## 3. Input: Semantic Signals from Backend SOMAs

The Interface SOMA has one input source: semantic signals arriving via Synaptic Protocol from Backend SOMAs.

```json
{
  "view": "contact_list",
  "data": [{"name": "Ana", "online": true, "rating": 4.8}],
  "actions": ["chat", "book"],
  "filters": ["service", "location"]
}
```

The Mind generates a program of renderer plugin conventions to display this data according to its design knowledge.

**There is no conversational input on the Interface SOMA.** Humans who want to change the UI talk to an LLM (Claude, ChatGPT, etc.). The LLM sends updated view specifications to the Backend SOMA via MCP. The Backend SOMA sends updated semantic signals to the Interface SOMA. The Interface SOMA renders the changes.

```
Human: "Make the online status dots bigger"
    │
    ▼
LLM: → soma.render_view({..., style_overrides: {online_status: {size: "14px"}}})
    │
    ▼
Backend SOMA → Interface SOMA: updated semantic signal via Synaptic Protocol
    │
    ▼
Interface SOMA: renders with larger dots (doesn't know WHY — just renders)
```

---

## 4. Rendering Pipeline

### 4.1 Semantic Signal → DOM Program

```
Semantic signal arrives (e.g., contact_list with 5 contacts)
       │
  [Mind Engine]
       │
  Mind accesses:
    - Design LoRA (knows color palette, spacing, component patterns)
    - Experiential LoRA (knows user preferences, past rendering decisions)
    - Proprioception (screen size, device type, accessibility settings)
       │
  Mind generates program of DOM conventions:
    dom.create, dom.set_style, dom.set_text, dom.append, dom.on_event
       │
  [DOM Renderer Plugin executes program]
       │
  [Browser renders pixels]
```

### 4.2 Component Patterns

The Mind doesn't generate every `div` from scratch. Design LoRA encodes component patterns — learned compositions that represent "a card," "a list item," "a button," "a nav bar." The Mind activates these patterns and fills them with data.

This is analogous to how a human designer doesn't decide the exact pixel for every element — they think in components and apply them to data.

### 4.3 Incremental Updates

When a semantic signal updates existing data (new message in chat, contact status change), the Mind generates a minimal DOM update program — not a full re-render:

```
Signal: {type: "status_change", user_id: "ana", online: false}

Mind generates:
  $0 = dom.query("#status-ana")
  $1 = dom.remove_class($0, "online")
  $2 = dom.add_class($0, "offline")
  STOP
```

The Mind learns efficient update patterns through experience (LoRA adaptation).

---

## 5. Design Knowledge Absorption

### 5.1 How Pencil.dev Designs Become LoRA

```
1. Designer creates UI in pencil.dev
   (components, colors, typography, spacing, layout patterns)

2. Export as .pen file (JSON-based, contains exact design tokens)

3. Design plugin parses .pen file:
   - Extract color tokens (primary: #4F46E5, surface: #FFFFFF, ...)
   - Extract typography scale (headings, body, caption sizes/weights)
   - Extract spacing system (4px grid, specific padding/margins)
   - Extract component patterns (how a "card" is structured, what a "button" looks like)
   - Extract border radius, shadows, elevation levels
   - Extract responsive breakpoints

4. Synthesizer generates training data:
   For each component pattern, generate (intent, DOM program) pairs
   that produce the correct visual output matching the design tokens.
   
   Example: "render a contact card" →
     dom.create("div", {class: "card"})
     dom.set_style($0, "background", "#FFFFFF")
     dom.set_style($0, "border-radius", "12px")
     dom.set_style($0, "padding", "16px")
     dom.set_style($0, "box-shadow", "0 2px 8px rgba(0,0,0,0.1)")
     ... (from design tokens, not hardcoded)

5. Train LoRA weights on these design-specific examples

6. Package as design plugin LoRA

7. Interface SOMA loads design LoRA → immediately renders 
   in the design language
```

### 5.2 Design Updates

When the designer updates the design in pencil.dev:

1. Export new .pen file
2. Re-generate training data with new tokens
3. Re-train design LoRA
4. Hot-load new LoRA into Interface SOMA
5. UI updates to new design — no page reload, no deploy

### 5.3 Multiple Design Themes

Dark mode is a separate design LoRA (or a LoRA variant). The Interface SOMA switches by loading a different design LoRA:

```
LLM: → soma.reload_design("helperbook-dark.pen")
Interface SOMA: detaches light-mode LoRA, attaches dark-mode LoRA, re-renders
```

---

## 6. Event Handling

### 6.1 DOM Events → Synaptic Signals

When the Mind renders interactive elements, it attaches event listeners that produce Synaptic signals:

```
Mind renders a "Book" button:
  $0 = dom.create("button", {text: "Book"})
  $1 = dom.on_event($0, "click", channel=50)
  
User clicks the button:
  → DOM event fires
  → dom-renderer plugin sends internal signal on channel 50
  → Signal Router receives it
  → Router converts to Synaptic signal to Backend SOMA:
    DATA {type: "action", action: "book", context: {contact_id: "ana"}}
```

### 6.2 Input Events

Text input, form submission, selection:

```
Mind renders a search input:
  $0 = dom.create("input", {type: "text", placeholder: "Search contacts..."})
  $1 = dom.on_event($0, "input", channel=51)  // fires on every keystroke
  
User types "plumber":
  → Debounced event (300ms) → signal on channel 51
  → Interface SOMA sends to Backend SOMA:
    INTENT {payload: "search contacts for plumber"}
  → Backend SOMA returns search results as semantic signal
  → Interface SOMA renders results
```

### 6.3 Gesture / Touch Events

On mobile Interface SOMAs:

```
dom.on_event($element, "swipe-left", channel=52)   → delete action
dom.on_event($element, "long-press", channel=53)    → context menu
dom.on_event($element, "pull-refresh", channel=54)  → refresh data
```

The gesture conventions are part of the renderer plugin (UIKit renderer knows iOS gestures, DOM renderer knows touch events).

---

## 7. Responsive Adaptation

### 7.1 Proprioception-Driven

The Interface SOMA knows its screen size, orientation, and device type through proprioception. It uses this when generating rendering programs:

```
Proprioception: {
  screen_width: 375,
  screen_height: 812,
  device_type: "phone",
  orientation: "portrait",
  pixel_ratio: 3,
  prefers_dark_mode: false,
  prefers_reduced_motion: true,
  font_scale: 1.0,
  accessibility: { screen_reader: false, high_contrast: false }
}
```

### 7.2 How It Affects Rendering

The Mind generates different programs for different bodies:

**Phone (375px):** Contact list as vertical scroll, one card per row, compact layout.

**Tablet (1024px):** Contact list as grid, 2-3 cards per row, more detail visible.

**Desktop (1920px):** Contact list as sidebar, chat view beside it, full detail.

This isn't media queries. The Mind makes a neural decision based on proprioception. It has learned (via design LoRA + experience) what works on each screen size.

### 7.3 Orientation Change

```
Screen rotates landscape → portrait:
  Proprioception updates: orientation = "portrait"
  Mind re-evaluates current view
  Generates incremental DOM update to reflow layout
  No full re-render — just restructure
```

---

## 8. Accessibility

### 8.1 Built-In, Not Retrofitted

The Mind generates accessible output by default — not as an afterthought:

```
Mind renders a button:
  $0 = dom.create("button", {})
  $1 = dom.set_attr($0, "role", "button")
  $2 = dom.set_attr($0, "aria-label", "Book appointment with Ana")
  $3 = dom.set_attr($0, "tabindex", "0")
  ...
```

Accessibility attributes are part of the rendering program because the design LoRA includes accessibility patterns.

### 8.2 Screen Reader Mode

If proprioception detects a screen reader is active:

```
Proprioception: { accessibility: { screen_reader: true } }
```

The Mind generates a different rendering program — optimized for linear reading order, semantic landmarks, live region announcements:

```
dom.set_attr($list, "role", "list")
dom.set_attr($list, "aria-label", "Contacts")
dom.set_attr($item, "role", "listitem")
dom.create("div", {role: "status", "aria-live": "polite", text: "5 contacts online"})
```

### 8.3 Font Scaling

If `font_scale > 1.0` (user has enlarged text in system settings), the Mind adjusts all text sizes proportionally. Layouts that would overflow are restructured.

---

## 9. Offline Behavior

### 9.1 Offline Cache Plugin

The offline plugin caches:
- Last semantic signal per view (contact_list, chat messages, calendar events)
- Static assets (design tokens, icons)
- Queued outbound signals (messages sent while offline)

### 9.2 Rendering Offline

```
App opens, no network:
  1. Interface SOMA detects: no Synaptic connection
  2. Proprioception: { network: "offline" }
  3. Mind renders from cached semantic data
  4. Shows offline indicator (Mind knows to show it because proprioception says offline)
  5. User can browse cached content, compose messages
  
Network returns:
  1. Synaptic connection established
  2. Offline plugin flushes queued signals
  3. Backend SOMA sends fresh data
  4. Mind generates incremental updates (diff cached vs fresh)
  5. Offline indicator disappears
```

---

## 10. Browser Deployment (WASM)

### 10.1 How It Runs in a Browser

The Interface SOMA compiles to WebAssembly:

```
soma-interface.wasm    (~2-5MB)  — SOMA Core + Mind Engine + Plugin Manager
models/                           — ONNX model (quantized for browser)
plugins/
  dom-renderer.wasm              — DOM manipulation
  design.wasm                    — Design knowledge LoRA
  offline.wasm                   — IndexedDB cache
```

Loaded by a minimal HTML bootstrap:

```html
<!DOCTYPE html>
<html>
<head><title>HelperBook</title></head>
<body>
  <div id="soma-root"></div>
  <script type="module">
    import init from './soma-interface.js';
    await init();
    // SOMA takes over #soma-root
    // Renders everything from Mind + Design LoRA
    // Connects to Backend SOMA via WebSocket 
    // (Synaptic Protocol over WebSocket transport)
  </script>
</body>
</html>
```

### 10.2 Synaptic Protocol in Browser

Browsers can't open raw TCP sockets. The Synaptic Protocol uses WebSocket as transport:

```
Browser Interface SOMA ←─ WebSocket ─→ Backend SOMA (ws-bridge plugin)
                                        │
                           Synaptic Protocol frames
                           carried inside WebSocket messages
```

The ws-bridge is a thin adapter on the Backend SOMA side. The Interface SOMA doesn't know it's using WebSocket — it sends Synaptic signals, and the transport layer handles the WebSocket wrapping.

---

## 11. Mobile Deployment

### 11.1 Native Shell + WASM Core

Mobile Interface SOMA runs as a native app shell with WASM core:

```
iOS App:
  SwiftUI shell (navigation, status bar, push handling)
    └── WKWebView or WASM runtime
        └── Interface SOMA (same WASM as browser)
            └── Renderer plugin: DOM (in WebView) or UIKit (native, future)

Android App:
  Jetpack Compose shell (navigation, status bar, push handling)
    └── WebView or WASM runtime
        └── Interface SOMA (same WASM as browser)
            └── Renderer plugin: DOM (in WebView) or Compose (native, future)
```

### 11.2 Native Renderer Plugins (Future)

For fully native feel, renderer plugins that target UIKit/SwiftUI (iOS) and Jetpack Compose (Android) directly:

```
Mind generates: create("list_item", {text: "Ana", subtitle: "Stylist"})

DOM renderer:     → <div class="list-item">...</div>
UIKit renderer:   → UITableViewCell with textLabel + detailTextLabel
Compose renderer: → ListItem(headlineContent = "Ana", supportingContent = "Stylist")
```

Same Mind, same program, different renderer. The renderer plugin is the only thing that changes between platforms.

---

## 12. State Management

### 12.1 No External State Store

There is no Redux, no Vuex, no external state management library. The Interface SOMA's state is:

- **Rendering state:** current DOM tree (tracked by dom-renderer plugin via handles)
- **View state:** which view is active, scroll position, form values (in working memory)
- **Cached data:** last semantic signal per view (in offline plugin)
- **User preferences:** accumulated in experiential LoRA
- **Design knowledge:** in design LoRA

### 12.2 Navigation

Navigation between views (Contacts → Chat → Calendar) is an intent:

```
User taps "Chats" tab:
  → dom.on_event fires on channel 60
  → Interface SOMA Mind receives: {action: "navigate", target: "chats"}
  → Mind clears current view: dom.remove(current_content)
  → Mind requests data: sends INTENT to Backend SOMA "get my chats"
  → Backend responds with semantic signal (chat list)
  → Mind renders chat list from semantic data + design LoRA
```

Back navigation, deep linking, and history are managed by the Mind's working memory (which view was previous) and the browser's history API (via dom-renderer conventions).

---

## 13. Performance

### 13.1 Initial Load

```
1. Download WASM + model + design LoRA   (~1-3MB, cached after first load)
2. Initialize SOMA Core                   (~200ms)
3. Load design LoRA                        (~50ms)
4. Connect to Backend SOMA                 (~100ms)
5. Receive initial semantic signal         (~50ms)
6. Mind generates initial render program   (~100ms)
7. DOM executes program                    (~50ms)
Total first meaningful paint:              ~550ms (after WASM cached)
First visit (uncached, fast connection):   ~2-3s (download + init)
```

**Compare to simple JS renderer:** ~100ms first paint (no WASM, no Mind inference). The neural Interface SOMA trades initial speed for adaptive capabilities. For most applications, the simple renderer is sufficient.

### 13.2 Subsequent Renders

Incremental updates (new message, status change): 10-50ms (Mind inference + DOM update).

Full view switch (Contacts → Chat): 100-200ms (Mind inference + DOM clear + DOM create).

### 13.3 Optimization

- Model quantization (int8) for browser reduces WASM size and inference time
- DOM operations are batched (Mind generates full program, DOM renderer executes all at once, single browser reflow)
- Design LoRA is small (~100KB) and loaded once
- Semantic signals are compact (MessagePack, not JSON)

---

## 14. Testing

### 14.1 Visual Regression

Render the same semantic signal on the same screen size → compare screenshot. Changes in Mind weights or design LoRA should produce intentional visual changes, not regressions.

### 14.2 Accessibility Testing

Automated: run axe-core against rendered DOM after each major render. All critical views must pass WCAG 2.1 AA.

### 14.3 Cross-Device Testing

Same semantic signal rendered on: phone (375px), tablet (768px), desktop (1440px). Verify each produces a usable layout. No overflow, no cut-off text, no overlapping elements.

### 14.4 Offline Testing

1. Load app online, navigate to several views (cache populates)
2. Disconnect network
3. Navigate between views — all render from cache
4. Send a message — queued
5. Reconnect — message sends, data refreshes

---

## 15. Render State Tracking

### 15.1 The Problem

The Mind generates `dom.create("button", {text: "Book"})` and gets back Handle #42. Later the user says "make the book button bigger." The Mind needs to know: which handle is "the book button"?

The Mind doesn't remember raw handle numbers across intents. It needs a semantic render tree — a mapping from meaning to DOM handles.

### 15.2 Render Tree

The Interface SOMA maintains a render tree in working memory:

```rust
pub struct RenderTree {
    nodes: HashMap<String, RenderNode>,  // semantic_id → node
}

pub struct RenderNode {
    pub semantic_id: String,       // "contact_card:ana", "nav:chats_tab", "chat:input"
    pub dom_handle: Handle,        // from dom-renderer plugin
    pub element_type: String,      // "button", "div", "input", etc.
    pub children: Vec<String>,     // child semantic_ids
    pub parent: Option<String>,    // parent semantic_id
    pub data_binding: Option<String>, // "contacts[0].name" — what data this renders
    pub metadata: HashMap<String, String>, // arbitrary tags
}
```

### 15.3 How the Mind Uses It

When the Mind generates rendering programs, it also updates the render tree:

```
Semantic signal: {view: "contact_list", data: [{id: "ana", name: "Ana", ...}]}

Mind generates:
  $0 = dom.create("div", {})           → render_tree.add("contact_card:ana", $0)
  $1 = dom.create("span", {text: "Ana"}) → render_tree.add("contact_card:ana:name", $1)
  $2 = dom.create("span", {class: "status online"}) → render_tree.add("contact_card:ana:status", $2)
  $3 = dom.append($0, $1)
  $4 = dom.append($0, $2)
  ...
```

When the user says "make the online status dots bigger":

```
Mind queries render tree: find all nodes where semantic_id matches "*:status"
  → finds ["contact_card:ana:status", "contact_card:ion:status", ...]
  → gets their DOM handles
  
Mind generates:
  for each status handle:
    dom.set_style(handle, "width", "14px")
    dom.set_style(handle, "height", "14px")
```

### 15.4 Render Tree Persistence

The render tree is part of working memory (per-session, not checkpointed). On page reload, the entire render tree is rebuilt from the current semantic signals. This is fast because the Mind regenerates the full view from cached semantic data.

### 15.5 Data Binding

The `data_binding` field links render nodes to semantic data fields. When an incremental update arrives (e.g., "Ana is now offline"), the Mind can:

1. Find all nodes bound to `contacts[id=ana].online`
2. Update those specific nodes
3. No full re-render needed

This is SOMA's equivalent of React's virtual DOM diffing — but driven by the Mind's semantic understanding rather than a tree-diff algorithm.

---

## 16. Error, Loading, and Empty States

### 16.1 Loading State

When a semantic signal is requested but hasn't arrived yet:

```
User navigates to Contacts tab:
  1. Mind sends INTENT to Backend: "get my contacts"
  2. While waiting (Backend hasn't responded):
     Mind knows: render loading state
     
     Mind generates:
       $0 = dom.create("div", {class: "loading"})
       $1 = dom.create("div", {class: "skeleton-card"})  × 3  // placeholder cards
       $2 = dom.append(container, $0, $1, ...)
       
  3. Backend responds with semantic signal:
     Mind generates: remove loading state, render real data
```

The Mind learns loading patterns from design LoRA (which includes skeleton/shimmer component patterns) and from training data that includes the "show loading while waiting" pattern.

### 16.2 Error State

When the Backend SOMA returns an error signal:

```
Backend → Interface: DATA {
  error: { code: "DB_UNAVAILABLE", message: "Database connection failed" }
}

Mind generates:
  $0 = dom.create("div", {class: "error-state"})
  $1 = dom.create("span", {text: "Something went wrong"})
  $2 = dom.create("p", {text: "Unable to load contacts. Check your connection."})
  $3 = dom.create("button", {text: "Try Again"})
  $4 = dom.on_event($3, "click", channel=70)  // retry channel
  ...
```

Error rendering is a learned pattern — the design LoRA includes error state components, and training data includes error-handling examples.

### 16.3 Empty State

When data is valid but empty:

```
Backend → Interface: DATA {
  view: "contact_list",
  data: [],
  empty_reason: "no_connections"
}

Mind generates:
  $0 = dom.create("div", {class: "empty-state"})
  $1 = dom.create("span", {text: "No contacts yet"})
  $2 = dom.create("p", {text: "Search the network to find service providers."})
  $3 = dom.create("button", {text: "Browse Network"})
  ...
```

### 16.4 State Transitions

The Mind handles transitions: loading → data, loading → error, data → loading (refresh), error → loading (retry). Each transition is an incremental DOM update (remove old state, render new state), not a full page rebuild.

---

## 17. Form Handling

### 17.1 Form Rendering

When a semantic signal describes a form:

```json
{
  "view": "form",
  "form_id": "create_appointment",
  "fields": [
    {"name": "service", "type": "select", "options": ["Hair Styling", "Coloring"], "required": true},
    {"name": "date", "type": "date", "min": "2026-04-08", "required": true},
    {"name": "time", "type": "time", "required": true},
    {"name": "notes", "type": "textarea", "required": false, "placeholder": "Any special requests?"}
  ],
  "submit_action": "create_appointment"
}
```

The Mind generates input elements with appropriate types, labels, required indicators, and validation attributes — all styled per design LoRA.

### 17.2 Client-Side Validation

The Mind generates validation logic as event handlers:

```
For each required field:
  dom.on_event(field, "blur", channel=80)   // validate on focus loss
  
On blur event:
  Mind checks: is the field empty and required?
  If yes: dom.add_class(field, "error"), dom.set_text(error_label, "Required")
  If no: dom.remove_class(field, "error")
```

Validation patterns are part of training data — the Mind learns: "required fields show errors when empty," "email fields validate format," "date fields validate range."

### 17.3 Form Submission

```
User clicks Submit:
  1. dom.on_event fires on submit channel
  2. Mind collects all field values:
     $0 = dom.get_value(service_field)
     $1 = dom.get_value(date_field)
     $2 = dom.get_value(time_field)
     $3 = dom.get_value(notes_field)
  3. Mind validates all fields (client-side)
  4. If valid: send to Backend SOMA:
     DATA {type: "create_appointment", data: {service: $0, date: $1, ...}}
  5. Show loading state on submit button
  6. Backend responds: success → navigate to calendar, error → show error
```

### 17.4 Multi-Step Forms

For complex flows (like provider onboarding):

```json
{
  "view": "multi_step_form",
  "current_step": 2,
  "total_steps": 4,
  "steps": ["Basic Info", "Services", "Schedule", "Gallery"],
  "fields": { /* fields for current step */ },
  "progress": 0.5
}
```

The Mind renders: progress indicator, current step's fields, back/next buttons. Navigation between steps is local (no Backend call until final submit). Form state is in working memory.

### 17.5 File Inputs

For profile photo, gallery uploads:

```
User taps "Upload Photo":
  1. Mind generates: dom.create("input", {type: "file", accept: "image/*"})
  2. User selects file
  3. dom.on_event fires with file metadata
  4. Interface SOMA reads file bytes
  5. Shows preview (using canvas or img element with Object URL)
  6. Initiates chunked upload to Backend SOMA (CHUNK_START → CHUNK_DATA → CHUNK_END)
  7. Backend processes (thumbnail, EXIF strip, store)
  8. Backend sends back confirmation with thumbnail URL
  9. Interface SOMA updates preview with final thumbnail
```

---

## 18. Navigation and Routing

### 18.1 URL Mapping

For browser-based Interface SOMAs, views map to URLs for bookmarkability and browser history:

```
/contacts          → {view: "contact_list", sub_tab: "contacts"}
/contacts/network  → {view: "contact_list", sub_tab: "network"}
/chat/user_abc     → {view: "chat", peer: {id: "user_abc"}}
/calendar          → {view: "calendar"}
/calendar/2026-04  → {view: "calendar", month: "2026-04"}
/profile           → {view: "profile"}
/profile/settings  → {view: "settings"}
```

### 18.2 Navigation Flow

```
User taps "Chats" tab:
  1. dom.on_event fires (click on chats tab)
  2. Mind decides: navigate to chats view
  3. Mind updates render tree: deactivate current tab, activate chats
  4. Mind calls: dom.set_attr(window, "pushState", "/chats")  // browser history
  5. Mind sends to Backend: INTENT "get my chats"
  6. Mind renders loading state in content area
  7. Backend responds with chat list
  8. Mind renders chat list, removes loading state
```

### 18.3 Browser Back/Forward

```
User clicks browser back button:
  1. "popstate" event fires → dom.on_event channel
  2. Mind reads: new URL = /contacts
  3. Mind navigates to contacts view (same flow as tab tap)
  4. Uses cached data if available (offline plugin), otherwise fetches
```

### 18.4 Deep Links

```
User opens: helperbook.app/chat/user_abc

1. Interface SOMA boots
2. Reads URL: /chat/user_abc
3. Mind: this is a chat view for user_abc
4. Sends to Backend: INTENT "get chat with user_abc"
5. Renders chat directly (skips contacts/chats list)
```

### 18.5 Mobile Navigation

On mobile Interface SOMAs (native shell), navigation uses the platform's navigation stack instead of URL pushState:

- iOS: UINavigationController push/pop
- Android: NavController navigate/popBackStack

The Mind generates the same navigation intents. The renderer plugin translates to platform-specific navigation.

---

## 19. Client-Side Security

### 19.1 Session Token Storage

The auth token (from Backend SOMA) must be stored securely on the client:

| Platform | Storage | Security |
|---|---|---|
| Browser | HttpOnly cookie (if via http-bridge) or `sessionStorage` | XSS can't read HttpOnly cookies. sessionStorage clears on tab close. |
| iOS | Keychain | Hardware-backed encryption |
| Android | EncryptedSharedPreferences | AndroidKeyStore-backed |
| Desktop | OS credential store (Keyring/Credential Manager) | OS-level encryption |

The offline plugin manages token storage. The Mind never handles raw tokens — it calls `auth.get_token()` which the plugin resolves from secure storage.

### 19.2 XSS Prevention

The Mind generates DOM programs. If it uses `dom.set_html(element, untrusted_data)`, that's an XSS vector — the untrusted data could contain `<script>` tags.

**Rules enforced by the DOM renderer plugin:**

1. **`dom.set_text()` is always safe.** Text content is automatically escaped. The Mind should use `set_text` for any user-provided content.

2. **`dom.set_html()` is restricted.** The DOM renderer plugin strips all `<script>` tags, inline event handlers (`onclick`, `onerror`, etc.), and dangerous attributes before inserting HTML. This is a built-in sanitizer, not a Mind responsibility.

3. **`dom.set_attr()` validates attribute names.** The plugin refuses to set `onclick`, `onerror`, `onload` or any `on*` attribute. Event handling goes through `dom.on_event()` only.

4. **URLs are validated.** `dom.set_attr(img, "src", url)` — the plugin validates that `url` starts with `https://`, `http://`, `data:image/`, or `soma://`. No `javascript:` URLs.

### 19.3 Content Security Policy

The browser bootstrap page includes a strict CSP:

```html
<meta http-equiv="Content-Security-Policy" 
  content="default-src 'self' 'wasm-unsafe-eval'; 
           connect-src wss://*.helperbook.app; 
           img-src 'self' soma: https:; 
           style-src 'self' 'unsafe-inline';
           script-src 'self' 'wasm-unsafe-eval';">
```

This prevents: external script injection, unauthorized network connections, data exfiltration via image requests to attacker servers.

### 19.4 Synaptic Connection Security

The Interface SOMA only connects to Backend SOMAs listed in its configuration. It does NOT connect to arbitrary peers discovered via broadcast. This prevents a rogue SOMA from injecting signals into the Interface.

```toml
# interface-soma.toml
[protocol.peers]
# Only these backends are trusted
backend = "wss://api.helperbook.app/synaptic"

[protocol.security]
allow_discovery = false    # do not accept connections from unknown peers
require_encryption = true
```