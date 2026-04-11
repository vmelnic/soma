// OpenAI function-tool definitions exposed to the chat brain.
//
// Commit "conversation-first" deliberately keeps this set tiny.
// Instead of generating one OpenAI tool per (port, capability) pair
// — which would require us to maintain a static catalog that drifts
// from what soma-next actually has loaded — we expose three
// introspection-friendly tools and let the model discover the live
// runtime through them:
//
//   list_ports   — returns the currently loaded port catalog with
//                  per-port capability lists and input/output schemas
//   list_skills  — returns the skill catalog for the active pack
//   invoke_port  — invokes any (port_id, capability_id) with input
//
// The chat brain's system prompt tells it to call list_ports /
// list_skills first when it doesn't know what's available, then
// invoke_port to actually run things. This is the same pattern
// HelperBook uses via raw MCP, just wrapped as OpenAI function
// tools so gpt-4o-mini's tool-calling mechanism can drive it.
//
// If the tool set grows later, it grows HERE, once, as code we own.
// The model's behavior is the same either way — it discovers tools
// through list_ports / list_skills, never through a hardcoded
// catalog in the prompt.

// Static OpenAI function-tool definitions. Shape matches
// https://platform.openai.com/docs/api-reference/chat/create
// tools[] → { type: "function", function: {...} }
export const DEFAULT_CHAT_TOOLS = Object.freeze([
  {
    type: "function",
    function: {
      name: "list_ports",
      description:
        "List the ports currently loaded in the SOMA runtime, with each port's capabilities and their input/output schemas. Call this first when you don't know what's available. Takes no arguments.",
      parameters: {
        type: "object",
        properties: {},
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "list_skills",
      description:
        "List the skills currently registered in the active pack, with their ids, namespaces, and descriptions. Call this to discover higher-level operations composed over ports. Takes no arguments.",
      parameters: {
        type: "object",
        properties: {},
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "invoke_port",
      description:
        "Invoke a specific port capability and return its result. Use list_ports first to discover which (port_id, capability_id) pairs exist and what input each takes. The `input` field is the capability-specific JSON input (e.g. {sql, params} for postgres.query).",
      parameters: {
        type: "object",
        properties: {
          port_id: {
            type: "string",
            description:
              "The id of the port to invoke, from the list_ports catalog (e.g. 'postgres', 'smtp', 'crypto').",
          },
          capability_id: {
            type: "string",
            description:
              "The capability id under that port (e.g. 'query', 'execute', 'send_plain', 'sha256').",
          },
          input: {
            type: "object",
            description:
              "Capability-specific input object. Shape depends on the port and capability. For postgres.query use {sql, params}; for crypto.sha256 use {data}; etc.",
            additionalProperties: true,
          },
        },
        required: ["port_id", "capability_id", "input"],
        additionalProperties: false,
      },
    },
  },
]);

// Build an `invokeTool(name, args)` handler closed over a
// SomaMcpClient instance. The handler dispatches the three
// introspection tools to the right MCP method and wraps the
// response in a uniform shape the chat brain loop understands:
//
//   { ok: true,  result: <any> }     // success
//   { ok: false, error: "..." }      // failure (tool-level, not network)
//
// Callers should catch network/transport errors themselves — those
// propagate out as thrown exceptions from the underlying
// SomaMcpClient.
export function makeInvokeTool(soma) {
  return async function invokeTool(name, args) {
    try {
      if (name === "list_ports") {
        const raw = await soma.callTool("list_ports", {});
        return { ok: true, result: soma.unwrap(raw) };
      }
      if (name === "list_skills") {
        const raw = await soma.callTool("list_skills", {});
        return { ok: true, result: soma.unwrap(raw) };
      }
      if (name === "invoke_port") {
        const portId = args?.port_id;
        const capabilityId = args?.capability_id;
        const input = args?.input ?? {};
        if (typeof portId !== "string" || portId === "") {
          return { ok: false, error: "invoke_port: missing port_id" };
        }
        if (typeof capabilityId !== "string" || capabilityId === "") {
          return { ok: false, error: "invoke_port: missing capability_id" };
        }
        const structured = await soma.invokePort(portId, capabilityId, input);
        return { ok: true, result: structured };
      }
      return { ok: false, error: `unknown tool: ${name}` };
    } catch (err) {
      return { ok: false, error: err.message || String(err) };
    }
  };
}
