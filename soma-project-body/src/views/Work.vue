<script setup>
import { onMounted, onUnmounted, ref } from 'vue';
import { ArrowRight, RefreshCw, Inbox, Radio, CheckCircle2, Circle } from 'lucide-vue-next';
import { useBodyStore } from '../stores/body.js';

const body = useBodyStore();
const sessions = ref([]);
const handoffs = ref({ openHandoffs: [], myClaims: [], othersClaimed: [] });
const err = ref(null);
const intervalRef = ref(null);

const BTN = 'inline-flex items-center gap-1.5 bg-claude-surface text-claude-text border border-claude-border rounded-md px-2.5 py-1 text-xs cursor-pointer transition-colors hover:border-claude-accent disabled:opacity-40 disabled:cursor-not-allowed';

async function refresh() {
  if (!body.connected) return;
  err.value = null;
  try {
    const r = await body.callTool('list_sessions', {});
    sessions.value = r?.sessions || r || [];
  } catch (e) { err.value = e.message; }
  try { handoffs.value = await body.fetchHandoffs(); }
  catch { /* ignore */ }
}

async function sendElsewhere(s) {
  err.value = null;
  try {
    await body.handoffSession({
      sessionId: s.goal_id || s.id,
      objective: s.goal || s.objective,
    });
    await refresh();
  } catch (e) { err.value = e.message; }
}

async function claim(sessionId) {
  err.value = null;
  try { await body.claimSession(sessionId); await refresh(); }
  catch (e) { err.value = e.message; }
}

const statusColor = (s) => {
  if (['Completed'].includes(s)) return 'text-claude-success';
  if (['Failed', 'Aborted', 'Error'].includes(s)) return 'text-claude-error';
  return 'text-claude-warn';
};

onMounted(() => { refresh(); intervalRef.value = setInterval(refresh, 1500); });
onUnmounted(() => { if (intervalRef.value) clearInterval(intervalRef.value); });
</script>

<template>
  <div class="p-4 flex flex-col gap-6 min-h-full">

    <section
      v-if="handoffs.openHandoffs.length"
      class="bg-claude-surface border border-claude-accent/60 rounded-lg p-4"
    >
      <div class="flex items-center gap-2 mb-3">
        <Inbox class="w-4 h-4 text-claude-accent" :stroke-width="1.75" />
        <h3 class="text-xs uppercase tracking-widest text-claude-accent">Pick up</h3>
      </div>
      <ul class="divide-y divide-claude-border">
        <li
          v-for="h in handoffs.openHandoffs" :key="h.session_id"
          class="flex flex-wrap items-center gap-2 py-2 font-mono text-sm"
        >
          <code class="text-claude-accent">{{ h.session_id.slice(0, 8) }}</code>
          <span class="flex-1 min-w-0 truncate">{{ h.handoff.value?.objective || '—' }}</span>
          <span class="text-xs text-claude-dim">from {{ (h.handoff.value?.from_device || '?').slice(0, 8) }}</span>
          <button :class="BTN" @click="claim(h.session_id)">
            <ArrowRight class="w-3.5 h-3.5" :stroke-width="1.75" />
            <span>take over</span>
          </button>
        </li>
      </ul>
    </section>

    <section>
      <div class="flex items-center justify-between mb-2">
        <div class="flex items-center gap-2">
          <Radio class="w-4 h-4 text-claude-muted" :stroke-width="1.75" />
          <h3 class="text-xs uppercase tracking-widest text-claude-muted">Sessions</h3>
        </div>
        <button :class="BTN" @click="refresh" :disabled="!body.connected">
          <RefreshCw class="w-3.5 h-3.5" :stroke-width="1.75" />
          <span>refresh</span>
        </button>
      </div>
      <p v-if="err" class="bg-claude-errBg text-claude-error font-mono text-sm px-2 py-1 rounded mb-2">{{ err }}</p>
      <p v-if="!body.connected" class="text-claude-muted text-sm">not connected</p>
      <ul v-else class="divide-y divide-claude-border">
        <li
          v-for="s in sessions" :key="s.id || s.goal_id"
          class="flex flex-wrap items-center gap-2 py-2 font-mono text-sm"
        >
          <code class="text-claude-accent">{{ (s.goal_id || s.id || '').slice(0, 8) }}</code>
          <span :class="['min-w-[8ch]', statusColor(s.status)]">{{ s.status || '…' }}</span>
          <span class="flex-1 min-w-0 truncate">{{ s.goal || s.objective || '' }}</span>
          <button
            :class="BTN"
            @click="sendElsewhere(s)"
            :disabled="['Completed','Failed','Aborted'].includes(s.status)"
          >
            <ArrowRight class="w-3.5 h-3.5" :stroke-width="1.75" />
            <span>send elsewhere</span>
          </button>
        </li>
        <li v-if="!sessions.length" class="text-claude-muted py-2 text-sm">no active sessions</li>
      </ul>
    </section>

    <section v-if="handoffs.myClaims.length || handoffs.othersClaimed.length">
      <h3 class="text-xs uppercase tracking-widest text-claude-muted mb-2">Handoff history</h3>
      <ul class="divide-y divide-claude-border font-mono text-sm">
        <li v-for="h in handoffs.myClaims" :key="'m-'+h.session_id" class="flex items-center gap-2 py-1.5">
          <CheckCircle2 class="w-4 h-4 text-claude-success" :stroke-width="1.75" />
          <code class="text-claude-success">{{ h.session_id.slice(0, 8) }}</code>
          <span class="text-claude-muted">— you claimed this</span>
        </li>
        <li v-for="h in handoffs.othersClaimed" :key="'o-'+h.session_id" class="flex items-center gap-2 py-1.5">
          <Circle class="w-4 h-4 text-claude-dim" :stroke-width="1.75" />
          <code class="text-claude-accent">{{ h.session_id.slice(0, 8) }}</code>
          <span class="text-claude-muted">— claimed by {{ (h.claim.value.device_id || '?').slice(0, 8) }}</span>
        </li>
      </ul>
    </section>

    <section>
      <h3 class="text-xs uppercase tracking-widest text-claude-muted mb-2">Event stream</h3>
      <ol class="divide-y divide-claude-border font-mono text-sm">
        <li v-for="(e, i) in body.events" :key="i" class="flex gap-2 py-1.5 items-baseline">
          <code class="text-claude-accent">{{ e.msg?.method || e.msg?.kind || 'event' }}</code>
          <span class="flex-1 min-w-0 truncate text-claude-muted">{{ JSON.stringify(e.msg?.params || e.msg).slice(0, 140) }}</span>
        </li>
        <li v-if="!body.events.length" class="text-claude-muted py-2 text-sm">no events yet</li>
      </ol>
    </section>

  </div>
</template>
