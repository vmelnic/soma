// Per-context cache of database table schemas, populated lazily
// from information_schema introspection after the brain touches
// a table for the first time.

// Map<contextId, Map<tableName, Array<{column_name, data_type}>>>
const caches = new Map();

export function hasTable(contextId, tableName) {
  return caches.get(contextId)?.has(tableName) ?? false;
}

export function setTableSchema(contextId, tableName, columns) {
  if (!caches.has(contextId)) caches.set(contextId, new Map());
  caches.get(contextId).set(tableName, columns);
}

export function formatSchemaCache(contextId) {
  const cache = caches.get(contextId);
  if (!cache || cache.size === 0) return "(no tables discovered yet)";

  const parts = [];
  for (const [table, columns] of cache) {
    const cols = columns.map(c => `${c.column_name} ${c.data_type}`).join(", ");
    parts.push(`${table} (${cols})`);
  }
  return parts.join("\n");
}

// Extract table names from SQL using simple regex.
// Only catches the most common patterns — not a SQL parser.
export function extractTableNames(sql) {
  if (!sql || typeof sql !== "string") return [];
  const re = /(?:FROM|INTO|UPDATE|JOIN|TABLE(?:\s+IF\s+(?:NOT\s+)?EXISTS)?)\s+([a-z_][a-z0-9_]*)/gi;
  const names = new Set();
  let match;
  while ((match = re.exec(sql)) !== null) {
    names.add(match[1].toLowerCase());
  }
  return [...names];
}

// Introspect a table via the postgres port and cache the result.
export async function introspectTable(soma, contextId, tableName) {
  try {
    const result = await soma.invokePort("postgres", "query", {
      sql: "SELECT column_name, data_type FROM information_schema.columns WHERE table_name = $1 ORDER BY ordinal_position",
      params: [tableName],
    });
    if (result.rows && result.rows.length > 0) {
      setTableSchema(contextId, tableName, result.rows);
    }
  } catch {
    // introspection failed — table may not exist yet, skip silently
  }
}
