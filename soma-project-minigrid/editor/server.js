const express = require('express');
const { execFileSync } = require('child_process');
const fs = require('fs');
const path = require('path');

const app = express();
app.use(express.json({ limit: '10mb' }));
app.use(express.static(path.join(__dirname, 'public')));

const PROJECT_ROOT = path.resolve(__dirname, '..');
const WORLDS_DIR = path.join(PROJECT_ROOT, 'worlds');
const OUTPUT_DIR = path.join(PROJECT_ROOT, 'output');
const RUN_BIN = path.join(PROJECT_ROOT, 'target', 'release', 'run-minigrid');

fs.mkdirSync(OUTPUT_DIR, { recursive: true });

app.get('/api/worlds', (req, res) => {
  const files = fs.readdirSync(WORLDS_DIR).filter(f => f.endsWith('.json'));
  const worlds = files.map(f => {
    const data = JSON.parse(fs.readFileSync(path.join(WORLDS_DIR, f), 'utf-8'));
    return { file: f, ...data };
  });
  res.json(worlds);
});

app.get('/api/worlds/:name', (req, res) => {
  const file = path.join(WORLDS_DIR, req.params.name);
  if (!fs.existsSync(file)) return res.status(404).json({ error: 'not found' });
  res.json(JSON.parse(fs.readFileSync(file, 'utf-8')));
});

app.post('/api/worlds', (req, res) => {
  const world = req.body;
  if (!world.name) return res.status(400).json({ error: 'name required' });
  const filename = world.name.replace(/[^a-zA-Z0-9_-]/g, '_') + '.json';
  fs.writeFileSync(path.join(WORLDS_DIR, filename), JSON.stringify(world, null, 2));
  res.json({ saved: filename });
});

app.delete('/api/worlds/:name', (req, res) => {
  const file = path.join(WORLDS_DIR, req.params.name);
  if (fs.existsSync(file)) fs.unlinkSync(file);
  res.json({ deleted: req.params.name });
});

app.post('/api/run', (req, res) => {
  const world = req.body;
  if (!world.name) return res.status(400).json({ error: 'name required' });

  const tmpFile = path.join(WORLDS_DIR, `_tmp_${Date.now()}.json`);
  fs.writeFileSync(tmpFile, JSON.stringify(world, null, 2));

  try {
    const result = execFileSync(
      RUN_BIN,
      [tmpFile, '--output', OUTPUT_DIR],
      { cwd: PROJECT_ROOT, timeout: 120000, encoding: 'utf-8' }
    );

    const gifFile = `${world.name}.gif`;
    const traceFile = `${world.name}.trace.json`;
    const gifPath = path.join(OUTPUT_DIR, gifFile);
    const tracePath = path.join(OUTPUT_DIR, traceFile);

    const response = {
      stdout: result,
      solved: result.includes('SOLVED'),
      gif: fs.existsSync(gifPath) ? `/output/${gifFile}` : null,
      trace: fs.existsSync(tracePath) ? JSON.parse(fs.readFileSync(tracePath, 'utf-8')) : null,
    };

    res.json(response);
  } catch (err) {
    const stdout = (err.stdout || '') + (err.stderr || '');
    const gifFile = `${world.name}.gif`;
    const traceFile = `${world.name}.trace.json`;
    const gifPath = path.join(OUTPUT_DIR, gifFile);
    const tracePath = path.join(OUTPUT_DIR, traceFile);

    res.json({
      stdout,
      solved: false,
      gif: fs.existsSync(gifPath) ? `/output/${gifFile}` : null,
      trace: fs.existsSync(tracePath) ? JSON.parse(fs.readFileSync(tracePath, 'utf-8')) : null,
      error: stdout.includes('FAILED') ? 'World not solved' : err.message,
    });
  } finally {
    if (fs.existsSync(tmpFile)) fs.unlinkSync(tmpFile);
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

app.use('/output', express.static(OUTPUT_DIR));

const PORT = process.env.PORT || 3333;
app.listen(PORT, () => {
  console.log(`minigrid editor: http://localhost:${PORT}`);
  console.log(`  worlds: ${WORLDS_DIR}`);
  console.log(`  output: ${OUTPUT_DIR}`);
  console.log(`  binary: ${RUN_BIN}`);
});
