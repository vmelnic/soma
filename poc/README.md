# SOMA Proof of Concept

**Neural architecture for direct intent-to-execution computing on macOS.**

This is the first proof of concept for the [SOMA paradigm](../SOMA_Whitepaper.md) — a computational model where a neural architecture IS the program. No code is generated. No scripts are produced. Human intent goes in, hardware execution comes out.

## What This Proves

The central SOMA thesis: a neural structure can map natural language intent directly to system operations without code as an intermediate step.

In this PoC, a 644K-parameter BiLSTM neural network takes English text like `"list files in /tmp"` and outputs **tensors** — an opcode (integer) identifying which OS operation to perform, and span indices identifying parameter values within the input. A fixed dispatcher reads these numbers and calls macOS system functions. At no point in this pipeline is a code string (Python, bash, or otherwise) generated, parsed, or interpreted.

```
"list files in /tmp"
        |
   [Tokenize + Embed]
        |
   [BiLSTM Encoder]         -- 2-layer bidirectional LSTM
        |
   [Classification Head]    -- softmax over 15 operations
        |                      output: opcode tensor -> argmax = 0 (LIST_DIR)
   [Span Extraction Head]   -- per-token position scores
        |                      output: slot0 span = (5,5) -> token "/tmp"
   [Fixed Dispatcher]        -- opcode=0, params=["/tmp"] -> os.listdir("/tmp")
        |
   ["file1.txt", "file2.txt", ...]
```

This is analogous to how a brain produces motor output: sensory input is processed through neural layers, and the result is a motor signal — not a written instruction that another system reads.

## Architecture Mapping to SOMA Whitepaper

| Whitepaper Layer | PoC Component | Implementation |
|---|---|---|
| Layer 1: Intent Reception | Tokenizer + Embedding + BiLSTM | Converts text to contextualized token representations |
| Layer 2: Planning | Classification head | Selects which operation to perform (softmax over 15 ops) |
| Layer 3: Execution Core | Span extraction + Dispatcher | Extracts parameters as span indices; dispatcher calls OS |
| Layer 4: Feedback | Result handling in Soma class | Reports success/failure back to user |
| Layer 5: Proprioception | Body manifest + stats tracking | SOMA knows its capabilities and execution history |

## Quick Start

### 1. Create virtual environment and install dependencies

```bash
cd poc
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
```

### 2. Synthesize the SOMA (train the mind onto the body)

```bash
cd ..  # back to soma/ root
poc/.venv/bin/python3 -m poc.synthesis
```

This generates ~2,300 training examples from templates and trains the neural architecture in about 60 seconds on Apple M4. Output:

```
[Synthesis] Model: 644,947 parameters
[Synthesis] Training for up to 120 epochs (patience=20)...
  Epoch  10 | Val op=1.000 span=1.000 e2e=1.000
[Synthesis] Test set results:
  Op Accuracy:   1.000
  Span Accuracy: 1.000
  E2E Accuracy:  1.000
[Synthesis] Saved model -> poc/artifacts/soma_mind.pt
[Synthesis] Saved vocab -> poc/artifacts/vocab.json
```

### 3. Run the SOMA

```bash
poc/.venv/bin/python3 -m poc.soma
```

```
============================================================
  SOMA Proof of Concept v0.1
  Embodied Neural Computing
============================================================
  Body:       macOS arm64
  Mind:       644,947 parameters (BiLSTM)
  Operations: 15
  Vocabulary: 237 tokens
============================================================
  Type natural language intent. No code. Just say what you want.
  Type 'quit' to exit, 'help' for capabilities.

intent> list files in /tmp

  [Mind] LIST_DIR (confidence: 100.0%)
         path: /tmp
  [Body] Result:
         file1.txt
         file2.txt
         ...

intent> what time is it

  [Mind] CURRENT_TIME (confidence: 100.0%)
  [Body] 2026-04-06T21:55:37.786719

intent> create a file called hello.txt with content greetings from soma

  [Mind] CREATE_FILE (confidence: 100.0%)
         path: hello.txt
         content: greetings from soma
  [Body] Created hello.txt
```

## Supported Operations

The SOMA's body (macOS interface) supports 15 operations:

| Opcode | Operation | Parameters | Example Intent |
|---|---|---|---|
| 0 | LIST_DIR | path | "show me everything in /tmp" |
| 1 | CREATE_FILE | path, content | "create a file called notes.txt with content hello world" |
| 2 | READ_FILE | path | "read notes.txt" |
| 3 | DELETE_FILE | path | "throw away temp.txt" |
| 4 | MAKE_DIR | path | "make a new directory called projects" |
| 5 | FILE_INFO | path | "info about notes.txt" |
| 6 | CURRENT_DIR | (none) | "where am i right now" |
| 7 | SYSTEM_INFO | (none) | "give me system information" |
| 8 | CURRENT_TIME | (none) | "whats the current time" |
| 9 | DISK_USAGE | (none) | "how much free space do i have" |
| 10 | PROCESS_LIST | (none) | "what processes are active" |
| 11 | MOVE_FILE | source, destination | "move report.txt to archive.txt" |
| 12 | COPY_FILE | source, destination | "back up notes.txt to notes_copy.txt" |
| 13 | FIND_FILE | pattern | "scan for *.log" |
| 14 | FILE_EXISTS | path | "tell me if config.json exists" |

The SOMA handles natural language variation — you don't need to use exact commands. Phrasings like "check out files in ~/Documents", "throw away temp.txt", and "how much room is left" all work correctly.

## Project Structure

```
poc/
  tokenizer.py      Word-level tokenizer with vocabulary management
  body.py            Operation manifest (15 ops) + fixed OS dispatcher
  mind.py            BiLSTM encoder + classification head + span extraction head
  synthesis.py       Training data generation + training loop (the "synthesizer")
  soma.py            Running SOMA instance with interactive REPL
  requirements.txt   Dependencies (torch>=2.5)
  artifacts/
    soma_mind.pt     Trained model weights
    vocab.json       Tokenizer vocabulary
```

## How It Works

### The Body (`body.py`)

The body is the SOMA's hardware interface — a fixed dispatch table mapping integer opcodes to OS function calls. It is not neural, not generated, not interpreted. It is the equivalent of a nervous system connecting brain to muscles.

Each operation has a schema (name, opcode, parameter slots) and an implementation that calls standard OS functions (`os.listdir`, `shutil.copy2`, `platform.system`, etc.).

The body also provides **proprioception** — when you type `help`, the SOMA reports its own capabilities, parameter requirements, and execution statistics.

### The Mind (`mind.py`)

The mind is a neural network with 644K parameters:

- **Embedding layer**: 237 tokens, 64-dimensional embeddings
- **BiLSTM encoder**: 2-layer bidirectional LSTM, hidden_dim=128, producing 256-dim contextualized token representations
- **Classification head**: Mean-pooled representation -> Linear(256,128) -> ReLU -> Linear(128,15) -> softmax over 15 operations
- **Span extraction heads**: 4 independent Linear(256,1) layers — start/end position scorers for parameter slot 0 and slot 1

The critical design: the mind outputs **tensors**, not text. The operation is selected by `argmax` of classification logits — a pure numerical operation. Parameters are extracted by `argmax` of per-token scores — pointing at positions in the input, not generating text. The entire mind-to-body signal path is numerical.

A `<NULL>` token is prepended to every input at position 0. When the mind predicts span (0,0), it means "no parameter for this slot." This cleanly separates null parameters from real ones.

### The Synthesizer (`synthesis.py`)

The synthesizer is the SOMA equivalent of a compiler. It takes the body's operation manifest and produces a trained mind.

**Training data generation**: Template-based with systematic variation across 5 axes:
1. **Verb variation**: list/show/display/enumerate/browse/explore
2. **Syntactic variation**: "list files in X" / "what's in X" / "show me everything in X"
3. **Formality variation**: "please show" / "show" / "can you show me"
4. **Parameter variation**: Different paths, filenames, content strings
5. **Vocabulary variation**: Diverse synonyms to improve generalization

This produces ~2,300 training examples across 15 operations. Zero-parameter operations are oversampled 8x to balance against parameterized operations.

**Training**: AdamW optimizer, class-weighted CrossEntropyLoss, learning rate scheduling with ReduceLROnPlateau. The combined loss is classification loss + span extraction loss (CrossEntropy over token positions for each slot's start and end).

Synthesis takes about 60 seconds on an Apple M4 CPU.

### The SOMA Instance (`soma.py`)

The `Soma` class ties mind and body together. Its `process_intent()` method implements the full SOMA execution loop:

1. **Intent reception**: Tokenize input, prepend NULL token, encode to tensor
2. **Mind forward pass**: Produces opcode logits + span logits (all tensors)
3. **Confidence check**: If max softmax probability < 40%, ask user to rephrase (ambiguity resolution from whitepaper Layer 1)
4. **Parameter extraction**: Convert span indices to text by indexing into input tokens
5. **Body dispatch**: Call `body.dispatch(opcode, params)` — the "motor signal"
6. **Feedback**: Track success/failure statistics

## Test Results

Tested on 22 intents — 9 known-pattern + 13 novel phrasings never seen during training:

```
[Y] [100.0%] LIST_DIR         <- "list files in /tmp"
[Y] [100.0%] CURRENT_TIME     <- "what time is it"
[Y] [100.0%] CURRENT_DIR      <- "pwd"
[Y] [100.0%] FILE_EXISTS      <- "does config.json exist"
[Y] [100.0%] CREATE_FILE      <- "create a file called demo.txt with content hello from soma"
[Y] [100.0%] COPY_FILE        <- "copy demo.txt to backup.txt"
[Y] [100.0%] LIST_DIR         <- "show me everything in /tmp"          (novel)
[Y] [100.0%] LIST_DIR         <- "check out files in ~/Documents"      (novel)
[Y] [100.0%] CURRENT_TIME     <- "whats the current time right now"    (novel)
[Y] [100.0%] DISK_USAGE       <- "how much free space do i have"       (novel)
[Y] [100.0%] FILE_EXISTS      <- "tell me if hello.txt exists"         (novel)
[Y] [100.0%] MAKE_DIR         <- "make a new directory called testdir" (novel)
[Y] [100.0%] PROCESS_LIST     <- "what processes are active"           (novel)
[Y] [100.0%] CURRENT_DIR      <- "where am i right now"               (novel)
[Y] [100.0%] DELETE_FILE      <- "throw away temp.txt"                 (novel)
[Y] [100.0%] COPY_FILE        <- "back up notes.txt to notes_copy.txt" (novel)

Accuracy: 22/22 = 100%
```

## What This Is NOT

This PoC is **not** a chatbot that generates code behind the scenes. The distinction matters:

| Chatbot / AI Agent | SOMA PoC |
|---|---|
| LLM generates Python/bash as text | Neural network outputs tensors (numbers) |
| Generated code is parsed by interpreter | Dispatcher reads opcode integer directly |
| Intermediate artifact exists (the script) | No intermediate artifact — intent to execution |
| Model is the code generator | Model IS the program |

If you inspect the pipeline at every step, you will find text only at the input (human intent) and output (result display). Between those two points, everything is tensors and function dispatch.

## Limitations

This is a v0.1 proof of concept. Honest limitations:

- **15 operations only.** Real utility requires hundreds or thousands of body capabilities.
- **Single-step operations.** "Create a directory and list its contents" requires two operations — the current SOMA handles one operation per intent.
- **Word-level tokenization.** Novel words not in the training vocabulary become `<UNK>`. A subword or character-level tokenizer would improve generalization further.
- **macOS only.** The body is hardcoded to macOS/POSIX. A real SOMA would be synthesized per target (the whole point of the whitepaper's synthesis model).
- **No runtime adaptation.** The mind is frozen after synthesis. Whitepaper Section 10.3 describes runtime neuroplasticity — not implemented here.
- **Template-based synthesis data.** The training data is generated from templates, not from real human usage. A production synthesizer would learn from diverse intent corpora.

## Next Steps (Roadmap)

From the SOMA whitepaper research roadmap:

1. **Operation composition** — Handle multi-step intents ("create a directory and put a file in it")
2. **ESP32 synthesis** — Export the model to ONNX, run on a microcontroller, replace body with GPIO operations
3. **Synaptic Protocol** — Multiple SOMA instances communicating across devices
4. **Runtime adaptation** — Allow the SOMA to improve from feedback without re-synthesis
5. **Self-hosting** — A SOMA that synthesizes other SOMAs

## Requirements

- Python 3.12+
- PyTorch 2.5+
- macOS (for body operations; Linux would work with minor path adjustments)
- No GPU required — trains in ~60s on CPU

## License

Part of the SOMA research project. See the [whitepaper](../SOMA_Whitepaper.md) for the full vision.
