// Brain abstraction layer for the SOMA PWA.
//
// Canonical request/response shape is OpenAI Chat Completions with tool
// calling. Every provider adapts to/from this canonical shape. The response
// always normalizes to: { text, tool_calls, usage, id, model }.

// ─────────────────────────── Providers ───────────────────────────

const passThroughProvider = (name, baseUrl, buildHeaders) => ({
  name,
  baseUrl,
  endpoint: '/chat/completions',
  buildHeaders,
  requestAdapter: (req) => req,
  responseAdapter: (res) => {
    const msg = res.choices?.[0]?.message || {};
    return {
      id: res.id,
      model: res.model,
      text: msg.content || '',
      tool_calls: msg.tool_calls || [],
      usage: res.usage,
    };
  },
});

const anthropicProvider = () => ({
  name: 'anthropic',
  baseUrl: 'https://api.anthropic.com/v1',
  endpoint: '/messages',
  buildHeaders: (apiKey) => ({
    'x-api-key': apiKey,
    'anthropic-version': '2023-06-01',
  }),
  requestAdapter: (req) => {
    const systems = (req.messages || [])
      .filter((m) => m.role === 'system')
      .map((m) => m.content)
      .join('\n\n');
    const msgs = (req.messages || [])
      .filter((m) => m.role !== 'system')
      .map((m) => ({
        role: m.role === 'assistant' ? 'assistant' : 'user',
        content: typeof m.content === 'string' ? m.content : JSON.stringify(m.content),
      }));
    const out = {
      model: req.model,
      max_tokens: req.max_tokens ?? 1024,
      messages: msgs,
    };
    if (systems) out.system = systems;
    if (typeof req.temperature === 'number') out.temperature = req.temperature;
    if (req.tools?.length) {
      out.tools = req.tools.map((t) => ({
        name: t.function.name,
        description: t.function.description || '',
        input_schema: t.function.parameters || { type: 'object', properties: {} },
      }));
    }
    return out;
  },
  responseAdapter: (raw) => {
    const blocks = raw.content || [];
    const text = blocks
      .filter((b) => b.type === 'text')
      .map((b) => b.text)
      .join('');
    const tool_calls = blocks
      .filter((b) => b.type === 'tool_use')
      .map((b) => ({
        id: b.id,
        type: 'function',
        function: { name: b.name, arguments: JSON.stringify(b.input) },
      }));
    return {
      id: raw.id,
      model: raw.model,
      text,
      tool_calls,
      usage: raw.usage
        ? {
            prompt_tokens: raw.usage.input_tokens,
            completion_tokens: raw.usage.output_tokens,
            total_tokens: (raw.usage.input_tokens || 0) + (raw.usage.output_tokens || 0),
          }
        : undefined,
    };
  },
});

export const PROVIDERS = {
  openai: passThroughProvider(
    'openai',
    'https://api.openai.com/v1',
    (apiKey) => ({ Authorization: `Bearer ${apiKey}` }),
  ),
  groq: passThroughProvider(
    'groq',
    'https://api.groq.com/openai/v1',
    (apiKey) => ({ Authorization: `Bearer ${apiKey}` }),
  ),
  glm: passThroughProvider(
    'glm',
    'https://open.bigmodel.cn/api/paas/v4',
    (apiKey) => ({ Authorization: `Bearer ${apiKey}` }),
  ),
  together: passThroughProvider(
    'together',
    'https://api.together.xyz/v1',
    (apiKey) => ({ Authorization: `Bearer ${apiKey}` }),
  ),
  ollama: passThroughProvider(
    'ollama',
    'http://localhost:11434/v1',
    () => ({}),
  ),
  lmstudio: passThroughProvider(
    'lmstudio',
    'http://localhost:1234/v1',
    () => ({}),
  ),
  anthropic: anthropicProvider(),
};

export function openAiCompatibleProvider({ name, baseUrl, authHeader = 'Authorization', authPrefix = 'Bearer ' }) {
  return passThroughProvider(
    name,
    baseUrl.replace(/\/+$/, ''),
    (apiKey) => (apiKey ? { [authHeader]: `${authPrefix}${apiKey}` } : {}),
  );
}

// ─────────────────────────── Brain instance ───────────────────────────

export function createBrain({ provider, apiKey = '', model, baseUrl, fetchImpl }) {
  if (!provider) throw new Error('createBrain: provider required');
  if (!model) throw new Error('createBrain: model required');
  const effectiveBaseUrl = (baseUrl || provider.baseUrl).replace(/\/+$/, '');
  const url = effectiveBaseUrl + provider.endpoint;

  async function chat(canonicalRequest, { signal } = {}) {
    const req = { model, ...canonicalRequest };
    const wireReq = provider.requestAdapter(req);
    const f = fetchImpl || globalThis.fetch;
    if (typeof f !== 'function') {
      throw new Error('brain.chat: no fetch implementation available');
    }
    const headers = {
      'content-type': 'application/json',
      ...provider.buildHeaders(apiKey),
    };
    const res = await f(url, {
      method: 'POST',
      headers,
      body: JSON.stringify(wireReq),
      signal,
    });
    if (!res.ok) {
      const body = await res.text().catch(() => '');
      throw new Error(`${provider.name} ${res.status}: ${body || res.statusText}`);
    }
    const raw = await res.json();
    return provider.responseAdapter(raw);
  }

  return { provider: provider.name, model, baseUrl: effectiveBaseUrl, chat };
}

// ─────────────────────────── Router (role-based) ───────────────────────────

export const ROLE_DECIDER = 'decider';
export const ROLE_NARRATOR = 'narrator';

export class BrainRouter {
  constructor() {
    this._brains = new Map();
  }
  set(role, brain) {
    this._brains.set(role, brain);
    return this;
  }
  get(role) {
    const b = this._brains.get(role);
    if (!b) throw new Error(`no brain registered for role '${role}'`);
    return b;
  }
  has(role) {
    return this._brains.has(role);
  }
  roles() {
    return [...this._brains.keys()];
  }
  chat(role, req, opts) {
    return this.get(role).chat(req, opts);
  }
}
