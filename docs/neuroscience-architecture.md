# SOMA as a Nervous System

How SOMA's architecture maps to biological neural systems, and why
every design decision has a neuroscience precedent.

## The core analogy

A biological organism has a brain (decides), a body (executes), and a
nervous system connecting them (observes, adapts, remembers). The brain
does not move muscles directly. It sends intentions. The body translates
intentions into action, reports back what happened, and over time
develops reflexes that bypass the brain entirely.

SOMA is this organism in software.

| Biology | SOMA | Role |
|---|---|---|
| Brain | LLM (or autonomous selector) | Intent, planning, judgment |
| Body | Runtime + ports | Execution, sensation, proprioception |
| Spinal cord | MCP interface | Signal pathway between brain and body |
| Muscles / senses | Port adapters | Actuators and sensors for external systems |
| Hippocampus | Episode store | Fast binding of specific experiences |
| Neocortex | Schema store | Slow extraction of general patterns |
| Basal ganglia | Routine store + plan-following | Compiled habits that bypass deliberation |
| Prefrontal cortex | Working memory / belief state | Active context for current task |
| Amygdala | Salience scoring | What's worth remembering |
| Sleep | Background consolidation cycle | Offline replay and pattern extraction |
| Prediction error | Observation filtering | Only encode what's surprising |
| Proprioception | dump_state / self_model | The body knowing its own state |

## Two learning systems: fast and slow

The most important neuroscience insight behind SOMA comes from
Complementary Learning Systems theory (McClelland, McNaughton &
O'Reilly, 1995). The brain uses two systems with opposite properties:

**The hippocampus** learns fast. One experience is enough. It binds
the specific details of what happened — who, where, when, what —
into a retrievable episode. It keeps similar episodes separate
(pattern separation) so they don't blur together.

**The neocortex** learns slow. It needs many exposures. It extracts
the statistical regularity — what's common across episodes — and
stores it as generalized knowledge. It overlaps similar patterns
(pattern completion) so they reinforce each other.

If you update the neocortex too fast, you get catastrophic
interference — new knowledge destroys old knowledge. If you only
have the hippocampus, you remember every detail but never generalize.
The two systems need each other.

SOMA implements both:

**Episode store = hippocampus.** Each completed session becomes an
episode — the full trace of what skills were selected, what ports
were called, what was observed, what the critic decided. Fast,
one-shot, specific. The ring buffer holds 1024 episodes. Retrieval
uses embedding similarity (like how a cue triggers episodic recall).

**Schema induction via PrefixSpan = neocortical consolidation.**
When enough similar episodes accumulate (3+ with the same goal
fingerprint), PrefixSpan extracts the common skill subsequence.
This is the generalization step — from "I did readdir then stat
then readfile in episodes 47, 52, and 61" to "the pattern for
file inspection is readdir → stat → readfile." The specific
episodes produced a general schema.

**Routine compilation = basal ganglia habit formation.** When a
schema reaches sufficient confidence (0.7+), it compiles into a
routine — a compiled step sequence that executes without deliberation.
The routine IS the memory. It's not consulted, it's executed.
Like how you don't think about each step of walking — the motor
program runs on its own. Compiled routines now support composition
(sub-routine calls via a call stack, max depth 16) and branching
(each step has `on_success`/`on_failure` paths) — mirroring how
biological motor programs nest sub-programs (reaching includes
grip adjustment) and branch on sensory feedback (surface is hot,
withdraw hand).

## What the brain does vs what the body does

In biology, the brain doesn't move muscles. It doesn't digest food.
It doesn't pump blood. It provides intent and judgment. The body
handles execution and reports back through the nervous system.

In SOMA, the LLM brain doesn't execute SQL. It doesn't send emails.
It doesn't hash passwords. It provides intent ("create an invoice
for Acme") and judgment ("this looks wrong, try a different approach").
The runtime handles execution through ports and reports back through
observations (PortCallRecord).

This separation is load-bearing in biology and in SOMA:

**The body persists. The brain is ephemeral.** A human body maintains
its reflexes, muscle memory, and immune system across sleep cycles,
anesthesia, and even altered states of consciousness. The body doesn't
lose its capabilities when the brain is offline. Similarly, SOMA's
routines, belief state, and episode history persist across LLM sessions.
Switch models, restart conversations, change providers — the body
remembers everything. The brain is disposable.

**The body is deterministic. The brain is probabilistic.** Muscle
contractions follow physics. Enzyme reactions follow chemistry.
The body doesn't hallucinate. Similarly, a compiled routine executes
a fixed skill path — readdir then stat then readfile, every time.
No variance, no hallucination. The LLM brain introduces creativity
and judgment, but also unreliability. The architecture confines
unreliability to the decision layer and keeps execution deterministic.

**The body gets faster. The brain does less.** A child learning to
walk requires intense prefrontal engagement — every step is
deliberate. An adult walks without thinking. The basal ganglia
took over. Similarly, the first time SOMA handles "create an
invoice," the LLM brain reasons through every step. After the
routine compiles, the brain just says "execute routine
create-invoice" and the body runs the compiled plan. The brain's
workload decreases as the body learns.

## The control loop as a sense-act cycle

Every nervous system operates on a sense-model-decide-act loop.
Sensory input arrives, the brain updates its model of the world,
decides on an action, and sends motor commands. The result generates
new sensory input, and the loop continues.

SOMA's 16-step control loop is this cycle:

1. **Sense** — check budgets, retrieve relevant episodes and
   schemas, query belief state. This is perception: what does the
   body currently know about the world?

2. **Model** — update the belief state from prior observations.
   The belief state is SOMA's world model — facts with confidence
   scores and provenance. Like the brain's internal model of body
   position (proprioception) and environment state.

3. **Decide** — score skill candidates on five dimensions (success
   probability, cost, latency, risk, information gain). The selector
   is a simplified prefrontal cortex — weighing options against
   competing objectives under uncertainty. When confidence is too
   low, the brain fallback activates — like asking someone for help
   when you don't know what to do.

4. **Act** — invoke the selected skill through a port. This is
   motor output. The port call goes to the external system (database,
   email server, API) and returns a typed observation (PortCallRecord).

5. **Evaluate** — the critic assesses the observation. Did we make
   progress? Are we stuck in a loop? Should we try something else?
   The critic detects three pathologies: loops (same belief hash
   repeating), dead ends (consecutive failures), and stalls (no
   progress). This mirrors how the brain detects when a motor plan
   isn't working — you stumble, you feel resistance, you notice
   you're not getting closer to the goal.

6. **Learn** — the full trace becomes an episode. Over time,
   episodes consolidate into schemas and routines. The body got
   better at this task.

## Prediction-error filtering

The brain doesn't encode everything. It encodes what's surprising.

Karl Friston's predictive coding framework says: the brain generates
predictions about incoming sensory data. When the prediction matches
reality, nothing happens — the signal is suppressed. Only prediction
errors (the mismatch between expected and actual) propagate upward
and get processed.

This is biologically efficient. You don't form a new memory every
time you open a door, because door-opening matches your prediction.
You DO form a memory when the door is locked and you expected it to
be open. The surprise is what matters.

SOMA implements this: when a routine fires and the session completes
successfully following the expected plan, no new episode is created.
The routine already captures this behavior — re-recording it is
noise. Only when something unexpected happens — the routine fails,
the critic triggers a revision, the observation deviates from the
expected effect — does a new episode get stored.

This keeps the episode store focused on what matters: novel
situations, failures to learn from, deviations that might indicate
a changing environment. The uninteresting confirmations are
suppressed, just as the brain suppresses expected sensory input.

## Salience: what's worth remembering

Not all experiences are equal. The amygdala tags experiences by
emotional significance — threat, reward, novelty. Tagged experiences
get preferential encoding and consolidation. You remember your first
day at a job better than your hundredth, not because day 100 was
less complex, but because day 1 had higher salience.

SOMA assigns a salience score to each episode based on:

- **Outcome quality** — a successfully completed goal is more
  valuable than one that timed out or was aborted. The organism
  should learn more from its successes.
- **Efficiency** — achieving a goal with less resource expenditure
  suggests a better strategy. The efficient path is worth
  reinforcing.

During schema induction, PrefixSpan weights episodes by salience.
High-salience episodes (successful, efficient) contribute more to
pattern discovery than low-salience ones. The organism learns
preferentially from its best experiences — like how athletes
mentally replay their best performances, not their worst ones.

## Sleep: offline consolidation

During slow-wave sleep, the hippocampus replays recent experiences
to the neocortex in compressed, temporally accelerated form. This
replay is selective — reward-tagged and emotionally salient
episodes get preferential replay (Bendor & Wilson, 2012). The
result is gist extraction: the neocortex stores the structural
pattern, the hippocampus releases the specific details.

SOMA's background consolidation thread is the sleep analogue. Every
five minutes (configurable), the system runs a full consolidation
cycle:

1. Replay all stored episodes
2. Group by similarity (embedding clustering)
3. Extract frequent skill subsequences (PrefixSpan, weighted by
   salience)
4. Induce schemas from recurring patterns
5. Compile routines from high-confidence schemas
6. Evict episodes that are well-captured by the new routines

Like biological sleep, this happens in the background without
interrupting the organism's ongoing activity. The MCP server
continues handling requests while the consolidation thread processes
memories. And like biological sleep, the result is generalization —
specific episodes become abstract patterns, detail is traded for
structure.

## Chunking: how experts beat working memory

George Miller (1956) showed that working memory holds 7 plus or
minus 2 items. Herbert Simon and William Chase (1973) showed that
chess grandmasters don't have bigger working memory — they have
bigger chunks. Where a novice sees 20 individual pieces, a
grandmaster sees "Sicilian Defense, Najdorf variation" — one chunk
that encodes a complex pattern.

Ericsson and Kintsch (1995) took this further with Long-Term Working
Memory theory: experts bypass the 7-item limit by holding pointers
into long-term memory, not the data itself. Working memory becomes
an index. The effective capacity is enormous because each pointer
retrieves a rich compiled structure.

SOMA applies this directly. The LLM brain has a limited context
window — the equivalent of working memory. Instead of filling it
with raw conversation history (the novice approach), SOMA fills it
with pointers:

- **Routine summaries** — "you have 5 compiled routines: create-
  invoice (4 steps), send-reminder (2 steps)..." Each routine is
  a chunk — one pointer that expands to a multi-step procedure.
- **Schema cache** — "you've worked with these tables: invoices
  (id, customer, amount, status)..." Instead of re-discovering
  the database schema every conversation, the cached structure is
  a chunk.
- **Runtime briefing** — "3 active schedules, last operation was
  a successful postgres query." Compressed state, not raw history.

Ten messages of conversational context plus these structured chunks
gives the brain more effective capacity than fifty messages of raw
history. The brain holds the index. The body holds the content.

## The brain fallback: asking for help

When a toddler encounters something genuinely novel — an unfamiliar
object, an unexpected sound — they look to a caregiver. The child's
own pattern matching produces low confidence, so they delegate the
decision to someone more knowledgeable. This is social referencing,
well-documented in developmental psychology.

SOMA's autonomous selector works the same way. When the predictor's
best candidate scores below a confidence threshold, the system can
delegate to an external brain — an LLM that has broader knowledge
than the runtime's pattern-matching heuristics. The brain picks
from the same candidates (it can't invent skills that don't exist,
just as a caregiver can't give the child abilities it doesn't
have), but it brings judgment about which option is most likely to
succeed.

The brain's choice becomes an episode. If it succeeds, the episode
feeds the learning pipeline. Eventually, the pattern becomes a
routine and the brain is no longer consulted for that class of
situations. The toddler grew up.

## Ports as the peripheral nervous system

The body interacts with the world through specialized organs.
The eyes detect light. The ears detect vibration. The hands
manipulate objects. Each organ has its own interface to reality,
but they all report to the same central nervous system in a
common format (neural impulses).

SOMA's ports are the peripheral nervous system. Each port connects
to a different external system — postgres speaks SQL, smtp speaks
email protocols, the filesystem speaks POSIX, HTTP speaks REST.
But they all report to the runtime in a common format:
PortCallRecord. The runtime doesn't know or care whether the
observation came from a database query or a sensor reading. It
processes the observation, updates belief state, and decides the
next action.

Adding a new port is like developing a new sense organ. The
central processing doesn't change. The body just gains a new
way to perceive or act on the world. A port for Stripe payments,
a port for a camera, a port for a temperature sensor — the
runtime handles them all identically because the interface is
uniform.

Auto-discovery (--pack auto) takes this further: the body wakes
up and senses what organs it has. It scans for available ports,
loads them, asks each one what it can do. No manifest, no
configuration. The body discovers its own capabilities — like
how a newborn gradually discovers what its limbs can do through
spontaneous movement and proprioceptive feedback.

## The scheduler as circadian rhythm

Biological organisms don't only react. They anticipate. Circadian
rhythms trigger hormone release, temperature regulation, and
metabolic changes on a schedule — not in response to stimuli, but
in anticipation of them. The body prepares for dawn before dawn
arrives.

SOMA's scheduler is this anticipatory system. It fires port calls
on a schedule — check this API every hour, send this reminder
every morning, poll this feed every five minutes. The scheduler
operates without the brain. It's a body-level rhythm, not a
conscious decision.

When the brain is involved (brain-routed schedules), the result
goes through interpretation before reaching the operator. This
mirrors how circadian signals reach consciousness: you don't
notice your cortisol rising at 6am, but you notice that you're
awake and alert. The body's rhythm produces the raw signal; the
brain interprets its significance.

## Belief state as the internal model

Every organism maintains an internal model of its environment.
This model includes: where am I (spatial awareness), what resources
are available (energy, materials), what's true about the world
(learned facts), and what's uncertain (gaps in knowledge).

SOMA's belief state is this internal model:

- **Resources** — typed entries with identity and version.
  What the body currently has access to.
- **Facts** — subject-predicate-value triples with confidence
  and provenance (Observed, Inferred, Asserted, Stale). What
  the body believes to be true about the world, and how sure it
  is.
- **Uncertainties** — explicit declarations of what is unknown.
  The body knows what it doesn't know.
- **Active bindings** — current variable values for the task at
  hand. The working context.
- **World hash** — a fingerprint of the entire belief state.
  Used to detect when the body is stuck in a loop (same world
  hash repeating means nothing is changing).

The belief state updates after every observation. A port call
returns a result; the belief state patches accordingly. This is
perception updating the world model — exactly what the brain does
after every sensory input.

## The policy engine as the immune system

The immune system protects the organism from harmful agents without
requiring conscious intervention. It operates on rules: recognize
self from non-self, escalate threats, remember past infections.
It gates what enters the body and what the body does to itself.

SOMA's policy engine serves the same function. Seven lifecycle
hooks gate actions at critical points: before candidate selection,
before input binding, before execution, before side effects,
before delegation, before rollback, before remote exposure. At
each hook, the policy engine evaluates the action against safety
rules:

- Destructive operations require confirmation (like how the immune
  system escalates novel threats to the adaptive immune response)
- Read-only operations are always allowed (like how the body
  freely observes its environment without restriction)
- Budget exhaustion terminates the session (like how the body
  stops activity when energy is depleted)

The policy engine doesn't make decisions. It constrains them. The
brain can want to do anything; the body's safety system determines
what's actually permitted. This separation prevents a hallucinating
brain from causing irreversible damage — the body's immune system
catches the pathogen before it reaches the organs.

## What this means

SOMA is not a metaphorical nervous system. It is a literal
implementation of the computational principles that neuroscience
has identified in biological nervous systems:

- Complementary learning systems (fast episodic + slow statistical)
- Predictive coding (encode only surprises)
- Salience-weighted consolidation (learn more from what matters)
- Chunking and long-term working memory (pointers, not data)
- Habit formation (compiled routines bypass deliberation)
- Sleep consolidation (offline batch processing)
- Peripheral nervous system uniformity (common observation format)
- Immune-like safety gating (policy hooks)

The architecture didn't start from neuroscience and work backward
to software. It started from the problem — how to build a runtime
that learns from execution — and converged on the same solutions
that evolution found. The convergence is evidence that these
patterns are not arbitrary. They are structural answers to the
problem of bounded agents operating in complex environments under
uncertainty.

## References

Anderson, J.R. (1982). Acquisition of Cognitive Skill. *Psychological
Review*, 89(4), 369-406.

Bendor, D. & Wilson, M.A. (2012). Biasing the content of hippocampal
replay during sleep. *Nature Neuroscience*, 15(10), 1439-1444.

Chase, W.G. & Simon, H.A. (1973). Perception in chess. *Cognitive
Psychology*, 4(1), 55-81.

Ericsson, K.A. & Kintsch, W. (1995). Long-term working memory.
*Psychological Review*, 102(2), 211-245.

Friston, K. (2006). A free energy principle for the brain. *Journal
of Physiology-Paris*, 100(1-3), 70-87.

Laird, J.E., Newell, A. & Rosenbloom, P.S. (1987). SOAR: An
Architecture for General Intelligence. *Artificial Intelligence*,
33(1), 1-64.

McClelland, J.L., McNaughton, B.L. & O'Reilly, R.C. (1995). Why
There Are Complementary Learning Systems in the Hippocampus and
Neocortex. *Psychological Review*, 102(3), 419-457.

Miller, G.A. (1956). The Magical Number Seven, Plus or Minus Two.
*Psychological Review*, 63(2), 81-97.

Tse, D. et al. (2007). Schemas and memory consolidation. *Science*,
316(5821), 76-82.
