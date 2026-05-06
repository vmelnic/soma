#!/usr/bin/env node
/**
 * Routine compiler: reads routines/*.md → outputs data/routines.json
 *
 * Usage: node engine/compile.mjs projects/helperbook
 */

import { readFileSync, writeFileSync, readdirSync, existsSync, mkdirSync } from 'fs';
import { join, basename } from 'path';

const projectDir = process.argv[2];
if (!projectDir) {
  console.error('usage: node engine/compile.mjs <project-dir>');
  process.exit(1);
}

const routinesDir = join(projectDir, 'routines');
const packsDir = join(projectDir, 'packs');
const dataDir = join(projectDir, 'data');

// --- Load valid skill IDs from manifests ---

function loadSkillIds() {
  const ids = new Set();
  if (!existsSync(packsDir)) return ids;
  for (const pack of readdirSync(packsDir)) {
    const manifest = join(packsDir, pack, 'manifest.json');
    if (!existsSync(manifest)) continue;
    const m = JSON.parse(readFileSync(manifest, 'utf8'));
    for (const skill of m.skills || []) {
      ids.add(skill.skill_id);
    }
  }
  return ids;
}

// --- Parse bind pairs respecting quotes and brackets ---

function parseBindPairs(str) {
  const pairs = [];
  let i = 0;
  while (i < str.length) {
    // Skip whitespace and commas
    while (i < str.length && (str[i] === ',' || str[i] === ' ')) i++;
    if (i >= str.length) break;

    // Read key
    const eqIdx = str.indexOf('=', i);
    if (eqIdx === -1) break;
    const key = str.slice(i, eqIdx).trim();
    i = eqIdx + 1;

    // Read value — respect quotes, brackets
    let val;
    if (str[i] === '"') {
      // Quoted string — find closing quote (not escaped)
      const end = str.indexOf('"', i + 1);
      if (end === -1) { val = str.slice(i + 1); i = str.length; }
      else { val = str.slice(i + 1, end); i = end + 1; }
    } else if (str[i] === '[' || str[i] === '{') {
      // JSON array/object — find matching bracket
      const open = str[i], close = open === '[' ? ']' : '}';
      let depth = 1, j = i + 1;
      while (j < str.length && depth > 0) {
        if (str[j] === '"') { j++; while (j < str.length && str[j] !== '"') j++; }
        else if (str[j] === open) depth++;
        else if (str[j] === close) depth--;
        j++;
      }
      const raw = str.slice(i, j);
      try { val = JSON.parse(raw); } catch { val = raw; }
      i = j;
    } else {
      // Bare value — read until comma or end
      let j = i;
      while (j < str.length && str[j] !== ',') j++;
      const raw = str.slice(i, j).trim();
      if (raw === 'true') val = true;
      else if (raw === 'false') val = false;
      else if (!raw.startsWith('$') && raw !== '' && !isNaN(Number(raw))) val = Number(raw);
      else val = raw;
      i = j;
    }
    pairs.push([key, val]);
  }
  return pairs;
}

// --- Parse routine markdown ---

function parseRoutine(filePath) {
  const raw = readFileSync(filePath, 'utf8');
  const lines = raw.split('\n');

  // Parse YAML frontmatter
  if (lines[0].trim() !== '---') {
    throw new Error(`${filePath}: missing frontmatter`);
  }
  let fmEnd = -1;
  for (let i = 1; i < lines.length; i++) {
    if (lines[i].trim() === '---') { fmEnd = i; break; }
  }
  if (fmEnd === -1) throw new Error(`${filePath}: unclosed frontmatter`);

  const fm = {};
  let currentKey = null;
  let currentIndent = 0;
  const matchObj = {};

  for (let i = 1; i < fmEnd; i++) {
    const line = lines[i];
    const trimmed = line.trim();
    if (!trimmed) continue;

    const kvMatch = trimmed.match(/^(\w+):\s*(.*)$/);
    if (kvMatch) {
      const [, key, val] = kvMatch;
      if (key === 'match') {
        currentKey = 'match';
        fm.match = matchObj;
      } else if (currentKey === 'match' && line.startsWith('  ')) {
        matchObj[key] = val;
      } else {
        fm[key] = val;
        currentKey = key;
      }
    }
  }

  if (!fm.id) throw new Error(`${filePath}: missing id in frontmatter`);

  // Parse steps
  const steps = [];
  let currentStep = null;
  const body = lines.slice(fmEnd + 1);

  for (const line of body) {
    const trimmed = line.trim();
    if (!trimmed) continue;

    // "step <skill_id>" or "step abandon"
    const stepMatch = trimmed.match(/^step\s+(.+)$/);
    if (stepMatch && !trimmed.startsWith('step abandon')) {
      if (currentStep) steps.push(currentStep);
      currentStep = {
        type: 'skill',
        skill_id: stepMatch[1],
        on_success: { action: 'continue' },
        on_failure: { action: 'continue' },
        conditions: [],
      };
      continue;
    }

    if (trimmed === 'step abandon') {
      if (currentStep) steps.push(currentStep);
      // Abandon step: a dummy skill that always abandons
      currentStep = {
        type: 'skill',
        skill_id: '__abandon__',
        on_success: { action: 'abandon' },
        on_failure: { action: 'abandon' },
        conditions: [],
        _abandon: true,
      };
      steps.push(currentStep);
      currentStep = null;
      continue;
    }

    if (!currentStep) continue;

    // "bind: key=val, key2=$ref, key3="literal with, commas""
    const bindMatch = trimmed.match(/^bind:\s*(.+)$/);
    if (bindMatch) {
      if (!currentStep.input_overrides) currentStep.input_overrides = {};
      for (const [k, v] of parseBindPairs(bindMatch[1])) {
        currentStep.input_overrides[k] = v;
      }
      continue;
    }

    // "on_success: <action>" or "on_failure: <action>"
    const onMatch = trimmed.match(/^on_(success|failure):\s*(.+)$/);
    if (onMatch) {
      currentStep[`on_${onMatch[1]}`] = parseNextStep(onMatch[2]);
      continue;
    }

    // "condition:" block
    if (trimmed === 'condition:') {
      const cond = { expression: {}, description: '', next_step: { action: 'continue' } };
      currentStep._pendingCondition = cond;
      continue;
    }

    if (currentStep._pendingCondition) {
      const condKv = trimmed.match(/^(\w+):\s*(.+)$/);
      if (condKv) {
        const [, key, val] = condKv;
        if (key === 'match') {
          // Support unquoted keys: {valid: false} → {"valid": false}
          const fixed = val.replace(/(\{|,)\s*(\w+)\s*:/g, '$1"$2":');
          currentStep._pendingCondition.expression = JSON.parse(fixed);
        } else if (key === 'next') {
          currentStep._pendingCondition.next_step = parseNextStep(val);
          currentStep.conditions.push(currentStep._pendingCondition);
          delete currentStep._pendingCondition;
        } else if (key === 'description') {
          currentStep._pendingCondition.description = val;
        }
      }
    }
  }

  if (currentStep) steps.push(currentStep);

  // Clean up internal fields
  for (const s of steps) {
    delete s._pendingCondition;
    delete s._abandon;
  }

  return {
    routine_id: fm.id,
    match_conditions: Object.entries(fm.match || {}).map(([k, v]) => ({
      condition_type: 'goal_fingerprint',
      expression: { goal_fingerprint: v },
      description: `${k}: ${v}`,
    })),
    steps,
  };
}

function parseNextStep(str) {
  str = str.trim();
  if (str === 'continue') return { action: 'continue' };
  if (str === 'complete') return { action: 'complete' };
  if (str === 'abandon') return { action: 'abandon' };
  const gotoMatch = str.match(/^goto\s+(\d+)$/);
  if (gotoMatch) return { action: 'goto', step_index: parseInt(gotoMatch[1]) };
  const callMatch = str.match(/^call\s+(.+)$/);
  if (callMatch) return { action: 'call_routine', routine_id: callMatch[1] };
  throw new Error(`unknown next_step: ${str}`);
}

// --- Build soma-next Routine struct ---

function buildRoutine(parsed) {
  return {
    routine_id: parsed.routine_id,
    namespace: 'builder',
    origin: 'pack_authored',
    match_conditions: parsed.match_conditions,
    compiled_skill_path: parsed.steps
      .filter(s => s.type === 'skill' && s.skill_id !== '__abandon__')
      .map(s => s.skill_id),
    compiled_steps: parsed.steps.map(s => {
      if (s.skill_id === '__abandon__') {
        return {
          type: 'skill',
          skill_id: parsed.steps.find(x => x.skill_id !== '__abandon__')?.skill_id || 'noop',
          on_success: serializeNextStep({ action: 'abandon' }),
          on_failure: serializeNextStep({ action: 'abandon' }),
          conditions: [],
        };
      }
      const step = {
        type: 'skill',
        skill_id: s.skill_id,
        on_success: serializeNextStep(s.on_success),
        on_failure: serializeNextStep(s.on_failure),
        conditions: s.conditions.map(c => ({
          expression: c.expression,
          description: c.description,
          next_step: serializeNextStep(c.next_step),
        })),
      };
      if (s.input_overrides && Object.keys(s.input_overrides).length > 0) {
        step.input_overrides = s.input_overrides;
      }
      return step;
    }),
    guard_conditions: [],
    expected_cost: 0.0,
    expected_effect: [],
    confidence: 1.0,
    priority: 0,
    exclusive: false,
    version: 1,
  };
}

function serializeNextStep(ns) {
  if (ns.action === 'continue') return { action: 'continue' };
  if (ns.action === 'complete') return { action: 'complete' };
  if (ns.action === 'abandon') return { action: 'abandon' };
  if (ns.action === 'goto') return { action: 'goto', step_index: ns.step_index, max_iterations: ns.max_iterations || null };
  if (ns.action === 'call_routine') return { action: 'call_routine', routine_id: ns.routine_id };
  return { action: 'continue' };
}

// --- Main ---

const skillIds = loadSkillIds();
console.log(`Loaded ${skillIds.size} skill IDs from manifests`);

const files = readdirSync(routinesDir).filter(f => f.endsWith('.md'));
console.log(`Found ${files.length} routine files\n`);

const routines = [];
let errors = 0;

for (const file of files.sort()) {
  const path = join(routinesDir, file);
  try {
    const parsed = parseRoutine(path);

    // Validate skill IDs
    const badSkills = parsed.steps
      .filter(s => s.skill_id !== '__abandon__')
      .filter(s => !skillIds.has(s.skill_id));

    if (badSkills.length > 0) {
      console.log(`  WARN: ${file}: unknown skills: ${badSkills.map(s => s.skill_id).join(', ')}`);
    }

    const routine = buildRoutine(parsed);
    routines.push(routine);
    console.log(`  OK: ${parsed.routine_id} (${parsed.steps.length} steps)`);
  } catch (err) {
    console.log(`  FAIL: ${file}: ${err.message}`);
    errors++;
  }
}

if (!existsSync(dataDir)) mkdirSync(dataDir, { recursive: true });
writeFileSync(join(dataDir, 'routines.json'), JSON.stringify(routines, null, 2));
console.log(`\nCompiled ${routines.length} routines → ${join(dataDir, 'routines.json')}`);
if (errors > 0) {
  console.log(`${errors} errors`);
  process.exit(1);
}
