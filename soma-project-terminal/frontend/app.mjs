// SOMA TERMINAL — frontend bootstrap + auth + contexts.
//
// Commit 1 wired the Fallout shell to /api/auth/*. Commit 2 adds:
//   - the contexts list inside the authenticated view
//   - the create-context form
//   - the view-context-detail "loaded" state
//
// No framework. Vanilla DOM manipulation — every element is created
// with `document.createElement` and every value is set via
// `textContent`, never innerHTML, so a maliciously named context
// can't break out of the terminal.

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
  } catch (err) {
    showError(err.message);
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
