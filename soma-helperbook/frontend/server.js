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
  const somaBinary = path.join(__dirname, '../../soma-core/target/release/soma');
  const somaConfig = path.join(__dirname, '../soma.toml');
  const modelsDir = path.join(__dirname, '../../models');

  try {
    soma = spawn(somaBinary, [
      '--config', somaConfig,
      '--model', modelsDir,
      '--mcp'
    ], {
      env: { ...process.env, SOMA_PG_PASSWORD: 'soma' },
      cwd: path.join(__dirname, '../../soma-core'),
      stdio: ['pipe', 'pipe', 'pipe']
    });

    soma.stdout.on('data', (data) => {
      stdoutBuffer += data.toString();
      const lines = stdoutBuffer.split('\n');
      stdoutBuffer = lines.pop(); // keep incomplete line in buffer

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
      // Reject all pending requests
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
        clientInfo: { name: 'soma-helperbook', version: '0.1.0' }
      }
    });
    // Send initialized notification
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
