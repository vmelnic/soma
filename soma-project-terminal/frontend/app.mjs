// SOMA TERMINAL — frontend bootstrap + auth flow.
//
// Commit 1 wires the Fallout shell to the backend's /api/auth/*
// endpoints. Four view states: loading (checks /api/me on boot),
// request-link (email form), link-sent (confirmation), authenticated
// (post-login placeholder).
//
// No framework. Vanilla DOM manipulation. The views live in the HTML
// as sibling <section class="view"> elements; we just toggle .hidden.

const views = {
  loading: document.getElementById("view-loading"),
  requestLink: document.getElementById("view-request-link"),
  linkSent: document.getElementById("view-link-sent"),
  authenticated: document.getElementById("view-authenticated"),
  error: document.getElementById("view-error"),
};

const footerStatus = document.getElementById("footer-status");

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

// -------------------------------------------------------------------
// boot sequence — check session on load
// -------------------------------------------------------------------

async function boot() {
  setStatus("BOOTING");
  // Retro boot-print effect: cycle through a few prompt dots so it
  // feels like the terminal is powering on. Functional no-op, just
  // ambience. Kept short so Playwright tests don't wait forever.
  await new Promise((r) => setTimeout(r, 250));

  try {
    const res = await fetch("/api/me", { credentials: "include" });
    if (res.ok) {
      const body = await res.json();
      if (body.status === "ok" && body.user) {
        enterAuthenticated(body.user);
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
// view: authenticated
// -------------------------------------------------------------------

function enterAuthenticated(user) {
  document.getElementById("user-email").textContent = user.email;
  showView("authenticated");
  setStatus("AUTHORIZED");
}

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

// -------------------------------------------------------------------
// error view reset
// -------------------------------------------------------------------

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
