#!/usr/bin/env node
/**
 * Test all 10 helperbook routines via WebSocket MCP create_goal.
 * Each test uses a fresh WebSocket connection.
 */

import WebSocket from 'ws';

const WS_URL = 'ws://127.0.0.1:9090';

const TESTS = [
  { name: 'list_providers',    objective: 'list_providers',    maxSteps: 5 },
  { name: 'login_otp',         objective: 'login_otp',         maxSteps: 5 },
  { name: 'verify_otp',        objective: 'verify_otp',        maxSteps: 5 },
  { name: 'get_user_profile',  objective: 'get_user_profile',  maxSteps: 5 },
  { name: 'list_appointments', objective: 'list_appointments', maxSteps: 5 },
  { name: 'list_contacts',     objective: 'list_contacts',     maxSteps: 5 },
  { name: 'send_message',      objective: 'send_message',      maxSteps: 5 },
  { name: 'book_appointment',  objective: 'book_appointment',  maxSteps: 8 },
  { name: 'submit_review',     objective: 'submit_review',     maxSteps: 8 },
  { name: 'cancel_appointment',objective: 'cancel_appointment',maxSteps: 8 },
];

function runOne(test) {
  return new Promise((resolve) => {
    const ws = new WebSocket(WS_URL);
    const timer = setTimeout(() => {
      try { ws.close(); } catch {}
      resolve({ name: test.name, status: 'TIMEOUT', detail: '' });
    }, 10000);

    ws.on('error', (err) => {
      clearTimeout(timer);
      resolve({ name: test.name, status: 'ERROR', detail: err.message });
    });

    ws.on('open', () => {
      ws.send(JSON.stringify({
        jsonrpc: '2.0',
        id: 1,
        method: 'create_goal',
        params: { objective: test.objective, max_steps: test.maxSteps },
      }));
    });

    ws.on('message', (data) => {
      clearTimeout(timer);
      try {
        const msg = JSON.parse(data.toString());
        if (msg.error) {
          ws.close();
          resolve({ name: test.name, status: 'MCP_ERROR', detail: msg.error.message || '' });
          return;
        }
        const result = msg.result || {};
        ws.close();
        resolve({
          name: test.name,
          status: result.status || 'unknown',
          detail: typeof result.result === 'string' ? result.result : JSON.stringify(result.result || ''),
        });
      } catch (e) {
        ws.close();
        resolve({ name: test.name, status: 'PARSE_ERROR', detail: e.message });
      }
    });
  });
}

async function main() {
  console.log('='.repeat(60));
  console.log('Helperbook Routine Test Suite');
  console.log('='.repeat(60));

  let passed = 0, failed = 0;

  for (const test of TESTS) {
    const r = await runOne(test);
    // Valid outcomes:
    //   completed — routine ran all steps successfully
    //   aborted — routine hit a condition→abandon (e.g. auth failed)
    //   waiting_for_input — policy requires confirmation (auth ops)
    const ok = ['completed', 'aborted', 'waiting_for_input'].includes(r.status);
    const sym = ok ? 'PASS' : 'FAIL';
    if (ok) passed++; else failed++;
    console.log(`  ${sym}: ${r.name.padEnd(25)} → ${r.status}`);
    if (!ok && r.detail) console.log(`        detail: ${r.detail.slice(0, 140)}`);
  }

  console.log();
  console.log(`Results: ${passed} passed, ${failed} failed out of ${TESTS.length}`);
  console.log('='.repeat(60));
  process.exit(failed > 0 ? 1 : 0);
}

main();
