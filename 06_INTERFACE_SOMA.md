# Interface SOMA — Specification

**Status:** Design  
**Depends on:** SOMA Core, Synaptic Protocol, Plugin System  
**Blocks:** Any user-facing SOMA application

---

## 1. What the Interface SOMA Is

The Interface SOMA is a SOMA instance that runs on the user's device. Its body is the display, input methods, and device sensors. Its purpose: receive semantic signals from Backend SOMAs and render adaptive, living interfaces — AND accept conversational input to reshape those interfaces.

It is NOT a frontend framework. It is NOT a template engine. It is NOT a React replacement. It is a neural mind that composes visual (or auditory) output from intent and semantic data, using its device as its body.

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

## 3. Dual Input: Semantic Signals + Conversational

The Interface SOMA has two input sources:

### 3.1 Semantic Signals from Backend SOMAs

Backend SOMAs send structured data via Synaptic Protocol. The Interface SOMA renders it. This is the runtime mode — the app is running, data flows, UI updates.

```json
{
  "view": "contact_list",
  "data": [{"name": "Ana", "online": true, "rating": 4.8}],
  "actions": ["chat", "book"],
  "filters": ["service", "location"]
}
```

The Mind generates a program of renderer plugin conventions to display this data according to its design knowledge.

### 3.2 Conversational Input from Human

The human talks to the Interface SOMA to shape the interface. This is the design/build mode — the human describes what they want to see.

```
Human: "Show me a bottom navigation with 4 tabs: 
        Contacts, Chats, Calendar, Profile"

Mind generates program:
  $0 = dom.create("nav", {class: "bottom-nav"})
  $1 = dom.create("button", {text: "Contacts", icon: "people"})
  $2 = dom.create("button", {text: "Chats", icon: "chat"})
  $3 = dom.create("button", {text: "Calendar", icon: "calendar"})
  $4 = dom.create("button", {text: "Profile", icon: "person"})
  $5 = dom.append($0, $1)
  $6 = dom.append($0, $2)
  $7 = dom.append($0, $3)
  $8 = dom.append($0, $4)
  $9 = dom.append(root, $0)
  STOP
```

The Mind applies design knowledge (from the design plugin LoRA) to choose colors, spacing, typography, icon style — the human doesn't specify CSS.

### 3.3 Both Modes Simultaneously

In production, both inputs are active. The human can talk to the Interface SOMA while it's rendering live data:

```
Human: "Make the online status dot bigger, it's hard to see"
  → Mind generates: dom.set_style(status_dot, "width", "12px"), etc.
  → The change persists in experiential memory
  → Next time the contact list renders, the dot is bigger
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
intent> "switch to dark mode"
  → Detach light-mode design LoRA
  → Attach dark-mode design LoRA
  → Re-render current view with new design knowledge
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

### 10.3 Conversational Input in Browser

The Interface SOMA renders a persistent input field (like a chat input or command palette). The human types intents here:

```
┌────────────────────────────────────────────┐
│                                            │
│   [Rendered HelperBook interface]           │
│   Contacts, chats, calendar, etc.          │
│                                            │
├────────────────────────────────────────────┤
│ 🔧 "add a search bar at the top"     [⏎]  │
└────────────────────────────────────────────┘
```

The input is always available (toggle with a keyboard shortcut or button). Intents typed here go to the Interface SOMA's Mind, which generates DOM manipulation programs. The conversational input can be hidden in production (end users don't need it) or exposed for administrators/builders.

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
1. Download WASM + model + design LoRA   (~5-10MB, cached after first load)
2. Initialize SOMA Core                   (~200ms)
3. Load design LoRA                        (~50ms)
4. Connect to Backend SOMA                 (~100ms)
5. Receive initial semantic signal         (~50ms)
6. Mind generates initial render program   (~100ms)
7. DOM executes program                    (~50ms)
Total first meaningful paint:              ~550ms (after WASM cached)
```

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
