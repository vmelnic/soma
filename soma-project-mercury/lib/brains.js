const MERCURY = {
  name: 'mercury',
  url: 'https://api.inceptionlabs.ai/v1/chat/completions',
  keyEnv: ['SOMA_MERCURY_API_KEY', 'INCEPTION_API_KEY'],
  model: 'mercury-2',
  color: '{cyan-fg}',
};

const KIMI = {
  name: 'kimi',
  url: 'https://api.moonshot.ai/v1/chat/completions',
  keyEnv: ['SOMA_KIMI_API_KEY', 'KIMI_API_KEY'],
  model: 'moonshot-v1-auto',
  color: '{yellow-fg}',
};

const GLM = {
  name: 'glm',
  url: 'https://api.z.ai/api/paas/v4/chat/completions',
  keyEnv: ['SOMA_GLM_API_KEY', 'GLM_API_KEY'],
  model: 'glm-5.1',
  color: '{green-fg}',
};

const ALL = [MERCURY, KIMI, GLM];

function getKey(brain) {
  for (const env of brain.keyEnv) {
    const val = process.env[env];
    if (val) return val;
  }
  return null;
}

async function call(brain, messages, opts = {}) {
  const key = getKey(brain);
  if (!key) {
    return { error: `no API key for ${brain.name}`, latency: 0, tokens: 0 };
  }

  const body = {
    model: opts.model || brain.model,
    messages,
    max_tokens: opts.maxTokens || 1024,
  };
  if (opts.temperature !== undefined) body.temperature = opts.temperature;
  if (opts.reasoningEffort) body.reasoning_effort = opts.reasoningEffort;

  const start = Date.now();
  try {
    const resp = await fetch(brain.url, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${key}`,
      },
      body: JSON.stringify(body),
      signal: AbortSignal.timeout(120_000),
    });

    const text = await resp.text();
    const latency = Date.now() - start;

    if (!resp.ok) {
      return { error: `HTTP ${resp.status}: ${text.slice(0, 200)}`, latency, tokens: 0 };
    }

    const data = JSON.parse(text);
    const msg = data.choices?.[0]?.message || {};
    const content = msg.content || msg.reasoning_content || '';
    const usage = data.usage || {};

    return {
      content,
      model: data.model || brain.model,
      latency,
      tokens: usage.total_tokens || 0,
      promptTokens: usage.prompt_tokens || 0,
      completionTokens: usage.completion_tokens || 0,
      finishReason: data.choices?.[0]?.finish_reason || 'unknown',
    };
  } catch (err) {
    return { error: err.message, latency: Date.now() - start, tokens: 0 };
  }
}

module.exports = { MERCURY, KIMI, GLM, ALL, call };
