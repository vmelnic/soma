// SomaClient — WebSocket client for the SOMA MCP WS transport.
//
// Responsibilities:
//   1. JSON-RPC 2.0 request/response correlation (client → server).
//   2. Reverse-port handling (server → client) — when the server sends
//      `reverse/invoke_port`, dispatch to a locally-registered port handler
//      and return the result as a JSON-RPC response on the same socket.
//
// This module is WebSocket-agnostic — pass any constructor that returns a
// WebSocket-shaped object (browser WebSocket, Node 'ws' package, a mock).

export class SomaClient {
  constructor({ url, wsImpl, onEvent, token } = {}) {
    if (!url) throw new Error('SomaClient: url required');
    this.url = url;
    this.wsImpl = wsImpl || globalThis.WebSocket;
    if (typeof this.wsImpl !== 'function') {
      throw new Error('SomaClient: no WebSocket implementation available');
    }
    this.onEvent = onEvent || (() => {});
    this._token = token || null;
    this._ws = null;
    this._nextId = 1;
    this._pending = new Map(); // id -> {resolve, reject}
    this._portHandlers = new Map(); // port_id -> async (capability_id, input) => result
    this._deviceId = null;
  }

  _id() {
    return String(this._nextId++);
  }

  connect() {
    return new Promise((resolve, reject) => {
      const ws = new this.wsImpl(this.url);
      this._ws = ws;
      const finishOpen = () => {
        ws.removeEventListener?.('error', onError);
        resolve();
      };
      const onError = (e) => {
        reject(new Error(`SomaClient connect error: ${e?.message || 'unknown'}`));
      };
      if (ws.addEventListener) {
        ws.addEventListener('open', finishOpen, { once: true });
        ws.addEventListener('error', onError, { once: true });
        ws.addEventListener('message', (e) => this._onMessage(e.data));
        ws.addEventListener('close', () => this._onClose());
      } else {
        // node 'ws' style
        ws.on('open', finishOpen);
        ws.on('error', onError);
        ws.on('message', (d) => this._onMessage(d.toString()));
        ws.on('close', () => this._onClose());
      }
    });
  }

  close() {
    try { this._ws?.close(); } catch { /* ignore */ }
  }

  _send(obj) {
    const text = JSON.stringify(obj);
    if (!this._ws || this._ws.readyState !== 1) {
      throw new Error('SomaClient: socket not open');
    }
    this._ws.send(text);
  }

  _onMessage(text) {
    let msg;
    try { msg = JSON.parse(text); }
    catch { return; }

    // Response to our own request?
    if ((msg.result !== undefined || msg.error !== undefined) && msg.method === undefined) {
      const p = this._pending.get(String(msg.id));
      if (!p) return;
      this._pending.delete(String(msg.id));
      if (msg.error) p.reject(new Error(`${msg.error.code}: ${msg.error.message}`));
      else p.resolve(msg.result);
      return;
    }

    // Server-initiated request (reverse-port).
    if (msg.method === 'reverse/invoke_port') {
      const { port_id, capability_id, input } = msg.params || {};
      const handler = this._portHandlers.get(port_id);
      const replyId = msg.id;
      if (!handler) {
        this._send({
          jsonrpc: '2.0',
          id: replyId,
          error: { code: -32601, message: `no handler for port '${port_id}'` },
        });
        return;
      }
      // Fire-and-forget: handler may be async.
      Promise.resolve()
        .then(() => handler(capability_id, input ?? {}))
        .then((result) => {
          this._send({ jsonrpc: '2.0', id: replyId, result: { result } });
        })
        .catch((err) => {
          this._send({
            jsonrpc: '2.0',
            id: replyId,
            error: { code: -32000, message: String(err?.message || err) },
          });
        });
      return;
    }

    // Unsolicited notification / stream event.
    this.onEvent(msg);
  }

  _onClose() {
    for (const { reject } of this._pending.values()) {
      reject(new Error('SomaClient: connection closed'));
    }
    this._pending.clear();
  }

  // ─────────────────── client → server requests ───────────────────

  request(method, params = {}) {
    return new Promise((resolve, reject) => {
      const id = this._id();
      this._pending.set(id, { resolve, reject });
      try {
        this._send({ jsonrpc: '2.0', id, method, params });
      } catch (e) {
        this._pending.delete(id);
        reject(e);
      }
    });
  }

  // Unwrap MCP content-array tool results to the real payload.
  async callTool(name, args = {}) {
    const raw = await this.request('tools/call', { name, arguments: args });
    if (raw && Array.isArray(raw.content) && raw.content[0]?.type === 'text') {
      try { return JSON.parse(raw.content[0].text); }
      catch { return raw.content[0].text; }
    }
    return raw;
  }

  listTools() { return this.request('tools/list', {}); }

  async initialize() {
    if (this._token) {
      await this.request('auth', { token: this._token });
    }
    return this.request('initialize', {});
  }

  // ─────────────────── reverse-port registration ───────────────────

  // handler: async (capability_id, input) => result
  registerLocalPort(portId, capabilities, handler) {
    if (typeof handler !== 'function') throw new Error('handler must be a function');
    this._portHandlers.set(portId, handler);
    return { port_id: portId, capabilities: capabilities || [] };
  }

  async announceLocalPorts(deviceId, manifests) {
    this._deviceId = deviceId;
    return this.request('reverse/register_ports', {
      device_id: deviceId,
      ports: manifests,
    });
  }

  async listRemotePorts() {
    return this.request('reverse/list_ports', {});
  }
}
