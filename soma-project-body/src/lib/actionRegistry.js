// Builds native tool-calling definitions from SOMA ports and MCP tools.
//
// Each port capability becomes a function the LLM can call directly.
// Each MCP tool (introspection, sessions, goals) becomes a function too.
// The registry also maps function names back to their dispatch method.

function sanitizeName(name) {
  return name.replace(/[^a-zA-Z0-9_]/g, '_');
}

function normalizeSchema(raw) {
  const s = raw && typeof raw === 'object' ? { ...raw } : {};
  if (!s.type) s.type = 'object';
  if (s.type === 'object' && !s.properties) s.properties = {};
  return s;
}

export function buildRegistry(ports, remotePorts, tools) {
  const definitions = [];
  const dispatch = {};

  // Port capabilities → tool definitions.
  for (const port of [...(ports || []), ...(remotePorts || [])]) {
    for (const cap of port.capabilities || []) {
      const capId = cap.capability_id || cap.id || cap.name;
      const name = sanitizeName(`${port.port_id}__${capId}`);
      const schema = normalizeSchema(cap.input_schema?.schema || cap.input_schema);

      definitions.push({
        type: 'function',
        function: {
          name,
          description: `[port ${port.port_id}] ${cap.purpose || cap.name || capId}`,
          parameters: schema,
        },
      });

      dispatch[name] = { kind: 'port', port_id: port.port_id, capability_id: capId };
    }
  }

  // MCP tools → tool definitions.
  for (const tool of tools || []) {
    if (tool.name === 'invoke_port') continue;
    const safeName = sanitizeName(tool.name);
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
