import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const projectRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));

function parseDotEnv(content) {
  const env = {};
  for (const rawLine of content.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) continue;
    const eq = line.indexOf("=");
    if (eq === -1) continue;
    const key = line.slice(0, eq).trim();
    let value = line.slice(eq + 1).trim();
    if (
      (value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'"))
    )
      value = value.slice(1, -1);
    env[key] = value;
  }
  return env;
}

let cache;
export async function loadEnv() {
  if (cache) return cache;
  const p = path.join(projectRoot, ".env");
  const entries = existsSync(p) ? parseDotEnv(await readFile(p, "utf8")) : {};
  cache = { ...entries, ...process.env };
  return cache;
}
