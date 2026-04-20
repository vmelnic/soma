import { test } from 'node:test';
import assert from 'node:assert/strict';

import { cameraPort } from './camera.js';
import { geoPort } from './geo.js';
import { hapticsPort } from './haptics.js';
import { clipboardPort } from './clipboard.js';
import { filesystemPort } from './filesystem.js';
import { registerAllPorts } from './index.js';
import { setPlatformForTesting } from './platform.js';

test('camera handler dispatches capture_image via adapter', async () => {
  let called = 0;
  const port = cameraPort({
    adapter: {
      async capture(input) {
        called++;
        assert.equal(input.facingMode, 'user');
        return { image_base64: 'YWJj', mime: 'image/jpeg' };
      },
    },
  });
  assert.deepEqual(port.manifest, { port_id: 'camera', capabilities: ['capture_image'] });
  const r = await port.handler('capture_image', { facingMode: 'user' });
  assert.equal(called, 1);
  assert.equal(r.image_base64, 'YWJj');
});

test('camera rejects unknown capability', async () => {
  const port = cameraPort({ adapter: { capture: async () => ({}) } });
  await assert.rejects(port.handler('record_video', {}), /unknown capability/);
});

test('geo handler returns normalized position', async () => {
  const port = geoPort({
    adapter: {
      async currentPosition() {
        return { lat: 44.43, lon: 26.10, accuracy_m: 12, ts: 1_710_000_000_000 };
      },
    },
  });
  const r = await port.handler('current_position', {});
  assert.equal(r.lat, 44.43);
  assert.equal(r.accuracy_m, 12);
});

test('haptics vibrate clamps duration and forwards', async () => {
  const log = [];
  const port = hapticsPort({
    adapter: {
      async vibrate(ms) { log.push(['vibrate', ms]); },
      async impact(s)    { log.push(['impact', s]); },
    },
  });
  const r1 = await port.handler('vibrate', { ms: 99999 });
  assert.equal(r1.ms, 2000);
  const r2 = await port.handler('impact', { style: 'heavy' });
  assert.equal(r2.style, 'heavy');
  assert.deepEqual(log, [['vibrate', 2000], ['impact', 'heavy']]);
});

test('clipboard read/write round-trip', async () => {
  let store = '';
  const port = clipboardPort({
    adapter: {
      async read() { return store; },
      async write(t) { store = t; },
    },
  });
  await port.handler('write', { text: 'hi' });
  const r = await port.handler('read', {});
  assert.equal(r.text, 'hi');
});

test('filesystem write → read → list → delete', async () => {
  const mem = new Map();
  const port = filesystemPort({
    adapter: {
      async read(p)        { return mem.get(p) ?? ''; },
      async write(p, d)    { mem.set(p, String(d ?? '')); return String(d ?? '').length; },
      async remove(p)      { return mem.delete(p); },
      async list(prefix)   { return [...mem.keys()].filter((k) => k.startsWith(prefix)); },
    },
  });
  const w = await port.handler('write', { path: '/a.txt', data: 'body' });
  assert.equal(w.bytes, 4);
  const r = await port.handler('read', { path: '/a.txt' });
  assert.equal(r.data, 'body');
  const l = await port.handler('list', { path: '/' });
  assert.deepEqual(l.entries, ['/a.txt']);
  const d = await port.handler('delete', { path: '/a.txt' });
  assert.equal(d.ok, true);
});

test('registerAllPorts registers handlers and announces manifests', async () => {
  setPlatformForTesting('browser');
  const registered = [];
  let announced = null;
  const fakeClient = {
    registerLocalPort(portId, caps, handler) {
      registered.push({ portId, caps, handlerOk: typeof handler === 'function' });
    },
    async announceLocalPorts(deviceId, manifests) {
      announced = { deviceId, manifests };
      return { registered: manifests.length };
    },
  };

  const stubPorts = [
    cameraPort({    adapter: { capture: async () => ({}) } }),
    geoPort({       adapter: { currentPosition: async () => ({}) } }),
    hapticsPort({   adapter: { vibrate: async () => {}, impact: async () => {} } }),
    clipboardPort({ adapter: { read: async () => '', write: async () => {} } }),
    filesystemPort({
      adapter: {
        read: async () => '', write: async () => 0, remove: async () => true, list: async () => [],
      },
    }),
  ];

  const r = await registerAllPorts(fakeClient, 'device-xyz', { ports: stubPorts });
  assert.equal(r.count, 5);
  assert.deepEqual(r.port_ids.sort(), ['camera', 'clipboard', 'filesystem', 'geo', 'haptics']);
  assert.equal(registered.length, 5);
  assert.ok(registered.every((e) => e.handlerOk));
  assert.equal(announced.deviceId, 'device-xyz');
  assert.equal(announced.manifests.length, 5);
});
