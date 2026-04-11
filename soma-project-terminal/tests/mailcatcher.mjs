// Helpers for talking to Mailcatcher's HTTP API from tests.
//
// Mailcatcher exposes a simple JSON API on :1080:
//   GET    /messages            → array of message summaries
//   GET    /messages/:id.plain  → plain-text body
//   DELETE /messages            → clear all
//
// Tests use this to pull the magic-link token out of the email the
// backend dispatched via soma-next's smtp port.

const MC_BASE = "http://127.0.0.1:1080";

export async function listMessages() {
  const res = await fetch(`${MC_BASE}/messages`);
  if (!res.ok) throw new Error(`mailcatcher list: ${res.status}`);
  return res.json();
}

export async function getPlainBody(id) {
  const res = await fetch(`${MC_BASE}/messages/${id}.plain`);
  if (!res.ok) throw new Error(`mailcatcher get plain: ${res.status}`);
  return res.text();
}

export async function clearAll() {
  const res = await fetch(`${MC_BASE}/messages`, { method: "DELETE" });
  if (!res.ok && res.status !== 404) {
    throw new Error(`mailcatcher clear: ${res.status}`);
  }
}

// Poll until at least one message arrives for the given recipient.
// Returns the message summary with `.id`.
export async function waitForMessageTo(recipient, { timeout = 5000, pollMs = 100 } = {}) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const msgs = await listMessages();
    const match = msgs.find((m) =>
      (m.recipients ?? []).some((r) => r.includes(recipient)),
    );
    if (match) return match;
    await new Promise((r) => setTimeout(r, pollMs));
  }
  throw new Error(`timed out waiting for mail to ${recipient}`);
}

// Extract a magic-link token from the plain-text email body.
export function extractToken(body) {
  const match = body.match(/token=([A-Za-z0-9]+)/);
  if (!match) throw new Error("no token= in body");
  return match[1];
}
