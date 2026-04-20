<script setup>
import { RouterLink, RouterView } from 'vue-router';
import { storeToRefs } from 'pinia';
import {
  Activity,
  MessageSquare,
  Cpu,
  Boxes,
  Smartphone,
  Plug,
  PlugZap,
  AlertCircle,
} from 'lucide-vue-next';
import { useBodyStore } from './stores/body.js';

const body = useBodyStore();
const { connected, connectError, serverUrl } = storeToRefs(body);

const BTN = 'inline-flex items-center gap-1.5 bg-claude-surface text-claude-text border border-claude-border rounded-md px-3 py-1.5 text-sm cursor-pointer transition-colors hover:border-claude-accent hover:bg-claude-raised disabled:opacity-40 disabled:cursor-not-allowed';
const INPUT = 'bg-claude-surface text-claude-text border border-claude-border rounded-md px-2 py-1.5 text-sm focus:outline-none focus:border-claude-accent';

async function tryConnect() {
  try { await body.connect(); } catch { /* surfaces via store */ }
}

const tabs = [
  { name: 'talk',    icon: MessageSquare },
  { name: 'work',    icon: Activity },
  { name: 'apps',    icon: Boxes },
  { name: 'devices', icon: Smartphone },
];
</script>

<template>
  <div class="flex flex-col h-[100dvh]">
    <header class="glow-accent flex items-center justify-between px-4 py-3 border-b border-claude-border">
      <div class="flex items-center gap-2">
        <Cpu class="w-5 h-5 text-claude-accent" :stroke-width="1.75" />
        <div class="font-semibold tracking-[0.18em] text-claude-text text-sm">SOMA</div>
        <span class="text-claude-dim text-xs tracking-widest hidden sm:inline">· THE BODY YOU CARRY</span>
      </div>
      <div class="flex items-center gap-2 text-sm">
        <span
          class="w-2 h-2 rounded-full transition-colors"
          :class="connected ? 'bg-claude-success shadow-[0_0_8px_theme(colors.claude-success)]' : 'bg-claude-error'"
          :title="connected ? 'connected' : 'disconnected'"
        ></span>
        <input
          v-model="serverUrl"
          @change="body.setServerUrl(serverUrl)"
          spellcheck="false"
          :class="[INPUT, 'w-[20ch] font-mono']"
        />
        <button :class="BTN" @click="connected ? body.disconnect() : tryConnect()">
          <component :is="connected ? Plug : PlugZap" class="w-4 h-4" :stroke-width="1.75" />
          <span>{{ connected ? 'disconnect' : 'connect' }}</span>
        </button>
      </div>
    </header>

    <main class="flex-1 overflow-auto">
      <p
        v-if="connectError"
        class="flex items-center gap-2 bg-claude-errBg text-claude-error font-mono text-sm px-4 py-2 m-0 border-b border-claude-error/40"
      >
        <AlertCircle class="w-4 h-4 shrink-0" :stroke-width="1.75" />
        <span>{{ connectError }}</span>
      </p>
      <RouterView />
    </main>

    <nav class="grid grid-cols-4 bg-claude-bg border-t border-claude-border">
      <RouterLink
        v-for="tab in tabs"
        :key="tab.name"
        :to="`/${tab.name}`"
        class="flex flex-col items-center gap-0.5 py-2.5 text-[11px] uppercase tracking-widest text-claude-muted no-underline transition-colors hover:text-claude-text"
        active-class="!text-claude-accent"
      >
        <component :is="tab.icon" class="w-5 h-5" :stroke-width="1.75" />
        <span>{{ tab.name }}</span>
      </RouterLink>
    </nav>
  </div>
</template>
