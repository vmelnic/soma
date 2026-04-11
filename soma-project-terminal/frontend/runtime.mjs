// Browser-side soma-next runtime.
//
// Commit 3 booted a single shared hello pack for every context.
// Commit 4 makes boot-with-manifest reusable so switching contexts
// hot-swaps the runtime to that context's own PackSpec. The wasm
// module itself loads exactly once — it's the large download — and
// `soma_boot_runtime` is called afresh for every pack.
//
// Contract with callers:
//   bootPack(manifestJsonString)  → { summary }
//     Reboots the runtime with the given manifest, returns the
//     parsed runtime summary. Safe to call repeatedly; each call
//     wipes the previous runtime state.
//
//   getCurrentPackId()             → string | null
//     Returns the id of the currently loaded pack so the UI can
//     avoid a needless hot-swap when a context opens with the same
//     manifest already loaded.
//
//   listPorts(), listSkills(), invokePort(port, cap, input)
//     Thin wrappers over the corresponding wasm exports. They
//     implicitly await the most recent boot via `bootPromise`.
//
// The hello pack is the fallback — if a context has no pack_spec
// yet, the caller is expected to pass the raw text of
// frontend/packs/hello/manifest.json into bootPack.

let wasmPromise = null;
let bootPromise = null;
let bootSummary = null;
let bootError = null;
let currentPackId = null;

async function ensureWasm() {
  if (!wasmPromise) {
    wasmPromise = (async () => {
      const mod = await import("./pkg/soma_next.js");
      await mod.default();
      return mod;
    })();
  }
  return wasmPromise;
}

// Parse the incoming manifest just far enough to extract the pack
// id. The wasm `soma_runtime_summary()` export only reports ports
// (not the pack id), so we track the id ourselves from the input
// the caller hands us — that way the UI can show which pack is
// loaded without a second export.
function extractPackId(manifestJsonString) {
  try {
    const parsed = JSON.parse(manifestJsonString);
    if (typeof parsed?.id === "string") return parsed.id;
  } catch {
    /* fall through — we still try to boot and surface the failure */
  }
  return null;
}

// Boot or re-boot the runtime with a given manifest. The wasm
// module only loads once; subsequent calls call soma_boot_runtime
// again with the new manifest, which re-initializes the runtime
// in-place (ports, skills, memory — all replaced).
//
// Returns { summary, packId, skills } — skills is the result of
// `soma_list_skills()` run immediately after boot, since the
// runtime summary export doesn't include skills.
export function bootPack(manifestJsonString) {
  const packId = extractPackId(manifestJsonString);
  bootPromise = (async () => {
    const mod = await ensureWasm();
    const summaryJson = mod.soma_boot_runtime(manifestJsonString);
    const summary = JSON.parse(summaryJson);
    let skills = [];
    try {
      skills = JSON.parse(mod.soma_list_skills());
    } catch {
      skills = [];
    }
    bootSummary = summary;
    bootError = null;
    currentPackId = packId;
    return { mod, summary, packId, skills };
  })().catch((err) => {
    bootError = err;
    throw err;
  });
  return bootPromise;
}

export function getSummary() {
  return bootSummary;
}

export function getBootError() {
  return bootError;
}

export function getCurrentPackId() {
  return currentPackId;
}

// Fetch the bundled hello manifest — used as the fallback when a
// context has no pack_spec yet. Cached after first load so opening
// many "draft" contexts doesn't re-hit the network.
let helloManifestPromise = null;
export function loadHelloManifest() {
  if (!helloManifestPromise) {
    helloManifestPromise = fetch("./packs/hello/manifest.json").then(
      (res) => {
        if (!res.ok) {
          throw new Error(
            `failed to fetch hello manifest: ${res.status}`,
          );
        }
        return res.text();
      },
    );
  }
  return helloManifestPromise;
}

// Convenience entry: pass the context row verbatim and the runtime
// does the right thing — boot the context's pack_spec if set,
// otherwise fall back to the shared hello manifest.
export async function bootForContext(context) {
  const spec = context?.pack_spec;
  if (typeof spec === "string" && spec.trim() !== "") {
    return bootPack(spec);
  }
  const hello = await loadHelloManifest();
  return bootPack(hello);
}

// ---- listing + invocation ------------------------------------------

async function awaitCurrentBoot() {
  if (!bootPromise) {
    throw new Error("runtime not booted — call bootPack first");
  }
  return bootPromise;
}

export async function listPorts() {
  const { mod } = await awaitCurrentBoot();
  return JSON.parse(mod.soma_list_ports());
}

export async function listSkills() {
  const { mod } = await awaitCurrentBoot();
  return JSON.parse(mod.soma_list_skills());
}

export async function invokePort(portId, capabilityId, input) {
  const { mod } = await awaitCurrentBoot();
  const json = mod.soma_invoke_port(
    portId,
    capabilityId,
    JSON.stringify(input ?? {}),
  );
  return JSON.parse(json);
}

// Inject a routine JSON string into the current wasm runtime. The
// routine is keyed by name in the runtime's routine store and
// subsequent plan-following dispatch can walk it. Commit 5 uses
// this on context open to push every stored routine back into the
// body after the pack has booted. Best-effort: malformed routines
// are silently swallowed (error surfaces in the memory panel).
export async function injectRoutine(routinePayload) {
  const { mod } = await awaitCurrentBoot();
  if (typeof mod.soma_inject_routine !== "function") {
    throw new Error("soma_inject_routine export not available");
  }
  const text =
    typeof routinePayload === "string"
      ? routinePayload
      : JSON.stringify(routinePayload);
  return mod.soma_inject_routine(text);
}
