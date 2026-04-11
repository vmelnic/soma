// Playwright global setup.
//
// Runs once before the test suite. Deletes TEST-CREATED rows only
// (not the whole tables) and clears Mailcatcher so each
// `npx playwright test` run starts from a clean test-operator
// slate WITHOUT nuking the real operator's data in the same DB.
//
// Every test fixture — `loginAs` in tests/helpers.mjs, the
// round-trip test in auth.spec.js, the replay test — uses the
// reserved `@somacorp.net` domain for synthetic emails. We use
// that pattern as the deletion selector. ON DELETE CASCADE on
// sessions/contexts/messages/memory/context_kv handles the
// dependent rows automatically; magic_tokens has no FK to users
// so we clean it up separately by email.
//
// WHY THIS MATTERS: the previous TRUNCATE-everything approach was
// destructive to any real operator sharing the same Postgres
// database. Running the test suite would wipe the dev user's
// sessions + contexts + generated packs, forcing them to re-login
// and re-create everything. Targeted deletion preserves real
// user data AND still gives tests a clean slate (each test
// operator uses a unique `operator-<timestamp>-<random>` email,
// so there is no prior state to begin with).
//
// We use psql via `docker exec` rather than a Node pg client
// because (a) the backend has zero runtime Node deps by design,
// and we're not breaking that for tests, and (b) docker exec is
// available on every dev machine that already ran
// ./scripts/start.sh.

import { execFileSync } from "child_process";

const MAILCATCHER_URL = "http://127.0.0.1:1080/messages";
const TEST_EMAIL_PATTERN = "%@somacorp.net";

async function clearMailcatcher() {
  try {
    const res = await fetch(MAILCATCHER_URL, { method: "DELETE" });
    if (!res.ok) {
      console.warn(
        `[global-setup] mailcatcher clear returned ${res.status}`,
      );
    }
  } catch (err) {
    console.warn(
      `[global-setup] mailcatcher clear failed: ${err.message}. ` +
        `Is docker compose up?`,
    );
  }
}

function cleanDb() {
  // Two-statement cleanup:
  //   1. DELETE FROM users WHERE email LIKE '%@somacorp.net'
  //      → cascades to sessions, contexts, messages, episodes,
  //        schemas, routines, context_kv (all FK'd to users or
  //        transitively through contexts, with ON DELETE CASCADE).
  //   2. DELETE FROM magic_tokens WHERE email LIKE '%@somacorp.net'
  //      → magic_tokens only has a TEXT email column, no FK, so
  //        it does NOT cascade. Clean it up directly.
  //
  // Both run in one psql invocation as a single SQL string so we
  // only pay the docker exec round-trip once.
  try {
    execFileSync(
      "docker",
      [
        "exec",
        "-i",
        "soma-terminal-postgres",
        "psql",
        "-U",
        "soma",
        "-d",
        "soma_terminal",
        "-c",
        `DELETE FROM users WHERE email LIKE '${TEST_EMAIL_PATTERN}'; ` +
          `DELETE FROM magic_tokens WHERE email LIKE '${TEST_EMAIL_PATTERN}';`,
      ],
      { stdio: ["ignore", "pipe", "pipe"] },
    );
  } catch (err) {
    throw new Error(
      `[global-setup] failed to clean test rows: ${err.message}. ` +
        `Did you run ./scripts/start.sh and ./scripts/setup-db.sh?`,
    );
  }
}

export default async function globalSetup() {
  cleanDb();
  await clearMailcatcher();
  console.log(
    "[global-setup] deleted test rows (@somacorp.net) + mailcatcher cleared",
  );
}
