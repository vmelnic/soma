// SOMA API Client — HTTP to MCP JSON-RPC bridge (soma-next)

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

  async invokePort(portId, capabilityId, input = {}) {
    return this.callTool('invoke_port', {
      port_id: portId,
      capability_id: capabilityId,
      input
    });
  }

  // Convenience: PostgreSQL
  async query(sql) {
    return this.invokePort('postgres', 'query', { sql });
  }

  async execute(sql) {
    return this.invokePort('postgres', 'execute', { sql });
  }

  async find(table, id) {
    return this.invokePort('postgres', 'find', { table, id });
  }

  async count(table, filter) {
    const input = { table };
    if (filter) input.filter = filter;
    return this.invokePort('postgres', 'count', input);
  }

  async insert(table, values) {
    return this.invokePort('postgres', 'insert', { table, values });
  }

  // Convenience: Redis
  async redisGet(key) {
    return this.invokePort('redis', 'get', { key });
  }

  async redisSet(key, value, ttl) {
    const input = { key, value };
    if (ttl) input.ttl = ttl;
    return this.invokePort('redis', 'set', input);
  }

  // Convenience: Auth
  async generateOtp(phone) {
    return this.invokePort('auth', 'otp_generate', { phone });
  }

  async verifyOtp(phone, code) {
    return this.invokePort('auth', 'otp_verify', { phone, code });
  }

  async createSession(userId) {
    return this.invokePort('auth', 'session_create', { user_id: userId });
  }

  // Convenience: list tools
  async listTools() {
    return this.call('tools/list');
  }

  /**
   * Extract rows from a soma-next invoke_port result.
   *
   * soma-next invoke_port returns a PortCallRecord directly in result:
   *   { result: { success: true, structured_result: { rows: [...], row_count: N } } }
   *
   * For postgres query: structured_result has { rows, count }
   * For postgres count: structured_result has { count }
   */
  static extractRows(response) {
    try {
      let record = response?.result;

      // MCP wraps tool results in { content: [{ type: "text", text: "..." }] }
      if (record?.content && Array.isArray(record.content)) {
        const text = record.content.find(c => c.type === 'text')?.text;
        if (text) {
          try { record = JSON.parse(text); } catch (_) { return null; }
        }
      }

      if (!record || !record.success) return null;

      const sr = record.structured_result;
      if (!sr) return null;

      // postgres query returns { rows: [...], count: N }
      if (sr.rows && Array.isArray(sr.rows)) {
        return sr.rows;
      }

      // postgres count returns { count: N }
      if ('count' in sr) {
        return [sr];
      }

      // postgres find returns { row: {...}, found: bool }
      if (sr.row) {
        return [sr.row];
      }

      // Fallback: return the structured_result as-is in an array
      return [sr];
    } catch (e) {
      console.warn('[api] Failed to extract rows:', e);
      return null;
    }
  }
}

// Global instance
const api = new SomaAPI();
