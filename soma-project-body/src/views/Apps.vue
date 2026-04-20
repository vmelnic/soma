<script setup>
import { onMounted, onUnmounted, ref } from 'vue';
import {
  RefreshCw, Upload, Download, Play, Sparkles, PackageOpen, Check, AlertTriangle,
} from 'lucide-vue-next';
import { useBodyStore } from '../stores/body.js';
import {
  diffRoutines, buildAppManifest, parseAppManifest, serializeAppManifest,
  loadAliases, setAlias, displayName, reviewRoutine,
} from '../lib/appforge.js';

const body = useBodyStore();
const routines = ref([]);
const aliases = ref(loadAliases());
const err = ref(null);
const newlyMined = ref([]);
const pollHandle = ref(null);
const pendingImport = ref(null);

const BTN = 'inline-flex items-center gap-1.5 bg-claude-surface text-claude-text border border-claude-border rounded-md px-2.5 py-1 text-xs cursor-pointer transition-colors hover:border-claude-accent disabled:opacity-40 disabled:cursor-not-allowed';
const INPUT = 'bg-claude-surface text-claude-text border border-claude-border rounded-md px-2 py-1 text-xs focus:outline-none focus:border-claude-accent';

async function refresh() {
  err.value = null;
  if (!body.connected) return;
  try {
    const r = await body.callTool('dump_state', { sections: 'routines' });
    const list = r?.routines || r?.data?.routines || [];
    const prev = routines.value;
    const { added } = diffRoutines(prev, list);
    routines.value = list;
    if (prev.length && added.length) newlyMined.value = [...newlyMined.value, ...added];
  } catch (e) { err.value = e.message; }
}

onMounted(() => { refresh(); pollHandle.value = setInterval(refresh, 4000); });
onUnmounted(() => { if (pollHandle.value) clearInterval(pollHandle.value); });

function nameFor(r) { return displayName(r, aliases.value); }

function updateAlias(r, name) {
  const id = r.routine_id || r.id;
  if (!id) return;
  aliases.value = setAlias(id, name);
}

function dismissMined(routineId) {
  newlyMined.value = newlyMined.value.filter((r) => (r.routine_id || r.id) !== routineId);
}

async function run(routineId) {
  err.value = null;
  try { await body.callTool('execute_routine', { routine_id: routineId }); }
  catch (e) { err.value = e.message; }
}

function exportRoutine(r) {
  const id = r.routine_id || r.id;
  const name = nameFor(r);
  const m = buildAppManifest({ name, description: r.description || '', routine: r });
  const blob = new Blob([serializeAppManifest(m)], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = `${name.replace(/[^\w.-]+/g, '_') || id}.somapp.json`;
  document.body.appendChild(a); a.click();
  setTimeout(() => { document.body.removeChild(a); URL.revokeObjectURL(url); }, 0);
}

async function doImport(parsed) {
  const routine = parsed.routine;
  await body.callTool('author_routine', {
    routine_id: routine.routine_id || routine.id || `imported_${Date.now()}`,
    match_conditions: routine.match_conditions || [],
    steps: routine.steps || routine.effective_steps || [],
    guard_conditions: routine.guard_conditions || [],
    priority: routine.priority ?? 0,
    exclusive: !!routine.exclusive,
    autonomous: false,
  });
  if (parsed.name) {
    updateAlias({ routine_id: routine.routine_id || routine.id }, parsed.name);
  }
  await refresh();
}

async function importFile(e) {
  err.value = null;
  const file = e.target.files?.[0];
  if (!file) return;
  try {
    const text = await file.text();
    const parsed = parseAppManifest(JSON.parse(text));
    const review = reviewRoutine(parsed.routine, body.ports.map((p) => p.port_id));
    if (!review.safe) {
      pendingImport.value = { routine: parsed.routine, parsed, warnings: review.warnings };
      return;
    }
    await doImport(parsed);
  } catch (ex) { err.value = `import failed: ${ex.message || ex}`; }
  finally { e.target.value = ''; }
}

async function confirmImport() {
  if (!pendingImport.value) return;
  err.value = null;
  try {
    await doImport(pendingImport.value.parsed);
  } catch (ex) { err.value = `import failed: ${ex.message || ex}`; }
  finally { pendingImport.value = null; }
}

function cancelImport() {
  pendingImport.value = null;
}
</script>

<template>
  <div class="p-4 min-h-full">
    <header class="flex justify-between items-center mb-4">
      <div class="flex items-center gap-2">
        <PackageOpen class="w-4 h-4 text-claude-muted" :stroke-width="1.75" />
        <h3 class="text-xs uppercase tracking-widest text-claude-muted">Apps</h3>
      </div>
      <div class="flex gap-2 items-center">
        <button :class="BTN" @click="refresh" :disabled="!body.connected">
          <RefreshCw class="w-3.5 h-3.5" :stroke-width="1.75" />
          <span>refresh</span>
        </button>
        <label :class="[BTN, 'cursor-pointer']">
          <Upload class="w-3.5 h-3.5" :stroke-width="1.75" />
          <span>import</span>
          <input type="file" accept=".somapp.json,.json,application/json" @change="importFile" class="hidden" />
        </label>
      </div>
    </header>

    <p v-if="err" class="bg-claude-errBg text-claude-error font-mono text-sm px-2 py-1.5 rounded mb-3">{{ err }}</p>

    <section v-if="pendingImport" class="border-2 border-claude-warn rounded-lg p-4 mb-4">
      <div class="flex items-center gap-2 mb-3">
        <AlertTriangle class="w-4 h-4 text-claude-warn" :stroke-width="1.75" />
        <h4 class="text-claude-warn text-xs uppercase tracking-widest">Safety review</h4>
      </div>
      <ul class="list-disc list-inside text-sm text-claude-text space-y-1 mb-4">
        <li v-for="(w, i) in pendingImport.warnings" :key="i">{{ w }}</li>
      </ul>
      <div class="flex gap-2">
        <button :class="BTN" @click="confirmImport">import anyway</button>
        <button :class="BTN" @click="cancelImport">cancel</button>
      </div>
    </section>

    <section v-if="newlyMined.length" class="bg-claude-warnBg border border-claude-warn/50 rounded-lg p-4 mb-4">
      <div class="flex items-center gap-2 mb-3">
        <Sparkles class="w-4 h-4 text-claude-warn" :stroke-width="1.75" />
        <h4 class="text-claude-warn text-xs uppercase tracking-widest">SOMA just learned</h4>
      </div>
      <div
        v-for="r in newlyMined" :key="r.routine_id || r.id"
        class="grid grid-cols-[1fr_16ch_auto] gap-2 items-center py-1"
      >
        <code class="text-claude-warn text-xs truncate">{{ (r.routine_id || r.id || '') }}</code>
        <input
          :class="INPUT"
          placeholder="name this app"
          :value="nameFor(r)"
          @change="updateAlias(r, $event.target.value)"
        />
        <button :class="BTN" @click="dismissMined(r.routine_id || r.id)">
          <Check class="w-3.5 h-3.5" :stroke-width="1.75" />
          <span>ok</span>
        </button>
      </div>
    </section>

    <ul class="divide-y divide-claude-border">
      <li
        v-for="r in routines" :key="r.routine_id || r.id"
        class="grid grid-cols-[1fr_auto] gap-x-3 gap-y-1 py-3 items-center"
      >
        <div class="font-semibold truncate">{{ nameFor(r) }}</div>
        <div class="col-start-1 row-start-2">
          <input
            :class="[INPUT, 'w-full max-w-xs']"
            placeholder="rename…"
            :value="aliases[r.routine_id || r.id] || ''"
            @change="updateAlias(r, $event.target.value)"
          />
        </div>
        <div class="col-start-2 row-span-2 flex gap-2">
          <button :class="BTN" @click="run(r.routine_id || r.id)" :disabled="!body.connected">
            <Play class="w-3.5 h-3.5" :stroke-width="1.75" />
            <span>run</span>
          </button>
          <button :class="BTN" @click="exportRoutine(r)">
            <Download class="w-3.5 h-3.5" :stroke-width="1.75" />
            <span>export</span>
          </button>
        </div>
      </li>
      <li v-if="!routines.length" class="text-claude-muted py-6 text-center text-sm">
        <Sparkles class="w-5 h-5 mx-auto mb-2 text-claude-dim" :stroke-width="1.5" />
        no routines yet — run a goal a few times and SOMA will learn one.
      </li>
    </ul>
  </div>
</template>
