/**
 * Resolve mustache-style bindings against a data context.
 *
 * resolveBinding('{{name}}', { name: 'Alice' }) => 'Alice'
 * resolveBinding('Hello {{name}}!', { name: 'Bob' }) => 'Hello Bob!'
 * resolveBinding('{{count}}', { count: 5 }) => 5  (preserves type for solo bindings)
 * resolveBinding('plain text', {}) => 'plain text'
 * resolveBinding(undefined, {}) => undefined
 */
export function resolveBinding(template, context) {
  if (template == null) return template;
  if (typeof template !== 'string') return template;

  // If the entire string is a single binding, return the raw value (preserves type)
  const soloMatch = template.match(/^\{\{(\w[\w.]*)\}\}$/);
  if (soloMatch) {
    return getPath(context, soloMatch[1]);
  }

  // Otherwise, interpolate all bindings as strings
  return template.replace(/\{\{(\w[\w.]*)\}\}/g, (_, path) => {
    const val = getPath(context, path);
    return val != null ? String(val) : '';
  });
}

/**
 * Resolve an entire bind map: { param: '{{field}}', literal: 'value' }
 */
export function resolveBindMap(bindMap, context) {
  if (!bindMap) return {};
  const result = {};
  for (const [key, val] of Object.entries(bindMap)) {
    result[key] = resolveBinding(val, context);
  }
  return result;
}

/**
 * Get a nested value by dot path: 'a.b.c' from { a: { b: { c: 1 } } }
 */
function getPath(obj, path) {
  if (!obj || !path) return undefined;
  const parts = path.split('.');
  let cur = obj;
  for (const part of parts) {
    if (cur == null) return undefined;
    cur = cur[part];
  }
  return cur;
}
