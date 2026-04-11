// Playwright global setup.
//
// Runs once before the test suite. Truncates the terminal tables and
// clears Mailcatcher so each `npx playwright test` run starts clean.
// Expects docker-compose to already be up (see README.md).
//
// We use psql via `docker exec` rather than a Node pg client because
// (a) the backend has zero runtime Node deps by design, and we're not
// breaking that for tests, and (b) docker exec is available on every
// dev machine that already ran ./scripts/start.sh.

import { execFileSync } from "child_process";

const MAILCATCHER_URL = "http://127.0.0.1:1080/messages";

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
        "TRUNCATE TABLE context_kv, episodes, schemas, routines, messages, contexts, sessions, magic_tokens, users RESTART IDENTITY CASCADE;",
      ],
      { stdio: ["ignore", "pipe", "pipe"] },
    );
  } catch (err) {
    throw new Error(
      `[global-setup] failed to truncate tables: ${err.message}. ` +
        `Did you run ./scripts/start.sh and ./scripts/setup-db.sh?`,
    );
  }
}

export default async function globalSetup() {
  cleanDb();
  await clearMailcatcher();
  console.log("[global-setup] db truncated + mailcatcher cleared");
}
