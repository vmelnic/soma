const express = require('express');
const { spawn } = require('child_process');
const path = require('path');

const PORT = process.env.PORT || 8080;
const app = express();

app.use(express.json());
app.use(express.static(__dirname));

// --- SOMA MCP Bridge ---

let soma = null;
let somaReady = false;
let mcpInitialized = false;
let pendingRequests = new Map();
let stdoutBuffer = '';

function startSoma() {
  const projectDir = path.join(__dirname, '..');
  const somaBinary = path.join(projectDir, 'bin', 'soma');

  const packs = ['postgres', 'redis', 'auth'];
  const packPaths = packs.map(p => path.join(projectDir, 'packs', p, 'manifest.json'));
  const pluginPath = packs.map(p => path.join(projectDir, 'packs', p)).join(':');

  const args = ['--mcp'];
  for (const p of packPaths) {
    args.push('--pack', p);
  }

  try {
    soma = spawn(somaBinary, args, {
      env: {
        ...process.env,
        SOMA_PORTS_PLUGIN_PATH: pluginPath,
        SOMA_PORTS_REQUIRE_SIGNATURES: 'false',
        SOMA_POSTGRES_URL: process.env.SOMA_POSTGRES_URL || 'host=localhost user=soma password=soma dbname=helperbook',
        SOMA_REDIS_URL: process.env.SOMA_REDIS_URL || 'redis://localhost:6379/0',
      },
      cwd: projectDir,
      stdio: ['pipe', 'pipe', 'pipe']
    });

    soma.stdout.on('data', (data) => {
      stdoutBuffer += data.toString();
      const lines = stdoutBuffer.split('\n');
      stdoutBuffer = lines.pop();

      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed) continue;
        try {
          const msg = JSON.parse(trimmed);
          if (msg.id !== undefined && pendingRequests.has(msg.id)) {
            const { resolve } = pendingRequests.get(msg.id);
            pendingRequests.delete(msg.id);
            resolve(msg);
          }
        } catch (e) {
          console.log('[SOMA stdout]', trimmed);
        }
      }
    });

    soma.stderr.on('data', (data) => {
      console.error('[SOMA stderr]', data.toString().trim());
    });

    soma.on('error', (err) => {
      console.error('[SOMA] Failed to start:', err.message);
      soma = null;
      somaReady = false;
    });

    soma.on('close', (code) => {
      console.log(`[SOMA] Process exited with code ${code}`);
      soma = null;
      somaReady = false;
      mcpInitialized = false;
      for (const [id, { reject }] of pendingRequests) {
        reject(new Error('SOMA process exited'));
      }
      pendingRequests.clear();
    });

    somaReady = true;
    console.log('[SOMA] Process spawned');
  } catch (err) {
    console.error('[SOMA] Could not spawn process:', err.message);
    console.log('[SOMA] Running in mock mode — frontend will use fallback data');
  }
}

function sendToSoma(message) {
  return new Promise((resolve, reject) => {
    if (!soma || !somaReady) {
      return reject(new Error('SOMA not running'));
    }

    const id = message.id;
    const timeout = setTimeout(() => {
      pendingRequests.delete(id);
      reject(new Error('SOMA request timeout (30s)'));
    }, 30000);

    pendingRequests.set(id, {
      resolve: (msg) => {
        clearTimeout(timeout);
        resolve(msg);
      },
      reject: (err) => {
        clearTimeout(timeout);
        reject(err);
      }
    });

    const line = JSON.stringify(message) + '\n';
    soma.stdin.write(line);
  });
}

async function ensureInitialized() {
  if (mcpInitialized) return;
  try {
    await sendToSoma({
      jsonrpc: '2.0',
      id: 0,
      method: 'initialize',
      params: {
        protocolVersion: '2024-11-05',
        capabilities: {},
        clientInfo: { name: 'soma-project-helperbook', version: '0.1.0' }
      }
    });
    soma.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n');
    mcpInitialized = true;
    console.log('[SOMA] MCP initialized');
  } catch (err) {
    console.error('[SOMA] MCP initialization failed:', err.message);
    throw err;
  }
}

// --- Routes ---

app.post('/api/mcp', async (req, res) => {
  try {
    await ensureInitialized();
    const result = await sendToSoma(req.body);
    res.json(result);
  } catch (err) {
    res.status(503).json({
      jsonrpc: '2.0',
      id: req.body.id || null,
      error: { code: -32000, message: err.message }
    });
  }
});

app.get('/api/status', (req, res) => {
  res.json({
    soma: somaReady && soma !== null,
    mcp: mcpInitialized
  });
});

// --- Start ---

startSoma();

app.listen(PORT, () => {
  console.log(`[HelperBook] Server running at http://localhost:${PORT}`);
  if (!somaReady) {
    console.log('[HelperBook] SOMA not available — frontend will use mock data');
  }
});
