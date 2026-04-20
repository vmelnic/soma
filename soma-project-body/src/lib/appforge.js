// AppForge — operator-facing packaging of SOMA routines.
//
// Thesis: apps aren't built, they're learned. After SOMA mines a routine
// from repeated episodes, the operator names it, exports it as a single
// `.somapp` file, and shares it. Any SOMA can import and run it.
//
// A `.somapp` file is plain JSON with this shape:
//   {
//     "format": "somapp/1",
//     "name": "Morning reconcile",
//     "description": "...",
//     "operator": "alice@somewhere",
//     "created_at": 1712345678901,
//     "required_ports": ["postgres", "smtp"],
//     "examples": ["reconcile yesterday's charges", "check anomalies"],
//     "routine": { /* verbatim SOMA routine spec from dump_state */ }
//   }
//
// The runtime's own `author_routine` MCP tool ingests the `routine` field
// back into a new SOMA instance.

export const SOMAPP_FORMAT = 'somapp/1';

/** Compute added / removed routines between two snapshots. */
export function diffRoutines(prev, current) {
  const idOf = (r) => r.routine_id || r.id;
  const prevIds = new Set((prev || []).map(idOf).filter(Boolean));
  const currIds = new Set((current || []).map(idOf).filter(Boolean));
  const added = (current || []).filter((r) => {
    const id = idOf(r);
    return id && !prevIds.has(id);
  });
  const removed = (prev || []).filter((r) => {
    const id = idOf(r);
    return id && !currIds.has(id);
  });
  return { added, removed };
}

/** Build a `.somapp` manifest from operator-supplied metadata + routine. */
export function buildAppManifest({
  name,
  description = '',
  operator = '',
  examples = [],
  routine,
  requiredPorts,
  createdAt = Date.now(),
}) {
  if (!name || typeof name !== 'string') throw new Error('buildAppManifest: name required');
  if (!routine || typeof routine !== 'object') throw new Error('buildAppManifest: routine required');
  return {
    format: SOMAPP_FORMAT,
    name,
    description,
    operator,
    created_at: createdAt,
    required_ports: requiredPorts || inferRequiredPorts(routine),
    examples: Array.isArray(examples) ? examples : [],
    routine,
  };
}

/** Validate and normalize an incoming `.somapp` manifest. */
export function parseAppManifest(obj) {
  if (!obj || typeof obj !== 'object') throw new Error('parseAppManifest: not an object');
  if (obj.format !== SOMAPP_FORMAT) {
    throw new Error(`parseAppManifest: unsupported format '${obj.format}' (expected ${SOMAPP_FORMAT})`);
  }
  if (!obj.name || typeof obj.name !== 'string') throw new Error('parseAppManifest: name required');
  if (!obj.routine || typeof obj.routine !== 'object') throw new Error('parseAppManifest: routine required');
  return {
    format: SOMAPP_FORMAT,
    name: obj.name,
    description: String(obj.description || ''),
    operator: String(obj.operator || ''),
    created_at: Number(obj.created_at) || Date.now(),
    required_ports: Array.isArray(obj.required_ports) ? obj.required_ports.slice() : [],
    examples: Array.isArray(obj.examples) ? obj.examples.slice() : [],
    routine: obj.routine,
  };
}

/** Serialize a manifest to a pretty JSON string suitable for download. */
export function serializeAppManifest(manifest) {
  return JSON.stringify(manifest, null, 2);
}

/** Inspect a routine and guess which ports it touches. Best-effort. */
export function inferRequiredPorts(routine) {
  const seen = new Set();
  const visit = (node) => {
    if (!node || typeof node !== 'object') return;
    if (Array.isArray(node)) {
      for (const child of node) visit(child);
      return;
    }
    if (typeof node.port_id === 'string') seen.add(node.port_id);
    // Compiled step paths look like "soma.ports.<port>.<cap>" — extract.
    if (typeof node.skill_id === 'string') {
      const m = node.skill_id.match(/^soma\.ports\.([^.]+)\./);
      if (m) seen.add(m[1]);
    }
    if (typeof node.skill === 'string') {
      const m = node.skill.match(/^soma\.ports\.([^.]+)\./);
      if (m) seen.add(m[1]);
    }
    for (const v of Object.values(node)) visit(v);
  };
  visit(routine);
  return [...seen].sort();
}

// ─────────────────────────── Safety review ───────────────────────────────────

const SHELL_PATTERN = /[;|`]|&&|\$\(/;

/**
 * Review a routine before import. Returns {safe, warnings}.
 * `availablePorts` is an array of port_id strings known to this runtime.
 */
export function reviewRoutine(routine, availablePorts = []) {
  const warnings = [];
  const knownPorts = new Set(availablePorts);

  // Autonomous flag
  if (routine.autonomous) {
    warnings.push('Routine is marked autonomous \u2014 it will run without operator confirmation');
  }

  // Step count
  const steps = routine.steps || routine.effective_steps || [];
  if (steps.length > 20) {
    warnings.push(`Routine has ${steps.length} steps (unusually large)`);
  }

  // Per-step checks
  steps.forEach((step, i) => {
    // Unknown port
    const skillId = step.skill_id || step.skill || '';
    const m = skillId.match(/^soma\.ports\.([^.]+)\./);
    if (m && !knownPorts.has(m[1])) {
      warnings.push(`Step references unknown port: ${m[1]}`);
    }

    // Shell-injection patterns in input values
    const inputs = step.inputs || step.input || {};
    if (typeof inputs === 'object' && inputs !== null) {
      for (const val of Object.values(inputs)) {
        if (typeof val === 'string' && SHELL_PATTERN.test(val)) {
          warnings.push(`Step ${i} input contains shell-like patterns`);
          break;
        }
      }
    }
  });

  return { safe: warnings.length === 0, warnings };
}

// ─────────────────────────── Aliases (pretty names) ───────────────────────────

const ALIAS_KEY = 'soma.routineAliases.v1';

export function loadAliases(storage = globalThis.localStorage) {
  if (!storage) return {};
  try {
    return JSON.parse(storage.getItem(ALIAS_KEY) || '{}');
  } catch { return {}; }
}

export function saveAliases(aliases, storage = globalThis.localStorage) {
  if (!storage) return;
  storage.setItem(ALIAS_KEY, JSON.stringify(aliases));
}

export function setAlias(id, name, storage = globalThis.localStorage) {
  const current = loadAliases(storage);
  const next = { ...current, [id]: name };
  saveAliases(next, storage);
  return next;
}

export function clearAlias(id, storage = globalThis.localStorage) {
  const current = loadAliases(storage);
  if (!(id in current)) return current;
  const next = { ...current };
  delete next[id];
  saveAliases(next, storage);
  return next;
}

export function displayName(routine, aliases) {
  const id = routine.routine_id || routine.id || '';
  if (aliases && aliases[id]) return aliases[id];
  return routine.name || id;
}
