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
    // panel flow (after boot completes).
    updateRuntimePanel();
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
    const { summary, packId, skills } = await bootForContext(currentContext);
    const portsList = (summary.ports ?? [])
      .map((p) => p.port_id || p.id || String(p))
      .join(", ");
    const skillsList = (skills ?? [])
      .map((s) => s.skill_id || s.id || String(s))
      .join(", ");
    const source = currentContext.pack_spec ? "context" : "fallback:hello";
    el.textContent =
      `  STATE:  READY\n` +
      `  PACK:   ${packId || "unknown"}\n` +
      `  SOURCE: ${source}\n` +
      `  PORTS:  ${portsList || "(none)"}\n` +
      `  SKILLS: ${skillsList || "(none)"}`;

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
    btn.disabled = true;
    statusEl.classList.remove("error");
    statusEl.textContent = "COMPILING PACK...";
    setStatus("GENERATING PACK");
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
      // Reboot the runtime into the freshly-generated pack.
      await updateRuntimePanel();
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
      btn.disabled = false;
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
