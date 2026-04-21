# SOMA as Context OS: Proof via soma-project-code

## The Problem

LLMs hit a wall when coding at project scale. A model that writes a perfect Express route in isolation will, across a 10-file project, forget its own database API, import nonexistent packages, use wrong relative paths, and produce files that don't compose. The failure mode isn't intelligence — it's context. As the project grows, coherence degrades until the model contradicts itself.

This is the 2026 LLM scaling problem: capability is sufficient, but context management is not. Bigger context windows don't fix it — they delay the cliff while adding cost and latency.

## What SOMA Solves

SOMA turns the runtime into an external context manager. Instead of stuffing everything into one prompt, the body decomposes work into bounded steps where each step gets exactly the context it needs:

- **Bounded reasoning per step.** Each file generation call receives the full content of previously written files, the project layout, and the step's purpose. The model never needs to hold the whole project in working memory.
- **Structured state feedback.** Every port invocation returns a typed `PortCallRecord` — success/failure, exit codes, stderr. The executor sees real results, not hallucinated ones.
- **Automatic coherence repair.** The executor auto-creates parent directories, reconciles missing dependencies by scanning `require()` calls against `package.json`, and surfaces actual error output when steps fail.

The brain (LLM) decides what to do. The body (SOMA) tracks what happened, feeds it back, and keeps the world consistent.

## What Was Proved

`soma-project-code` generates a complete Node.js Express API with user CRUD and SQLite, driven entirely by Haiku through SOMA ports. To verify: `cd soma-project-code && node plan.js "your goal" && node execute.js`.

The pipeline:

1. **plan.js** — Haiku decomposes the goal into ordered port invocations (mkdir, writefile, npm_install, npm_test, git init/add/commit). No file content at this stage.
2. **execute.js** — Walks the plan. For each `writefile` step, calls Haiku with full content of all previously written files as context. For runner/git steps, invokes ports directly and checks real exit codes.

The generated project includes database initialization, a model layer, route handlers, a controller, middleware, tests, and a git repository. All CRUD operations work end-to-end. Tests run and pass.

## Why This Matters

The interesting result is not that Haiku can write an Express API — it can do that in a single prompt. The result is that **the same small model scales to arbitrary project size without coherence loss**, because SOMA externalizes what the model can't hold:

- File content accumulates in the executor, not the prompt history.
- Directory structure is tracked by the filesystem port, not the model's memory.
- Dependency consistency is enforced by the reconciliation step, not by hoping the model remembers.
- Failure detection uses real exit codes from real processes, not the model's confidence.

A small model with structured state feedback matches the output quality of a large model with a massive context window — at a fraction of the cost, with deterministic coherence guarantees that no context window can provide.

## The Architecture Constraint

This works because SOMA enforces the body/brain split. The runtime never contains domain logic — it doesn't know Express, SQLite, or npm. It knows ports, capabilities, observations, and skills. The ports (`filesystem`, `runner`, `git`, `search`, `patch`) provide typed interfaces to external systems. The brain provides intent. The body provides proprioception.

A hand doesn't decide where to reach. It reports what it's touching so the brain can adjust. SOMA is the hand.
