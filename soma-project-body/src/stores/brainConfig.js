// Pinia store: operator-configurable brain router.
//
// Two roles (decider, narrator) each point at a provider + model + key.
// Keys are stored in localStorage (PWA-local). Swap providers live without
// re-mounting the rest of the app.

import { defineStore } from 'pinia';
import { ref, computed } from 'vue';
import {
  PROVIDERS, createBrain, BrainRouter,
  ROLE_DECIDER, ROLE_NARRATOR,
} from '../lib/brain.js';

const LS_KEY = 'soma.brainConfig.v1';

function load() {
  try {
    const raw = localStorage.getItem(LS_KEY);
    return raw ? JSON.parse(raw) : null;
  } catch { return null; }
}

export const NARRATOR_MODES = ['terse', 'debug', 'alarm'];

function defaults() {
  return {
    [ROLE_DECIDER]:  { provider: 'openai',    model: 'gpt-4o-mini',                apiKey: '' },
    [ROLE_NARRATOR]: { provider: 'openai',    model: 'gpt-4o-mini',                apiKey: '', narratorMode: 'terse' },
  };
}

export const useBrainConfigStore = defineStore('brainConfig', () => {
  const config = ref(load() || defaults());

  function save() {
    localStorage.setItem(LS_KEY, JSON.stringify(config.value));
  }

  function setRole(role, cfg) {
    config.value = { ...config.value, [role]: { ...config.value[role], ...cfg } };
    save();
  }

  const router = computed(() => {
    const r = new BrainRouter();
    for (const [role, c] of Object.entries(config.value)) {
      const prov = PROVIDERS[c.provider];
      if (!prov || !c.model) continue;
      r.set(role, createBrain({
        provider: prov,
        apiKey: c.apiKey,
        model: c.model,
      }));
    }
    return r;
  });

  function providers() { return Object.keys(PROVIDERS); }

  return { config, setRole, router, providers };
});

export { ROLE_DECIDER, ROLE_NARRATOR };
