// Pinia store: connection state + live body data from SOMA.
//
// Owns the SomaClient instance and exposes reactive views into ports,
// sessions, events, and the local device id.

import { defineStore } from 'pinia';
import { reactive, ref } from 'vue';
import { SomaClient } from '../lib/somaClient.js';
import { registerAllPorts } from '../ports/index.js';
import { selectHandoffs } from '../lib/handoff.js';

export const useBodyStore = defineStore('body', () => {
  const serverUrl = ref(
    localStorage.getItem('soma.serverUrl') || 'ws://127.0.0.1:7890',
  );
  const deviceId = ref(
    localStorage.getItem('soma.deviceId') || crypto.randomUUID(),
  );
  localStorage.setItem('soma.deviceId', deviceId.value);

  const authToken = ref(localStorage.getItem('soma.authToken') || '');
  const connected = ref(false);
  const connectError = ref(null);
  const remotePorts = ref([]);      // ports registered by other devices
  const localPorts = ref([]);       // ports this device offers to SOMA
  const ports = ref([]);            // ports loaded in the runtime (list_ports)
  const tools = ref([]);            // MCP tools exposed by the runtime
  const events = reactive([]);      // narrator-observable event stream
  const client = { value: null };   // non-reactive to avoid Vue proxying WS

  function setServerUrl(url) {
    serverUrl.value = url;
    localStorage.setItem('soma.serverUrl', url);
  }

  function setAuthToken(token) {
    authToken.value = token;
    localStorage.setItem('soma.authToken', token);
  }

  async function connect() {
    connectError.value = null;
    if (client.value) {
      try { client.value.close(); } catch { /* ignore */ }
    }
    const c = new SomaClient({
      url: serverUrl.value,
      token: authToken.value || undefined,
      onEvent: (msg) => {
        events.unshift({ ts: Date.now(), msg });
        if (events.length > 200) events.length = 200;
      },
    });
    try {
      await c.connect();
      await c.initialize();
      const t = await c.listTools();
      tools.value = t?.tools || [];
      const r = await c.listRemotePorts().catch(() => ({ ports: [] }));
      remotePorts.value = r?.ports || [];
      const p = await c.callTool('list_ports', {}).catch(() => ({ ports: [] }));
      ports.value = p?.ports || [];
      client.value = c;
      connected.value = true;

      // Announce this device's sensors as reverse-routed ports.
      try {
        const reg = await registerAllPorts(c, deviceId.value);
        localPorts.value = reg.port_ids;
      } catch (e) {
        // Non-fatal: session still works for goal submission, just without
        // this device contributing ports. Surface quietly.
        connectError.value = `local ports: ${e.message || e}`;
      }
    } catch (e) {
      connectError.value = e.message || String(e);
      connected.value = false;
      throw e;
    }
  }

  function disconnect() {
    if (client.value) {
      try { client.value.close(); } catch { /* ignore */ }
      client.value = null;
    }
    connected.value = false;
  }

  async function announcePorts(manifests) {
    if (!client.value) throw new Error('not connected');
    return client.value.announceLocalPorts(deviceId.value, manifests);
  }

  function registerLocalPort(portId, capabilities, handler) {
    if (!client.value) throw new Error('not connected');
    return client.value.registerLocalPort(portId, capabilities, handler);
  }

  async function refreshRemotePorts() {
    if (!client.value) return;
    const r = await client.value.listRemotePorts().catch(() => ({ ports: [] }));
    remotePorts.value = r?.ports || [];
  }

  async function callTool(name, args) {
    if (!client.value) throw new Error('not connected');
    return client.value.callTool(name, args);
  }

  // ─────────────────────────── Session handoff ───────────────────────────

  async function handoffSession({ sessionId, toDevice, objective }) {
    return callTool('handoff_session', {
      session_id: sessionId,
      from_device: deviceId.value,
      to_device: toDevice || undefined,
      objective: objective || undefined,
    });
  }

  async function claimSession(sessionId) {
    return callTool('claim_session', {
      session_id: sessionId,
      device_id: deviceId.value,
    });
  }

  async function fetchHandoffs() {
    if (!client.value) return { openHandoffs: [], myClaims: [], othersClaimed: [] };
    const dump = await callTool('dump_world_state', {}).catch(() => ({ facts: [] }));
    const facts = dump?.facts || [];
    return selectHandoffs(facts, deviceId.value);
  }

  return {
    serverUrl, deviceId, authToken, connected, connectError,
    remotePorts, localPorts, ports, tools, events,
    setServerUrl, setAuthToken, connect, disconnect,
    announcePorts, registerLocalPort, refreshRemotePorts, callTool,
    handoffSession, claimSession, fetchHandoffs,
  };
});
