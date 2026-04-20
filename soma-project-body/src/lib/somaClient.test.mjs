import { test } from 'node:test';
import assert from 'node:assert/strict';

import { SomaClient } from './somaClient.js';

// Mock WebSocket that loops sent messages back via a pluggable responder.
function makeMockWs(responder) {
  const listeners = { open: [], message: [], close: [], error: [] };
  const ws = {
    readyState: 0,
    send(text) {
      // Simulate async reply(ies).
      const replies = responder(JSON.parse(text)) || [];
      const arr = Array.isArray(replies) ? replies : [replies];
      queueMicrotask(() => {
        for (const r of arr) {
          if (r === undefined) continue;
          const text = typeof r === 'string' ? r : JSON.stringify(r);
          for (const l of listeners.message) l({ data: text });
        }
      });
    },
    close() {
      for (const l of listeners.close) l();
    },
    addEventListener(event, fn) {
      listeners[event]?.push(fn);
    },
    removeEventListener(event, fn) {
      listeners[event] = (listeners[event] || []).filter((x) => x !== fn);
    },
  };
  const wsImpl = function FakeWS(_url) {
    ws.readyState = 1;
    // open event next tick
    queueMicrotask(() => {
      for (const l of listeners.open) l();
    });
    return ws;
  };
  return { ws, wsImpl, listeners };
}

test('connect + list_tools round-trip', async () => {
  const { wsImpl } = makeMockWs((req) => {
    if (req.method === 'tools/list') {
      return { jsonrpc: '2.0', id: req.id, result: { tools: [{ name: 'x' }] } };
    }
  });
  const c = new SomaClient({ url: 'ws://fake', wsImpl });
  await c.connect();
  const r = await c.listTools();
  assert.equal(r.tools.length, 1);
  assert.equal(r.tools[0].name, 'x');
});

test('callTool unwraps MCP content text JSON', async () => {
  const { wsImpl } = makeMockWs((req) => {
    if (req.method === 'tools/call') {
      return {
        jsonrpc: '2.0',
        id: req.id,
        result: {
          content: [{ type: 'text', text: JSON.stringify({ ports: ['a', 'b'] }) }],
        },
      };
    }
  });
  const c = new SomaClient({ url: 'ws://fake', wsImpl });
  await c.connect();
  const r = await c.callTool('list_ports');
  assert.deepEqual(r, { ports: ['a', 'b'] });
});

test('server-initiated reverse/invoke_port dispatches to local handler and replies', async () => {
  const responses = [];
  let initiateCapture = null;

  const { wsImpl, listeners } = makeMockWs((req) => {
    responses.push(req);
    if (req.method === 'reverse/register_ports') {
      return { jsonrpc: '2.0', id: req.id, result: { registered: 1 } };
    }
    // reply to a reverse/invoke_port response from the client — capture it
    if (req.jsonrpc === '2.0' && req.method === undefined) {
      initiateCapture = req;
      return;
    }
  });

  const c = new SomaClient({ url: 'ws://fake', wsImpl });
  await c.connect();

  let handlerCalls = 0;
  c.registerLocalPort('camera', ['capture_image'], async (cap, input) => {
    handlerCalls++;
    assert.equal(cap, 'capture_image');
    assert.deepEqual(input, { fmt: 'jpg' });
    return { image_base64: 'YWJj', latency_ms: 42 };
  });

  await c.announceLocalPorts('phone-abc', [{ port_id: 'camera', capabilities: ['capture_image'] }]);

  // Simulate server pushing a reverse/invoke_port request.
  const serverReq = {
    jsonrpc: '2.0',
    id: 'rinv-1-1',
    method: 'reverse/invoke_port',
    params: { port_id: 'camera', capability_id: 'capture_image', input: { fmt: 'jpg' } },
  };
  for (const l of listeners.message) l({ data: JSON.stringify(serverReq) });

  // Let handler + send resolve.
  await new Promise((r) => setTimeout(r, 10));

  assert.equal(handlerCalls, 1);
  assert.ok(initiateCapture, 'client should have sent a reply frame');
  assert.equal(initiateCapture.id, 'rinv-1-1');
  assert.equal(initiateCapture.result.result.image_base64, 'YWJj');
  assert.equal(initiateCapture.result.result.latency_ms, 42);
});

test('reverse/invoke_port for unknown port returns JSON-RPC error', async () => {
  let capturedReply = null;
  const { wsImpl, listeners } = makeMockWs((req) => {
    if (req.jsonrpc === '2.0' && req.method === undefined) {
      capturedReply = req;
    }
  });
  const c = new SomaClient({ url: 'ws://fake', wsImpl });
  await c.connect();

  const serverReq = {
    jsonrpc: '2.0',
    id: 'rinv-nope',
    method: 'reverse/invoke_port',
    params: { port_id: 'ghost', capability_id: 'x', input: {} },
  };
  for (const l of listeners.message) l({ data: JSON.stringify(serverReq) });
  await new Promise((r) => setTimeout(r, 5));

  assert.ok(capturedReply);
  assert.equal(capturedReply.error.code, -32601);
  assert.match(capturedReply.error.message, /no handler for port 'ghost'/);
});

test('request rejects on error response', async () => {
  const { wsImpl } = makeMockWs((req) => ({
    jsonrpc: '2.0', id: req.id, error: { code: -32603, message: 'boom' },
  }));
  const c = new SomaClient({ url: 'ws://fake', wsImpl });
  await c.connect();
  await assert.rejects(c.request('whatever'), /-32603: boom/);
});
