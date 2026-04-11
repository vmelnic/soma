// SOMA TERMINAL — frontend bootstrap + auth + contexts + chat.
//
// Commit 1 wired the Fallout shell to /api/auth/*. Commit 2 added
// the contexts registry. Commit 3 adds:
//   - the per-context chat transcript (brain side)
//   - the browser-side soma-next runtime panel (body side)
//
// No framework. Vanilla DOM manipulation — every element is created
// with `document.createElement` and every value is set via
// `textContent`, never innerHTML, so a maliciously named context
// can't break out of the terminal.

import {
  bootForContext,
  getBootError,
  injectRoutine,
  executeSkill,
  getCurrentPackSpec,
} from "./runtime.mjs";

const views = {
  loading: document.getElementById("view-loading"),
  requestLink: document.getElementById("view-request-link"),
  linkSent: document.getElementById("view-link-sent"),
  authenticated: document.getElementById("view-authenticated"),
  contextDetail: document.getElementById("view-context-detail"),
  error: document.getElementById("view-error"),
};

const footerStatus = document.getElementById("footer-status");

// The currently open context, if any. Populated by `loadContextView`
// and cleared on logout / back.
let currentContext = null;

function showView(name) {
  for (const [key, el] of Object.entries(views)) {
    if (el) el.classList.toggle("hidden", key !== name);
  }
}

function setStatus(text) {
  footerStatus.textContent = text;
}

function showError(detail) {
  document.getElementById("error-detail").textContent = detail;
  showView("error");
  setStatus("ERROR");
}

function clearChildren(el) {
  while (el.firstChild) el.removeChild(el.firstChild);
}

// -------------------------------------------------------------------
// boot sequence — check session on load
// -------------------------------------------------------------------

async function boot() {
  setStatus("BOOTING");
  // Retro boot-print effect: a short pause so it feels like the
  // terminal is powering on. Kept short so Playwright tests don't
  // wait forever.
  await new Promise((r) => setTimeout(r, 250));

  try {
    const res = await fetch("/api/me", { credentials: "include" });
    if (res.ok) {
      const body = await res.json();
      if (body.status === "ok" && body.user) {
        await enterAuthenticated(body.user);
        return;
      }
    }
  } catch (err) {
    console.warn("[terminal] /api/me failed:", err.message);
  }
  enterRequestLink();
}

// -------------------------------------------------------------------
// view: request magic link
// -------------------------------------------------------------------

function enterRequestLink() {
  currentContext = null;
  showView("requestLink");
  setStatus("AWAITING OPERATOR EMAIL");
  const input = document.getElementById("input-email");
  if (input) input.focus();
}

document
  .getElementById("form-request-link")
  .addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const email = document.getElementById("input-email").value.trim();
    const statusEl = document.getElementById("status-request");
    statusEl.classList.remove("hidden", "error");
    statusEl.textContent = "> TRANSMITTING REQUEST...";
    setStatus("TRANSMITTING");

    try {
      const res = await fetch("/api/auth/request-link", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ email }),
      });
      const body = await res.json();
      if (!res.ok || body.status !== "dispatched") {
        statusEl.classList.add("error");
        statusEl.textContent = `> TRANSMISSION FAILED: ${body.error || res.status}`;
        setStatus("ERROR");
        return;
      }
      enterLinkSent(email);
    } catch (err) {
      statusEl.classList.add("error");
      statusEl.textContent = `> NETWORK ERROR: ${err.message}`;
      setStatus("ERROR");
    }
  });

// -------------------------------------------------------------------
// view: link sent
// -------------------------------------------------------------------

function enterLinkSent(email) {
  document.getElementById("sent-to").textContent = email;
  showView("linkSent");
  setStatus("LINK DISPATCHED — AWAITING VERIFICATION");
}

document.getElementById("btn-back").addEventListener("click", () => {
  enterRequestLink();
});

// -------------------------------------------------------------------
// view: authenticated — contexts list + create form
// -------------------------------------------------------------------

async function enterAuthenticated(user) {
  document.getElementById("user-email").textContent = user.email;
  showView("authenticated");
  setStatus("AUTHORIZED");
  await refreshContexts();
  const nameInput = document.getElementById("input-context-name");
  if (nameInput) nameInput.focus();
}

function renderEmptyContexts(listEl, message) {
  clearChildren(listEl);
  const p = document.createElement("p");
  p.className = "meta";
  p.id = "contexts-empty";
  p.textContent = message;
  listEl.appendChild(p);
}

function renderContextEntry(listEl, ctx, idx) {
  const entry = document.createElement("button");
  entry.type = "button";
  entry.className = "context-entry";
  entry.dataset.contextId = ctx.id;

  const index = document.createElement("span");
  index.className = "ctx-index";
  index.textContent = `[${String(idx + 1).padStart(3, "0")}]`;

  const title = document.createElement("span");
  title.className = "ctx-title";
  title.textContent = ctx.name;

  const desc = document.createElement("span");
  desc.className = "ctx-desc";
  desc.textContent = ctx.description || "";

  entry.appendChild(index);
  entry.appendChild(title);
  entry.appendChild(desc);
  entry.addEventListener("click", () => loadContextView(ctx.id));
  listEl.appendChild(entry);
}

async function refreshContexts() {
  const listEl = document.getElementById("contexts-list");
  try {
    const res = await fetch("/api/contexts", { credentials: "include" });
    if (!res.ok) {
      renderEmptyContexts(listEl, `FAILED TO LOAD CONTEXTS — ${res.status}`);
      return;
    }
    const body = await res.json();
    const rows = body.contexts ?? [];
    if (rows.length === 0) {
      renderEmptyContexts(
        listEl,
        "NO CONTEXTS FOUND ON THIS OPERATOR PROFILE.",
      );
      return;
    }
    clearChildren(listEl);
    rows.forEach((ctx, idx) => renderContextEntry(listEl, ctx, idx));
  } catch (err) {
    renderEmptyContexts(listEl, `NETWORK ERROR — ${err.message}`);
  }
}

document
  .getElementById("form-create-context")
  .addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const nameEl = document.getElementById("input-context-name");
    const descEl = document.getElementById("input-context-description");
    const statusEl = document.getElementById("status-create-context");

    const name = nameEl.value.trim();
    const description = descEl.value.trim();
    statusEl.classList.remove("hidden", "error");
    statusEl.textContent = "> REGISTERING CONTEXT...";
    setStatus("REGISTERING");

    try {
      const res = await fetch("/api/contexts", {
        method: "POST",
        credentials: "include",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name, description }),
      });
      const body = await res.json();
      if (!res.ok || body.status !== "ok") {
        statusEl.classList.add("error");
        statusEl.textContent = `> FAILED: ${body.error || res.status}`;
        setStatus("ERROR");
        return;
      }
      nameEl.value = "";
      descEl.value = "";
      statusEl.textContent = `> CONTEXT ${body.context.name} REGISTERED`;
      setStatus("AUTHORIZED");
      await refreshContexts();
    } catch (err) {
      statusEl.classList.add("error");
      statusEl.textContent = `> NETWORK ERROR: ${err.message}`;
      setStatus("ERROR");
    }
  });

// -------------------------------------------------------------------
// commit 7 — mic button → MediaRecorder → /api/transcribe → input
// -------------------------------------------------------------------
//
// Single-button toggle: idle → recording → transcribing → idle.
// We hold the active MediaRecorder + stream in closure state so a
// second click on the same button can stop the recording cleanly.
// On stop, the accumulated blob is POSTed to /api/transcribe (raw
// body, audio/* content-type) and the returned text is dropped
// into #input-chat so the operator can review and hit TRANSMIT.
//
// If the browser doesn't support MediaRecorder or getUserMedia,
// the button is marked `.unsupported` and refuses to start — no
// SpeechRecognition fallback yet; that's a separate commit.

let voiceRecorder = null;
let voiceStream = null;
let voiceChunks = [];

function setMicState(state) {
  const btn = document.getElementById("btn-mic");
  if (!btn) return;
  btn.classList.remove("recording", "transcribing", "unsupported");
  if (state === "recording") {
    btn.classList.add("recording");
    btn.textContent = "[ STOP ]";
  } else if (state === "transcribing") {
    btn.classList.add("transcribing");
    btn.textContent = "[ TRANSCRIBING... ]";
  } else if (state === "unsupported") {
    btn.classList.add("unsupported");
    btn.textContent = "[ MIC UNSUPPORTED ]";
    btn.disabled = true;
  } else {
    btn.textContent = "[ MIC ]";
  }
}

function micSupported() {
  return (
    typeof window !== "undefined" &&
    typeof window.MediaRecorder !== "undefined" &&
    typeof navigator !== "undefined" &&
    navigator.mediaDevices &&
    typeof navigator.mediaDevices.getUserMedia === "function"
  );
}

async function startVoiceRecording() {
  if (!micSupported()) {
    setMicState("unsupported");
    return;
  }
  voiceChunks = [];
  try {
    voiceStream = await navigator.mediaDevices.getUserMedia({ audio: true });
  } catch (err) {
    const statusEl = document.getElementById("chat-status");
    statusEl.classList.add("error");
    statusEl.textContent = `MIC BLOCKED: ${err.message}`;
    return;
  }
  voiceRecorder = new MediaRecorder(voiceStream);
  voiceRecorder.ondataavailable = (ev) => {
    if (ev.data && ev.data.size > 0) voiceChunks.push(ev.data);
  };
  voiceRecorder.onstop = onVoiceRecordingStop;
  voiceRecorder.start();
  setMicState("recording");
  setStatus("DICTATING");
}

function stopVoiceRecording() {
  if (voiceRecorder && voiceRecorder.state !== "inactive") {
    voiceRecorder.stop();
  }
  if (voiceStream) {
    for (const track of voiceStream.getTracks()) track.stop();
    voiceStream = null;
  }
}

async function onVoiceRecordingStop() {
  setMicState("transcribing");
  setStatus("TRANSCRIBING");
  const mimeType = voiceRecorder?.mimeType || "audio/webm";
  const blob = new Blob(voiceChunks, { type: mimeType });
  voiceRecorder = null;
  voiceChunks = [];
  if (voiceStream) {
    for (const track of voiceStream.getTracks()) track.stop();
    voiceStream = null;
  }
  const statusEl = document.getElementById("chat-status");
  try {
    const res = await fetch("/api/transcribe", {
      method: "POST",
      credentials: "include",
      headers: { "Content-Type": mimeType },
      body: blob,
    });
    const body = await res.json();
    if (!res.ok || body.status !== "ok") {
      statusEl.classList.add("error");
      statusEl.textContent = `TRANSCRIBE FAILED: ${body.error || res.status}`;
      setStatus("ERROR");
      setMicState("idle");
      return;
    }
    const inputEl = document.getElementById("input-chat");
    // Append to any existing draft so hitting mic mid-typing
    // doesn't clobber what the operator already wrote.
    const existing = (inputEl.value || "").trim();
    inputEl.value = existing ? `${existing} ${body.text}` : body.text;
    inputEl.focus();
    statusEl.classList.remove("error");
    statusEl.textContent = `dictated via ${body.model || "whisper"}`;
    setStatus("AUTHORIZED");
  } catch (err) {
    statusEl.classList.add("error");
    statusEl.textContent = `TRANSCRIBE NETWORK ERROR: ${err.message}`;
    setStatus("ERROR");
  } finally {
    setMicState("idle");
  }
}

// Initialise the mic button once at module load. If MediaRecorder
// isn't available we mark the button unsupported; otherwise the
// click handler toggles between start and stop.
(function initMicButton() {
  const btn = document.getElementById("btn-mic");
  if (!btn) return;
  if (!micSupported()) {
    setMicState("unsupported");
    return;
  }
  btn.addEventListener("click", async () => {
    if (voiceRecorder && voiceRecorder.state === "recording") {
      stopVoiceRecording();
    } else {
      await startVoiceRecording();
    }
  });
})();

// -------------------------------------------------------------------
// view: context detail
// -------------------------------------------------------------------

async function loadContextView(contextId) {
  setStatus("LOADING CONTEXT");
  try {
    const res = await fetch(
      `/api/contexts/${encodeURIComponent(contextId)}`,
      { credentials: "include" },
    );
    if (!res.ok) {
      const body = await res.json().catch(() => ({}));
      showError(`FAILED TO LOAD CONTEXT — ${body.error || res.status}`);
      return;
    }
    const body = await res.json();
    currentContext = body.context;
    document.getElementById("ctx-name").textContent = currentContext.name;
    document.getElementById("ctx-description").textContent =
      currentContext.description || "(none)";
    document.getElementById("ctx-kind").textContent = (
      currentContext.kind || "draft"
    ).toUpperCase();
    document.getElementById("ctx-created").textContent =
      currentContext.created_at || "—";
    document.getElementById("ctx-updated").textContent =
      currentContext.updated_at || "—";
    showView("contextDetail");
    setStatus(`CONTEXT ${currentContext.name}`);

    // Kick off the in-tab wasm runtime, the chat transcript, and
    // the memory panel in parallel — none of them depend on each
    // other, and the user can start typing while the wasm is still
    // initializing. Routine injection happens inside the runtime
    // panel flow (after boot completes). The skills panel waits
    // for the wasm to come up before rendering so the runtime
    // state is accurate when the operator clicks RUN.
    updateRuntimePanel().then(() => renderSkillsPanel());
    refreshMemoryPanel();
    await refreshTranscript();
    const chatInput = document.getElementById("input-chat");
    if (chatInput) chatInput.focus();
  } catch (err) {
    showError(err.message);
  }
}

// -------------------------------------------------------------------
// chat transcript
// -------------------------------------------------------------------

function renderEmptyTranscript(listEl) {
  clearChildren(listEl);
  const p = document.createElement("p");
  p.className = "meta";
  p.id = "chat-empty";
  p.textContent = "(no messages yet — describe what you want to build)";
  listEl.appendChild(p);
}

function appendMessageDom(listEl, msg) {
  const wrapper = document.createElement("div");
  wrapper.className = `chat-msg ${msg.role}`;
  wrapper.dataset.messageId = msg.id ?? "";

  const role = document.createElement("span");
  role.className = "chat-role";
  role.textContent = msg.role === "user" ? "[YOU]" : "[BRAIN]";

  const body = document.createElement("span");
  body.className = "chat-body";
  body.textContent = msg.content;

  wrapper.appendChild(role);
  wrapper.appendChild(body);
  listEl.appendChild(wrapper);
}

function scrollTranscriptToBottom() {
  const listEl = document.getElementById("chat-transcript");
  if (listEl) listEl.scrollTop = listEl.scrollHeight;
}

async function refreshTranscript() {
  if (!currentContext) return;
  const listEl = document.getElementById("chat-transcript");
  try {
    const res = await fetch(
      `/api/contexts/${encodeURIComponent(currentContext.id)}/messages`,
      { credentials: "include" },
    );
    if (!res.ok) {
      renderEmptyTranscript(listEl);
      return;
    }
    const body = await res.json();
    const rows = body.messages ?? [];
    clearChildren(listEl);
    if (rows.length === 0) {
      renderEmptyTranscript(listEl);
      return;
    }
    rows.forEach((m) => appendMessageDom(listEl, m));
    scrollTranscriptToBottom();
  } catch (err) {
    renderEmptyTranscript(listEl);
    console.warn("[terminal] transcript load failed:", err.message);
  }
}

document
  .getElementById("form-chat")
  .addEventListener("submit", async (ev) => {
    ev.preventDefault();
    if (!currentContext) return;
    const inputEl = document.getElementById("input-chat");
    const statusEl = document.getElementById("chat-status");
    const content = inputEl.value.trim();
    if (!content) return;

    statusEl.classList.remove("error");
    statusEl.textContent = "TRANSMITTING...";
    setStatus("BRAIN WORKING");
    const listEl = document.getElementById("chat-transcript");

    // Optimistically paint the user's message so the transcript
    // feels responsive while we wait on the brain. Hold a direct
    // reference to the bubble so the failure path can pop it off
    // the transcript without a querySelectorAll scan.
    const emptyHint = document.getElementById("chat-empty");
    if (emptyHint) emptyHint.remove();
    appendMessageDom(listEl, { role: "user", content });
    const optimisticBubble = listEl.lastElementChild;
    scrollTranscriptToBottom();
    inputEl.value = "";

    // Called on any failure path — remove the optimistic bubble,
    // restore the operator's draft so they don't have to retype
    // it, and re-render the empty-hint if the transcript is now
    // empty. This is the only place that rolls the UI back.
    const rollbackOptimistic = () => {
      if (optimisticBubble && optimisticBubble.parentNode) {
        optimisticBubble.remove();
      }
      if (!listEl.querySelector(".chat-msg")) {
        renderEmptyTranscript(listEl);
      }
      inputEl.value = content;
    };

    try {
      const res = await fetch(
        `/api/contexts/${encodeURIComponent(currentContext.id)}/messages`,
        {
          method: "POST",
          credentials: "include",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ content }),
        },
      );
      const body = await res.json();
      if (!res.ok || body.status !== "ok") {
        rollbackOptimistic();
        statusEl.classList.add("error");
        statusEl.textContent = `FAILED: ${body.error || res.status}`;
        setStatus("ERROR");
        return;
      }
      // Replace the optimistic bubble with the canonical version
      // so the message id + timestamp match the backend.
      if (optimisticBubble && optimisticBubble.parentNode) {
        optimisticBubble.remove();
      }
      appendMessageDom(listEl, body.user_message);
      appendMessageDom(listEl, body.assistant_message);
      scrollTranscriptToBottom();
      statusEl.textContent = `via ${body.model || "brain"}`;
      setStatus("AUTHORIZED");
    } catch (err) {
      rollbackOptimistic();
      statusEl.classList.add("error");
      statusEl.textContent = `NETWORK ERROR: ${err.message}`;
      setStatus("ERROR");
    }
  });

// -------------------------------------------------------------------
// browser-side soma-next runtime panel
// -------------------------------------------------------------------

async function updateRuntimePanel() {
  if (!currentContext) return;
  const el = document.getElementById("runtime-summary");
  el.classList.remove("error");
  el.textContent =
    "  STATE:  BOOTING...\n  PORTS:  —\n  SKILLS: —";
  try {
    const { summary, packId, fullSkills } = await bootForContext(
      currentContext,
    );
    const portsList = (summary.ports ?? [])
      .map((p) => p.port_id || p.id || String(p))
      .join(", ");

    // The wasm runtime's soma_list_skills() only sees wasm-scope
    // skills — bridge skills are stripped before soma_boot_runtime.
    // For an honest count we walk the FULL pack and split by scope
    // tag. Mixed packs show `1 wasm / 4 bridge`; empty is `(none)`.
    const allSkills = fullSkills ?? [];
    const isBridge = (s) => (s?.tags ?? []).includes("scope:bridge");
    const wasmCount = allSkills.filter((s) => !isBridge(s)).length;
    const bridgeCount = allSkills.filter(isBridge).length;
    const skillsLine =
      allSkills.length === 0
        ? "(none)"
        : `${wasmCount} wasm / ${bridgeCount} bridge`;

    const source = currentContext.pack_spec ? "context" : "fallback:hello";
    el.textContent =
      `  STATE:  READY\n` +
      `  PACK:   ${packId || "unknown"}\n` +
      `  SOURCE: ${source}\n` +
      `  PORTS:  ${portsList || "(none)"}\n` +
      `  SKILLS: ${skillsLine}`;

    // After the runtime is ready, push any stored routines for
    // this context back into the wasm body. Fire-and-forget —
    // malformed routines shouldn't break the rest of the UI.
    rehydrateRoutines().catch((err) =>
      console.warn("[terminal] routine rehydrate failed:", err.message),
    );
  } catch (err) {
    el.classList.add("error");
    const msg = (getBootError() && getBootError().message) || err.message;
    el.textContent = `  STATE:  FAILED\n  ERROR:  ${msg}`;
  }
}

// -------------------------------------------------------------------
// memory panel + routine rehydration
// -------------------------------------------------------------------

async function refreshMemoryPanel() {
  if (!currentContext) return;
  const el = document.getElementById("memory-summary");
  el.classList.remove("error");
  el.textContent = "  EPISODES: —\n  SCHEMAS:  —\n  ROUTINES: —";
  try {
    const res = await fetch(
      `/api/contexts/${encodeURIComponent(currentContext.id)}/memory`,
      { credentials: "include" },
    );
    if (!res.ok) {
      el.classList.add("error");
      el.textContent = `  ERROR: ${res.status}`;
      return;
    }
    const body = await res.json();
    const m = body.memory ?? { episodes: [], schemas: [], routines: [] };
    el.textContent =
      `  EPISODES: ${m.episodes.length}\n` +
      `  SCHEMAS:  ${m.schemas.length}\n` +
      `  ROUTINES: ${m.routines.length}`;
  } catch (err) {
    el.classList.add("error");
    el.textContent = `  ERROR: ${err.message}`;
  }
}

// -------------------------------------------------------------------
// commit 9 — SKILLS panel: render + run
// -------------------------------------------------------------------
//
// Reads the full skill list from the currently-booted pack (the
// one runtime.mjs stashed in currentPackSpec after bootPack) and
// renders a card per skill. Each card shows the skill name, its
// scope tag (wasm / bridge), a JSON textarea pre-filled with a
// guess at the input schema, and a RUN button that calls
// runtime.mjs:executeSkill. Results land inline under the card.

function scopeBadge(skill) {
  const tags = skill?.tags ?? [];
  if (tags.includes("scope:bridge")) return "bridge";
  if (tags.includes("scope:wasm")) return "wasm";
  // Fallback: empty capability_requirements → bridge
  return (skill?.capability_requirements ?? []).length === 0
    ? "bridge"
    : "wasm";
}

// Generate a sensible default JSON input from the skill's declared
// input schema so the operator has something to edit instead of an
// empty textarea.
function defaultInputForSkill(skill) {
  const schema = skill?.inputs?.schema;
  if (!schema || typeof schema !== "object") return {};
  const props = schema.properties ?? {};
  const required = schema.required ?? Object.keys(props);
  const out = {};
  for (const key of required) {
    const type = props[key]?.type || "string";
    if (type === "string") out[key] = "";
    else if (type === "number" || type === "integer") out[key] = 0;
    else if (type === "boolean") out[key] = false;
    else if (type === "array") out[key] = [];
    else out[key] = null;
  }
  return out;
}

function renderSkillsPanel() {
  const listEl = document.getElementById("skills-list");
  if (!listEl) return;
  clearChildren(listEl);

  const pack = getCurrentPackSpec();
  const skills = pack?.skills ?? [];
  if (skills.length === 0) {
    const p = document.createElement("p");
    p.className = "meta";
    p.id = "skills-empty";
    p.textContent = "(no skills yet — generate a pack first)";
    listEl.appendChild(p);
    return;
  }

  for (const skill of skills) {
    const card = document.createElement("div");
    card.className = "skill-entry";
    card.dataset.skillId = skill.skill_id;

    const head = document.createElement("div");
    head.className = "skill-head";
    const name = document.createElement("span");
    name.className = "skill-name";
    name.textContent = skill.name || skill.skill_id;
    const scope = document.createElement("span");
    const scopeName = scopeBadge(skill);
    scope.className = `skill-scope ${scopeName}`;
    scope.textContent = scopeName.toUpperCase();
    head.appendChild(name);
    head.appendChild(scope);

    const desc = document.createElement("p");
    desc.className = "skill-desc";
    desc.textContent = skill.description || "";

    const textarea = document.createElement("textarea");
    textarea.className = "skill-input";
    textarea.spellcheck = false;
    textarea.value = JSON.stringify(defaultInputForSkill(skill), null, 2);

    const actions = document.createElement("div");
    actions.className = "skill-actions";
    const runBtn = document.createElement("button");
    runBtn.type = "button";
    runBtn.className = "btn btn-run-skill";
    runBtn.textContent = "[ RUN ]";

    const resultEl = document.createElement("pre");
    resultEl.className = "skill-result hidden";

    runBtn.addEventListener("click", async () => {
      if (!currentContext) return;
      resultEl.classList.remove("error");
      resultEl.classList.remove("hidden");
      resultEl.textContent = "running...";
      let input;
      try {
        const raw = textarea.value.trim();
        input = raw === "" ? {} : JSON.parse(raw);
      } catch (err) {
        resultEl.classList.add("error");
        resultEl.textContent = `invalid JSON input: ${err.message}`;
        return;
      }
      try {
        const result = await executeSkill(
          skill.skill_id,
          input,
          currentContext.id,
        );
        if (result.status !== "ok") {
          resultEl.classList.add("error");
          resultEl.textContent = `error: ${result.error}`;
          return;
        }
        resultEl.textContent = JSON.stringify(result.record, null, 2);
        // Bridge writes change persistent state — refresh the
        // memory panel so the UI reflects any new KV rows or
        // side-effects the operator might have just caused.
        if (scopeName === "bridge") refreshMemoryPanel();
      } catch (err) {
        resultEl.classList.add("error");
        resultEl.textContent = `error: ${err.message}`;
      }
    });

    actions.appendChild(runBtn);

    card.appendChild(head);
    if (skill.description) card.appendChild(desc);
    card.appendChild(textarea);
    card.appendChild(actions);
    card.appendChild(resultEl);
    listEl.appendChild(card);
  }
}

// Fetch the stored routines for the current context and inject
// each one into the wasm runtime. Runs after updateRuntimePanel
// completes so the body is guaranteed booted first. Failures on
// individual routines are logged; the panel still reports the
// count from the store even if injection fails (commit 5 is
// about isolation, not runtime-level rehydration correctness).
async function rehydrateRoutines() {
  if (!currentContext) return;
  try {
    const res = await fetch(
      `/api/contexts/${encodeURIComponent(currentContext.id)}/memory`,
      { credentials: "include" },
    );
    if (!res.ok) return;
    const body = await res.json();
    const routines = body.memory?.routines ?? [];
    for (const row of routines) {
      try {
        await injectRoutine(row.payload);
      } catch (err) {
        console.warn(
          `[terminal] soma_inject_routine for ${row.name} failed:`,
          err.message,
        );
      }
    }
  } catch (err) {
    console.warn("[terminal] routine rehydrate fetch failed:", err.message);
  }
}

// Navigating back to the authenticated view re-fetches the operator
// record so we don't need to stash it; /api/me is the same call
// boot() uses and takes ~1ms.
async function backToAuthenticated() {
  currentContext = null;
  try {
    const res = await fetch("/api/me", { credentials: "include" });
    if (res.ok) {
      const body = await res.json();
      if (body.status === "ok" && body.user) {
        await enterAuthenticated(body.user);
        return;
      }
    }
  } catch {
    /* fall through to request-link */
  }
  enterRequestLink();
}

document
  .getElementById("btn-ctx-back")
  .addEventListener("click", () => backToAuthenticated());

// -------------------------------------------------------------------
// generate-pack button — commit 6
// -------------------------------------------------------------------
//
// Posts to /api/contexts/:id/pack/generate, which runs the
// reasoning brain (gpt-5-mini or fake) and stores the resulting
// PackSpec on the context row. On success we swap `currentContext`
// to the updated row and refresh the runtime panel — which reboots
// the wasm body with the newly-generated pack visible.

document
  .getElementById("btn-generate-pack")
  .addEventListener("click", async () => {
    if (!currentContext) return;
    const btn = document.getElementById("btn-generate-pack");
    const statusEl = document.getElementById("generate-status");

    // Visible busy state: pulse the button in phosphor green,
    // change its label, and announce the stage in the status span.
    // gpt-5-mini at reasoning_effort=low typically takes 5-30s,
    // and real-mode first-time runs have been observed up to 60s,
    // so the UI has to commit to the wait rather than going silent.
    btn.disabled = true;
    btn.classList.add("generating");
    const originalLabel = btn.textContent;
    btn.textContent = "[ COMPILING... ]";
    statusEl.classList.remove("error");
    statusEl.textContent = "calling gpt-5-mini, may take up to 60s...";
    setStatus("GENERATING PACK");

    const restoreButton = () => {
      btn.disabled = false;
      btn.classList.remove("generating");
      btn.textContent = originalLabel;
    };

    try {
      const res = await fetch(
        `/api/contexts/${encodeURIComponent(currentContext.id)}/pack/generate`,
        { method: "POST", credentials: "include" },
      );
      const body = await res.json();
      if (!res.ok || body.status !== "ok") {
        statusEl.classList.add("error");
        statusEl.textContent = `FAILED: ${body.error || res.status}`;
        setStatus("ERROR");
        return;
      }
      currentContext = body.context;
      // Refresh the metadata line (kind flipped draft → active).
      document.getElementById("ctx-kind").textContent = (
        currentContext.kind || "draft"
      ).toUpperCase();
      document.getElementById("ctx-updated").textContent =
        currentContext.updated_at || "—";
      statusEl.textContent = `via ${body.model || "brain"} → ${body.minimal?.pack_id || "pack"}`;
      setStatus("PACK READY");
      // Reboot the runtime into the freshly-generated pack and
      // re-render the SKILLS panel so the operator can RUN the
      // new skills immediately.
      await updateRuntimePanel();
      renderSkillsPanel();
      // Memory panel counts aren't affected but the routine store
      // was just wiped by the wasm reboot — nothing to rehydrate
      // until the operator starts generating episodes, which is
      // the commit-5-deferred path.
      await refreshMemoryPanel();
    } catch (err) {
      statusEl.classList.add("error");
      statusEl.textContent = `NETWORK ERROR: ${err.message}`;
      setStatus("ERROR");
    } finally {
      restoreButton();
    }
  });

document
  .getElementById("btn-ctx-delete")
  .addEventListener("click", async () => {
    if (!currentContext) return;
    setStatus("DELETING CONTEXT");
    try {
      const res = await fetch(
        `/api/contexts/${encodeURIComponent(currentContext.id)}`,
        { method: "DELETE", credentials: "include" },
      );
      if (!res.ok) {
        const body = await res.json().catch(() => ({}));
        showError(`DELETE FAILED — ${body.error || res.status}`);
        return;
      }
      await backToAuthenticated();
    } catch (err) {
      showError(err.message);
    }
  });

// -------------------------------------------------------------------
// logout / error reset
// -------------------------------------------------------------------

document
  .getElementById("btn-logout")
  .addEventListener("click", async () => {
    setStatus("LOGGING OUT");
    try {
      await fetch("/api/auth/logout", {
        method: "POST",
        credentials: "include",
      });
    } catch (err) {
      console.warn("[terminal] logout request failed:", err.message);
    }
    enterRequestLink();
  });

document.getElementById("btn-error-back").addEventListener("click", () => {
  enterRequestLink();
});

// -------------------------------------------------------------------
// kick off
// -------------------------------------------------------------------

boot().catch((err) => {
  console.error("[terminal] boot failed:", err);
  showError(err.message || String(err));
});
