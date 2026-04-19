# LLM + Agents vs. SOMA

Two architectures that look similar from the outside and are opposite on the inside.

## The one-line difference

- **Agents** — LLM controls everything, your code does the work, memory dies with the chat.
- **SOMA** — LLM says *what* it wants, SOMA controls and does the work, memory is permanent and structured.

---

## Today: LLM + Agent framework

```
┌──────┐      ┌──────┐      ┌─────────────┐      ┌──────────┐
│ User │ ───▶ │ LLM  │ ───▶ │ "call DB"   │ ───▶ │ Your code│
└──────┘      └──────┘      └─────────────┘      └──────────┘
                  ▲                                   │
                  │                result             │
                  └───────────────────────────────────┘
                  ▼
              "now send email" ───▶ your code ───▶ result
                  ▼
              "now check if…"  ───▶ your code ───▶ result
                  ▼
                 ...
```

- Who wrote the database code? **You.**
- Who wrote the email code? **You.**
- Who wrote the glue between them? **You.**
- Who decides the order of steps? **The LLM.**
- Where is the memory? **In the chat history.**
- Chat history after 20 steps? **Huge. LLM starts forgetting.**
- New chat session? **Start over. Everything lost.**

The LLM is doing three jobs at once: deciding, calling, and remembering. All three degrade as context grows.

---

## SOMA

Two sides. Brain (LLM) decides *what*. Body (SOMA runtime) decides *how* and remembers.

```
╔════════════════════ BRAIN SIDE (any LLM) ══════════════════════╗
║                                                                ║
║  1. LLM connects to SOMA over MCP                              ║
║  2. LLM calls  dump_state   ──▶ ~5KB: ports, skills,           ║
║                                  episodes, belief state        ║
║                                  (replaces reading 20k LoC)    ║
║  3. LLM calls  list_ports   ──▶ postgres, smtp, crypto, redis… ║
║                                  with every capability listed  ║
║                                  (cached once, never re-asked) ║
║  4. User: "register user John and send welcome email"          ║
║  5. LLM picks one of two tools:                                ║
║        simple  → invoke_port   (one port, one call)            ║
║        complex → create_goal   (hand to SOMA's control loop)   ║
║  6. LLM calls  create_goal("register user + welcome email")    ║
║  7. LLM is DONE DECIDING. Waits.                               ║
║                                                                ║
╚═══════════════════════════╦════════════════════════════════════╝
                            │  goal
                            ▼
╔════════════════════ BODY SIDE (SOMA runtime) ═════════════════╗
║                                                                ║
║  SOMA receives the goal                                        ║
║  step 1 ─▶ picks postgres.insert   (scored best)               ║
║          ─▶ calls it ─▶ watches result ─▶ ok                   ║
║          ─▶ updates belief: "user John exists" (conf 1.0)      ║
║  step 2 ─▶ picks smtp.send_plain   (next best move)            ║
║          ─▶ calls it ─▶ watches result ─▶ ok                   ║
║          ─▶ updates belief: "welcome email sent" (conf 1.0)    ║
║  critic ─▶ goal satisfied ─▶ stop                              ║
║  saves full trace as  episode #47                              ║
║                                                                ║
╚═══════════════════════════╦════════════════════════════════════╝
                            │  small result
                            ▼
╔══════════════════════ BRAIN SIDE again ═══════════════════════╗
║                                                                ║
║  8. LLM gets back:  { status: completed, steps: 2 }            ║
║  9. LLM tells user: "Done. John is registered and got his      ║
║                      welcome email."                           ║
║                                                                ║
║  Total LLM context used : system prompt + dump_state + 1 result║
║  Total LLM decisions    : one — which MCP tool to call         ║
║  Everything else        : happened inside SOMA                 ║
╚════════════════════════════════════════════════════════════════╝
```

- Who wrote the database code? **Nobody. The postgres port exists.**
- Who wrote the email code? **Nobody. The smtp port exists.**
- Who wrote the glue? **Nobody. SOMA's control loop handles it.**
- Who decides the steps? **SOMA, not the LLM.**
- Where is the memory? **In SOMA. Permanently.**

---

## Memory, three layers

```
┌─────────────────────────────────────────────────────────────────┐
│  BELIEF STATE      — what SOMA knows RIGHT NOW                   │
│  ─────────────────                                               │
│    "user John exists"       conf 1.0   source: observed          │
│    "email was sent"         conf 1.0   source: observed          │
│    "smtp port is available" conf 1.0                             │
│    (updated after every single step, not at end)                 │
├─────────────────────────────────────────────────────────────────┤
│  EPISODES          — complete history of WHAT HAPPENED           │
│  ─────────────────                                               │
│    episode #47  goal: "register + email"                         │
│       step 1: postgres.insert   12ms   ok                        │
│       step 2: smtp.send        340ms   ok                        │
│       outcome: success          total cost: 352ms                │
│    episode #46  goal: "find users nearby"                        │
│       step 1: postgres.query         ok                          │
│       step 2: geo.radius_filter      ok                          │
│       …                                                          │
│    (up to 1024 episodes, all queryable)                          │
├─────────────────────────────────────────────────────────────────┤
│  ROUTINES          — shortcuts SOMA learned from its history     │
│  ─────────────────                                               │
│    "register + email" succeeded 3 times                          │
│      ─▶ SOMA mined the pattern                                   │
│      ─▶ compiled a routine: postgres.insert → smtp.send          │
│      ─▶ next run: skips the thinking, runs the shortcut          │
│    (muscle memory — first time slow, fifth time instant)         │
└─────────────────────────────────────────────────────────────────┘
```

---

## What persists across sessions

| Situation                             | Agents                    | SOMA                         |
| ------------------------------------- | ------------------------- | ---------------------------- |
| New chat session                      | Start over                | `dump_state` → ~5KB, resume  |
| New LLM entirely (Claude → GPT)       | Rewrite prompts           | Same MCP, zero context lost  |
| Runtime crashed and restarted         | Chat history gone         | State on disk, nothing lost  |
| Swap model mid-conversation           | Partial amnesia           | Zero context lost            |
| 20 steps into a task                  | Context window degrading  | Brain context is still ~5KB  |

---

## Division of responsibility

| Concern                 | Agents                 | SOMA                             |
| ----------------------- | ---------------------- | -------------------------------- |
| Integration code        | You write it           | A port exists (or you write one) |
| Orchestration / glue    | You write it           | SOMA's control loop              |
| Step selection          | LLM, every step        | SOMA's scorer + critic           |
| Observation handling    | LLM interprets raw I/O | `PortCallRecord`, typed          |
| Memory of past runs     | Chat transcript        | Episodes on disk                 |
| Learning from repetition| None                   | Mined routines                   |
| LLM context growth      | Linear in steps        | Flat (~5KB baseline)             |
| Portability across LLMs | Prompt-specific        | Wire protocol is MCP             |

---

## When to use which

- **Agents** — one-shot tasks, no repetition, glue code you control, no long-term state.
- **SOMA** — repeated operations, real integrations, long-running goals, state that must survive restarts, multiple LLMs over time.

The shift: stop writing orchestration logic inside prompts. Put it in a body that remembers.
