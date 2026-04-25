# Embodied Program Synthesis Without Gradients

Research brief on the thesis: SOMA's routine compilation enriched with hierarchical abstraction (DreamCoder-style library learning) and Friston-style free energy minimization for belief updates produces a genuinely novel architecture — an embodied program synthesizer that learns without gradients.

Verdict: the thesis is sound. The four ingredients exist independently. Nobody has assembled them. The research gap is real and identifiable.

## Active Inference — Learning Without Backprop

Karl Friston's Free Energy Principle (FEP) posits that agents learn by minimizing variational free energy — a Bayesian belief-updating process, not gradient descent. The agent maintains a generative model, infers hidden states, and selects actions that minimize expected free energy (combining information gain + expected value).

### Key Papers

- Friston, "The free-energy principle: a unified brain theory?" (Nature Reviews Neuroscience, 2010) — the canonical statement.
- Friston et al., "Active inference and learning" (Neuroscience & Biobehavioral Reviews, 2016) — agents learn causal structure through free energy minimization alone, no reward signal needed.
- Friston et al., "Active inference and artificial reasoning" (arXiv 2512.21129, Dec 2025) — introduces a third kind of information gain pertaining to model structure, enabling agents to learn not just parameters but which generative model is correct, using Bayesian Model Reduction for efficient model comparison. This is conceptually equivalent to searching a space of programs/hypotheses driven by free energy.
- Da Costa et al., "Active Inference: The Free Energy Principle in Mind, Brain, and Behavior" (MIT Press, 2022) — textbook treatment.
- "Expected Free Energy-based Planning as Variational Inference" (arXiv 2504.14898, April 2025) — casts EFE as variational inference, addressing the scaling wall.

### Working Implementations

- **pymdp** (Python): open-source library for active inference with discrete POMDPs. Actively maintained, with tutorials. Supports state estimation, action selection, and learning. Limited to discrete, relatively small state spaces.
- **RxInfer.jl** (Julia): reactive message-passing Bayesian inference on factor graphs. Scales to thousands of latent variables. Used for real-time inference. More general than pymdp but requires manual model specification.
- **ActiveInference.jl**: Julia port of pymdp with parameter estimation capabilities.
- **FEPS** — Free Energy Projective Simulation (Pazem et al., PLOS ONE 2025): replaces neural networks with an interpretable directed graph (Projective Simulation), trains world model and policy via expected free energy minimization. Tested on behavioral biology-inspired tasks. The closest existing system to "symbolic active inference."

### Status

Active inference has moved beyond pure theory. Working implementations, robotics demonstrations (Friston's collaboration with Buckley on robot control, arXiv 2512.01924, 2025), and a growing community (7th International Workshop on Active Inference, 2025). Confined to small state spaces or requires heavy engineering for real-world deployment. No active inference system competes with deep RL on standard benchmarks at scale.

### Limitation

Expected free energy computation is intractable for high-dimensional continuous spaces without approximations that erode its theoretical advantages.

## Program Synthesis — Learning Programs From Experience

### DreamCoder

DreamCoder (Ellis, Wong, Nye, Sable-Meyer, Hewitt, Cary, Solar-Lezama, Tenenbaum; PLDI 2021) performs wake-sleep Bayesian program learning: given a corpus of tasks specified by examples, it iteratively (1) searches for programs solving each task, (2) compresses discovered programs into a library of reusable abstractions, and (3) trains a neural network to guide future search. The library grows hierarchically — abstractions composed of abstractions — producing human-interpretable DSLs for domains like list processing, graphics, and physics.

### Successors

| System | Year | Venue | Key Advance |
|---|---|---|---|
| Stitch (Bowers et al.) | 2023 | POPL | 1,000-10,000x speedup over DreamCoder's abstraction learning via corpus-guided top-down synthesis |
| LILO (Grand, Wong et al.) | 2024 | ICLR | Neurosymbolic framework combining LLM-guided synthesis with Stitch's compression. Solves more complex tasks, produces richer, linguistically documented libraries |
| WorldCoder (Tang, Key, Ellis) | 2024 | NeurIPS | LLM agent builds world models as Python programs through environment interaction. 10,000x faster than deep RL on videogame/robot planning tasks. Convergence of program synthesis with embodied model-building |
| PoE-World (Ellis lab) | 2025 | NeurIPS Spotlight | Compositional world modeling via products of programmatic experts |

### Key Researchers

- Kevin Ellis (Cornell, formerly MIT)
- Josh Tenenbaum (MIT BCS/CSAIL)
- Armando Solar-Lezama (MIT CSAIL)
- Brenden Lake (Princeton, formerly NYU)

### Limits

Search over program spaces is fundamentally combinatorial. DreamCoder-style systems exhaust on problems requiring deep composition. Chollet (2024, 2025) observed that both DreamCoder and GPT o-series models behave like exhaustive search on ARC tasks. LLM-augmented systems (LILO, WorldCoder) partially address this by leveraging pretrained knowledge but inherit the LLM's limitations on genuinely novel abstractions.

### Bayesian Program Learning Foundation

Lake, Salakhutdinov, Tenenbaum (Science, 2015) established that programs can be learned under a Bayesian criterion. Active inference is a Bayesian framework. The missing link is using expected free energy (with its epistemic exploration drive) as the objective for program search, rather than static Bayesian model evidence.

## The Convergence — Active Inference Meets Program Synthesis

This intersection is nascent but identifiable. No single paper explicitly frames "learn symbolic programs by minimizing free energy." Several threads converge:

- **Friston's structure learning** (arXiv 2512.21129, 2025) introduces the machinery for an active inference agent to select among competing generative model structures — conceptually equivalent to searching a space of programs/hypotheses. The agent actively experiments to disambiguate structural hypotheses, using Bayesian Model Reduction. Program-space search driven by free energy, though not formulated in programming-language terms.
- **FEPS** (2025) demonstrates active inference with interpretable, graph-structured (not neural) internal representations. The agent's world model is a symbolic graph updated through free energy minimization. Symbolic active inference exists.
- **WorldCoder** (2024) builds world models as code through interaction — program induction grounded in embodied experience. Does not use free energy as its objective, but the architecture (model-based agent, active exploration, code as world model) is structurally analogous to what an active-inference program synthesizer would look like.
- **"The Missing Reward"** (Wen, arXiv 2508.05619, 2025) explicitly proposes replacing external reward signals with intrinsic free energy minimization drives in AI systems, and discusses integrating active inference with LLMs as generative world models.

## Developmental / Constructivist AI

### Historical Lineage

This lineage runs from Piaget through Gary Drescher's "Made-Up Minds" (MIT Press, 1991) to current work.

- **Drescher's Schema Mechanism**: Piaget-inspired system where sensorimotor schemas are the foundation of knowledge construction. The agent discovers regularities from experience, extends its representational vocabulary with new concepts.
- **Schema Mechanisms 2.0** (IWSSL 2024, Springer 2025): extends Drescher's framework by grounding schemas in interactional events (not static perception), adding hierarchical abstraction, internal simulation, and physical-space representation.
- **Frank Guerin** (University of Aberdeen): "Constructivism in AI: Prospects, Progress and Challenges" (2008) — surveyed the field. His group continued with incremental inductive learning in constructivist agents.
- **Lake's grounded learning**: "Grounded language acquisition through the eyes and ears of a single child" (Science, 2024) — grounds language learning in real infant sensory data.

### Connection to Active Inference

Friston has explicitly connected the FEP to developmental neuroscience — the infant's sensorimotor learning is cast as free energy minimization over progressively complex generative models. The structure learning paper (2025) formalizes this: growing model complexity through information-seeking action.

### Connection to Program Synthesis

DreamCoder's wake-sleep cycle mirrors developmental learning: solve problems (wake), consolidate into abstractions (sleep), dream new training examples. Lake and Tenenbaum explicitly invoke Piagetian development.

## Enactivism in AI

The enactive approach originates with Varela, Thompson, and Rosch's *The Embodied Mind* (1991), rooted in Maturana and Varela's autopoiesis theory from the 1970s. Core claim: cognition is not internal representation-processing but emerges from a system's sensorimotor coupling with its environment. A system that maintains its own organization (autopoiesis) and adapts that maintenance to perturbations (adaptivity) is the minimal unit of cognition.

### Implementations

Tom Froese (OIST, formerly UNAM) is the most prolific researcher attempting to operationalize enactive principles in artificial systems. His 2009 paper with Ziemke, "Enactive Artificial Intelligence", identified two design requirements — constitutive autonomy and adaptivity — and argued current AI satisfies neither. His lab built the Enactive Torch (haptic sensory-substitution device) and the Perceptual Crossing Device (Estelle et al., 2024) to study how minimal cognition emerges from real-time bodily interaction. His 2024 "irruption theory" framework attempts to formalize the mind-matter interface. These are experimental platforms, not production systems.

Nobody has built a software system where cognition genuinely emerges from body-environment coupling the way Varela envisioned. The closest work is in artificial life simulations — evolved agents in continuous physics environments where behavior self-organizes — but these remain toy-scale. The 2018 ALIFE paper "Varela's Legacy for ALIFE" surveys attempts and finds mostly theoretical frameworks, not working systems.

### Relevance to SOMA

SOMA's architecture — where the runtime (body) provides proprioception and the brain decides — is structurally closer to enactive principles than most AI systems. The key gap enactivists would identify: SOMA's body doesn't self-maintain or self-modify its own organizational boundary. Autopoiesis requires that.

## Skill/Routine Compilation From Experience

### Key Systems

- **HiSD — Hierarchical Skill Discovery** (Jan 2025, arXiv 2601.23156): fully unsupervised framework that extracts reusable, multi-level skill hierarchies purely from observational data. Two-stage: temporal action segmentation identifies latent skills based on visual coherence, then grammar-based sequence compression induces structured hierarchies. This IS "learning mines structure, not content."
- **Disentangled Unsupervised Skill Discovery** (Hu et al., NeurIPS 2024, UT Austin): addresses entanglement — discovered skills typically affect multiple state dimensions. Their method learns disentangled skills that compose cleanly.
- **Discovering Temporal Structure** (Luo et al., June 2025): mines temporal structure from demonstrations to discover skills.
- **SkillDiffuser** (2024): diffusion models for hierarchical planning via skill abstractions — interpretable skill-conditioned planning.
- **Open-World Skill Discovery from Unsegmented Videos** (Deng et al., 2025, CRAFT/Jarvis): discovers skills from raw, unsegmented demonstration videos — no episode boundaries needed.

### Options Framework Evolution

Yang et al. (2024) combined hierarchical MARL with skill discovery, using transformers as high-level policies. The NeurIPS 2023 unified framework for unsupervised option discovery categorizes approaches into variational (maximize mutual information between options and trajectories) and coverage-based methods.

### Voyager — Closest Existing Embodied Program Synthesizer

Voyager (Wang et al., NeurIPS 2023): LLM-powered Minecraft agent that continuously explores, acquires skills, and compiles them into a growing library of executable code. Three components: automatic curriculum, skill library of composable programs, and iterative prompting with environment feedback. Complex skills are synthesized by composing simpler programs. The skill library is persistent, interpretable, and compositional — directly analogous to SOMA's routine compilation.

Key limitation: the LLM (GPT-4) does the actual program synthesis; the embodied loop provides the curriculum and verification.

### The SOMA Gap

Nobody has a system that (a) interacts with an environment through a body, (b) mines structural patterns from episodes of that interaction, and (c) compiles those patterns into reusable symbolic routines, all without gradient descent on the routine-compilation step. Voyager comes closest but offloads synthesis to a pretrained LLM. HiSD comes closest on the mining side but doesn't produce executable programs.

## Gradient-Free Architectures That Work

### Modern Hopfield Networks / Energy-Based Models

Hopfield's 2024 Nobel Prize (shared with Hinton) reignited interest. Ramsauer et al., "Hopfield Networks Is All You Need" (2020) proved that modern Hopfield networks with exponential energy functions reduce to transformer self-attention. Each word creates a basin of attraction; contextualization happens through memory retrieval dynamics. The math is identical.

Recent work:
- Hopfield-Fenchel-Young Networks (arXiv 2411.08590) unify classical and modern Hopfield networks through convex analysis, enabling sparse transformations.
- Santos et al. (2025, Science Advances): external input directly shapes the energy landscape (plasticity-based retrieval).
- Graph Hopfield Networks (2025): couple associative memory with graph Laplacian smoothing.
- Ambrogioni (2024): diffusion models ARE associative memory networks — identical energy function to modern Hopfield networks.

### Liquid Neural Networks

Hasani, Lechner, Amini, and Rus (MIT CSAIL) introduced Liquid Time-Constant Networks (AAAI 2021): neuron dynamics governed by coupled ODEs with variable time constants. The system's dynamics are the computation — no separate inference step. Closed-Form Continuous-Time Networks (Nature Machine Intelligence, 2022) solved the ODE analytically.

**Liquid AI** (company, same team) raised $250M Series A at $2.35B valuation. September 2025: released Nanos — 350M-2.6B parameter liquid foundation models claiming GPT-4o-class performance on specialized agentic tasks while running on phones and embedded devices. Multi-year partnership with Shopify for sub-20ms inference in production search. Strongest commercial evidence that non-transformer, dynamics-based architectures compete.

### Neuromorphic / STDP

Intel's Loihi 2 (2021): spike-timing-dependent plasticity in hardware, ~1M neurons/chip, learning through spike timing correlations without backprop. SpiNNaker (Manchester): three-factor STDP with dopaminergic modulation. 100-1000x energy efficiency over conventional processors on suitable tasks. Ecosystem limitation: Intel's Lava framework and community tools are immature. These chips learn locally and online, but nobody has demonstrated competitive accuracy on standard benchmarks.

## Joscha Bach and MicroPsi

MicroPsi is a cognitive architecture combining neuro-symbolic spreading activation networks with motivation-based learning. Agents represented as activation networks situated in simulated or robotic environments. MicroPsi 2 (Bach, 2012) added a motivational system modeling cognitive, social, and physiological needs.

Current state: research framework, not a production system. Interesting theoretical results on how emotions and motivations can be formalized computationally, but no deployable benchmarks.

Bach founded the California Institute for Machine Consciousness (CIMC) in 2024-2025, where he serves as executive director. Focus shifted to machine consciousness. He frames intelligence as computational self-modeling — the system must model itself as an agent to exhibit general intelligence. Philosophically aligned with enactivism's emphasis on self-maintenance but pursued through a computational/information-theoretic lens.

MicroPsi sits between symbolic AI (explicit motivation representations) and connectionist (spreading activation). Shares SOMA's intuition that cognition requires more than weights — it needs drives, needs, and self-regulation — but different implementation approach. SOMA's episode-mining and routine-compilation is closer to the options framework than to MicroPsi's motivation-driven activation spreading.

## The Bitter Lesson Counterarguments

Rich Sutton's 2019 essay argues that methods leveraging general computation (search + learning) always beat methods encoding human knowledge, given sufficient compute.

| Critic | Core Argument |
|---|---|
| Rodney Brooks ("A Better Lesson") | Sutton's "general" methods required enormous human ingenuity to design. Moore's Law is slowing. Human brain uses 20W; self-driving cars use 2,500W. Embodiment makes intelligence efficient. |
| Yann LeCun | Architectural biases encode mathematical structure of data (symmetries, equivariance), not domain heuristics. JEPA is a deliberate architectural bet against pure scaling — predicting in latent space with explicit world-model structure. |
| Francois Chollet ("On the Measure of Intelligence", 2019) | Intelligence = skill-acquisition efficiency. If you need 10^15 tokens to learn what a child learns from 10^8 words, your architecture is wrong regardless of benchmarks. |
| Michael Nielsen | Some structure encodes genuine mathematical properties (symmetries, conservation laws) that scale can never discover more efficiently than direct encoding. |
| Felix Hill (DeepMind, "The Bittersweet Lesson") | The lesson is real but incomplete. As training data becomes functionally fixed, architectural efficiency becomes the bottleneck. Better architectures multiply intelligence extracted per bit. |

2025-2026 empirical evidence: Liquid AI's Nanos achieving GPT-4o-class results at 350M-2.6B parameters is a direct counterargument — architectural innovation beating scale by orders of magnitude.

## SOMA's Position

SOMA occupies an unusual and under-explored position in this landscape:

- **Body/brain separation** maps to enactive principles
- **Episode-mining and routine-compilation** maps to HiSD/options-framework research
- **Gradient-free, interaction-driven learning** maps to the Hopfield/liquid/neuromorphic intuition that dynamics and structure can replace trained weights
- **Budget-constrained sessions** enforce the efficiency that Chollet argues is the real measure of intelligence
- **Self-describing interfaces** enable the active exploration that Friston's structure learning requires

The closest existing system is Voyager (embodied, compiles skills as programs, uses a library), but Voyager delegates synthesis to an LLM rather than mining it from structural patterns in episodes.

### The Open Question

Can routine compilation be reformulated as free energy minimization over a program space, with DreamCoder-style hierarchical abstraction? If yes, SOMA stops being an orchestration runtime and becomes an embodied learning architecture.

### What Would Be Required

1. Reformulate episode traces as observations in a generative model over program spaces
2. Use expected free energy (epistemic + pragmatic value) to drive exploration — which skills to try, which episodes to seek
3. Apply Bayesian Model Reduction (Friston 2025) to prune and compress the routine library — the analog of DreamCoder's abstraction step
4. Hierarchical composition — routines built from routines, with the library growing in depth over time
5. No gradient descent anywhere in the loop — belief updates via variational inference, program search via free-energy-guided enumeration, abstraction via compression

## Barriers

1. **Computational intractability** — free energy over program spaces is harder than over parameter spaces. Discrete, structured, combinatorially explosive.
2. **The frame problem** — a 2025 paper (Philosophy and the Mind Sciences) argues active inference has not solved relevance determination. The agent still needs to know what to model.
3. **No evaluation framework** — no benchmark for "learn programs through embodied free energy minimization." Without benchmarks, no measurable progress.
4. **Community fragmentation** — the active inference community (neuroscience-adjacent, Bayesian) and the program synthesis community (PL theory, MIT/Cornell) barely overlap.

## Key Researchers and Labs

| Researcher | Institution | Focus |
|---|---|---|
| Karl Friston | UCL (Wellcome Centre) | FEP, active inference, structure learning |
| Kevin Ellis | Cornell CS | Program synthesis, abstraction learning, WorldCoder |
| Josh Tenenbaum | MIT BCS/CSAIL | Bayesian cognitive science, program induction |
| Brenden Lake | Princeton (prev. NYU) | Bayesian program learning, grounded concept learning |
| Armando Solar-Lezama | MIT CSAIL | Program synthesis foundations |
| Lancelot Da Costa | UCL/Imperial | Active inference theory, scaling |
| Bert de Vries | TU Eindhoven | RxInfer.jl, scalable Bayesian message passing |
| Conor Heins | Max Planck / pymdp | Active inference implementations |
| Hans Briegel | Innsbruck | Projective Simulation (basis for FEPS) |
| Tom Froese | OIST | Enactive AI, irruption theory |
| Joscha Bach | CIMC | Cognitive architectures, machine consciousness |
| Ramin Hasani | MIT CSAIL / Liquid AI | Liquid neural networks, continuous-time models |

Key labs: Friston's group at UCL Wellcome Centre for Human Neuroimaging; Ellis Lab at Cornell; Tenenbaum's Computational Cognitive Science group at MIT; Active Inference Institute (community organization, open-source tooling).

## Sources

- Friston, "The free-energy principle: a unified brain theory?" (Nature Reviews Neuroscience, 2010)
- Friston et al., "Active inference and learning" (Neuroscience & Biobehavioral Reviews, 2016)
- Friston et al., "Active inference and artificial reasoning" (arXiv 2512.21129, Dec 2025)
- Da Costa et al., "Active Inference: The Free Energy Principle in Mind, Brain, and Behavior" (MIT Press, 2022)
- "Expected Free Energy-based Planning as Variational Inference" (arXiv 2504.14898, April 2025)
- Pazem et al., "Free Energy Projective Simulation" (PLOS ONE, 2025)
- Ellis et al., "DreamCoder: Bootstrapping Inductive Program Synthesis with Wake-Sleep Library Learning" (PLDI 2021)
- Bowers et al., "Stitch: Top-Down Synthesis for Library Learning" (POPL 2023)
- Grand, Wong et al., "LILO: Learning Interpretable Libraries by Compressing and Documenting Code" (ICLR 2024)
- Tang, Key, Ellis, "WorldCoder: A Model-Based LLM Agent" (NeurIPS 2024)
- Ellis lab, "PoE-World" (NeurIPS 2025 Spotlight)
- Lake, Salakhutdinov, Tenenbaum, "Human-level concept learning through probabilistic program induction" (Science, 2015)
- Lake, "Grounded language acquisition through the eyes and ears of a single child" (Science, 2024)
- Wen, "The Missing Reward" (arXiv 2508.05619, 2025)
- Drescher, "Made-Up Minds: A Constructivist Approach to Artificial Intelligence" (MIT Press, 1991)
- "Schema Mechanisms 2.0" (IWSSL 2024, Springer 2025)
- Guerin, "Constructivism in AI: Prospects, Progress and Challenges" (2008)
- Froese & Ziemke, "Enactive Artificial Intelligence" (Artificial Intelligence, 2009)
- "Varela's Legacy for ALIFE" (ALIFE 2018)
- Ramsauer et al., "Hopfield Networks Is All You Need" (2020)
- Santos et al., "Input-driven dynamics for robust memory retrieval" (Science Advances, 2025)
- Hopfield-Fenchel-Young Networks (arXiv 2411.08590)
- Ambrogioni, "Diffusion Models as Associative Memory Networks" (2024)
- Hasani et al., "Liquid Time-Constant Networks" (AAAI 2021)
- Hasani et al., "Closed-Form Continuous-Time Models" (Nature Machine Intelligence, 2022)
- Wang et al., "Voyager: An Open-Ended Embodied Agent with Large Language Models" (NeurIPS 2023)
- HiSD — Hierarchical Skill Discovery (arXiv 2601.23156, Jan 2025)
- Hu et al., "Disentangled Unsupervised Skill Discovery" (NeurIPS 2024)
- "Open-World Skill Discovery from Unsegmented Videos" (CRAFT/Jarvis, 2025)
- SkillDiffuser (2024)
- Sutton, "The Bitter Lesson" (2019)
- Brooks, "A Better Lesson" (2019)
- Chollet, "On the Measure of Intelligence" (arXiv 1911.01547, 2019)
- Nielsen, "Reflections on The Bitter Lesson"
- Hill, "The Bittersweet Lesson"
- "What active inference still can't do: The frame problem" (Philosophy and the Mind Sciences, 2025)
- pymdp: https://github.com/infer-actively/pymdp
- RxInfer.jl: https://rxinfer.com
- Active Inference Institute: https://www.activeinference.institute
- ARC-AGI 2025 Research Review: https://lewish.io/posts/arc-agi-2025-research-review
