// Shared test helpers. The interesting one is `loginAs` — it runs
// the full magic-link round trip (request → mailcatcher → verify)
// and returns the session token + user record so the caller can
// hit authenticated endpoints without replaying the whole dance.

import {
  clearAll,
  waitForMessageTo,
  getPlainBody,
  extractToken,
} from "./mailcatcher.mjs";

// Run a magic-link login for a fresh email and return
// `{ email, userId, sessionToken, authHeader }`.
//
// `request` is a Playwright APIRequestContext (either the fixture or
// a per-test newContext()). Each call creates a new user because
// email is unique by default — caller can pass `email` to pin it.
export async function loginAs(request, { email } = {}) {
  const actualEmail =
    email ?? `operator-${Date.now()}-${Math.random().toString(36).slice(2, 8)}@somacorp.net`;

  // We do NOT call mailcatcher.clearAll() here because parallel tests
  // would race each other. Instead we rely on `waitForMessageTo`
  // matching the specific recipient.

  const reqRes = await request.post("/api/auth/request-link", {
    headers: { "Content-Type": "application/json" },
    data: { email: actualEmail },
  });
  if (!reqRes.ok()) {
    throw new Error(
      `request-link for ${actualEmail} failed: ${reqRes.status()}`,
    );
  }

  const msg = await waitForMessageTo(actualEmail, { timeout: 8000 });
  const body = await getPlainBody(msg.id);
  const rawToken = extractToken(body);

  const verifyRes = await request.get(`/api/auth/verify?token=${rawToken}`, {
    headers: { Accept: "application/json" },
  });
  if (!verifyRes.ok()) {
    throw new Error(
      `verify for ${actualEmail} failed: ${verifyRes.status()}`,
    );
  }
  const verifyBody = await verifyRes.json();
  if (verifyBody.status !== "ok") {
    throw new Error(`verify returned ${JSON.stringify(verifyBody)}`);
  }
  return {
    email: actualEmail,
    userId: verifyBody.user.id,
    sessionToken: verifyBody.session_token,
    authHeader: `Bearer ${verifyBody.session_token}`,
  };
}

// Convenience: clear the mailcatcher before a test body runs.
export { clearAll };
