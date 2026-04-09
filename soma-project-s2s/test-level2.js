#!/usr/bin/env node
// =============================================================================
// Level 2: SOMA-to-SOMA capability delegation via MCP.
//
// Starts two SOMA instances:
//   Peer A: filesystem port, listens on TCP 9100
//   Peer B: postgres port, connects to Peer A as --peer, MCP on stdin/stdout
//
// Tests:
//   1. list_peers on B shows peer-0 (Peer A)
//   2. invoke_remote_skill from B → A (filesystem readdir)
//   3. invoke_port on B locally (postgres query)
//   4. Cross-peer: B delegates filesystem work to A via invoke_remote_skill
//
// Usage: node test-level2.js
// =============================================================================
'use strict';

const { spawn } = require('child_process');
const path = require('path');
const readline = require('readline');

const PROJECT_ROOT = path.resolve(__dirname);
const SOMA_BIN = path.join(PROJECT_ROOT, 'bin', 'soma');
const FS_MANIFEST = path.join(PROJECT_ROOT, 'packs', 'filesystem', 'manifest.json');
const PG_MANIFEST = path.join(PROJECT_ROOT, 'packs', 'postgres', 'manifest.json');

const PEER_A_ADDR = '127.0.0.1:9100';

// ---------------------------------------------------------------------------
// MCP client: talks to a SOMA --mcp process over stdin/stdout JSON-RPC
// ---------------------------------------------------------------------------

class McpClient {
  constructor(proc, name) {
    this.proc = proc;
    this.name = name;
    this.nextId = 1;
    this.pending = new Map();
    this.rl = readline.createInterface({ input: proc.stdout });
    this.rl.on('line', (line) => {
      const trimmed = line.trim();
      if (!trimmed) return;
      try {
        const resp = JSON.parse(trimmed);
        if (resp.id !== undefined && this.pending.has(resp.id)) {
          this.pending.get(resp.id)(resp);
          this.pending.delete(resp.id);
        }
      } catch {
        // Not JSON — ignore stderr leaking into stdout etc.
      }
    });
    proc.stderr.on('data', (data) => {
      process.stderr.write(`  [${name}] ${data}`);
    });
  }

  send(method, params, timeoutMs = 15000) {
    return new Promise((resolve, reject) => {
      const id = this.nextId++;
      const req = { jsonrpc: '2.0', id, method, params };
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`Timeout (${timeoutMs}ms) on ${method}`));
      }, timeoutMs);
      this.pending.set(id, (resp) => {
        clearTimeout(timer);
        resolve(resp);
      });
      this.proc.stdin.write(JSON.stringify(req) + '\n');
    });
  }

  async callTool(name, args, timeoutMs = 15000) {
    const resp = await this.send('tools/call', { name, arguments: args }, timeoutMs);
    if (resp.error) return { error: resp.error };
    // Unwrap MCP content wrapper.
    try {
      const text = resp.result.content[0].text;
      return { result: JSON.parse(text) };
    } catch {
      return { result: resp.result };
    }
  }

  close() {
    this.proc.stdin.end();
    this.proc.kill('SIGTERM');
  }
}

// ---------------------------------------------------------------------------
// Start helpers
// ---------------------------------------------------------------------------

function startPeerA() {
  // Peer A: filesystem, TCP listener, CLI repl mode (not MCP).
  return new Promise((resolve, reject) => {
    const proc = spawn(SOMA_BIN, [
      '--listen', PEER_A_ADDR,
      '--pack', FS_MANIFEST,
      'repl',
    ], {
      cwd: PROJECT_ROOT,
      stdio: ['pipe', 'pipe', 'pipe'],
      env: { ...process.env, SOMA_PORTS_PLUGIN_PATH: '' },
    });

    let ready = false;
    const timer = setTimeout(() => {
      if (!ready) { proc.kill(); reject(new Error('Peer A did not start')); }
    }, 10000);

    proc.stderr.on('data', (data) => {
      const line = data.toString();
      process.stderr.write(`  [peer-a] ${line}`);
      if (line.includes('TCP transport listening')) {
        ready = true;
        clearTimeout(timer);
        setTimeout(() => resolve(proc), 500);
      }
    });
    proc.on('error', reject);
    proc.on('exit', (code) => {
      if (!ready) { clearTimeout(timer); reject(new Error(`Peer A exited: ${code}`)); }
    });
  });
}

function startPeerB() {
  // Peer B: postgres + peer connection to A, MCP mode.
  const proc = spawn(SOMA_BIN, [
    '--mcp',
    '--pack', PG_MANIFEST,
    '--peer', PEER_A_ADDR,
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
    if (passed) {
      results.passed++;
      console.log(`  PASS  ${name}`);
    } else {
      results.failed++;
      console.log(`  FAIL  ${name}: ${detail}`);
    }
  }

  console.log('\n=== Level 2: SOMA-to-SOMA Delegation via MCP ===\n');

  // Start Peer A (filesystem, TCP listener)
  console.log('Starting Peer A (filesystem, TCP listener)...');
  let peerAProc;
  try {
    peerAProc = await startPeerA();
    test('Peer A starts and listens', true);
  } catch (err) {
    test('Peer A starts and listens', false, err.message);
    return results;
  }

  // Start Peer B (postgres, MCP, connects to A)
  console.log('Starting Peer B (postgres, MCP, --peer A)...');
  let peerB;
  try {
    peerB = startPeerB();
    // Initialize MCP
    const init = await peerB.send('initialize', {});
    test('Peer B MCP initializes', !!init.result, JSON.stringify(init.error || ''));
  } catch (err) {
    test('Peer B MCP initializes', false, err.message);
    peerAProc.kill();
    return results;
  }

  try {
    // Test 1: list_peers shows peer-0
    console.log('\n--- Test: list_peers ---');
    const peersResp = await peerB.callTool('list_peers', {});
    if (peersResp.result) {
      const peers = peersResp.result.peers || [];
      test('list_peers returns peers', peers.length > 0, `count: ${peers.length}`);
      const peer0 = peers.find(p => p.peer_id === 'peer-0');
      test('peer-0 is registered', !!peer0, `peers: ${JSON.stringify(peers)}`);
      if (peer0) {
        test('peer-0 has executor', peer0.has_executor === true);
      }
    } else {
      test('list_peers returns peers', false, JSON.stringify(peersResp.error));
    }

    // Test 2: invoke_remote_skill from B → A (readdir /tmp)
    console.log('\n--- Test: invoke_remote_skill (B → A: readdir /tmp) ---');
    const remoteResp = await peerB.callTool('invoke_remote_skill', {
      peer_id: 'peer-0',
      skill_id: 'readdir',
      input: { path: '/tmp' },
    }, 30000);
    if (remoteResp.result) {
      const r = remoteResp.result;
      test('Remote skill returns result', true);
      test('Response has skill_id', r.skill_id === 'readdir', `skill_id: ${r.skill_id}`);
      test('Response has peer_id', typeof r.peer_id === 'string', `peer_id: ${r.peer_id}`);
      test('Response has trace_id', typeof r.trace_id === 'string');
      console.log(`    observation: ${JSON.stringify(r.observation).slice(0, 200)}`);
    } else {
      test('Remote skill returns result', false, JSON.stringify(remoteResp.error));
    }

    // Test 3: invoke_port locally on B (postgres list_ports)
    console.log('\n--- Test: list_ports on Peer B ---');
    const portsResp = await peerB.callTool('list_ports', {});
    if (portsResp.result) {
      const ports = portsResp.result.ports || [];
      test('Peer B has loaded ports', ports.length > 0, `count: ${ports.length}`);
      const pg = ports.find(p => p.port_id === 'postgres');
      test('Postgres port loaded', !!pg);
    } else {
      test('Peer B has loaded ports', false, JSON.stringify(portsResp.error));
    }

    // Test 4: invoke_port on B (postgres query) — requires running helperbook DB
    console.log('\n--- Test: invoke_port on Peer B (postgres query) ---');
    const queryResp = await peerB.callTool('invoke_port', {
      port_id: 'postgres',
      capability_id: 'query',
      input: { sql: 'SELECT count(*) AS cnt FROM users' },
    }, 10000);
    if (queryResp.result && queryResp.result.success) {
      test('Postgres query succeeds', true);
      console.log(`    result: ${JSON.stringify(queryResp.result.structured_result).slice(0, 200)}`);
    } else if (queryResp.error) {
      test('Postgres query succeeds', false, `DB may not be running: ${queryResp.error.message}`);
    } else {
      test('Postgres query succeeds', false, `result: ${JSON.stringify(queryResp.result).slice(0, 200)}`);
    }

    // Test 5: tools/list shows new tools
    console.log('\n--- Test: tools/list includes distributed tools ---');
    const toolsResp = await peerB.send('tools/list', {});
    if (toolsResp.result && toolsResp.result.tools) {
      const tools = toolsResp.result.tools;
      const names = tools.map(t => t.name);
      test('list_peers tool exists', names.includes('list_peers'));
      test('invoke_remote_skill tool exists', names.includes('invoke_remote_skill'));
      test('transfer_routine tool exists', names.includes('transfer_routine'));
      test(`Total tools: ${tools.length}`, tools.length === 19, `count: ${tools.length}`);
    } else {
      test('tools/list returns tools', false, JSON.stringify(toolsResp));
    }

    // Test 6: invoke_remote_skill with invalid peer
    console.log('\n--- Test: Error handling ---');
    const badPeerResp = await peerB.callTool('invoke_remote_skill', {
      peer_id: 'nonexistent-peer',
      skill_id: 'readdir',
      input: {},
    });
    test('Invalid peer returns error', !!badPeerResp.error,
      badPeerResp.error ? '' : 'expected error, got result');

    // Test 7: dump_state shows peer info
    console.log('\n--- Test: dump_state ---');
    const stateResp = await peerB.callTool('dump_state', { sections: ['skills'] });
    if (stateResp.result) {
      test('dump_state works', true);
    } else {
      test('dump_state works', false, JSON.stringify(stateResp.error));
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
  .catch(err => {
    console.error('Fatal error:', err);
    process.exit(2);
  });
