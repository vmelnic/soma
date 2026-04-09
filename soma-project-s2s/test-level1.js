#!/usr/bin/env node
// =============================================================================
// Level 1: Prove SOMA-to-SOMA transport works.
//
// Starts a SOMA instance with --listen, then sends raw wire-protocol messages
// over TCP: InvokeSkill (filesystem readdir) and Ping/Pong heartbeat.
//
// Usage: node test-level1.js
// =============================================================================
'use strict';

const { spawn } = require('child_process');
const net = require('net');
const path = require('path');

const PROJECT_ROOT = path.resolve(__dirname);
const SOMA_BIN = path.join(PROJECT_ROOT, 'bin', 'soma');
const FS_MANIFEST = path.join(PROJECT_ROOT, 'packs', 'filesystem', 'manifest.json');

const LISTEN_ADDR = '127.0.0.1';
const LISTEN_PORT = 9100;

// ---------------------------------------------------------------------------
// Wire protocol helpers: 4-byte big-endian length prefix + JSON payload
// ---------------------------------------------------------------------------

function encodeFrame(obj) {
  const json = JSON.stringify(obj);
  const payload = Buffer.from(json, 'utf8');
  const header = Buffer.alloc(4);
  header.writeUInt32BE(payload.length, 0);
  return Buffer.concat([header, payload]);
}

function decodeFrames(buf) {
  const frames = [];
  let offset = 0;
  while (offset + 4 <= buf.length) {
    const len = buf.readUInt32BE(offset);
    if (offset + 4 + len > buf.length) break;
    const json = buf.slice(offset + 4, offset + 4 + len).toString('utf8');
    frames.push(JSON.parse(json));
    offset += 4 + len;
  }
  return { frames, remaining: buf.slice(offset) };
}

// ---------------------------------------------------------------------------
// Send a message and wait for one response frame
// ---------------------------------------------------------------------------

function sendAndReceive(host, port, message, timeoutMs = 15000) {
  return new Promise((resolve, reject) => {
    const client = new net.Socket();
    let buf = Buffer.alloc(0);
    const timer = setTimeout(() => {
      client.destroy();
      reject(new Error(`Timeout waiting for response after ${timeoutMs}ms`));
    }, timeoutMs);

    client.connect(port, host, () => {
      client.write(encodeFrame(message));
    });

    client.on('data', (data) => {
      buf = Buffer.concat([buf, data]);
      const { frames } = decodeFrames(buf);
      if (frames.length > 0) {
        clearTimeout(timer);
        client.destroy();
        resolve(frames[0]);
      }
    });

    client.on('error', (err) => {
      clearTimeout(timer);
      reject(err);
    });
  });
}

// ---------------------------------------------------------------------------
// Start SOMA with --listen and wait for it to be ready
// ---------------------------------------------------------------------------

function startSoma(listenAddr, packManifest) {
  return new Promise((resolve, reject) => {
    const proc = spawn(SOMA_BIN, [
      '--listen', listenAddr,
      '--pack', packManifest,
      'repl',
    ], {
      cwd: PROJECT_ROOT,
      stdio: ['pipe', 'pipe', 'pipe'],
      env: { ...process.env, SOMA_PORTS_PLUGIN_PATH: '' },
    });

    let ready = false;
    const timer = setTimeout(() => {
      if (!ready) {
        proc.kill();
        reject(new Error('SOMA did not start within 10s'));
      }
    }, 10000);

    proc.stderr.on('data', (data) => {
      const line = data.toString();
      process.stderr.write(`  [peer-a] ${line}`);
      if (line.includes('TCP transport listening on') || line.includes('listening')) {
        ready = true;
        clearTimeout(timer);
        // Give it a moment to actually bind
        setTimeout(() => resolve(proc), 500);
      }
    });

    proc.stdout.on('data', (data) => {
      // REPL output — ignore
    });

    proc.on('error', (err) => {
      clearTimeout(timer);
      reject(err);
    });

    proc.on('exit', (code) => {
      if (!ready) {
        clearTimeout(timer);
        reject(new Error(`SOMA exited with code ${code} before becoming ready`));
      }
    });
  });
}

// ---------------------------------------------------------------------------
// Test runner
// ---------------------------------------------------------------------------

async function runTests() {
  const results = { passed: 0, failed: 0, tests: [] };

  function test(name, passed, detail) {
    results.tests.push({ name, passed, detail });
    if (passed) {
      results.passed++;
      console.log(`  PASS  ${name}`);
    } else {
      results.failed++;
      console.log(`  FAIL  ${name}: ${detail}`);
    }
  }

  console.log('\n=== Level 1: SOMA-to-SOMA Transport ===\n');

  // Start Peer A: filesystem worker with TCP listener
  console.log(`Starting Peer A (filesystem) on ${LISTEN_ADDR}:${LISTEN_PORT}...`);
  let peerA;
  try {
    peerA = await startSoma(`${LISTEN_ADDR}:${LISTEN_PORT}`, FS_MANIFEST);
    test('Peer A starts and listens on TCP', true);
  } catch (err) {
    test('Peer A starts and listens on TCP', false, err.message);
    return results;
  }

  try {
    // Test 1: Ping/Pong heartbeat
    console.log('\n--- Test: Ping/Pong heartbeat ---');
    try {
      const nonce = Date.now();
      const resp = await sendAndReceive(LISTEN_ADDR, LISTEN_PORT, {
        type: 'ping',
        nonce,
      });
      const isPong = resp.type === 'pong';
      const nonceMatch = resp.nonce === nonce;
      test('Ping returns Pong', isPong, `got type: ${resp.type}`);
      test('Pong echoes nonce', nonceMatch, `sent ${nonce}, got ${resp.nonce}`);
      test('Pong includes load', typeof resp.load === 'number', `load: ${resp.load}`);
    } catch (err) {
      test('Ping/Pong heartbeat', false, err.message);
    }

    // Test 2: InvokeSkill — readdir on /tmp
    console.log('\n--- Test: InvokeSkill (readdir /tmp) ---');
    try {
      const resp = await sendAndReceive(LISTEN_ADDR, LISTEN_PORT, {
        type: 'invoke_skill',
        peer_id: 'test-client',
        skill_id: 'readdir',
        input: { path: '/tmp' },
      }, 30000);

      const isSkillResult = resp.type === 'skill_result';
      test('InvokeSkill returns skill_result', isSkillResult, `got type: ${resp.type}`);

      if (isSkillResult && resp.response) {
        const r = resp.response;
        test('Response has skill_id', r.skill_id === 'readdir', `skill_id: ${r.skill_id}`);
        test('Response has peer_id', typeof r.peer_id === 'string', `peer_id: ${r.peer_id}`);
        test('Response has trace_id', typeof r.trace_id === 'string', `trace_id: ${r.trace_id}`);
        test('Response has timestamp', typeof r.timestamp === 'string', `timestamp: ${r.timestamp}`);
        // Success depends on whether the reference pack's readdir skill actually executes.
        // Even if the skill fails (no port for filesystem), the transport layer works.
        test('Transport roundtrip complete', true);
        console.log(`    observation: ${JSON.stringify(r.observation).slice(0, 200)}`);
      }
    } catch (err) {
      test('InvokeSkill (readdir /tmp)', false, err.message);
    }

    // Test 3: SubmitGoal — remote goal submission
    console.log('\n--- Test: SubmitGoal ---');
    try {
      const resp = await sendAndReceive(LISTEN_ADDR, LISTEN_PORT, {
        type: 'submit_goal',
        peer_id: 'test-client',
        request: {
          goal: 'list files in /tmp',
          constraints: [],
          budgets: {
            risk_limit: 1.0,
            latency_limit_ms: 30000,
            resource_limit: 100.0,
            step_limit: 10,
          },
          trust_required: 'untrusted',
          request_result: true,
          request_trace: false,
        },
      }, 15000);

      const isGoalResult = resp.type === 'goal_result';
      test('SubmitGoal returns goal_result', isGoalResult, `got type: ${resp.type}`);

      if (isGoalResult && resp.response) {
        const r = resp.response;
        test('Goal accepted or processed', ['accepted', 'rejected'].includes(r.status),
          `status: ${r.status}`);
        if (r.session_id) {
          test('Goal returns session_id', true, `session_id: ${r.session_id}`);
        }
        if (r.reason) {
          console.log(`    reason: ${r.reason}`);
        }
      } else if (resp.type === 'error') {
        test('SubmitGoal returns goal_result', false, `error: ${resp.details}`);
      }
    } catch (err) {
      test('SubmitGoal', false, err.message);
    }

    // Test 4: Multiple sequential messages on separate connections
    console.log('\n--- Test: Multiple connections ---');
    try {
      const [r1, r2, r3] = await Promise.all([
        sendAndReceive(LISTEN_ADDR, LISTEN_PORT, { type: 'ping', nonce: 1 }),
        sendAndReceive(LISTEN_ADDR, LISTEN_PORT, { type: 'ping', nonce: 2 }),
        sendAndReceive(LISTEN_ADDR, LISTEN_PORT, { type: 'ping', nonce: 3 }),
      ]);
      const allPongs = r1.type === 'pong' && r2.type === 'pong' && r3.type === 'pong';
      test('Concurrent connections handled', allPongs);
      const noncesCorrect = r1.nonce === 1 && r2.nonce === 2 && r3.nonce === 3;
      test('Each connection gets correct nonce', noncesCorrect);
    } catch (err) {
      test('Multiple connections', false, err.message);
    }

    // Test 5: Error handling — unknown skill
    console.log('\n--- Test: Error handling ---');
    try {
      const resp = await sendAndReceive(LISTEN_ADDR, LISTEN_PORT, {
        type: 'invoke_skill',
        peer_id: 'test-client',
        skill_id: 'nonexistent_skill_xyz',
        input: {},
      }, 30000);
      // Should get a result (possibly error or failed observation), not a crash
      test('Unknown skill does not crash listener',
        resp.type === 'skill_result' || resp.type === 'error', `type: ${resp.type}`);
    } catch (err) {
      test('Unknown skill handling', false, err.message);
    }

  } finally {
    // Cleanup
    console.log('\nStopping Peer A...');
    peerA.kill('SIGTERM');
    await new Promise(r => setTimeout(r, 1000));
    if (!peerA.killed) peerA.kill('SIGKILL');
  }

  // Summary
  console.log('\n=== Results ===');
  console.log(`  ${results.passed} passed, ${results.failed} failed, ${results.tests.length} total\n`);

  return results;
}

runTests()
  .then(r => process.exit(r.failed > 0 ? 1 : 0))
  .catch(err => {
    console.error('Fatal error:', err);
    process.exit(2);
  });
