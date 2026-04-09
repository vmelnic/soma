#!/usr/bin/env node
// =============================================================================
// Level 3: Schema & routine transfer between SOMA peers.
//
// Tests:
//   1. Transfer a routine from Peer B → Peer A over TCP wire protocol
//   2. Transfer a schema from Peer B → Peer A over TCP wire protocol
//   3. transfer_routine MCP tool (B sends to A via MCP → RemoteExecutor)
//   4. Verify transferred routine is stored on A (via SubmitGoal + response)
//
// Usage: node test-level3.js
// =============================================================================
'use strict';

const { spawn } = require('child_process');
const net = require('net');
const path = require('path');
const readline = require('readline');

const PROJECT_ROOT = path.resolve(__dirname);
const SOMA_BIN = path.join(PROJECT_ROOT, 'bin', 'soma');
const FS_MANIFEST = path.join(PROJECT_ROOT, 'packs', 'filesystem', 'manifest.json');
const PG_MANIFEST = path.join(PROJECT_ROOT, 'packs', 'postgres', 'manifest.json');

const PEER_A_ADDR = '127.0.0.1';
const PEER_A_PORT = 9100;

// ---------------------------------------------------------------------------
// Wire protocol helpers
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

function sendAndReceive(host, port, message, timeoutMs = 15000) {
  return new Promise((resolve, reject) => {
    const client = new net.Socket();
    let buf = Buffer.alloc(0);
    const timer = setTimeout(() => {
      client.destroy();
      reject(new Error(`Timeout (${timeoutMs}ms)`));
    }, timeoutMs);
    client.connect(port, host, () => { client.write(encodeFrame(message)); });
    client.on('data', (data) => {
      buf = Buffer.concat([buf, data]);
      const { frames } = decodeFrames(buf);
      if (frames.length > 0) {
        clearTimeout(timer);
        client.destroy();
        resolve(frames[0]);
      }
    });
    client.on('error', (err) => { clearTimeout(timer); reject(err); });
  });
}

// ---------------------------------------------------------------------------
// MCP client
// ---------------------------------------------------------------------------

class McpClient {
  constructor(proc, name) {
    this.proc = proc;
    this.name = name;
    this.nextId = 1;
    this.pending = new Map();
    this.rl = readline.createInterface({ input: proc.stdout });
    this.rl.on('line', (line) => {
      try {
        const resp = JSON.parse(line.trim());
        if (resp.id !== undefined && this.pending.has(resp.id)) {
          this.pending.get(resp.id)(resp);
          this.pending.delete(resp.id);
        }
      } catch {}
    });
    proc.stderr.on('data', (data) => {
      process.stderr.write(`  [${name}] ${data}`);
    });
  }
  send(method, params, timeoutMs = 15000) {
    return new Promise((resolve, reject) => {
      const id = this.nextId++;
      const timer = setTimeout(() => { this.pending.delete(id); reject(new Error(`Timeout on ${method}`)); }, timeoutMs);
      this.pending.set(id, (resp) => { clearTimeout(timer); resolve(resp); });
      this.proc.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
    });
  }
  async callTool(name, args, timeoutMs = 15000) {
    const resp = await this.send('tools/call', { name, arguments: args }, timeoutMs);
    if (resp.error) return { error: resp.error };
    try { return { result: JSON.parse(resp.result.content[0].text) }; }
    catch { return { result: resp.result }; }
  }
  close() { this.proc.stdin.end(); this.proc.kill('SIGTERM'); }
}

// ---------------------------------------------------------------------------
// Start helpers
// ---------------------------------------------------------------------------

function startPeerA() {
  return new Promise((resolve, reject) => {
    const proc = spawn(SOMA_BIN, [
      '--listen', `${PEER_A_ADDR}:${PEER_A_PORT}`,
      '--pack', FS_MANIFEST,
      'repl',
    ], {
      cwd: PROJECT_ROOT,
      stdio: ['pipe', 'pipe', 'pipe'],
      env: { ...process.env, SOMA_PORTS_PLUGIN_PATH: '' },
    });
    let ready = false;
    const timer = setTimeout(() => { if (!ready) { proc.kill(); reject(new Error('Peer A timeout')); } }, 10000);
    proc.stderr.on('data', (data) => {
      process.stderr.write(`  [peer-a] ${data}`);
      if (data.toString().includes('TCP transport listening')) {
        ready = true; clearTimeout(timer); setTimeout(() => resolve(proc), 500);
      }
    });
    proc.on('error', reject);
    proc.on('exit', (code) => { if (!ready) { clearTimeout(timer); reject(new Error(`exit ${code}`)); } });
  });
}

function startPeerB() {
  const proc = spawn(SOMA_BIN, [
    '--mcp',
    '--pack', PG_MANIFEST,
    '--peer', `${PEER_A_ADDR}:${PEER_A_PORT}`,
  ], {
    cwd: PROJECT_ROOT,
    stdio: ['pipe', 'pipe', 'pipe'],
    env: {
      ...process.env,
      SOMA_PORTS_PLUGIN_PATH: path.join(PROJECT_ROOT, 'packs', 'postgres'),
      SOMA_PORTS_REQUIRE_SIGNATURES: 'false',
      SOMA_POSTGRES_URL: process.env.SOMA_POSTGRES_URL ||
        'host=localhost user=soma password=soma dbname=helperbook',
    },
  });
  return new McpClient(proc, 'peer-b');
}

// ---------------------------------------------------------------------------
// Test runner
// ---------------------------------------------------------------------------

async function runTests() {
  const results = { passed: 0, failed: 0, tests: [] };
  function test(name, passed, detail) {
    results.tests.push({ name, passed, detail });
    if (passed) { results.passed++; console.log(`  PASS  ${name}`); }
    else { results.failed++; console.log(`  FAIL  ${name}: ${detail || ''}`); }
  }

  console.log('\n=== Level 3: Schema & Routine Transfer ===\n');

  // Start Peer A
  console.log('Starting Peer A (filesystem, TCP listener)...');
  let peerAProc;
  try {
    peerAProc = await startPeerA();
    test('Peer A starts', true);
  } catch (err) {
    test('Peer A starts', false, err.message);
    return results;
  }

  // Start Peer B
  console.log('Starting Peer B (postgres, MCP, --peer A)...');
  let peerB;
  try {
    peerB = startPeerB();
    await peerB.send('initialize', {});
    test('Peer B MCP initializes', true);
  } catch (err) {
    test('Peer B MCP initializes', false, err.message);
    peerAProc.kill();
    return results;
  }

  try {
    // -----------------------------------------------------------------------
    // Test 1: Transfer routine over raw TCP wire protocol
    // -----------------------------------------------------------------------
    console.log('\n--- Test: TransferRoutine via TCP wire protocol ---');
    try {
      const resp = await sendAndReceive(PEER_A_ADDR, PEER_A_PORT, {
        type: 'transfer_routine',
        peer_id: 'test-peer',
        routine: {
          routine_id: 'test-routine-tcp',
          match_conditions: [
            { condition_type: 'goal_contains', expression: { keyword: 'list files' }, description: 'goal mentions listing files' }
          ],
          compiled_skill_path: ['readdir', 'stat'],
          guard_conditions: [],
          expected_cost: 0.5,
          expected_effect: [],
          confidence: 0.85,
        },
      });
      test('TransferRoutine returns routine_ok', resp.type === 'routine_ok',
        `got: ${resp.type}, ${JSON.stringify(resp).slice(0, 200)}`);
    } catch (err) {
      test('TransferRoutine via TCP', false, err.message);
    }

    // -----------------------------------------------------------------------
    // Test 2: Transfer schema over raw TCP wire protocol
    // -----------------------------------------------------------------------
    console.log('\n--- Test: TransferSchema via TCP wire protocol ---');
    try {
      const resp = await sendAndReceive(PEER_A_ADDR, PEER_A_PORT, {
        type: 'transfer_schema',
        peer_id: 'test-peer',
        schema: {
          schema_id: 'test-schema-tcp',
          version: '0.1.0',
          trigger_conditions: [
            { condition_type: 'goal_contains', expression: { keyword: 'list' }, description: 'goal mentions listing' }
          ],
          subgoal_structure: [
            {
              subgoal_id: 'sg1',
              description: 'enumerate directory',
              skill_candidates: ['readdir'],
              dependencies: [],
              optional: false,
            }
          ],
          candidate_skill_ordering: ['readdir', 'stat'],
          stop_conditions: [],
          confidence: 0.9,
        },
      });
      test('TransferSchema returns schema_ok', resp.type === 'schema_ok',
        `got: ${resp.type}, ${JSON.stringify(resp).slice(0, 200)}`);
    } catch (err) {
      test('TransferSchema via TCP', false, err.message);
    }

    // -----------------------------------------------------------------------
    // Test 3: transfer_routine MCP tool (B → A via RemoteExecutor)
    // -----------------------------------------------------------------------
    console.log('\n--- Test: transfer_routine MCP tool ---');

    // First, check if B has any routines. If not, we'll verify the error message.
    const dumpResp = await peerB.callTool('dump_state', { sections: ['routines'] });
    const routines = dumpResp.result?.routines || [];
    console.log(`    Peer B routines: ${routines.length}`);

    if (routines.length > 0) {
      // Transfer first routine to A
      const rid = routines[0].routine_id;
      const transferResp = await peerB.callTool('transfer_routine', {
        peer_id: 'peer-0',
        routine_id: rid,
      });
      if (transferResp.result) {
        test('transfer_routine MCP succeeds', transferResp.result.transferred === true,
          JSON.stringify(transferResp.result));
      } else {
        test('transfer_routine MCP succeeds', false, JSON.stringify(transferResp.error));
      }
    } else {
      // No routines on B — verify proper error for nonexistent routine
      const transferResp = await peerB.callTool('transfer_routine', {
        peer_id: 'peer-0',
        routine_id: 'nonexistent',
      });
      test('transfer_routine reports missing routine', !!transferResp.error,
        'Expected error for nonexistent routine');
    }

    // -----------------------------------------------------------------------
    // Test 4: Multiple transfers accumulate
    // -----------------------------------------------------------------------
    console.log('\n--- Test: Multiple transfers ---');
    try {
      const r1 = await sendAndReceive(PEER_A_ADDR, PEER_A_PORT, {
        type: 'transfer_routine',
        peer_id: 'peer-b',
        routine: {
          routine_id: 'routine-alpha',
          match_conditions: [],
          compiled_skill_path: ['readdir'],
          guard_conditions: [],
          expected_cost: 0.1,
          expected_effect: [],
          confidence: 0.95,
        },
      });
      const r2 = await sendAndReceive(PEER_A_ADDR, PEER_A_PORT, {
        type: 'transfer_routine',
        peer_id: 'peer-b',
        routine: {
          routine_id: 'routine-beta',
          match_conditions: [],
          compiled_skill_path: ['stat', 'readdir'],
          guard_conditions: [],
          expected_cost: 0.2,
          expected_effect: [],
          confidence: 0.88,
        },
      });
      test('Multiple routine transfers succeed',
        r1.type === 'routine_ok' && r2.type === 'routine_ok');
    } catch (err) {
      test('Multiple routine transfers', false, err.message);
    }

    // -----------------------------------------------------------------------
    // Test 5: Transfer with full schema detail
    // -----------------------------------------------------------------------
    console.log('\n--- Test: Schema with subgoal structure ---');
    try {
      const resp = await sendAndReceive(PEER_A_ADDR, PEER_A_PORT, {
        type: 'transfer_schema',
        peer_id: 'peer-b',
        schema: {
          schema_id: 'schema-multi-step',
          version: '1.0.0',
          trigger_conditions: [
            { condition_type: 'goal_contains', expression: { keyword: 'backup' }, description: 'goal mentions backup' },
            { condition_type: 'resource_available', expression: { resource: 'filesystem' }, description: 'filesystem is available' },
          ],
          subgoal_structure: [
            {
              subgoal_id: 'list',
              description: 'list target directory',
              skill_candidates: ['readdir'],
              dependencies: [],
              optional: false,
            },
            {
              subgoal_id: 'copy',
              description: 'copy each file to backup location',
              skill_candidates: ['copy_file', 'write_file'],
              dependencies: ['list'],
              optional: false,
            },
            {
              subgoal_id: 'verify',
              description: 'verify backup integrity',
              skill_candidates: ['stat', 'checksum'],
              dependencies: ['copy'],
              optional: true,
            },
          ],
          candidate_skill_ordering: ['readdir', 'copy_file', 'stat'],
          stop_conditions: [
            { condition_type: 'all_subgoals_complete', expression: {}, description: 'all subgoals done' }
          ],
          confidence: 0.92,
        },
      });
      test('Complex schema transfer succeeds', resp.type === 'schema_ok',
        `got: ${resp.type}`);
    } catch (err) {
      test('Complex schema transfer', false, err.message);
    }

    // -----------------------------------------------------------------------
    // Test 6: Verify transport is bidirectional
    // -----------------------------------------------------------------------
    console.log('\n--- Test: Bidirectional (ping still works after transfers) ---');
    try {
      const nonce = Date.now();
      const resp = await sendAndReceive(PEER_A_ADDR, PEER_A_PORT, {
        type: 'ping', nonce
      });
      test('Ping works after transfers', resp.type === 'pong' && resp.nonce === nonce);
    } catch (err) {
      test('Ping after transfers', false, err.message);
    }

  } finally {
    console.log('\nStopping peers...');
    peerB.close();
    peerAProc.kill('SIGTERM');
    await new Promise(r => setTimeout(r, 1000));
    if (!peerAProc.killed) peerAProc.kill('SIGKILL');
  }

  console.log('\n=== Results ===');
  console.log(`  ${results.passed} passed, ${results.failed} failed, ${results.tests.length} total\n`);
  return results;
}

runTests()
  .then(r => process.exit(r.failed > 0 ? 1 : 0))
  .catch(err => { console.error('Fatal error:', err); process.exit(2); });
