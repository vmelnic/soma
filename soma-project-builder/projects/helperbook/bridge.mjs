import { createServer } from 'node:http';
import { WebSocket } from 'ws';

const SOMA_WS = process.env.SOMA_WS_URL || 'ws://127.0.0.1:9090';
const PORT = parseInt(process.env.BRIDGE_PORT || '3000', 10);

let reqId = 1;
let ws = null;
let pending = new Map();
let ready = false;

function connect() {
  ws = new WebSocket(SOMA_WS);
  ws.on('open', () => {
    const init = { jsonrpc: '2.0', id: reqId++, method: 'initialize', params: { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'bridge', version: '0.1.0' } } };
    ws.send(JSON.stringify(init));
  });
  ws.on('message', (data) => {
    const msg = JSON.parse(data.toString());
    if (msg.id && pending.has(msg.id)) {
      pending.get(msg.id)(msg);
      pending.delete(msg.id);
    }
    if (!ready && msg.result?.serverInfo) {
      ready = true;
      console.log(`bridge: connected to soma (${msg.result.serverInfo.name})`);
    }
  });
  ws.on('close', () => { ready = false; setTimeout(connect, 1000); });
  ws.on('error', () => {});
}

function rpc(method, params) {
  return new Promise((resolve, reject) => {
    if (!ready) return reject(new Error('soma not connected'));
    const id = reqId++;
    pending.set(id, resolve);
    ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }));
    setTimeout(() => { if (pending.has(id)) { pending.delete(id); reject(new Error('timeout')); } }, 30000);
  });
}

function readBody(req) {
  return new Promise((resolve) => {
    const chunks = [];
    req.on('data', c => chunks.push(c));
    req.on('end', () => resolve(Buffer.concat(chunks).toString()));
  });
}

const server = createServer(async (req, res) => {
  console.log(`bridge: ${req.method} ${req.url}`);
  res.setHeader('Content-Type', 'application/json');
  res.setHeader('Access-Control-Allow-Origin', '*');
  res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
  res.setHeader('Access-Control-Allow-Headers', 'Content-Type');

  if (req.method === 'OPTIONS') {
    res.writeHead(204);
    res.end();
    return;
  }

  if (req.method === 'GET' && req.url === '/health') {
    res.end(JSON.stringify({ ok: ready }));
    return;
  }

  if (req.method !== 'POST') {
    res.writeHead(405);
    res.end(JSON.stringify({ error: 'method not allowed' }));
    return;
  }

  const parts = req.url.split('/').filter(Boolean);
  if (parts.length < 1) {
    res.writeHead(400);
    res.end(JSON.stringify({ error: 'POST /:routine_id or POST /port/:port_id/:capability' }));
    return;
  }

  let input = {};
  try {
    const body = await readBody(req);
    if (body) input = JSON.parse(body);
  } catch {
    res.writeHead(400);
    res.end(JSON.stringify({ error: 'invalid json' }));
    return;
  }

  try {
    let toolName, toolArgs;

    if (parts[0] === 'port' && parts.length >= 3) {
      toolName = 'invoke_port';
      toolArgs = { port_id: parts[1], capability_id: parts[2], input };
    } else {
      toolName = 'execute_routine';
      toolArgs = { routine_id: parts[0], input };
    }

    const resp = await rpc('tools/call', { name: toolName, arguments: toolArgs });

    if (resp.error) {
      res.writeHead(502);
      res.end(JSON.stringify({ error: resp.error.message }));
      return;
    }

    const content = resp.result?.content?.[0];
    let result;
    try { result = JSON.parse(content?.text || '{}'); } catch { result = content; }

    const ok = result.status === 'completed' || result.success === true;
    res.writeHead(ok ? 200 : 422);
    res.end(JSON.stringify(result));
  } catch (e) {
    res.writeHead(502);
    res.end(JSON.stringify({ error: e.message }));
  }
});

connect();
server.listen(PORT, () => console.log(`bridge: listening on :${PORT}`));
