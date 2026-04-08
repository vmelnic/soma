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

  async execute(sql) {
    return this.callTool('soma.postgres.execute', { sql });
  }

  async getState() {
    return this.callTool('soma.get_state');
  }

  async generateUuid() {
    return this.callTool('soma.crypto.random_uuid');
  }

  /**
   * Extract rows from an MCP tool result.
   * The MCP response shape is:
   *   { result: { content: [{ type: "text", text: JSON }] } }
   * where the text field contains the tool result JSON:
   *   { success: true, result: <Value> }
   *
   * With the fix to tool_plugin_call, <Value> is now full JSON
   * (e.g. a List of Maps) instead of a display string like "[12 items]".
   */
  static extractRows(response) {
    try {
      const content = response?.result?.content;
      if (!content || !content.length) return null;

      const text = content[0].text;
      const parsed = typeof text === 'string' ? JSON.parse(text) : text;
      if (!parsed.success) return null;

      const result = parsed.result;

      // result is now a serialized Value enum
      // Value::List serializes as { "List": [...] }
      // Value::Map serializes as { "Map": {...} }
      // Value::String serializes as { "String": "..." }
      // Value::Int serializes as { "Int": N }
      // etc.
      return SomaAPI.unwrapValue(result);
    } catch (e) {
      console.warn('[api] Failed to extract rows:', e);
      return null;
    }
  }

  /**
   * Recursively unwrap SOMA Value enum JSON to plain JS values.
   * Value::String("x") serializes as {"String": "x"}
   * Value::Int(42) serializes as {"Int": 42}
   * Value::List([...]) serializes as {"List": [...]}
   * Value::Map({...}) serializes as {"Map": {...}}
   * etc.
   */
  static unwrapValue(val) {
    if (val === null || val === undefined) return null;

    // Primitive types — serde tagged enum format
    if (typeof val === 'object' && !Array.isArray(val)) {
      if ('Null' in val) return null;
      if ('Bool' in val) return val.Bool;
      if ('Int' in val) return val.Int;
      if ('Float' in val) return val.Float;
      if ('String' in val) return val.String;
      if ('Handle' in val) return val.Handle;
      if ('Bytes' in val) return val.Bytes;
      if ('Signal' in val) return val.Signal;
      if ('List' in val) {
        return val.List.map(item => SomaAPI.unwrapValue(item));
      }
      if ('Map' in val) {
        const out = {};
        for (const [k, v] of Object.entries(val.Map)) {
          out[k] = SomaAPI.unwrapValue(v);
        }
        return out;
      }
      // If it's a plain object (not a tagged Value), return as-is
      // This handles the case where the Value was already unwrapped
      return val;
    }

    // Already a plain value (string, number, boolean)
    return val;
  }
}

// Global instance
const api = new SomaAPI();
