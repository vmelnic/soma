<script setup>
import { onMounted } from 'vue';
import {
  RefreshCw, Smartphone, Network, Brain, Camera, MapPin, Vibrate,
  Clipboard, HardDrive, Mic, Bell, Nfc, ScanText, KeyRound,
} from 'lucide-vue-next';
import { useBodyStore } from '../stores/body.js';
import { useBrainConfigStore, ROLE_DECIDER, ROLE_NARRATOR, NARRATOR_MODES } from '../stores/brainConfig.js';

const body = useBodyStore();
const brains = useBrainConfigStore();

const ROLES = [ROLE_DECIDER, ROLE_NARRATOR];

const BTN = 'inline-flex items-center gap-1.5 bg-claude-surface text-claude-text border border-claude-border rounded-md px-2.5 py-1 text-xs cursor-pointer transition-colors hover:border-claude-accent disabled:opacity-40 disabled:cursor-not-allowed';
const INPUT = 'bg-claude-surface text-claude-text border border-claude-border rounded-md px-2 py-1.5 text-sm focus:outline-none focus:border-claude-accent';

const PORT_ICONS = {
  camera:        Camera,
  geo:           MapPin,
  haptics:       Vibrate,
  clipboard:     Clipboard,
  filesystem:    HardDrive,
  mic:           Mic,
  notifications: Bell,
  nfc:           Nfc,
  ocr:           ScanText,
};

async function refresh() { await body.refreshRemotePorts(); }
function updateRole(role, field, value) { brains.setRole(role, { [field]: value }); }

onMounted(refresh);
</script>

<template>
  <div class="p-4 min-h-full flex flex-col gap-7">

    <section>
      <div class="flex items-center gap-2 mb-3">
        <Smartphone class="w-4 h-4 text-claude-muted" :stroke-width="1.75" />
        <h3 class="text-xs uppercase tracking-widest text-claude-muted">This device</h3>
      </div>
      <div class="font-mono text-sm flex gap-3 items-baseline">
        <span class="text-claude-muted min-w-[10ch]">id</span>
        <code class="text-claude-accent">{{ body.deviceId.slice(0, 8) }}</code>
      </div>
      <div class="font-mono text-sm flex gap-3 items-center mt-1.5">
        <span class="text-claude-muted min-w-[10ch]">auth token</span>
        <input
          :class="[INPUT, 'flex-1 max-w-xs']"
          type="password"
          :value="body.authToken"
          @change="body.setAuthToken($event.target.value)"
          placeholder="optional — for --mcp-ws-token"
        />
        <KeyRound class="w-3.5 h-3.5 text-claude-muted" :stroke-width="1.75" />
      </div>
      <div class="font-mono text-sm flex gap-3 items-baseline mt-1.5">
        <span class="text-claude-muted min-w-[10ch]">local ports</span>
        <span v-if="!body.localPorts.length" class="text-claude-muted">none registered (connect first)</span>
        <span v-else class="flex flex-wrap gap-2">
          <span
            v-for="id in body.localPorts" :key="id"
            class="inline-flex items-center gap-1 bg-claude-surface border border-claude-border rounded px-2 py-0.5 text-xs"
          >
            <component :is="PORT_ICONS[id] || Camera" class="w-3 h-3 text-claude-accent" :stroke-width="1.75" />
            <span>{{ id }}</span>
          </span>
        </span>
      </div>
    </section>

    <section>
      <div class="flex items-center justify-between mb-3">
        <div class="flex items-center gap-2">
          <Network class="w-4 h-4 text-claude-muted" :stroke-width="1.75" />
          <h3 class="text-xs uppercase tracking-widest text-claude-muted">Attached devices</h3>
        </div>
        <button :class="BTN" @click="refresh" :disabled="!body.connected">
          <RefreshCw class="w-3.5 h-3.5" :stroke-width="1.75" />
          <span>refresh</span>
        </button>
      </div>
      <ul class="divide-y divide-claude-border font-mono text-sm">
        <li
          v-for="p in body.remotePorts" :key="p.port_id + p.device_id"
          class="flex items-center gap-3 py-2"
        >
          <component :is="PORT_ICONS[p.port_id] || Camera" class="w-4 h-4 text-claude-accent" :stroke-width="1.75" />
          <code class="text-claude-accent">{{ p.port_id }}</code>
          <span class="text-claude-muted">{{ p.device_id }}</span>
        </li>
        <li v-if="!body.remotePorts.length" class="text-claude-muted py-2 text-sm">no remote ports registered</li>
      </ul>
    </section>

    <section>
      <div class="flex items-center gap-2 mb-3">
        <Brain class="w-4 h-4 text-claude-muted" :stroke-width="1.75" />
        <h3 class="text-xs uppercase tracking-widest text-claude-muted">Brains</h3>
      </div>
      <div
        v-for="role in ROLES" :key="role"
        class="grid grid-cols-[10ch_auto_1fr_1fr] gap-2 items-center mb-2"
      >
        <label class="font-mono uppercase text-xs text-claude-muted">{{ role }}</label>
        <select
          :class="INPUT"
          :value="brains.config[role].provider"
          @change="updateRole(role, 'provider', $event.target.value)"
        >
          <option v-for="p in brains.providers()" :key="p" :value="p">{{ p }}</option>
        </select>
        <input
          :class="INPUT"
          :value="brains.config[role].model"
          @change="updateRole(role, 'model', $event.target.value)"
          placeholder="model id"
        />
        <input
          :class="INPUT"
          type="password"
          :value="brains.config[role].apiKey"
          @change="updateRole(role, 'apiKey', $event.target.value)"
          placeholder="api key"
        />
        <select
          v-if="role === ROLE_NARRATOR"
          :class="INPUT"
          class="col-start-2"
          :value="brains.config[role].narratorMode || 'terse'"
          @change="updateRole(role, 'narratorMode', $event.target.value)"
        >
          <option v-for="m in NARRATOR_MODES" :key="m" :value="m">{{ m }}</option>
        </select>
      </div>
    </section>

  </div>
</template>
