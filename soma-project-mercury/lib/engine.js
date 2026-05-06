const { MERCURY, KIMI, GLM, call } = require('./brains');
const fs = require('fs');
const path = require('path');

const DATA_FILE = path.join(__dirname, '..', 'data', 'episodes.json');
const ROUTINE_FILE = path.join(__dirname, '..', 'data', 'routines.json');

const CATEGORIES = ['factual', 'reasoning', 'creative', 'coding', 'general'];

const CATEGORY_PATTERNS = {
  factual: /\b(what is|who is|when did|where is|how many|capital of|define|name)\b/i,
  reasoning: /\b(why|explain|because|if .* then|conclude|prove|logic|cause|reason)\b/i,
  creative: /\b(invent|imagine|create|write|story|poem|fiction|design|name .* that)\b/i,
  coding: /\b(code|function|program|algorithm|bug|error|implement|refactor|sql|api)\b/i,
};

function classify(query) {
  for (const [cat, re] of Object.entries(CATEGORY_PATTERNS)) {
    if (re.test(query)) return cat;
  }
  return 'general';
}

class ConsensusEngine {
  constructor(onTrace) {
    this.onTrace = onTrace || (() => {});
    this.episodes = [];
    this.routines = {};
    this.load();
  }

  load() {
    try {
      if (fs.existsSync(DATA_FILE)) {
        this.episodes = JSON.parse(fs.readFileSync(DATA_FILE, 'utf8'));
      }
    } catch { this.episodes = []; }
    try {
      if (fs.existsSync(ROUTINE_FILE)) {
        this.routines = JSON.parse(fs.readFileSync(ROUTINE_FILE, 'utf8'));
      }
    } catch { this.routines = {}; }
  }

  save() {
    const dir = path.dirname(DATA_FILE);
    if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(DATA_FILE, JSON.stringify(this.episodes, null, 2));
    fs.writeFileSync(ROUTINE_FILE, JSON.stringify(this.routines, null, 2));
  }

  getRoutines() {
    return Object.entries(this.routines).map(([cat, r]) => ({
      category: cat,
      chain: r.chain.map(b => b[0].toUpperCase()).join('→'),
      episodes: r.episodeCount,
      avgLatency: r.avgLatency,
    }));
  }

  async run(query) {
    const category = classify(query);
    const routine = this.routines[category];
    const totalStart = Date.now();

    this.onTrace({ type: 'classify', category });

    let steps;
    if (routine && routine.episodeCount >= 3) {
      this.onTrace({ type: 'routine', category, chain: routine.chain });
      steps = await this.executeChain(query, routine.chain);
    } else {
      steps = await this.fullPipeline(query);
    }

    const totalLatency = Date.now() - totalStart;
    const totalTokens = steps.reduce((s, st) => s + (st.tokens || 0), 0);
    const finalAnswer = steps[steps.length - 1].content || '[no answer]';

    const episode = {
      query,
      category,
      steps: steps.map(s => ({
        brain: s.brain,
        latency: s.latency,
        tokens: s.tokens,
        hasContent: !!s.content,
        role: s.role,
      })),
      totalLatency,
      totalTokens,
      timestamp: new Date().toISOString(),
    };
    this.episodes.push(episode);
    this.compileRoutines();
    this.save();

    this.onTrace({ type: 'done', totalLatency, totalTokens });

    return { answer: finalAnswer, category, steps, totalLatency, totalTokens };
  }

  async fullPipeline(query) {
    const steps = [];

    // Step 1: Mercury drafts fast
    this.onTrace({ type: 'step', brain: 'mercury', role: 'draft', status: 'running' });
    const draft = await call(MERCURY, [
      { role: 'user', content: query },
    ], { maxTokens: 1024, reasoningEffort: 'medium' });
    draft.brain = 'mercury';
    draft.role = 'draft';
    this.onTrace({ type: 'step', brain: 'mercury', role: 'draft', status: 'done', ...draft });
    steps.push(draft);

    if (draft.error) {
      return steps;
    }

    // Step 2: GLM evaluates and improves
    this.onTrace({ type: 'step', brain: 'glm', role: 'evaluate', status: 'running' });
    const evaluation = await call(GLM, [
      { role: 'user', content: query },
      { role: 'assistant', content: draft.content },
      { role: 'user', content: 'Evaluate this answer. If it is correct, repeat it. If it has errors or can be improved, provide a better answer. Be concise.' },
    ], { maxTokens: 2048 });
    evaluation.brain = 'glm';
    evaluation.role = 'evaluate';
    this.onTrace({ type: 'step', brain: 'glm', role: 'evaluate', status: 'done', ...evaluation });
    steps.push(evaluation);

    // Step 3: Kimi synthesizes
    const bestSoFar = (!evaluation.error && evaluation.content) ? evaluation.content : draft.content;
    this.onTrace({ type: 'step', brain: 'kimi', role: 'synthesize', status: 'running' });
    const synthesis = await call(KIMI, [
      { role: 'user', content: `Question: ${query}\n\nDraft answer: ${bestSoFar}\n\nSynthesize a clear, concise final answer.` },
    ], { maxTokens: 1024 });
    synthesis.brain = 'kimi';
    synthesis.role = 'synthesize';
    this.onTrace({ type: 'step', brain: 'kimi', role: 'synthesize', status: 'done', ...synthesis });
    steps.push(synthesis);

    return steps;
  }

  async executeChain(query, chain) {
    const steps = [];
    let previousContent = '';

    for (let i = 0; i < chain.length; i++) {
      const brainName = chain[i];
      const brain = { mercury: MERCURY, kimi: KIMI, glm: GLM }[brainName];
      if (!brain) continue;

      const role = i === 0 ? 'draft' : i === chain.length - 1 ? 'synthesize' : 'evaluate';
      this.onTrace({ type: 'step', brain: brainName, role, status: 'running' });

      let messages;
      if (i === 0) {
        messages = [{ role: 'user', content: query }];
      } else if (role === 'synthesize') {
        messages = [{ role: 'user', content: `Question: ${query}\n\nDraft answer: ${previousContent}\n\nSynthesize a clear, concise final answer.` }];
      } else {
        messages = [
          { role: 'user', content: query },
          { role: 'assistant', content: previousContent },
          { role: 'user', content: 'Evaluate this answer. If correct, repeat it. If improvable, provide a better answer. Be concise.' },
        ];
      }

      const opts = { maxTokens: 1024 };
      if (brainName === 'mercury') opts.reasoningEffort = 'medium';

      const result = await call(brain, messages, opts);
      result.brain = brainName;
      result.role = role;
      this.onTrace({ type: 'step', brain: brainName, role, status: 'done', ...result });
      steps.push(result);

      if (result.content) previousContent = result.content;
      if (result.error) break;
    }

    return steps;
  }

  compileRoutines() {
    for (const cat of CATEGORIES) {
      const catEpisodes = this.episodes.filter(e => e.category === cat);
      if (catEpisodes.length < 3) continue;

      const recent = catEpisodes.slice(-10);

      // Analyze which brains contributed
      const brainValue = { mercury: 0, glm: 0, kimi: 0 };
      const brainLatency = { mercury: 0, glm: 0, kimi: 0 };
      let count = 0;

      for (const ep of recent) {
        count++;
        for (const step of ep.steps) {
          if (step.hasContent) brainValue[step.brain]++;
          brainLatency[step.brain] += step.latency;
        }
      }

      // Skip brains that add latency without consistent value
      const chain = [];
      const brains = ['mercury', 'glm', 'kimi'];
      for (const b of brains) {
        const valueRate = brainValue[b] / count;
        const avgLat = brainLatency[b] / count;
        // Keep brain if it produces content >60% of the time
        // OR if it's the first/last brain (always keep drafter and synthesizer)
        if (valueRate > 0.6 || b === 'mercury') {
          chain.push(b);
        }
      }

      // Always have at least 2 in chain
      if (chain.length < 2 && !chain.includes('kimi')) chain.push('kimi');
      if (chain.length < 1) chain.push('mercury');

      const avgLatency = Math.round(
        recent.reduce((s, e) => s + e.totalLatency, 0) / recent.length
      );

      this.routines[cat] = {
        chain,
        episodeCount: catEpisodes.length,
        avgLatency,
        compiledAt: new Date().toISOString(),
      };
    }
  }
}

module.exports = { ConsensusEngine, classify };
