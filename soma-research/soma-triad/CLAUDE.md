# soma-triad — LTC + HDC + SDM

## What this is

A non-transformer reasoning architecture that separates concerns:

- **SDM** — knowledge store. No weights. Content-addressable RAM. Write episodes, read by similarity.
- **HDC** — compositional algebra. No weights. Bind/bundle/permute over high-dimensional vectors. Analogical reasoning, role-filler structures, sequence encoding.
- **LTC** — controller. Tiny ODE-based network (~1-10M params). Learns WHEN to retrieve from SDM and WHICH HDC operations to apply. Does NOT store knowledge or do composition — only sequences operations.

The triad replaces a transformer for domain-specific reasoning. The controller is the only trainable part, and it only learns control flow — not world knowledge or language.

## Key invariants

- **Knowledge never goes into weights.** All domain knowledge lives in SDM. If you're training the LTC on domain content, stop.
- **HDC has zero learnable parameters.** The codebook is generated deterministically from a seed. Bind/bundle/permute are algebraic operations, not neural layers.
- **LTC stays tiny.** If the controller exceeds 10M parameters, the decomposition is wrong. The controller should be trainable on hundreds of episodes, not millions.
- **SDM has no query projection.** Unlike soma-brain's SDM, there is no `nn.Linear` in the read path. Pure cosine similarity over normalized vectors.
- **The division is strict.** SDM answers "what have I seen before?" HDC answers "how do these relate?" LTC answers "what operation should I do next?"

## Architecture

```
Intent/WorldState → Encode (HDC codebook) → LTC controller
                                                  ↓
                              ┌─────────── operation signal ───────────┐
                              ↓                                        ↓
                     SDM.read(query)                          HDC.bind/unbind/bundle
                              ↓                                        ↓
                     retrieved episodes                    composed structure
                              ↓                                        ↓
                     back to LTC ──────────→ next operation or EMIT result
```

## Build and test

```bash
pip install -e .
pytest
```

## Dependencies

- torch (for LTC controller only — SDM and HDC are pure numpy)
- numpy
