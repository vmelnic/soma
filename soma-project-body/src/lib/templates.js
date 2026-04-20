import nunjucks from 'nunjucks';

import deciderSystem from '../templates/decider-system.txt?raw';
import narratorTerse from '../templates/narrator-terse.txt?raw';
import narratorDebug from '../templates/narrator-debug.txt?raw';
import narratorAlarm from '../templates/narrator-alarm.txt?raw';
import narratorContext from '../templates/narrator-context.txt?raw';

const env = new nunjucks.Environment(null, { autoescape: false });

const TEMPLATES = {
  'decider-system': deciderSystem,
  'narrator-terse': narratorTerse,
  'narrator-debug': narratorDebug,
  'narrator-alarm': narratorAlarm,
  'narrator-context': narratorContext,
};

export function render(name, vars = {}) {
  const src = TEMPLATES[name];
  if (!src) throw new Error(`template '${name}' not found`);
  return env.renderString(src, vars).trim();
}

export const NARRATOR_MODES = ['terse', 'debug', 'alarm'];
