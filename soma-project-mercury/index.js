#!/usr/bin/env node

require('dotenv').config();
const blessed = require('blessed');
const { ConsensusEngine } = require('./lib/engine');

const BRAIN_COLORS = {
  mercury: 'cyan',
  kimi: 'yellow',
  glm: 'green',
};

const BRAIN_ICONS = {
  mercury: '◈',
  kimi: '◆',
  glm: '◉',
};

const screen = blessed.screen({
  smartCSR: true,
  title: 'SOMA Consensus — Three-Brain Composition',
  fullUnicode: true,
});

// ── Layout ──────────────────────────────────────────────

const chatBox = blessed.log({
  parent: screen,
  label: ' Conversation ',
  top: 0,
  left: 0,
  width: '70%',
  height: '70%',
  border: { type: 'line' },
  style: {
    border: { fg: 'blue' },
    label: { fg: 'white', bold: true },
  },
  scrollable: true,
  alwaysScroll: true,
  scrollbar: { ch: '│', style: { fg: 'blue' } },
  tags: true,
  mouse: true,
});

const traceBox = blessed.log({
  parent: screen,
  label: ' Brain Trace ',
  top: 0,
  left: '70%',
  width: '30%',
  height: '45%',
  border: { type: 'line' },
  style: {
    border: { fg: 'magenta' },
    label: { fg: 'white', bold: true },
  },
  scrollable: true,
  alwaysScroll: true,
  tags: true,
  mouse: true,
});

const routineBox = blessed.box({
  parent: screen,
  label: ' Routines ',
  top: '45%',
  left: '70%',
  width: '30%',
  height: '25%',
  border: { type: 'line' },
  style: {
    border: { fg: 'yellow' },
    label: { fg: 'white', bold: true },
  },
  tags: true,
  content: '{gray-fg}no routines yet{/gray-fg}',
});

const statusBox = blessed.box({
  parent: screen,
  label: ' Status ',
  top: '70%',
  left: 0,
  width: '70%',
  height: 3,
  border: { type: 'line' },
  style: {
    border: { fg: 'gray' },
    label: { fg: 'white' },
  },
  tags: true,
  content: ' {bold}SOMA Consensus{/bold} — type a question, press Enter',
});

const inputBox = blessed.textbox({
  parent: screen,
  label: ' Ask ',
  bottom: 0,
  left: 0,
  width: '100%',
  height: 3,
  border: { type: 'line' },
  style: {
    border: { fg: 'white' },
    label: { fg: 'cyan', bold: true },
    focus: { border: { fg: 'cyan' } },
  },
  inputOnFocus: true,
  mouse: true,
});

// ── Engine ──────────────────────────────────────────────

let busy = false;

function onTrace(event) {
  if (event.type === 'classify') {
    traceBox.log(`{bold}category:{/bold} {white-fg}${event.category}{/white-fg}`);
  } else if (event.type === 'routine') {
    const chain = event.chain.map(b => `{${BRAIN_COLORS[b]}-fg}${b}{/${BRAIN_COLORS[b]}-fg}`).join('→');
    traceBox.log(`{bold}routine:{/bold} ${chain}`);
  } else if (event.type === 'step') {
    const icon = BRAIN_ICONS[event.brain] || '●';
    const color = BRAIN_COLORS[event.brain] || 'white';
    if (event.status === 'running') {
      traceBox.log(`{${color}-fg}${icon} ${event.brain}{/${color}-fg} ${event.role}...`);
    } else if (event.status === 'done') {
      if (event.error) {
        traceBox.log(`  {red-fg}✗ ${event.error.slice(0, 60)}{/red-fg}`);
      } else {
        traceBox.log(`  {${color}-fg}✓{/${color}-fg} ${event.latency}ms | ${event.tokens}tok`);
      }
    }
  } else if (event.type === 'done') {
    traceBox.log(`{bold}total:{/bold} ${event.totalLatency}ms | ${event.totalTokens}tok`);
    traceBox.log('─'.repeat(28));
  }
  screen.render();
}

const engine = new ConsensusEngine(onTrace);
updateRoutineDisplay();

function updateRoutineDisplay() {
  const routines = engine.getRoutines();
  if (routines.length === 0) {
    routineBox.setContent(' {gray-fg}no routines yet\n (need 3+ episodes per category){/gray-fg}');
  } else {
    const lines = routines.map(r =>
      ` {bold}${r.category}{/bold}: ${r.chain} (${r.episodes} eps, ~${r.avgLatency}ms)`
    );
    routineBox.setContent(lines.join('\n'));
  }
  screen.render();
}

function setStatus(text) {
  statusBox.setContent(` ${text}`);
  screen.render();
}

async function handleQuery(query) {
  if (busy) return;
  busy = true;

  chatBox.log(`{bold}{white-fg}You:{/white-fg}{/bold} ${query}`);
  chatBox.log('');
  setStatus('{bold}thinking...{/bold} composition pipeline running');
  screen.render();

  try {
    const result = await engine.run(query);

    chatBox.log(`{bold}{cyan-fg}Answer:{/cyan-fg}{/bold} ${result.answer}`);
    chatBox.log(`{gray-fg}[${result.category}] ${result.totalLatency}ms | ${result.totalTokens}tok{/gray-fg}`);
    chatBox.log('');

    updateRoutineDisplay();
    setStatus('{bold}ready{/bold} — type a question, press Enter');
  } catch (err) {
    chatBox.log(`{red-fg}Error: ${err.message}{/red-fg}`);
    chatBox.log('');
    setStatus('{bold}ready{/bold} — type a question, press Enter');
  }

  busy = false;
  inputBox.focus();
  screen.render();
}

// ── Input handling ──────────────────────────────────────

inputBox.on('submit', (value) => {
  const query = (value || '').trim();
  inputBox.clearValue();
  screen.render();

  if (!query) {
    inputBox.focus();
    return;
  }

  if (query === '/quit' || query === '/exit') {
    process.exit(0);
  }

  if (query === '/clear') {
    chatBox.setContent('');
    traceBox.setContent('');
    inputBox.focus();
    screen.render();
    return;
  }

  if (query === '/routines') {
    const routines = engine.getRoutines();
    if (routines.length === 0) {
      chatBox.log('{gray-fg}no compiled routines yet{/gray-fg}');
    } else {
      for (const r of routines) {
        chatBox.log(`{bold}${r.category}{/bold}: ${r.chain} — ${r.episodes} episodes, avg ${r.avgLatency}ms`);
      }
    }
    chatBox.log('');
    inputBox.focus();
    screen.render();
    return;
  }

  if (query === '/reset') {
    engine.episodes = [];
    engine.routines = {};
    engine.save();
    chatBox.log('{yellow-fg}episodes and routines cleared{/yellow-fg}');
    chatBox.log('');
    updateRoutineDisplay();
    inputBox.focus();
    screen.render();
    return;
  }

  handleQuery(query);
});

screen.key(['escape', 'C-c'], () => process.exit(0));
screen.key(['tab'], () => inputBox.focus());

// ── Boot ────────────────────────────────────────────────

chatBox.log('{bold}SOMA Consensus{/bold} — three-brain composition engine');
chatBox.log('');
chatBox.log('{cyan-fg}◈ Mercury{/cyan-fg} (diffusion) → draft');
chatBox.log('{green-fg}◉ GLM{/green-fg} (reasoning) → evaluate');
chatBox.log('{yellow-fg}◆ Kimi{/yellow-fg} (autoregressive) → synthesize');
chatBox.log('');
chatBox.log('Commands: /clear /routines /reset /quit');
chatBox.log('');

const epCount = engine.episodes.length;
const rtCount = Object.keys(engine.routines).length;
if (epCount > 0) {
  chatBox.log(`{gray-fg}loaded ${epCount} episodes, ${rtCount} routines from disk{/gray-fg}`);
  chatBox.log('');
}

inputBox.focus();
screen.render();
