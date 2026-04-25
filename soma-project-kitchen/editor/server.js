const express = require('express');
const { execFileSync } = require('child_process');
const path = require('path');
const fs = require('fs');

const app = express();
const PORT = 3334;
const PROJECT_ROOT = path.resolve(__dirname, '..');

app.use(express.json({ limit: '10mb' }));
app.use(express.static(path.join(__dirname, 'public')));

app.post('/api/run', (req, res) => {
  const scenario = req.body;
  const tmpFile = path.join(PROJECT_ROOT, 'scenarios', '_tmp_run.json');
  fs.writeFileSync(tmpFile, JSON.stringify(scenario));

  try {
    execFileSync(
      path.join(PROJECT_ROOT, 'target', 'release', 'run-kitchen'),
      ['scenarios/_tmp_run.json', '--output', 'output'],
      { cwd: PROJECT_ROOT, timeout: 120000, encoding: 'utf-8' }
    );

    const tracePath = path.join(PROJECT_ROOT, 'output', `${scenario.name}.trace.json`);
    if (fs.existsSync(tracePath)) {
      const trace = JSON.parse(fs.readFileSync(tracePath, 'utf-8'));
      res.json(trace);
    } else {
      res.status(500).json({ error: 'No trace file generated' });
    }
  } catch (err) {
    res.status(500).json({ error: err.message, stdout: err.stdout, stderr: err.stderr });
  } finally {
    try { fs.unlinkSync(tmpFile); } catch (_) {}
  }
});

app.post('/api/flush-memory', (req, res) => {
  const dataDir = path.join(PROJECT_ROOT, 'data');
  let deleted = 0;
  if (fs.existsSync(dataDir)) {
    const files = fs.readdirSync(dataDir);
    for (const f of files) {
      fs.unlinkSync(path.join(dataDir, f));
      deleted++;
    }
  }
  res.json({ deleted });
});

app.listen(PORT, () => {
  console.log(`Kitchen editor: http://localhost:${PORT}`);
});
