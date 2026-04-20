// Builds native tool-calling definitions from SOMA skills.
//
// Skills are the brain's vocabulary. Ports are the body's internals.
// Each skill becomes a tool definition; dispatch resolves skill → port call.

function sanitizeName(name) {
  return name.replace(/[^a-zA-Z0-9_]/g, '_');
}

function normalizeSchema(raw) {
  const s = raw && typeof raw === 'object' ? { ...raw } : {};
  if (!s.type) s.type = 'object';
  if (s.type === 'object' && !s.properties) s.properties = {};
  return s;
}

// Parse "port:<port_id>/<capability_id>" → { port_id, capability_id }
function parseCapabilityReq(req) {
  const m = req.match(/^port:([^/]+)\/(.+)$/);
  if (!m) return null;
  return { port_id: m[1], capability_id: m[2] };
}

export function buildRegistry(skills, tools) {
  const definitions = [];
  const dispatch = {};

  // Only expose MCP control tools — the LLM uses invoke_port for all port
  // interactions, guided by the port catalog in the system prompt.
  const passthrough = ['create_goal_async', 'get_goal_status', 'list_ports', 'list_capabilities', 'invoke_port'];
  for (const tool of tools || []) {
    if (!passthrough.includes(tool.name)) continue;
    const safeName = sanitizeName(tool.name);
    if (dispatch[safeName]) continue;
    const schema = normalizeSchema(tool.inputSchema || tool.input_schema);

    definitions.push({
      type: 'function',
      function: {
        name: safeName,
        description: tool.description || tool.name,
        parameters: schema,
      },
    });

    dispatch[safeName] = { kind: 'tool', name: tool.name };
  }

  return { definitions, dispatch };
}
