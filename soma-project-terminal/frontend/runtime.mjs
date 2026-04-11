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
// The FULL pack the last bootForContext received (including both
// wasm-scope and bridge-scope skills). The wasm runtime only ever
// sees a filtered subset — bridge skills don't exist on its side
// — but the JS-side skill executor needs the full list to
// dispatch by scope.
let currentPackSpec = null;

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

// Return true if this skill is a bridge-scope skill (not resident
// in the wasm runtime). Looks at the `bridge:PORT:CAP` tag the
// pack expansion layer embeds, or falls back to empty
// capability_requirements as a secondary signal.
function isBridgeSkill(skill) {
  const tags = skill?.tags ?? [];
  if (tags.includes("scope:bridge")) return true;
  // Fallback signal: a skill with no capability_requirements is
  // almost certainly a bridge skill, since wasm skills need their
  // port wired.
  return (
    (!skill?.capability_requirements ||
      skill.capability_requirements.length === 0) &&
    tags.some((t) => typeof t === "string" && t.startsWith("bridge:"))
  );
}

// Parse a `bridge:PORT:CAP` tag into { portId, capabilityId } or
// null if the skill isn't bridge-scoped.
function parseBridgeTag(skill) {
  const tag = (skill?.tags ?? []).find(
    (t) => typeof t === "string" && t.startsWith("bridge:"),
  );
  if (!tag) return null;
  const parts = tag.slice("bridge:".length).split(":");
  if (parts.length < 2) return null;
  return { portId: parts[0], capabilityId: parts.slice(1).join(":") };
}

// Strip bridge-scope skills from a pack before handing it to the
// wasm runtime. The wasm runtime doesn't host bridge ports and
// would reject any skill whose capability_requirements references
// one. We keep a separate full-pack reference in currentPackSpec
// so the JS executor can still dispatch to bridge skills by id.
function stripBridgeSkills(packSpec) {
  if (!packSpec || !Array.isArray(packSpec.skills)) return packSpec;
  const wasmSkills = packSpec.skills.filter((s) => !isBridgeSkill(s));
  const wasmSkillIds = new Set(wasmSkills.map((s) => s.skill_id));
  return {
    ...packSpec,
    skills: wasmSkills,
    exposure: {
      ...(packSpec.exposure ?? {}),
      local_skills: (packSpec.exposure?.local_skills ?? []).filter((id) =>
        wasmSkillIds.has(id),
      ),
    },
  };
}

// Boot or re-boot the runtime with a given manifest. The wasm
// module only loads once; subsequent calls call soma_boot_runtime
// again with the new manifest, which re-initializes the runtime
// in-place (ports, skills, memory — all replaced).
//
// Returns { summary, packId, skills, fullSkills } — skills is the
// result of `soma_list_skills()` (wasm-resident only); fullSkills
// is the complete skill list from the input pack (wasm + bridge).
// The JS-side skill executor uses fullSkills; the runtime panel
// uses `skills` to show what actually lives inside the body.
export function bootPack(manifestJsonString) {
  const packId = extractPackId(manifestJsonString);
  // Parse the input so we can stash the full pack + filter bridge
  // skills before handing to wasm.
  let parsedPack = null;
  try {
    parsedPack = JSON.parse(manifestJsonString);
  } catch {
    parsedPack = null;
  }
  currentPackSpec = parsedPack;
  const wasmManifest = parsedPack
    ? JSON.stringify(stripBridgeSkills(parsedPack))
    : manifestJsonString;

  bootPromise = (async () => {
    const mod = await ensureWasm();
    const summaryJson = mod.soma_boot_runtime(wasmManifest);
    const summary = JSON.parse(summaryJson);
    let skills = [];
    try {
      skills = JSON.parse(mod.soma_list_skills());
    } catch {
      skills = [];
    }
    const fullSkills = parsedPack?.skills ?? skills;
    bootSummary = summary;
    bootError = null;
    currentPackId = packId;
    return { mod, summary, packId, skills, fullSkills };
  })().catch((err) => {
    bootError = err;
    throw err;
  });
  return bootPromise;
}

export function getCurrentPackSpec() {
  return currentPackSpec;
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

// -------------------------------------------------------------------
// commit 9 — JS-side skill executor
// -------------------------------------------------------------------
//
// Unified invocation surface. Pass a skill id + input and this
// function decides whether to run the skill inside the wasm runtime
// (soma_invoke_port) or hop through the backend bridge (POST to
// /api/contexts/:id/port/:portId/:capId).
//
// Dispatch rules:
//   1. Look up the skill in `currentPackSpec.skills` (the full
//      list — wasm AND bridge). If no pack is loaded, throw.
//   2. If the skill carries a `bridge:PORT:CAP` tag, parse it and
//      POST to the bridge route. Needs `contextId` because the
//      bridge is context-scoped.
//   3. Otherwise look at `capability_requirements[0]`, extract
//      port + capability, and call soma_invoke_port.
//
// Return value: { status: "ok" | "error", record, error? } where
// `record` is PortCallRecord-shaped for both paths — the UI can
// treat wasm and bridge results uniformly.

function findSkillInCurrentPack(skillId) {
  if (!currentPackSpec || !Array.isArray(currentPackSpec.skills)) {
    throw new Error("no pack loaded — call bootPack first");
  }
  const skill = currentPackSpec.skills.find((s) => s.skill_id === skillId);
  if (!skill) throw new Error(`unknown skill: ${skillId}`);
  return skill;
}

async function invokeBridgeSkill(contextId, portId, capabilityId, input) {
  if (!contextId) {
    throw new Error("executeSkill requires contextId for bridge skills");
  }
  const res = await fetch(
    `/api/contexts/${encodeURIComponent(contextId)}/port/${encodeURIComponent(portId)}/${encodeURIComponent(capabilityId)}`,
    {
      method: "POST",
      credentials: "include",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ input }),
    },
  );
  const body = await res.json().catch(() => ({}));
  if (!res.ok || body.status !== "ok") {
    return {
      status: "error",
      error: body.error || `bridge returned ${res.status}`,
    };
  }
  return { status: "ok", record: body.record };
}

async function invokeWasmSkill(skill, input) {
  // Prefer the bridge tag even for wasm skills (they have
  // `bridge:dom:append_heading` too — the expansion layer writes
  // it as a breadcrumb regardless of scope). That way one parse
  // path handles both cases.
  let portId = null;
  let capabilityId = null;
  const parsed = parseBridgeTag(skill);
  if (parsed) {
    portId = parsed.portId;
    capabilityId = parsed.capabilityId;
  } else {
    const req = skill.capability_requirements?.[0];
    const m = req && req.match(/^port:([^/]+)\/(.+)$/);
    if (!m) {
      return {
        status: "error",
        error: `wasm skill missing capability_requirements`,
      };
    }
    portId = m[1];
    capabilityId = m[2];
  }
  const { mod } = await awaitCurrentBoot();
  try {
    const json = mod.soma_invoke_port(
      portId,
      capabilityId,
      JSON.stringify(input ?? {}),
    );
    const record = JSON.parse(json);
    return { status: "ok", record };
  } catch (err) {
    return {
      status: "error",
      error: `wasm invoke failed: ${err.message || err}`,
    };
  }
}

export async function executeSkill(skillId, input, contextId) {
  const skill = findSkillInCurrentPack(skillId);
  if (isBridgeSkill(skill)) {
    const parsed = parseBridgeTag(skill);
    if (!parsed) {
      return {
        status: "error",
        error: `bridge skill missing bridge:X:Y tag`,
      };
    }
    return invokeBridgeSkill(
      contextId,
      parsed.portId,
      parsed.capabilityId,
      input,
    );
  }
  return invokeWasmSkill(skill, input);
}
