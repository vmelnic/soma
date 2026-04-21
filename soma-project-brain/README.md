# soma-project-brain

External LLM brain driving SOMA via MCP — proves the brain/body separation end-to-end.

## What this proves

The full autonomous loop with an external brain: SOMA runs the control loop, selects skills, pauses when inputs are missing, and an external LLM (Claude Haiku) composes the bindings. The body never decides *what* to do — it provides proprioception so the brain can.

| Feature | Test |
|---|---|
| Async goal lifecycle | `create_goal_async` → background execution → `get_goal_status` polling |
| Brain-as-input-source | `WaitingForInput` → `inspect_session` → LLM call → `provide_session_input` |
| Belief projection | TOON-encoded belief state sent to LLM as context |
| Push notifications | `notifications/goal/trace_step` streams live trace events |
| Goal decomposition | Complex goals split into sub-goals, mapped to skills via single LLM call |
| Plan injection | `inject_plan` seeds multi-step plans before execution starts |
| Routine matching | `find_routines` checks for existing routines before decomposing |
| Skill redirect | Brain can override body's skill selection via `_redirect_skill` |

## Architecture

```
User goal → brain.js → create_goal_async → SOMA (stdio MCP)
                                              ↓
                                         control loop
                                              ↓
                                       WaitingForInput
                                              ↓
              brain.js ← inspect_session + inspect_belief_projection
                  ↓
             Claude Haiku (compose bindings)
                  ↓
              provide_session_input → SOMA resumes → next step
```

## Prerequisites

- SOMA binary built from `soma-project-body` (`../soma-project-body/bin/soma`)
- Packs from `soma-project-body/packs/` (ports as `.dylib`)
- Node.js 18+
- Anthropic API key

## Setup

```bash
cp .env.example .env   # set ANTHROPIC_API_KEY
```

## Usage

```bash
# Simple goal (body handles skill selection, brain provides inputs)
node brain.js "list all tables in the database"

# Complex goal (brain decomposes, injects plan, provides inputs at each step)
node brain.js "create a users table and insert Alice and Bob"

# With custom step limit
node brain.js "query all users" 5
```

## Environment

| Variable | Default | Description |
|---|---|---|
| `ANTHROPIC_API_KEY` | — | Anthropic API key (required) |
| `ANTHROPIC_MODEL` | `claude-haiku-4-5-20251001` | Model for brain LLM calls |
