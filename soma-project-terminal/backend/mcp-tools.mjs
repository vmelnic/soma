// OpenAI function-tool definitions exposed to the chat brain.
//
// The chat brain has ONE tool: `invoke_port`. Every port capability
// in the SOMA runtime is reachable through it. The brain doesn't
// need separate `list_ports` / `list_skills` tools because the port
// catalog is embedded directly in the system prompt
// (see SomaMcpClient.getPortCatalogSummary() → buildSystemPrompt).
//
// Why one tool instead of many:
//
// - gpt-4o-mini on a fresh context was running list_ports +
//   list_skills + invoke_port repeatedly, burning the MAX_TOOL_LOOPS
//   budget on rediscovery every turn and never producing a final
//   reply. Removing the discovery tools forces it to use what's
//   already in the prompt instead of looping on introspection.
// - ports don't change between turns — they're baked into the
//   platform pack at startup. A dynamic discovery tool for static
//   data is cargo-cult tool design.
// - a single tool with a clear structured input is the minimum
//   surface area the model has to get right.
//
// If the tool set grows later (e.g. a `search_memory` convenience
// tool), it grows HERE, as code WE own, not by exposing more MCP
// introspection endpoints to the LLM.

export const DEFAULT_CHAT_TOOLS = Object.freeze([
  {
    type: "function",
    function: {
      name: "invoke_port",
      description:
        "Invoke a specific port capability and return its result. The port catalog is listed in your system prompt — use those exact (port_id, capability_id) pairs. The `input` field is the capability-specific JSON input (e.g. {sql, params} for postgres.query, {data} for crypto.sha256, {to, subject, body} for smtp.send_plain).",
      parameters: {
        type: "object",
        properties: {
          port_id: {
            type: "string",
            description:
              "Port id from the catalog in your system prompt (e.g. 'postgres', 'smtp', 'crypto').",
          },
          capability_id: {
            type: "string",
            description:
              "Capability id under that port (e.g. 'query', 'execute', 'send_plain', 'sha256', 'random_string').",
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
  {
    type: "function",
    function: {
      name: "execute_routine",
      description:
        "Execute a compiled routine by ID. Routines are pre-learned procedures that run faster than step-by-step tool calls.",
      parameters: {
        type: "object",
        properties: {
          routine_id: {
            type: "string",
            description: "Routine ID from the available routines list",
          },
          input: {
            type: "object",
            description: "Optional input bindings",
            additionalProperties: true,
          },
        },
        required: ["routine_id"],
        additionalProperties: false,
      },
    },
  },
]);

// Build an `invokeTool(name, args)` handler closed over a
// SomaMcpClient instance. The only supported tool is `invoke_port`
// — list_ports / list_skills were removed because the catalog is
// already in the system prompt.
//
// Returns { ok: true, result: <any> } on success or
// { ok: false, error: "..." } on failure. Callers should catch
// network/transport errors themselves — those propagate out as
// thrown exceptions from the underlying SomaMcpClient.
export function makeInvokeTool(soma) {
  return async function invokeTool(name, args) {
    try {
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
      if (name === "execute_routine") {
        const raw = await soma.callTool("execute_routine", args ?? {});
        return { ok: true, result: soma.unwrap(raw) };
      }
      return { ok: false, error: `unknown tool: ${name}` };
    } catch (err) {
      return { ok: false, error: err.message || String(err) };
    }
  };
}
