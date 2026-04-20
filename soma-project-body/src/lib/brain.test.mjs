import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
  PROVIDERS,
  createBrain,
  BrainRouter,
  ROLE_DECIDER,
  ROLE_NARRATOR,
  responseText,
  openAiCompatibleProvider,
} from './brain.js';

// A minimal fake fetch that captures the outgoing request and returns a
// pre-built response. Lets us exercise the adapters without real HTTP.
function fakeFetch(responseBuilder) {
  const calls = [];
  const impl = async (url, init) => {
    calls.push({ url, headers: init.headers, body: JSON.parse(init.body) });
    const { status = 200, body } = responseBuilder({ url, init });
    return {
      ok: status >= 200 && status < 300,
      status,
      statusText: 'OK',
      json: async () => body,
      text: async () => JSON.stringify(body),
    };
  };
  return { impl, calls };
}

test('openai provider passes request through and preserves response', async () => {
  const { impl, calls } = fakeFetch(() => ({
    body: {
      id: 'chatcmpl-1',
      model: 'gpt-4o-mini',
      choices: [{ index: 0, message: { role: 'assistant', content: 'pong' }, finish_reason: 'stop' }],
      usage: { prompt_tokens: 3, completion_tokens: 1, total_tokens: 4 },
    },
  }));
  const brain = createBrain({
    provider: PROVIDERS.openai,
    apiKey: 'sk-test',
    model: 'gpt-4o-mini',
    fetchImpl: impl,
  });
  const resp = await brain.chat({
    messages: [{ role: 'user', content: 'ping' }],
    max_tokens: 16,
    temperature: 0.2,
  });

  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, 'https://api.openai.com/v1/chat/completions');
  assert.equal(calls[0].headers.Authorization, 'Bearer sk-test');
  assert.equal(calls[0].body.model, 'gpt-4o-mini');
  assert.equal(calls[0].body.messages[0].content, 'ping');
  assert.equal(calls[0].body.temperature, 0.2);
  assert.equal(calls[0].body.max_tokens, 16);

  assert.equal(resp.choices[0].message.content, 'pong');
  assert.equal(responseText(resp), 'pong');
});

test('anthropic adapter: system extraction + message role coercion + response reshape', async () => {
  const { impl, calls } = fakeFetch(() => ({
    body: {
      id: 'msg_01',
      model: 'claude-haiku-4-5-20251001',
      content: [{ type: 'text', text: 'hello friend' }],
      stop_reason: 'end_turn',
      usage: { input_tokens: 10, output_tokens: 2 },
    },
  }));
  const brain = createBrain({
    provider: PROVIDERS.anthropic,
    apiKey: 'sk-ant-test',
    model: 'claude-haiku-4-5-20251001',
    fetchImpl: impl,
  });
  const resp = await brain.chat({
    messages: [
      { role: 'system', content: 'Be brief.' },
      { role: 'system', content: 'No emojis.' },
      { role: 'user', content: 'say hi' },
    ],
    max_tokens: 32,
    temperature: 0,
  });

  assert.equal(calls[0].url, 'https://api.anthropic.com/v1/messages');
  assert.equal(calls[0].headers['x-api-key'], 'sk-ant-test');
  assert.equal(calls[0].headers['anthropic-version'], '2023-06-01');
  // system messages joined, not forwarded in messages[]
  assert.equal(calls[0].body.system, 'Be brief.\n\nNo emojis.');
  assert.equal(calls[0].body.messages.length, 1);
  assert.equal(calls[0].body.messages[0].role, 'user');
  assert.equal(calls[0].body.messages[0].content, 'say hi');
  assert.equal(calls[0].body.max_tokens, 32);

  // canonical (OpenAI-shaped) response
  assert.equal(resp.choices[0].message.role, 'assistant');
  assert.equal(resp.choices[0].message.content, 'hello friend');
  assert.equal(resp.choices[0].finish_reason, 'stop');
  assert.equal(resp.usage.prompt_tokens, 10);
  assert.equal(resp.usage.completion_tokens, 2);
  assert.equal(resp.usage.total_tokens, 12);
});

test('ollama provider uses localhost and no auth header', async () => {
  const { impl, calls } = fakeFetch(() => ({
    body: {
      choices: [{ index: 0, message: { role: 'assistant', content: 'local reply' }, finish_reason: 'stop' }],
    },
  }));
  const brain = createBrain({
    provider: PROVIDERS.ollama,
    model: 'llama3.1:8b',
    fetchImpl: impl,
  });
  await brain.chat({ messages: [{ role: 'user', content: 'hi' }] });
  assert.equal(calls[0].url, 'http://localhost:11434/v1/chat/completions');
  assert.equal(calls[0].headers.Authorization, undefined);
});

test('openAiCompatibleProvider builds arbitrary providers', async () => {
  const { impl, calls } = fakeFetch(() => ({
    body: { choices: [{ message: { role: 'assistant', content: 'ok' }, finish_reason: 'stop' }] },
  }));
  const custom = openAiCompatibleProvider({
    name: 'self-hosted',
    baseUrl: 'https://my-gpu-box.lan:8000/v1/',
  });
  const brain = createBrain({ provider: custom, apiKey: 'k', model: 'any', fetchImpl: impl });
  await brain.chat({ messages: [{ role: 'user', content: 'x' }] });
  // trailing slash stripped, single /chat/completions appended
  assert.equal(calls[0].url, 'https://my-gpu-box.lan:8000/v1/chat/completions');
  assert.equal(calls[0].headers.Authorization, 'Bearer k');
});

test('BrainRouter routes decider vs narrator independently', async () => {
  const deciderFetch = fakeFetch(() => ({
    body: { choices: [{ message: { role: 'assistant', content: 'pick X' }, finish_reason: 'stop' }] },
  }));
  const narratorFetch = fakeFetch(() => ({
    body: {
      id: 'msg_2',
      model: 'claude-haiku-4-5-20251001',
      content: [{ type: 'text', text: 'narrating' }],
      stop_reason: 'end_turn',
    },
  }));
  const router = new BrainRouter()
    .set(
      ROLE_DECIDER,
      createBrain({
        provider: PROVIDERS.openai,
        apiKey: 'sk',
        model: 'gpt-4o-mini',
        fetchImpl: deciderFetch.impl,
      }),
    )
    .set(
      ROLE_NARRATOR,
      createBrain({
        provider: PROVIDERS.anthropic,
        apiKey: 'sk-ant',
        model: 'claude-haiku-4-5-20251001',
        fetchImpl: narratorFetch.impl,
      }),
    );

  const d = await router.chat(ROLE_DECIDER, { messages: [{ role: 'user', content: 'decide' }] });
  const n = await router.chat(ROLE_NARRATOR, { messages: [{ role: 'user', content: 'narrate' }] });

  assert.equal(responseText(d), 'pick X');
  assert.equal(responseText(n), 'narrating');
  assert.equal(deciderFetch.calls[0].url, 'https://api.openai.com/v1/chat/completions');
  assert.equal(narratorFetch.calls[0].url, 'https://api.anthropic.com/v1/messages');
  assert.deepEqual(router.roles().sort(), [ROLE_DECIDER, ROLE_NARRATOR].sort());
});

test('chat surfaces HTTP errors with provider + status + body', async () => {
  const { impl } = fakeFetch(() => ({ status: 429, body: { error: { message: 'rate limit' } } }));
  const brain = createBrain({
    provider: PROVIDERS.openai,
    apiKey: 'sk',
    model: 'gpt-4o-mini',
    fetchImpl: impl,
  });
  await assert.rejects(
    brain.chat({ messages: [{ role: 'user', content: 'x' }] }),
    /openai 429/,
  );
});

test('createBrain validates required fields', () => {
  assert.throws(() => createBrain({ provider: PROVIDERS.openai, apiKey: 'sk' }), /model required/);
  assert.throws(() => createBrain({ model: 'gpt-4o-mini' }), /provider required/);
});

test('router throws for unknown role', () => {
  const router = new BrainRouter();
  assert.throws(() => router.get('ghost'), /no brain registered/);
});
