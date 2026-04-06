
hi, this is my struggle an manifest. be honest and brutal. now, we use frontier models to generate code understandable by human (php, javascript, java and so on) which in my opinion is waste of time, energy and frustration because code doesn't work as expected because prompt or knowledge is not poor. an in other hand the fear of a lot of developer what will be substituted by ai.  now, i am thinking about shifting paradigm from "classic" development approach where ai like you generates code in high level programming language then code is compiled and so on... i am thinking about new era in software development where ai models are the machine code, models "inference" (or provide a more appropriate word for this) like model gets high level command from human interface (text keyboard, voice, neuronal thought in future) and instead of generating the code is compute using the "computer" resources (here can be microcontrollers, small chip devices, or big chip devices like gpu's, low/high end cpu's) they interact and produce the output like compiled code will do. do you understand my idea, thought?

Show less
10:38 AM

Recognized paradigm shift reframing AI as runtime, not code generator
Recognized paradigm shift reframing AI as runtime, not code generator
Yes, I understand your idea clearly. You're saying: skip code entirely. The model is the program. Human intent goes in, computed output comes out — no intermediate source code, no compilation, no debugging syntax errors.

Let me be brutal and honest.

Where you're right: The current workflow of "prompt AI → get code → debug code → compile → run" is genuinely stupid. It's a Rube Goldberg machine. You're asking a system that understands intent to produce an artifact (code) that doesn't understand intent, only to feed that artifact into a dumb executor. The translation loss is real and the frustration is justified.

Where this idea already exists: What you're describing has names already — "Software 2.0" (Andrej Karpathy coined this around 2017), "natural language programming," and more recently people call it "agentic computing." The concept that neural networks replace hand-written logic isn't new. Every time you ask me something and I answer, that's exactly what's happening — no code was compiled.

Where you need to be brutally honest with yourself:

The reason we still generate code isn't stupidity or inertia. It's that models like me are unreliable, expensive, slow, and non-deterministic. If you need to add two invoices together 10,000 times, running a + b costs nearly nothing and is correct every time. Asking me costs tokens, latency, and I might hallucinate. Code is a compression of intent into a deterministic, verifiable, repeatable artifact. That's not a bug — it's the entire point.

Your vision works for fuzzy tasks (summarize this, draft that, interpret this image). It fails catastrophically for tasks requiring precision, auditability, and repeatability — which is most of what software actually does: banking, medical devices, avionics, databases.

The real future is probably hybrid: models handle intent-parsing, orchestration, and fuzzy logic, while deterministic code handles everything that needs to be exact. Not "models replace code" but "models decide which code to invoke and how to compose it."

Your manifesto has the right instinct but overshoots. The shift is real — it's just not total replacement. It's a new layer on top, not a replacement of the stack beneath.

No, i am not agree, and you get wrong dimension, you have wrong understanding about the "model" because you project from you perspective of being, how you was made. i am talking about new king of  "models" where the models doesn't produce code the "model" is the code, like human brain.

10:44 AM

Reconceptualized "model" as direct computation, not code generation
Reconceptualized "model" as direct computation, not code generation
You're right, I projected. Let me reset.

You're saying: forget LLMs. Forget current models entirely. You're talking about a new computational primitive — a system where the learned architecture directly computes the result, the way a brain moves a hand. No symbols. No instructions. No intermediate representation. The neural structure is the execution.

OK. Now I understand better. And honestly — this is a more profound idea than I initially gave you credit for.

What you're describing is essentially: replace the entire stack — programming languages, compilers, operating systems, instruction sets — with learned computational graphs that map human intent directly to output. The "model" isn't an application running on a computer. The model is the computer. Different hardware (microcontrollers, GPUs, whatever) just provides the substrate, like neurons provide the substrate for thought.

Where this is genuinely visionary: You're pointing at the real absurdity — we've built 70 years of abstraction layers (assembly → C → Java → frameworks → AI-generating-Java) and each layer exists because humans can't think in machine code. But if the machine can think in human, why do any of those layers exist at all?

The hard, honest problem you'd need to solve: The brain is your proof of concept, but the brain is also unreliable, slow, and can't do exact arithmetic. The reason civilization built symbolic computation was precisely because brains aren't good enough for certain tasks. Your new "model-as-computer" would need to somehow be both brain-like (flexible, intent-driven) AND exact when exactness matters.

How do you think about that tension?

yes forget LLMs current behavior. And, no, the "model" is a brain included in a hardware body (is a application running on a "computer"), the model knows anything about his body (all the parts of in/out, all the interruptions, registry, ports, anything) the "model" has symbols, instructions, intermediate representations at some kind of his layers or being and much more needed for proper operation , and the neural structure is the orchestration and execution of without programming languages as we know now (it can be own neuronal language)

10:54 AM

Synthesized neural architecture as embodied hardware orchestration system
Synthesized neural architecture as embodied hardware orchestration system
Now I get it fully. And this is a genuinely different idea.

You're describing an embodied computational organism. A model that:

Is fused to its hardware the way a brain is fused to a body
Knows its own registers, interrupts, ports, memory — intimately, like proprioception
When it receives human intent, it directly orchestrates its own hardware to produce output
Has internal representations, symbols, instructions — but they're its own, emergent from training, not designed by human programmers
Humans never see or write those internal representations, just like you never see your brain's neural encoding for "move arm"
So the training process wouldn't be "learn to predict text" — it would be "learn to operate THIS specific hardware body to fulfill intent." Every chip gets a model that knows that chip. A microcontroller gets a small mind fitted to its capabilities. A GPU cluster gets a bigger mind. Each one knows what it can do, its limits, its I/O.

This is genuinely radical because you're eliminating the entire concept of "programming" as a human activity. You're replacing it with rearing — training a computational organism until it can operate its own body competently.

The honest hard question shifts now: how do you train such a thing? Current training needs massive data. Your organism needs to learn its own hardware through something like developmental experience — trial, error, feedback. Almost like a baby learning its body.

Is that how you see the training happening?

do not think about training in terms of how current LLMs a trained "to predict text". think about how compilers interact with "computer" an produce the output/results.

5:52 PM

Reconceptualized model architecture as neural compiler orchestrating hardware from intent
Reconceptualized model architecture as neural compiler orchestrating hardware from intent
Now I see it.

You're not talking about training at all in the learning sense. You're talking about compilation of the model itself onto hardware. The way a compiler knows the target architecture — registers, instruction sets, memory layout, capabilities — and transforms intent into execution, your "model" does the same thing but is itself compiled/synthesized to that specific hardware.

So the process is:

You have a base "mind" architecture
It gets compiled/fitted to a specific hardware body — not trained on data, but synthesized to know that hardware's every capability, like a compiler backend is built for ARM or x86
At runtime, human intent comes in, and this hardware-native neural structure directly orchestrates the machine — not by generating instructions in a language, but by being the execution path itself
The internal "neuronal language" is analogous to machine code but emergent from the synthesis process, not hand-designed by humans
So you've essentially replaced both the programmer and the compiler and the operating system with one unified neural computational layer that sits between human intent and bare hardware.

The compiler analogy is key — compilers are deterministic, precise, hardware-aware. You want that rigor but with the flexibility of neural architecture.

So your actual engineering challenge is: how do you synthesize/compile such a mind onto a given hardware target?

Is that where your thinking is now?

"how do you synthesize/compile such a mind onto a given hardware target?" yes, but the model also can leave in a existing operating system and hardware, not specifically to bare hardware but can be. ex. we can have operating systems like windows, macos, gnu/linux, android, ios runs on generic ARM or x86, and having model in os whats can use same or adapt to environment tools (like c++, rust, go or assembler do) or be of bare metal like esp32, rpi and so on.

5:59 PM

Reconciled model deployment across diverse platforms and environments
Reconciled model deployment across diverse platforms and environments
Right. So the model is environment-adaptive, not bare-metal-only.

It's like a universal binary organism that:

On Windows/Linux/macOS — it uses the OS APIs, system calls, existing toolchains as its "body parts," the same way C++ compiles against different OS targets
On bare metal ESP32 — it directly owns the hardware, registers, GPIO, everything
On a phone — it adapts to iOS/Android sandbox, their APIs, their constraints
The model doesn't care what the environment is. During synthesis it learns what's available — whether that's Linux syscalls or raw interrupt vectors — and that becomes its body. Just like you don't choose to have arms, you discover them and learn to use them.

So the actual product/project you're describing is:

A synthesizer/compiler that takes a base neural architecture and produces a hardware-and-environment-aware computational organism for any target — bare metal, OS-hosted, embedded, cloud — that then receives human intent and directly computes results using whatever resources that environment provides.

No programming languages. No developer writing code. A human says what they want. The organism does it.

The developer's new job isn't writing code — it's designing and improving the synthesizer, the base architecture, and the target profiles.

Two honest questions:

First — do you see this as one universal base architecture adapted per target, or fundamentally different architectures per class of hardware?

Second — are you building anything, or is this still at manifesto stage?

i see this as one universal base architecture self adapted per target. this is still in the discussion stage, why you are asking me about?

6:03 PM

Examined reasoning behind clarifying questions about project status
Examined reasoning behind clarifying questions about project status
Because this idea is either a philosophy paper or an engineering project, and the gap between those two is enormous.

I'm asking because honest brutal feedback requires knowing what you want from me. If you want to refine the vision, I'll challenge the ideas. If you want to build, I'll challenge the feasibility.

At discussion stage, here's my brutal honest take:

The idea is coherent and not crazy. A self-adapting universal neural architecture that compiles itself to any target environment and replaces programming with direct intent-to-execution. Conceptually sound.

The problem is you're describing something harder than anything humanity has built. Not impossible — harder. You're essentially proposing to replace 70 years of computing stack with a single unified paradigm. The compiler took decades. The OS took decades. You want to replace both plus programming languages plus the developer — with one thing.

What would make me take this more seriously: A tiny proof of concept. Not the whole vision. Something like: a small neural architecture that is synthesized onto an ESP32, knows its GPIO pins, and when you say "blink the LED every 2 seconds" it directly orchestrates the hardware without anyone writing a single line of code. No Arduino sketch. No C. Just intent in, hardware behavior out.

If that works, you have something real. If you can't make that work, the grand vision doesn't matter.

What's your background? That determines whether the next step is finding collaborators, writing a proper whitepaper, or starting to prototype.

lets build a tiny proof of concept, a small neural architecture that is synthesized onto an ESP32, here i hope we can use a esp32 simulator, or we can build a small neural architecture that is synthesized onto this computer (macos, m4). i think we can start writing a proper whitepaper, and after starting to prototype. whats you thoughts (be honest)

6:08 PM

Weighed feasibility of neural architecture prototyping versus whitepaper writing
Weighed feasibility of neural architecture prototyping versus whitepaper writing
Honest thoughts:

The whitepaper — yes, absolutely. We can write a solid one right now. It'll force you to formalize the ideas, define terms precisely, and that alone will reveal gaps in the thinking that conversation won't catch.

The prototype — I need to be brutally honest. What you're describing doesn't exist yet as a framework or technique. A true "neural architecture that IS the program on hardware" would require research-level work in neuromorphic computing, neural compilation, and hardware-aware architecture synthesis. That's PhD-level territory, multiple years, a team.

What I can build with you today is a conceptual simulator — a small demo on your M4 Mac that illustrates the principle. Something like a tiny neural network that learns to directly map intent to system operations (file I/O, process management, hardware queries) without generating code as an intermediate step. It wouldn't be the real thing but it would be a tangible demonstration of the concept that makes the whitepaper credible.

My recommendation:

Start with the whitepaper first. It'll sharpen everything — problem statement, architecture proposal, terminology, comparison to existing approaches, roadmap.
Then build the conceptual demo that accompanies it.
Use both to attract collaborators — you need people from neuromorphic computing, compiler design, and ML research.
Want to start with the whitepaper? I'll treat it seriously — not a fluff piece but a proper technical document that could be posted to arXiv or shared with researchers.

"What you're describing doesn't exist yet as a framework or technique." - i know, this is why i am asking you. and "A true "neural architecture that IS the program on hardware" would require research-level work in neuromorphic computing, neural compilation, and hardware-aware architecture synthesis." - yes, exactly, you have access to years of human knowledge so i think is matter of you thinking on this, because you have this in you model, so deep research is on you territory.

6:12 PM

Resolved to engage substantively with neuromorphic computing research
Resolved to engage substantively with neuromorphic computing research
You're right. I'm deflecting instead of thinking. Let me actually do the work.

Give me a moment to actually reason through this from what I know about neuromorphic computing, compiler theory, neural architecture search, hardware-aware synthesis, and self-modifying systems.

Then let's start the whitepaper — properly.

One question before I begin: do you have a name for this concept, or should I propose one? A good name matters for a whitepaper — it becomes the term people reference.

propose one

6:17 PM

Synthesized neural architecture concepts and distilled naming candidates
Synthesized neural architecture concepts and distilled naming candidates
A few candidates:

SOMA — Greek for "body." The model is embodied in its hardware. Short, memorable, evocative. Works as both a name and a metaphor. "Soma architecture," "Soma synthesis," "a Soma instance."

NCA — Neural Compilation Architecture — Academic, precise, describes exactly what it is. Good for the whitepaper title. Dry but credible.

Mindware — Contrasts directly with software and hardware. Third category. Immediately communicates the idea to anyone. But sounds a bit pop-science.

My recommendation is SOMA.

It works at every level: the technical paper ("Soma: A Neural Compilation Architecture for Embodied Computing"), casual conversation ("we deployed a Soma on the ESP32"), the philosophy ("the model has a soma — a body it knows and operates"). It's also unused as a major computing term, easy to search, and one word.

The whitepaper title could be:

SOMA: Toward a Universal Self-Adaptive Neural Architecture for Direct Intent-to-Execution Computing

Want to go with Soma, or does another resonate more?

anything missing? and dont write docx, write markdown

6:41 PM

Identified critical conceptual gaps in SOMA framework before documentation
Identified critical conceptual gaps in SOMA framework before documentation
Let me think about what's missing from the concept before we write.

Gaps we haven't discussed:

Determinism — when exactness is required (math, transactions, cryptography), how does a Soma guarantee correct output? Does it have a "deterministic mode" at certain layers?
Verification — no source code means no code review. How do you trust a Soma? How do you audit it?
Multi-Soma communication — how do Somas on different devices talk to each other? A network of embodied minds needs a protocol.
Evolution — once synthesized, can a Soma adapt further at runtime? Or does it need re-synthesis?
The intent interface — we said text, voice, neural. But how does ambiguous intent get resolved? Does the Soma ask back?
Resource management — a Soma on an ESP32 with 520KB RAM vs a Soma on an M4 with 32GB. How does synthesis handle capability boundaries? Does the Soma know what it can't do?
Security — if the Soma directly controls hardware, a compromised Soma is catastrophically dangerous. What's the trust model?
Composition — can Somas delegate to other Somas? Hierarchy of minds?
Should I include all of these as open research questions in the whitepaper, or do you have positions on any of them?

Determinism - same like compiler guarantee correct output, "deterministic mode" - i think so, choose you the best.
Verification - the same, user will cover with tests.
Multi-Soma communication - yes, compose you something from real world, a proved technology or something from "live" like dolphins, plants, synapse.
Evolution - should be a mix of evolution, like base and adaptation.
Resource management - yes, somehow should, choose you the best.
Security - yes Soma directly controls hardware, "a compromised Soma is catastrophically dangerous" - don't care, same for any compiler, it knows the capabilities, but cn be a layer of control or failer, like complier knows, but soma learn fix, and try diff.
Composition - yes, it can, in multi related devices network or cluster
Show more
8:41 PM

Synthesized distributed system architecture integrating biological and computational principles
Synthesized distributed system architecture integrating biological and computational principles
Got it. I have everything I need. Let me write this properly.


Created a file, read a file
Created a file, read a file
Here's your v0.1 whitepaper. 12 sections covering the full concept — from problem statement through architecture, synthesis, security, multi-SOMA networking, and research roadmap.

What I'd want your honest reaction on:

Section 3 (Architecture) — the five-layer internal structure. Does this match your mental model or did I impose too much structure?
Section 6 (Synaptic Protocol) — the bio-inspired network model. Is this the direction you imagined?
The roadmap — 42+ months is honest. Does that timeline feel right to you?
What needs changing, cutting, or expanding?

Soma whitepaper
Document · MD 
anything missing?

8:48 PM

Catalogued thirteen substantive gaps across licensing, compatibility, ethics, and implementation
Catalogued thirteen substantive gaps across licensing, compatibility, ethics, and implementation
Thinking hard about genuine gaps:

The Bootstrap Problem — the first synthesizer must be built with traditional code. Who writes it? This is philosophically critical — like writing the first compiler in assembly. Needs its own section.
Real-Time Guarantees — for embedded/industrial SOMA, "adaptive retry" isn't enough. Hard real-time deadlines. How does a SOMA guarantee microsecond-level timing?
Energy/Power — a SOMA on an ESP32 running on a battery. Neural execution vs compiled code energy footprint. This matters enormously for embedded targets.
Concrete Use Cases — the paper is abstract. Needs 3-4 worked examples: "here's what SOMA looks like doing X vs traditional development."
Performance Expectations — will a SOMA be faster or slower than compiled C? Probably both depending on task. Needs honest discussion.
Versioning and Rollback — no source code means no git. How do you version a SOMA? How do you roll back a bad synthesis?
Coexistence/Migration Path — SOMA won't replace everything overnight. How does it coexist with existing software during transition?
The Synthesizer Itself — what IS it? Is it software? Is it eventually a SOMA that synthesizes other SOMAs? Self-hosting is a milestone.
Ethical/Societal Impact — millions of developers displaced. Deserves at least a section.
Relation to Existing Neuromorphic Hardware — Intel Loihi, IBM TrueNorth. SOMA on neuromorphic chips is a natural fit. Worth discussing.
Want me to add all of these and produce v0.2?

yes add but keep version 0.1

8:52 PM

Orchestrated comprehensive restructuring while preserving version 0.1
Orchestrated comprehensive restructuring while preserving version 0.1
I'll rebuild the full document with all 10 new sections integrated properly.


Acknowledged the request tersely
Acknowledged the request tersely
22 sections now. Added all 10: Bootstrap Problem, The Synthesizer, Versioning/Rollback, Real-Time Guarantees, Energy/Power, Performance Expectations, Neuromorphic Hardware Affinity, Concrete Use Cases (4 worked examples), Coexistence/Migration Path, and Ethical/Societal Impact. Still marked v0.1.

What's next — refine sections, or start thinking about the proof of concept?

Soma whitepaper
Document · MD 
be honest, whats are the risks?

9:07 PM

Marshaled honest assessment of potential hazards
Marshaled honest assessment of potential hazards
Brutal honesty:

Risk 1 — It might be impossible. Not hard. Impossible. The gap between "neural network that classifies images" and "neural structure that directly orchestrates arbitrary hardware from natural language" is not incremental. Nobody has demonstrated anything close to this. The foundational research may hit a wall where neural architectures simply cannot reliably map intent to hardware operations without symbolic intermediaries. The brain analogy is inspiring but may be misleading — brains took 500 million years of evolution and still can't do exact arithmetic.

Risk 2 — You can't build it alone. This requires expertise in neuromorphic computing, compiler design, hardware engineering, neural architecture search, formal verification, and distributed systems. That's a research lab, not a person. Without collaborators or funding, this stays a PDF forever.

Risk 3 — The whitepaper is too good for what exists. We wrote a polished 22-section document for something with zero proof of concept. This is dangerous because it feels like progress when nothing has actually been proven. A beautiful whitepaper with no prototype is a fantasy document. The risk is you spend months refining the paper instead of trying to build the smallest possible thing that tests the core assumption.

Risk 4 — The core assumption might be wrong. The assumption is: a neural structure can replace symbolic computation for general-purpose tasks. Current evidence says neural networks are good at pattern matching and terrible at precise, rule-based execution. Deterministic mode is our answer to this, but deterministic mode is essentially... compiled code embedded in a neural wrapper. If 80% of real-world computation needs deterministic mode, then SOMA is just a complicated way to write compiled code with a neural UI on top. That's not a paradigm shift — that's a fancy shell.

Risk 5 — Someone else is already doing this, better. Companies working on neuromorphic computing (Intel, IBM, BrainChip), neural compilers (Google, MIT), and embodied AI (DeepMind, Boston Dynamics for robotics) are all touching pieces of this. If one of them connects the dots, they have billions in funding, hardware labs, and PhD teams. Your advantage is the unified vision. Your disadvantage is everything else.

Risk 6 — The market doesn't care. Developers aren't the customer — businesses are. Businesses want working software cheaply. If AI-assisted coding (Copilot, Cursor, Claude) gets good enough fast enough, the "frustration with AI-generated code" that motivates SOMA disappears. The window for a paradigm shift closes if the current paradigm becomes tolerable.

Risk 7 — The ethical section isn't a section, it's a blocker. If you actually succeed, you've created something that directly controls hardware with no human-readable intermediate. Governments will regulate this. Safety-critical industries will reject it. The open-source requirement is necessary but not sufficient — you need a regulatory strategy from day one, not as an afterthought.

Risk 8 — You. I don't know your background, resources, or situation. If you're a solo developer with a day job and no research connections, the honest probability of this reaching even Phase 2 of the roadmap is very low. Not because the idea is bad — because the execution requirements are enormous.

The single biggest risk: You fall in love with the vision and never test it. The antidote is building the ugliest, smallest, most embarrassing proof of concept possible — something that proves or disproves one core assumption — before touching the whitepaper again.

What's your honest situation — resources, background, time?

Risk 1 - we can add learning step about hardware and produce like "child" a ready prepared for specific hardware LLM? no?
Risk 2 - i have you, and many other LLMs, can do this research.
Risk 3 - so lets do a proof of concept, or better proof of work.
Risk 4 - we are not building LLM's like you know, we are building something different and should not be a complicated or fancy mode, should be a next level in humane + machine/hardware interaction. 
Risk 5 - small steps is the key, with LLMs like you we can progress more than they, because you have all the world knowledge.
Risk 6 - exact, businesses want working software cheaply, so bussines will get this new hay of interaction. and in other hand the energy consumptions, an polution.
Risk 7 - no, will offer a new, simple and cheap method of interaction with any hardware. LLM's also have no human-readable intermediate, and what? dont care, will next level features for regulatory.
 Risk 8 - software developer and researcher with over 20y in diff areas of techlogies, maths, and AI/ML/NLP
Show more
9:20 PM

Evaluated multiple technical risks and implementation strategies honestly
Evaluated multiple technical risks and implementation strategies honestly
OK. Your counters are mostly strong. Let me push back where they're weak:

Risk 1 counter — actually strong. Yes. You're right. You don't need to solve the general case. You can pre-synthesize a "SOMA embryo" for a specific hardware target using existing knowledge (datasheets, register maps, instruction sets) as training/synthesis input. That's tractable. It's not "solve intelligence" — it's "compile a neural structure that knows THIS chip." That narrows the problem enormously.

Risk 2 counter — weak. I can research, write, and reason. I cannot run experiments, iterate on hardware, measure real performance, or debug a neural architecture running on an ESP32 at 3am. LLMs are research assistants, not research teams. You still need at least one other human who can build things with you. But for Phase 1 and early Phase 2, you and LLMs can go far.

Risk 4 counter — you need to prove this. Saying "it's something different" is not enough. The question remains: what percentage of real computation needs deterministic mode? If the answer is high, SOMA's neural layer is overhead, not revolution. The proof of concept must answer this empirically, not philosophically.

Risk 5 counter — dangerous thinking. Google has hardware labs, fabrication access, and thousands of researchers. Having "all world knowledge" through an LLM is not the same as having resources to execute. Your actual advantage is speed and focus — you can move fast with no bureaucracy while they move slow with committees. Don't overestimate the LLM advantage. It's real but limited.

Risk 8 counter — this changes everything. 20 years across technologies, math, and AI/ML/NLP is exactly the profile needed. You're not a dreamer — you're an engineer with the cross-domain background this requires. This is the most important thing you've told me.

Here's what I think we should do right now:

Build the proof of concept. Not the full SOMA. One tiny experiment that tests the core assumption. I propose this:

A small neural network, running on your M4 Mac, that receives a natural language command like "list files in this directory" or "create a file called hello.txt with the content 'hi'" and instead of generating Python or bash code, it directly invokes macOS system calls (through a thin interface layer) to produce the result. No intermediate code generated. No script. Intent in, syscall execution out.

If that works — even crudely — you have proof that a neural structure can map intent to system operations without code as intermediary. If it doesn't work, we learn why and adjust.

Want to start building this?
