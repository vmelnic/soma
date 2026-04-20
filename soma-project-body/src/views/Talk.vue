<script setup>
import { ref, watch } from 'vue';
import { Send, User, Sparkles, Info, AlertCircle, X, Mic, Volume2, VolumeX, FileText } from 'lucide-vue-next';
import { useBodyStore } from '../stores/body.js';
import { useBrainConfigStore, ROLE_DECIDER, ROLE_NARRATOR } from '../stores/brainConfig.js';
import { sttAvailable, startListening, ttsAvailable, speak, stopSpeaking } from '../lib/speech.js';
import { render } from '../lib/templates.js';
import { buildRegistry } from '../lib/actionRegistry.js';

const body = useBodyStore();
const brains = useBrainConfigStore();

const input = ref('');
const log = ref([]);
const busy = ref(false);
const conversationHistory = ref([]);
const listening = ref(false);
const ttsEnabled = ref(localStorage.getItem('soma.tts') !== 'off');
let sttHandle = null;

function toggleTts() {
  ttsEnabled.value = !ttsEnabled.value;
  localStorage.setItem('soma.tts', ttsEnabled.value ? 'on' : 'off');
  if (!ttsEnabled.value) stopSpeaking();
}

function toggleMic() {
  if (listening.value) {
    sttHandle?.stop();
    sttHandle = null;
    listening.value = false;
    return;
  }
  listening.value = true;
  sttHandle = startListening({
    onResult(text) {
      input.value = text;
      listening.value = false;
      sttHandle = null;
      submit();
    },
    onError() {
      listening.value = false;
      sttHandle = null;
    },
  });
}

watch(
  () => brains.config[ROLE_DECIDER],
  async (newCfg, oldCfg) => {
    if (!oldCfg || !conversationHistory.value.length) return;
    if (newCfg.provider === oldCfg.provider && newCfg.model === oldCfg.model) return;
    try {
      const sessions = await body.callTool('list_sessions', {}).catch(() => []);
      const active = (sessions?.sessions || sessions || []).find(
        (s) => !['Completed', 'Failed', 'Aborted'].includes(s.status),
      );
      if (active) {
        conversationHistory.value.push({
          role: 'system',
          content: `[brain-swap] New decider: ${newCfg.provider}/${newCfg.model}. Active session: ${active.goal_id || active.id}, status: ${active.status || 'unknown'}.`,
        });
        push('system', `decider swapped to ${newCfg.provider}/${newCfg.model}`);
      }
    } catch { /* best effort */ }
  },
  { deep: true },
);

const TOKEN_BUDGET = 2000;
function estimateTokens(text) { return Math.ceil((text || '').length / 4); }

function trimmedHistory() {
  const hist = conversationHistory.value;
  if (!hist.length) return [];
  let total = 0;
  let start = hist.length;
  for (let i = hist.length - 1; i >= 0; i--) {
    const cost = estimateTokens(hist[i].content);
    if (total + cost > TOKEN_BUDGET) break;
    total += cost;
    start = i;
  }
  return hist.slice(start).slice(-10);
}

function clearConversation() {
  log.value = [];
  conversationHistory.value = [];
}

const INPUT = 'bg-claude-surface text-claude-text border border-claude-border rounded-md px-3 py-2.5 text-base focus:outline-none focus:border-claude-accent';
const BTN_PRIMARY = 'inline-flex items-center gap-2 bg-claude-accent text-claude-bg border border-claude-accent rounded-md px-4 py-2.5 text-sm font-semibold cursor-pointer transition-colors hover:bg-claude-hover hover:border-claude-hover disabled:opacity-40 disabled:cursor-not-allowed';

function push(kind, text) {
  log.value.push({ kind, text, ts: Date.now() });
  if (kind === 'narrator' && ttsEnabled.value) speak(text);
}

// ─── Execute a resolved action against SOMA ───

async function executeAction(entry, args) {
  if (entry.kind === 'port') {
    const result = await body.callTool('invoke_port', {
      port_id: entry.port_id,
      capability_id: entry.capability_id,
      input: args,
    });
    push('system', `${entry.port_id}.${entry.capability_id} → ${result?.success ? 'ok' : 'failed'}`);
    return result;
  }
  const result = await body.callTool(entry.name, args);
  push('system', `${entry.name} → ok`);
  return result;
}

function pushResult(actionResult) {
  const payload = actionResult?.structured_result ?? actionResult?.raw_result ?? actionResult;
  if (payload == null) return;
  const preview = typeof payload === 'string' ? payload : JSON.stringify(payload, null, 2);
  if (preview && preview !== 'null' && preview !== '{}') {
    push('result', preview);
  }
}

// ─── Narrator pipeline (separate LLM call) ───

async function narrate(text, action, actionResult) {
  if (!brains.router.has(ROLE_NARRATOR)) return;
  const mode = brains.config[ROLE_NARRATOR]?.narratorMode || 'terse';
  if (mode === 'alarm' && actionResult?.success) return;

  const narrator = brains.router.get(ROLE_NARRATOR);
  const messages = [
    { role: 'system', content: render(`narrator-${mode}`) },
  ];
  const lastNarration = [...log.value].reverse().find((e) => e.kind === 'narrator');
  if (lastNarration) messages.push({ role: 'assistant', content: lastNarration.text });

  const resultSnippet = (() => {
    const r = actionResult?.structured_result ?? actionResult?.raw_result ?? actionResult;
    return r != null ? JSON.stringify(r).slice(0, 800) : '(no data)';
  })();

  messages.push({
    role: 'user',
    content: render('narrator-context', {
      user_text: text,
      action_json: JSON.stringify(action),
      result_snippet: resultSnippet,
    }),
  });

  const resp = await narrator.chat({ messages, max_tokens: mode === 'debug' ? 120 : 60 });
  const narrationText = resp.text?.trim();
  if (narrationText && narrationText !== '—') push('narrator', narrationText);
}

// ─── Main submit: decider with native tool calling ───

async function submit() {
  const text = input.value.trim();
  if (!text || busy.value) return;
  input.value = '';
  push('user', text);
  conversationHistory.value.push({ role: 'user', content: text });
  busy.value = true;

  try {
    const { definitions, dispatch } = buildRegistry(
      body.ports, body.remotePorts, body.tools,
    );

    const decider = brains.router.get(ROLE_DECIDER);
    const systemContent = render('decider-system', {});

    const resp = await decider.chat({
      messages: [
        { role: 'system', content: systemContent },
        ...trimmedHistory().slice(0, -1),
        { role: 'user', content: text },
      ],
      tools: definitions,
      temperature: 0,
      max_tokens: 1024,
    });

    // LLM responded with text (chat/conversation).
    if (resp.text && !resp.tool_calls?.length) {
      push('narrator', resp.text);
      conversationHistory.value.push({ role: 'assistant', content: resp.text });
      return;
    }

    // LLM chose a tool call.
    for (const tc of resp.tool_calls || []) {
      const fnName = tc.function.name;
      const args = JSON.parse(tc.function.arguments || '{}');
      const entry = dispatch[fnName];

      if (!entry) {
        push('error', `unknown action: ${fnName}`);
        continue;
      }

      conversationHistory.value.push({
        role: 'assistant',
        content: `[action] ${fnName}(${JSON.stringify(args)})`,
      });

      const actionResult = await executeAction(entry, args);
      pushResult(actionResult);
      await narrate(text, { name: fnName, args }, actionResult);
    }
  } catch (e) {
    push('error', e.message || String(e));
  } finally {
    busy.value = false;
  }
}

const kindMeta = {
  user:     { icon: User,        cls: 'text-claude-text' },
  narrator: { icon: Sparkles,    cls: 'text-claude-accent' },
  system:   { icon: Info,        cls: 'text-claude-muted' },
  result:   { icon: FileText,    cls: 'text-claude-muted' },
  error:    { icon: AlertCircle, cls: 'text-claude-error' },
};
</script>

<template>
  <div class="flex flex-col h-full">
    <div class="flex-1 overflow-auto px-4 py-4 flex flex-col gap-3">
      <div
        v-for="(entry, i) in log"
        :key="i"
        class="flex items-start gap-3 leading-relaxed"
      >
        <component
          :is="kindMeta[entry.kind].icon"
          class="w-4 h-4 mt-1 shrink-0"
          :class="kindMeta[entry.kind].cls"
          :stroke-width="1.75"
        />
        <pre
          v-if="entry.kind === 'result'"
          class="text-xs text-claude-muted bg-claude-surface border border-claude-border rounded px-2 py-1.5 overflow-x-auto max-h-48 whitespace-pre-wrap m-0"
        >{{ entry.text }}</pre>
        <span
          v-else
          class="text-sm"
          :class="[kindMeta[entry.kind].cls, entry.kind === 'system' ? 'text-xs' : '']"
        >{{ entry.text }}</span>
      </div>
      <p v-if="!log.length" class="text-claude-muted text-sm max-w-md m-0 mt-4 leading-relaxed">
        Tell SOMA what you want. The <span class="text-claude-text">decider</span> picks the action, SOMA does it, the <span class="text-claude-accent">narrator</span> speaks.
      </p>
    </div>

    <form class="flex gap-2 px-4 py-3 border-t border-claude-border bg-claude-bg" @submit.prevent="submit">
      <input
        v-model="input"
        :disabled="busy || listening"
        :placeholder="listening ? 'listening…' : 'register a new user, then email them…'"
        :class="[INPUT, 'flex-1']"
      />
      <button
        v-if="sttAvailable()"
        type="button"
        class="inline-flex items-center justify-center w-10 h-10 rounded-md border transition-colors"
        :class="listening ? 'border-claude-accent text-claude-accent bg-claude-accent/10' : 'border-claude-border text-claude-muted hover:text-claude-text hover:border-claude-text'"
        title="Voice input"
        @click="toggleMic"
      >
        <Mic class="w-4 h-4" :stroke-width="1.75" />
      </button>
      <button
        v-if="ttsAvailable()"
        type="button"
        class="inline-flex items-center justify-center w-10 h-10 rounded-md border border-claude-border transition-colors"
        :class="ttsEnabled ? 'text-claude-accent' : 'text-claude-muted'"
        :title="ttsEnabled ? 'Mute narrator' : 'Unmute narrator'"
        @click="toggleTts"
      >
        <component :is="ttsEnabled ? Volume2 : VolumeX" class="w-4 h-4" :stroke-width="1.75" />
      </button>
      <button
        v-if="log.length"
        type="button"
        class="inline-flex items-center justify-center w-10 h-10 rounded-md border border-claude-border text-claude-muted hover:text-claude-text hover:border-claude-text transition-colors"
        title="Clear conversation"
        @click="clearConversation"
      >
        <X class="w-4 h-4" :stroke-width="1.75" />
      </button>
      <button type="submit" :class="BTN_PRIMARY" :disabled="busy || !body.connected">
        <Send class="w-4 h-4" :stroke-width="1.75" />
        <span>go</span>
      </button>
    </form>
  </div>
</template>
