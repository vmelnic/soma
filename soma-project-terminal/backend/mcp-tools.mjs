// OpenAI function-tool definitions exposed to the chat brain.
//
// The brain has a focused set of tools that cover port invocation,
// scheduling, routine lifecycle, world state, and distributed ops.
// Port discovery tools (list_ports, list_skills, etc.) are NOT
// exposed — the port catalog is embedded in the system prompt to
// prevent the model from burning tool-call loops on rediscovery.
//
// All tools beyond invoke_port route through MCP callTool pass-through
// to the soma-next subprocess. Adding a tool = adding it here + the
// corresponding handler in soma-next's MCP server.

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
      description: "Mark a compiled routine to fire automatically when its conditions match the world state. Verify the routine is safe before enabling.",
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
  {
    type: "function",
    function: {
      name: "author_routine",
      description:
        "Create or update a routine from a structured definition. Re-authoring an existing routine_id bumps the version. Provide match_conditions, steps, and optionally guard_conditions, priority, exclusive, policy_scope, autonomous.",
      parameters: {
        type: "object",
        properties: {
          routine_id: { type: "string", description: "Unique identifier for the routine" },
          match_conditions: {
            type: "array",
            description: "Conditions that trigger this routine (goal_fingerprint or world_state)",
            items: {
              type: "object",
              properties: {
                condition_type: { type: "string" },
                expression: { description: "JSON expression to match against context" },
                description: { type: "string" },
              },
              required: ["condition_type", "expression", "description"],
            },
          },
          steps: {
            type: "array",
            description: "Ordered execution steps",
            items: {
              type: "object",
              properties: {
                type: { type: "string", enum: ["skill", "sub_routine"] },
                skill_id: { type: "string", description: "For skill steps" },
                routine_id: { type: "string", description: "For sub_routine steps" },
                on_success: { type: "object", description: "Action on success (default: continue)" },
                on_failure: { type: "object", description: "Action on failure (default: abandon)" },
              },
              required: ["type"],
            },
          },
          guard_conditions: { type: "array", description: "Optional conditions that must ALL pass" },
          priority: { type: "integer", description: "Higher fires first (default 0)" },
          exclusive: { type: "boolean", description: "If true, blocks lower-priority matches (default false)" },
          policy_scope: { type: "string", description: "Optional policy namespace override" },
          autonomous: { type: "boolean", description: "If true, reactive monitor fires this automatically (default false)" },
        },
        required: ["routine_id", "match_conditions", "steps"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "list_routine_versions",
      description: "List all versions of a routine, including history and current. Check before re-authoring.",
      parameters: {
        type: "object",
        properties: {
          routine_id: { type: "string", description: "The routine ID to list versions for" },
        },
        required: ["routine_id"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "rollback_routine",
      description: "Roll back a routine to a previous version from its history.",
      parameters: {
        type: "object",
        properties: {
          routine_id: { type: "string", description: "The routine ID to roll back" },
          target_version: { type: "integer", description: "The version number to roll back to" },
        },
        required: ["routine_id", "target_version"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "replicate_routine",
      description: "Replicate a compiled routine to remote peers. Omit peer_ids to replicate to all known peers.",
      parameters: {
        type: "object",
        properties: {
          routine_id: { type: "string", description: "The routine ID to replicate" },
          peer_ids: {
            type: "array",
            items: { type: "string" },
            description: "Target peer IDs (optional, defaults to all known peers)",
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
      name: "review_routine",
      description: "Review a routine's safety profile before marking it autonomous. Returns what the routine does, which skills it touches, side effects, and a recommendation.",
      parameters: {
        type: "object",
        properties: {
          routine_id: { type: "string", description: "The routine to review" },
        },
        required: ["routine_id"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "sync_beliefs",
      description: "Synchronize world state facts with a remote peer. Sends current facts and merges the result.",
      parameters: {
        type: "object",
        properties: {
          peer_id: { type: "string", description: "The peer identifier to sync beliefs with" },
        },
        required: ["peer_id"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "migrate_session",
      description: "Migrate an active session to a remote peer. Transfers goal, working memory, belief, observations, budget, trace, and policy context atomically.",
      parameters: {
        type: "object",
        properties: {
          session_id: { type: "string", description: "The session UUID to migrate" },
          peer_id: { type: "string", description: "The target peer identifier" },
        },
        required: ["session_id", "peer_id"],
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
