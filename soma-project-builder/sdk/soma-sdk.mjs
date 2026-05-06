/**
 * SOMA SDK — thin JS client wrapping the HTTP bridge.
 * Framework-agnostic ES module. No state, no reactivity, no deps.
 *
 * Usage:
 *   const soma = new SomaSDK('http://localhost:3000');
 *   const result = await soma.execute('get_user_profile', { token, table: 'users', id });
 *   const portResult = await soma.invoke('auth', 'session_create', { user_id });
 *   const health = await soma.health();
 */

export class SomaSDK {
  constructor(baseUrl) {
    this.baseUrl = baseUrl.replace(/\/+$/, '');
  }

  async execute(routineId, input = {}) {
    const res = await fetch(`${this.baseUrl}/${routineId}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(input),
    });
    const data = await res.json();
    if (!res.ok) {
      const err = new Error(data.error || `execute failed: ${res.status}`);
      err.status = res.status;
      err.data = data;
      throw err;
    }
    return data;
  }

  async invoke(portId, capabilityId, input = {}) {
    const res = await fetch(`${this.baseUrl}/port/${portId}/${capabilityId}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(input),
    });
    const data = await res.json();
    if (!res.ok) {
      const err = new Error(data.error || `invoke failed: ${res.status}`);
      err.status = res.status;
      err.data = data;
      throw err;
    }
    return data;
  }

  async health() {
    const res = await fetch(`${this.baseUrl}/health`);
    return res.json();
  }
}

export default SomaSDK;
