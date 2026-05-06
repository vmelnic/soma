# qwen-knn

Frozen base LM + kNN-LM. Zero training. Day-1 falsification of "knowledge in RAM, not in weights."

See `docs/qwen-knn.md` in the repo root for the thesis. See `CLAUDE.md` here for invariants.

## Layout

```
src/
  build_index.py    # forward-only pass over corpus -> FAISS
  infer.py          # generate with kNN-LM logit blending
  eval.py           # perplexity + completion vs vanilla base
  config.py         # model + paths + lambda/k defaults
corpus/             # local source texts (small samples; full corpus on Windows)
index/              # FAISS artefacts (Windows-only in practice)
eval/               # held-out files + hand-labelled queries
scripts/
  sync.sh           # push code to win
  run.sh            # run a script on win, stream output
  fetch.sh          # pull a file back
requirements.txt
```

## Quickstart

Configure SSH alias `win` for the 3090 host. Then:

```bash
./scripts/sync.sh
./scripts/run.sh src/build_index.py --corpus corpus/rust-book --out index/rust-book.faiss
./scripts/run.sh src/eval.py --index index/rust-book.faiss --held-out eval/held_out
```

## Falsification

If `bench.py` shows no perplexity reduction on the in-corpus domain and no completion improvement on hand-labelled queries, the line is dead. Record the result. Move on.

## Status

See `RESULTS.md`. First smoke test (Qwen2.5-3B + SOMA corpus) produced a **negative result** — kNN-LM regressed perplexity at default settings. Pipeline is proven; the open question is whether any (λ, k, index, corpus, base-model) configuration produces a win. Next moves are listed in `RESULTS.md` ordered cheapest first.
