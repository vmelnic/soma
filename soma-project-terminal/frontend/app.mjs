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

    // Kick off the in-tab wasm runtime and the chat transcript in
    // parallel — neither depends on the other, and the user can
    // start typing while the wasm is still initializing.
    updateRuntimePanel();
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
    // feels responsive while we wait on the brain.
    const emptyHint = document.getElementById("chat-empty");
    if (emptyHint) emptyHint.remove();
    appendMessageDom(listEl, { role: "user", content });
    scrollTranscriptToBottom();
    inputEl.value = "";

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
        statusEl.classList.add("error");
        statusEl.textContent = `FAILED: ${body.error || res.status}`;
        setStatus("ERROR");
        return;
      }
      // Replace the optimistic user bubble with the canonical one
      // so the message id + timestamp match the backend.
      const listElFresh = document.getElementById("chat-transcript");
      const userBubbles = listElFresh.querySelectorAll(
        ".chat-msg.user[data-message-id='']",
      );
      const lastOptimistic = userBubbles[userBubbles.length - 1];
      if (lastOptimistic) lastOptimistic.remove();
      appendMessageDom(listElFresh, body.user_message);
      appendMessageDom(listElFresh, body.assistant_message);
      scrollTranscriptToBottom();
      statusEl.textContent = `via ${body.model || "brain"}`;
      setStatus("AUTHORIZED");
    } catch (err) {
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
  } catch (err) {
    el.classList.add("error");
    const msg = (getBootError() && getBootError().message) || err.message;
    el.textContent = `  STATE:  FAILED\n  ERROR:  ${msg}`;
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
