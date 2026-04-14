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
  {
    type: "function",
    function: {
      name: "schedule",
      description:
        "Create a scheduled action. delay_ms = one-shot, interval_ms = recurring. Provide message for chat notifications or port_id+capability_id for port calls. Optional max_fires to auto-stop.",
      parameters: {
        type: "object",
        properties: {
          label: { type: "string", description: "Human-readable label" },
          delay_ms: { type: "integer", description: "Fire once after N ms" },
          interval_ms: { type: "integer", description: "Fire every N ms" },
          message: { type: "string", description: "Text to show in chat (no port call)" },
          port_id: { type: "string", description: "Port to invoke" },
          capability_id: { type: "string", description: "Capability on the port" },
          input: { type: "object", description: "Port call payload", additionalProperties: true },
          max_fires: { type: "integer", description: "Stop after N fires" },
          brain: { type: "boolean", description: "Route result through LLM for interpretation" },
        },
        required: ["label"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "list_schedules",
      description: "List all active schedules.",
      parameters: { type: "object", properties: {}, additionalProperties: false },
    },
  },
  {
    type: "function",
    function: {
      name: "cancel_schedule",
      description: "Cancel a schedule by UUID.",
      parameters: {
        type: "object",
        properties: {
          schedule_id: { type: "string", description: "Schedule UUID to cancel" },
        },
        required: ["schedule_id"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "trigger_consolidation",
      description: "Force the learning pipeline to run now (episode → schema → routine compilation).",
      parameters: { type: "object", properties: {}, additionalProperties: false },
    },
  },
  {
    type: "function",
    function: {
      name: "patch_world_state",
      description: "Add or remove facts from SOMA's world state. Facts are conditions that can trigger autonomous routines.",
      parameters: {
        type: "object",
        properties: {
          add_facts: { type: "array", description: "Facts to add: [{fact_id, subject, predicate, value, confidence}]", items: { type: "object" } },
          remove_fact_ids: { type: "array", description: "Fact IDs to remove", items: { type: "string" } },
        },
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "dump_world_state",
      description: "Show the current world state — all known facts about the world.",
      parameters: { type: "object", properties: {}, additionalProperties: false },
    },
  },
  {
    type: "function",
    function: {
      name: "set_routine_autonomous",
      description: "Mark a compiled routine to fire automatically when its conditions match the world state.",
      parameters: {
        type: "object",
        properties: {
          routine_id: { type: "string", description: "Routine ID" },
          autonomous: { type: "boolean", description: "Enable or disable autonomous execution" },
        },
        required: ["routine_id", "autonomous"],
        additionalProperties: false,
      },
    },
  },
]);

// Build an `invokeTool(name, args)` handler closed over a
// SomaMcpClient instance. All tools beyond invoke_port route
// through soma.callTool as MCP pass-through.
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
      // All other known tools route through MCP callTool.
      const knownTools = new Set(DEFAULT_CHAT_TOOLS.map(t => t.function.name));
      if (knownTools.has(name)) {
        const raw = await soma.callTool(name, args ?? {});
        return { ok: true, result: soma.unwrap(raw) };
      }
      return { ok: false, error: `unknown tool: ${name}` };
    } catch (err) {
      return { ok: false, error: err.message || String(err) };
    }
  };
}
