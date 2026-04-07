// SOMA API Client — HTTP to MCP JSON-RPC bridge

class SomaAPI {
  constructor(baseUrl = '') {
    this.baseUrl = baseUrl;
    this.initialized = false;
    this.connected = false;
    this._requestId = 1;
  }

  _nextId() {
    return this._requestId++;
  }

  async checkStatus() {
    try {
      const res = await fetch(`${this.baseUrl}/api/status`);
      const data = await res.json();
      this.connected = data.soma;
      return data;
    } catch (e) {
      this.connected = false;
      return { soma: false, mcp: false };
    }
  }

  async call(method, params = {}) {
    const response = await fetch(`${this.baseUrl}/api/mcp`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: this._nextId(),
        method,
        params
      })
    });
    const data = await response.json();
    if (data.error) {
      throw new Error(data.error.message || 'MCP error');
    }
    return data;
  }

  async callTool(name, args = {}) {
    return this.call('tools/call', { name, arguments: args });
  }

  // Convenience methods
  async listTools() {
    return this.call('tools/list');
  }

  async listTables() {
    return this.callTool('soma.postgres.list_tables');
  }

  async query(sql) {
    return this.callTool('soma.postgres.query', { sql });
  }

  async find(spec) {
    return this.callTool('soma.postgres.find', { spec: JSON.stringify(spec) });
  }

  async count(spec) {
    return this.callTool('soma.postgres.count', { spec: JSON.stringify(spec) });
  }

  async getState() {
    return this.callTool('soma.get_state');
  }

  async generateUuid() {
    return this.callTool('soma.crypto.random_uuid');
  }
}

// Global instance
const api = new SomaAPI();
