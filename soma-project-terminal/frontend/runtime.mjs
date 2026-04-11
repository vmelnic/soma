// Browser-side soma-next runtime.
//
// Commit 3 goal: prove that the terminal app has a real SOMA runtime
// inside the browser tab, not just a chat shell over HTTP. The body
// lives here; the brain lives on the backend (gpt-4o-mini via
// /api/contexts/:id/messages). The two halves of the "two-brain"
// architecture run side-by-side in the same page.
//
// Commit 3 scope is intentionally small — it boots the hello pack
// shipped in frontend/packs/hello/manifest.json and exposes a
// summary of the loaded ports and skills. Dynamic per-context pack
// loading arrives in commit 4, LLM-to-PackSpec in commit 6.
//
// The wasm bundle comes from soma-project-web's build — copied into
// frontend/pkg/ by scripts/copy-frontend-assets.sh. Zero build steps
// in this repo.

let bootPromise = null;
let bootSummary = null;
let bootError = null;

async function loadManifest(url) {
  const res = await fetch(url);
  if (!res.ok) {
    throw new Error(
      `failed to fetch ${url}: ${res.status} ${res.statusText}`,
    );
  }
  return res.text();
}

// Single-shot boot — first caller triggers the wasm init, every
// subsequent caller awaits the same promise. Makes it safe to call
// from multiple entry points (chat panel opening, runtime button,
// Playwright test) without racing.
export function bootBrowserRuntime() {
  if (bootPromise) return bootPromise;
  bootPromise = (async () => {
    const mod = await import("./pkg/soma_next.js");
    await mod.default();
    const manifest = await loadManifest("./packs/hello/manifest.json");
    const summaryJson = mod.soma_boot_runtime(manifest);
    const summary = JSON.parse(summaryJson);
    bootSummary = summary;
    return { mod, summary };
  })().catch((err) => {
    bootError = err;
    throw err;
  });
  return bootPromise;
}

// Cheap accessors so callers don't have to await again if the boot
// already finished. Returns null until the runtime is ready.
export function getSummary() {
  return bootSummary;
}

export function getBootError() {
  return bootError;
}

// List the loaded ports (port_id, kind, capabilities) as a JS array.
// Returns null if the runtime hasn't booted yet.
export async function listPorts() {
  const { mod } = await bootBrowserRuntime();
  return JSON.parse(mod.soma_list_ports());
}

// List the loaded skills (skill_id, namespace, pack, description).
export async function listSkills() {
  const { mod } = await bootBrowserRuntime();
  return JSON.parse(mod.soma_list_skills());
}

// Invoke a port capability directly and return the PortCallRecord.
// Thin wrapper over soma_invoke_port so the chat UI can eventually
// offer "run this" buttons next to plan fragments.
export async function invokePort(portId, capabilityId, input) {
  const { mod } = await bootBrowserRuntime();
  const json = mod.soma_invoke_port(
    portId,
    capabilityId,
    JSON.stringify(input ?? {}),
  );
  return JSON.parse(json);
}
