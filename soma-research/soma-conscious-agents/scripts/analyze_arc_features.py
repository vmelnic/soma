#!/usr/bin/env python3
"""Build an evidence-labeled feature inventory for the local ARC-AGI-2/3 corpora.

This is corpus characterization, not a solver and not a benchmark score.  It reads
published answers for ARC-AGI-2 and published source/metadata for ARC-AGI-3, so its
output must never be presented as blind evaluation evidence.
"""
from __future__ import annotations

import ast
import json
import math
import re
import statistics
from collections import Counter, deque
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Sequence

ROOT = Path(__file__).resolve().parents[1]
ARC2 = ROOT / "data/external/ARC-AGI-2/data/evaluation"
ARC3 = ROOT / "data/external/ARC-AGI-3"
OUT_MD = ROOT / "artifacts/ARC-AGI-2-3-STRUCTURAL-INVENTORY.md"
OUT_JSON = ROOT / "artifacts/arc-feature-inventory.json"

Grid = list[list[int]]


def shape(g: Grid) -> tuple[int, int]:
    return len(g), len(g[0]) if g else 0


def palette(g: Grid) -> set[int]:
    return {v for row in g for v in row}


def flat(g: Grid) -> list[int]:
    return [v for row in g for v in row]


def background(g: Grid) -> int:
    counts = Counter(flat(g))
    return min(counts, key=lambda k: (-counts[k], k))


def transpose(g: Grid) -> Grid:
    return [list(row) for row in zip(*g)]


def rotate90(g: Grid) -> Grid:
    return [list(row) for row in zip(*g[::-1])]


def flips_and_rotations(g: Grid) -> dict[str, Grid]:
    r0 = g
    r1 = rotate90(r0)
    r2 = rotate90(r1)
    r3 = rotate90(r2)
    mirror = [row[::-1] for row in g]
    return {
        "identity": r0,
        "rotate-90": r1,
        "rotate-180": r2,
        "rotate-270": r3,
        "reflect-horizontal": g[::-1],
        "reflect-vertical": mirror,
        "reflect-diagonal": transpose(g),
        "reflect-antidiagonal": rotate90(rotate90(transpose(g))),
    }


def contains_submatrix(big: Grid, small: Grid) -> bool:
    bh, bw = shape(big)
    sh, sw = shape(small)
    if sh > bh or sw > bw:
        return False
    return any(
        all(big[r + y][c : c + sw] == small[y] for y in range(sh))
        for r in range(bh - sh + 1)
        for c in range(bw - sw + 1)
    )


def nearest_scale(src: Grid, dst: Grid) -> tuple[int, int] | None:
    sh, sw = shape(src)
    dh, dw = shape(dst)
    if not sh or not sw or dh % sh or dw % sw:
        return None
    fy, fx = dh // sh, dw // sw
    if fy < 1 or fx < 1:
        return None
    expected = [[src[y // fy][x // fx] for x in range(dw)] for y in range(dh)]
    return (fy, fx) if expected == dst and (fy > 1 or fx > 1) else None


def tiled(src: Grid, dst: Grid) -> tuple[int, int] | None:
    sh, sw = shape(src)
    dh, dw = shape(dst)
    if not sh or not sw or dh % sh or dw % sw:
        return None
    fy, fx = dh // sh, dw // sw
    if fy == fx == 1:
        return None
    expected = [[src[y % sh][x % sw] for x in range(dw)] for y in range(dh)]
    return (fy, fx) if expected == dst else None


def uniform_recolor(src: Grid, dst: Grid) -> bool:
    if shape(src) != shape(dst):
        return False
    mapping: dict[int, int] = {}
    for a, b in zip(flat(src), flat(dst)):
        if a in mapping and mapping[a] != b:
            return False
        mapping[a] = b
    return any(a != b for a, b in mapping.items())


def constant_separators(g: Grid) -> tuple[int, int]:
    h, w = shape(g)
    rows = sum(len(set(row)) == 1 for row in g)
    cols = sum(len({g[y][x] for y in range(h)}) == 1 for x in range(w))
    return rows, cols


def min_period(values: Sequence[object]) -> int | None:
    n = len(values)
    for p in range(1, n):
        if all(values[i] == values[i % p] for i in range(n)):
            return p
    return None


def periodicity(g: Grid) -> tuple[int | None, int | None]:
    h, w = shape(g)
    row_period = min_period([tuple(r) for r in g]) if h > 1 else None
    cols = [tuple(g[y][x] for y in range(h)) for x in range(w)]
    col_period = min_period(cols) if w > 1 else None
    return row_period, col_period


def symmetry(g: Grid) -> set[str]:
    return {name for name, value in flips_and_rotations(g).items() if name != "identity" and value == g}


def component_count(g: Grid) -> int:
    h, w = shape(g)
    bg = background(g)
    seen: set[tuple[int, int]] = set()
    result = 0
    for y in range(h):
        for x in range(w):
            if (y, x) in seen or g[y][x] == bg:
                continue
            result += 1
            color = g[y][x]
            q = deque([(y, x)])
            seen.add((y, x))
            while q:
                cy, cx = q.popleft()
                for ny, nx in ((cy - 1, cx), (cy + 1, cx), (cy, cx - 1), (cy, cx + 1)):
                    if 0 <= ny < h and 0 <= nx < w and (ny, nx) not in seen and g[ny][nx] == color:
                        seen.add((ny, nx))
                        q.append((ny, nx))
    return result


def fmt_shapes(pairs: list[tuple[Grid, Grid]]) -> str:
    counts = Counter((shape(i), shape(o)) for i, o in pairs)
    items = []
    for ((ih, iw), (oh, ow)), n in sorted(counts.items()):
        s = f"{ih}×{iw}→{oh}×{ow}"
        if n > 1:
            s += f" ({n})"
        items.append(s)
    return "; ".join(items)


def all_or_some(flags: list[bool], name: str) -> str | None:
    if not any(flags):
        return None
    return name if all(flags) else f"some {name}"


def analyze_arc2_task(path: Path) -> dict:
    data = json.loads(path.read_text())
    pairs = [(x["input"], x["output"]) for split in ("train", "test") for x in data[split]]
    relations: set[str] = set()
    geometric: set[str] = set()
    structural: set[str] = set()

    same = [shape(i) == shape(o) for i, o in pairs]
    smaller = [shape(o)[0] <= shape(i)[0] and shape(o)[1] <= shape(i)[1] and shape(o) != shape(i) for i, o in pairs]
    larger = [shape(o)[0] >= shape(i)[0] and shape(o)[1] >= shape(i)[1] and shape(o) != shape(i) for i, o in pairs]
    transposed_dims = [shape(o) == shape(i)[::-1] and shape(o) != shape(i) for i, o in pairs]
    exact = [i == o for i, o in pairs]
    crop = [contains_submatrix(i, o) and shape(i) != shape(o) for i, o in pairs]
    embed = [contains_submatrix(o, i) and shape(i) != shape(o) for i, o in pairs]
    d4_names = []
    scale_flags, tile_flags = [], []
    recolors = []
    edit_ratios = []
    palette_kinds = []
    component_delta = []
    separator_flags = []
    symmetry_change = []
    periodic_flags = []

    for i, o in pairs:
        matches = [name for name, transformed in flips_and_rotations(i).items() if name != "identity" and transformed == o]
        d4_names.extend(matches)
        scale_flags.append(nearest_scale(i, o) is not None or nearest_scale(o, i) is not None)
        tile_flags.append(tiled(i, o) is not None or tiled(o, i) is not None)
        recolors.append(uniform_recolor(i, o))
        if shape(i) == shape(o):
            edit_ratios.append(sum(a != b for a, b in zip(flat(i), flat(o))) / len(flat(i)))
        pi, po = palette(i), palette(o)
        if pi == po:
            palette_kinds.append("same palette")
        elif po < pi:
            palette_kinds.append("output palette subset")
        elif pi < po:
            palette_kinds.append("output adds colors")
        else:
            palette_kinds.append("palette replacement")
        component_delta.append(component_count(o) - component_count(i))
        separator_flags.append(any(constant_separators(i)) or any(constant_separators(o)))
        symmetry_change.append(symmetry(i) != symmetry(o))
        periodic_flags.append(any(v is not None for v in periodicity(i) + periodicity(o)))

    for flags, name in ((same, "same carrier"), (smaller, "carrier reduction"), (larger, "carrier expansion"),
                        (transposed_dims, "transposed dimensions"), (exact, "identity"), (crop, "exact crop/extraction"),
                        (embed, "exact embedding"), (scale_flags, "exact cell scaling"), (tile_flags, "exact tiling")):
        value = all_or_some(flags, name)
        if value:
            relations.add(value)
    if not (all(same) or all(smaller) or all(larger)):
        relations.add("mixed carrier relation")

    if d4_names:
        geometric.add("D4 transform: " + "/".join(sorted(set(d4_names))))
    value = all_or_some(recolors, "uniform color map")
    if value:
        geometric.add(value)
    if edit_ratios:
        mean_edit = statistics.mean(edit_ratios)
        geometric.add(("sparse" if mean_edit <= 0.25 else "dense") + f" same-shape edit ({mean_edit:.0%})")
    geometric.update(sorted(set(palette_kinds)))

    if any(separator_flags):
        structural.add("constant-line partition signal")
    if any(periodic_flags):
        structural.add("exact periodicity signal")
    if any(symmetry_change):
        structural.add("symmetry changes")
    if any(d < 0 for d in component_delta):
        structural.add("components merge/disappear")
    if any(d > 0 for d in component_delta):
        structural.add("components split/appear")
    if all(d == 0 for d in component_delta):
        structural.add("component count preserved")

    return {
        "id": path.stem,
        "train": len(data["train"]),
        "test": len(data["test"]),
        "shapes": fmt_shapes(pairs),
        "carrier": sorted(relations),
        "value_geometry": sorted(geometric),
        "structure": sorted(structural),
    }


LEXICAL_SIGNATURES = {
    "collision": ("collid",),
    "rotation": ("rotat",),
    "scaling": ("scale",),
    "target/goal": ("target", "goal"),
    "adversary/hazard": ("enemy", "hazard"),
    "resource/lives": ("energy", "lives", "health"),
    "switch/button": ("switch", "button"),
    "key/door/portal": ("key", "door", "portal"),
    "time/sequence": ("timer", "sequence"),
    "inventory/attachment": ("inventory", "attach", "detach"),
    "explicit rule state": ("rule",),
    "stochastic branch": ("random", "randint", "choice"),
    "gravity": ("gravity",),
    "walls/path": ("wall", "path"),
    "undo state": ("undo",),
}


def identifier_words(source: str) -> set[str]:
    try:
        tree = ast.parse(source)
    except SyntaxError:
        return set(re.findall(r"[A-Za-z_][A-Za-z0-9_]*", source.lower()))
    words: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Name):
            words.update(node.id.lower().split("_"))
            words.add(node.id.lower())
        elif isinstance(node, ast.Attribute):
            words.update(node.attr.lower().split("_"))
            words.add(node.attr.lower())
        elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            words.update(node.name.lower().split("_"))
            words.add(node.name.lower())
    return words


def analyze_arc3() -> list[dict]:
    observation_path = ARC3 / "INITIAL-OBSERVATIONS.json"
    observations = {x["game_id"]: x for x in json.loads(observation_path.read_text())}
    result = []
    for folder in sorted(p for p in ARC3.iterdir() if p.is_dir()):
        metadata = json.loads((folder / "metadata.json").read_text())
        source_path = next(folder.glob("*.py"))
        source = source_path.read_text(errors="replace")
        words = identifier_words(source)
        signatures = []
        for label, needles in LEXICAL_SIGNATURES.items():
            if any(any(word == n or word.startswith(n) for word in words) for n in needles):
                signatures.append(label)
        obs = observations[metadata["game_id"]]
        actions = obs["actions"]
        modalities = []
        if any(a in actions for a in ("ACTION1", "ACTION2", "ACTION3", "ACTION4")):
            modalities.append("directional")
        if "ACTION5" in actions:
            modalities.append("context action")
        if "ACTION6" in actions:
            modalities.append("coordinate click")
        if "ACTION7" in actions:
            modalities.append("undo")
        baselines = metadata["baseline_actions"]
        result.append({
            "id": metadata["game_id"],
            "title": metadata.get("title", folder.name[:4].upper()),
            "levels": len(baselines),
            "tags": metadata.get("tags", []),
            "actions": actions,
            "modalities": modalities,
            "baseline_min": min(baselines),
            "baseline_median": statistics.median(baselines),
            "baseline_max": max(baselines),
            "initial_palette": obs["initial_palette"],
            "initial_shape": obs["initial_shape"],
            "signatures": signatures,
            "source_lines": len(source.splitlines()),
        })
    return result


def aggregate_features(tasks: list[dict], key: str) -> Counter:
    out: Counter = Counter()
    for task in tasks:
        normalized = set()
        for feature in task[key]:
            feature = feature.removeprefix("some ")
            feature = re.sub(r"^(sparse|dense) same-shape edit \(\d+%\)$", r"\1 same-shape edit", feature)
            normalized.add(feature)
        out.update(normalized)
    return out


def md_escape(value: object) -> str:
    return str(value).replace("|", "\\|").replace("\n", " ")


def controls(actions: list[str]) -> str:
    compact = {f"ACTION{i}": f"A{i}" for i in range(1, 8)}
    return ", ".join(compact.get(a, a) for a in actions)


def render(arc2: list[dict], arc3: list[dict]) -> str:
    carrier_counts = aggregate_features(arc2, "carrier")
    value_counts = aggregate_features(arc2, "value_geometry")
    structure_counts = aggregate_features(arc2, "structure")
    modality_counts = Counter(m for e in arc3 for m in e["modalities"])
    signature_counts = Counter(s for e in arc3 for s in e["signatures"])

    lines: list[str] = []
    add = lines.append
    add("# ARC-AGI-2 and ARC-AGI-3 Observed Feature Inventory")
    add("")
    add("Generated from the local published ARC-AGI-2 evaluation corpus and the official public ARC-AGI-3 environments. This is a requirements analysis for SOMA, not a benchmark score. Published outputs and environment source are intentionally inspected; therefore none of the results below are blind-test evidence.")
    add("")
    add("## Coverage and evidence boundary")
    add("")
    add(f"- ARC-AGI-2: **{len(arc2)} evaluation tasks**, including all published train and test input/output pairs stored locally.")
    add(f"- ARC-AGI-3: **{len(arc3)} public environments** and **{sum(e['levels'] for e in arc3)} levels** from official metadata, source, and seed-0 initial observations.")
    add("- ARC-AGI-2 features are exact measurements of grids. They describe visible regularities, not the unique intended program.")
    add("- ARC-AGI-3 controls, level counts, baselines, and initial frames are runtime/metadata observations. Mechanic signatures are lexical evidence from published source and are labeled accordingly; they are not claims that a solver inferred those mechanics from play.")
    add("- Private ARC tasks/environments and unexplored ARC-AGI-3 trajectories are outside this inventory.")
    add("")
    add("Official sources: [ARC-AGI-2 repository](https://github.com/arcprize/ARC-AGI-2), [ARC-AGI toolkit](https://github.com/arcprize/ARC-AGI), [ARC-AGI-3 technical report](https://arcprize.org/media/ARC_AGI_3_Technical_Report.pdf), [ARC-AGI-3 overview](https://arcprize.org/arc-agi/3), [game schema](https://github.com/arcprize/docs/blob/main/docs/arc-agi/3/game-schema.mdx), and [available games](https://github.com/arcprize/docs/blob/main/docs/arc-agi/3/available-games.mdx).")
    add("")
    add("## Feature vocabulary")
    add("")
    add("### Observation and carrier")
    add("")
    add("- Finite colored grids, changing dimensions, coordinate frames, palettes, backgrounds, and foreground ambiguity.")
    add("- Carrier-preserving transformations, reductions/crops, embeddings/expansions, transposed carriers, and mixed input-output carrier relations.")
    add("- Parts, connected components, holes, containment, contact, adjacency, ordering, repetition, separators, partitions, and negative space.")
    add("- Object identity across movement, rotation, reflection, scaling, recoloring, overlap, occlusion, splitting, merging, completion, and deletion.")
    add("- Multiple possible observer factorizations: pixels, components, regions, motifs, relations, and temporal entities.")
    add("")
    add("### Programs and induction")
    add("")
    add("- Selection by intrinsic and relational properties: color, size, count, shape, symmetry, position, uniqueness, correspondence, and role.")
    add("- Typed spatial morphisms: crop, project, embed, translate, align, tile, repeat, mirror, rotate, transpose, scale, compose, overlay, mask, and complete.")
    add("- Value transformations: recolor, copy, substitute, count, encode, decode, propagate, and conditionally rewrite.")
    add("- Relational operations: match, compare, pair, order, group, infer axes, infer correspondences, and preserve invariants.")
    add("- Hierarchical composition: infer reusable subprograms, bind variables, construct macros, recurse over objects/regions, and choose among alternative decompositions.")
    add("- Posterior inference over complete typed programs, with trace semantics and predictive verification on held-out observations.")
    add("")
    add("### Interaction and active world models")
    add("")
    add("- Unknown goals and mechanics, latent state, sparse success/failure feedback, irreversible actions, and optional undo.")
    add("- Directional actions, context-dependent actions, coordinate clicks, action availability, animation/frame stacks, terminal states, and multiple levels.")
    add("- Active experiments that distinguish competing causal laws; counterfactual prediction before action; belief updates after observation.")
    add("- Object permanence and state tracking through time, event segmentation, causal credit assignment, resource/risk management, and long-horizon planning.")
    add("- Within-environment transfer across levels and structural transfer across environments.")
    add("- Action efficiency as part of the objective, using the official human-action baseline rather than completion alone.")
    add("")
    add("## ARC-AGI-2 aggregate measurements")
    add("")
    add("The counts below mean that a detector fired in at least one published pair of a task. `some` means the relation did not hold across every pair in that task.")
    add("")
    add("### Carrier relations")
    add("")
    add("| Observed feature | Tasks |")
    add("|---|---:|")
    for name, count in sorted(carrier_counts.items(), key=lambda x: (-x[1], x[0])):
        add(f"| {md_escape(name)} | {count} |")
    add("")
    add("### Value and geometric relations")
    add("")
    add("| Observed feature | Tasks |")
    add("|---|---:|")
    for name, count in sorted(value_counts.items(), key=lambda x: (-x[1], x[0])):
        add(f"| {md_escape(name)} | {count} |")
    add("")
    add("### Structural signals")
    add("")
    add("| Observed feature | Tasks |")
    add("|---|---:|")
    for name, count in sorted(structure_counts.items(), key=lambda x: (-x[1], x[0])):
        add(f"| {md_escape(name)} | {count} |")
    add("")
    add("## ARC-AGI-2 task inventory")
    add("")
    add("Every row aggregates all published train and test pairs. Compact detectors are deliberately conservative: for example, `exact crop` means literal contiguous containment and does not imply that cropping is the intended rule.")
    add("")
    add("| Task | Train/Test | Shape relations | Carrier features | Value/geometric features | Structural signals |")
    add("|---|---:|---|---|---|---|")
    for t in arc2:
        add("| " + " | ".join([
            f"`{t['id']}`", f"{t['train']}/{t['test']}", md_escape(t["shapes"]),
            md_escape(", ".join(t["carrier"])), md_escape(", ".join(t["value_geometry"])),
            md_escape(", ".join(t["structure"])),
        ]) + " |")
    add("")
    add("## ARC-AGI-3 aggregate measurements")
    add("")
    add("### Interaction modalities")
    add("")
    add("| Runtime-observed modality | Environments |")
    add("|---|---:|")
    for name, count in sorted(modality_counts.items(), key=lambda x: (-x[1], x[0])):
        add(f"| {md_escape(name)} | {count} |")
    add("")
    add("### Published-source mechanic signatures")
    add("")
    add("These are identifier-level signals in the official environment source. They establish implemented mechanism families, but do not establish that an agent can discover or use them.")
    add("")
    add("| Source signature | Environments |")
    add("|---|---:|")
    for name, count in sorted(signature_counts.items(), key=lambda x: (-x[1], x[0])):
        add(f"| {md_escape(name)} | {count} |")
    add("")
    add("## ARC-AGI-3 environment inventory")
    add("")
    add("Action notation follows the official schema: A1–A4 are directional, A5 is context dependent, A6 is a coordinate click, and A7 is undo. Human baseline values are action counts per level, summarized as minimum/median/maximum.")
    add("")
    add("| Environment | Levels | Tags | Available actions | Modalities | Human actions min/median/max | Initial frame | Source-observed signatures |")
    add("|---|---:|---|---|---|---:|---|---|")
    for e in arc3:
        median = int(e["baseline_median"]) if float(e["baseline_median"]).is_integer() else e["baseline_median"]
        add("| " + " | ".join([
            f"`{e['id']}`", str(e["levels"]), md_escape(", ".join(e["tags"])), controls(e["actions"]),
            md_escape(", ".join(e["modalities"])), f"{e['baseline_min']}/{median}/{e['baseline_max']}",
            f"{e['initial_shape'][0]}×{e['initial_shape'][1]}, {len(e['initial_palette'])} colors",
            md_escape(", ".join(e["signatures"]) or "none detected"),
        ]) + " |")
    add("")
    add("## Unified requirements for SOMA")
    add("")
    add("The corpora imply one generative architecture, not a catalog of task handlers.")
    add("")
    add("1. **Boundary transduction.** Convert an external observation/action boundary into a neutral typed relational carrier without ARC-specific operators.")
    add("2. **Headset posterior.** Maintain competing observer interfaces: entity formation, coordinate frame, scale, topology, object permanence, action affordances, and observation/action morphisms. The headset is inferred before action and remains revisable.")
    add("3. **World-law posterior.** Infer latent state and local/global transition laws jointly with the headset. A complete Markov state includes any memory needed to make prediction conditionally closed.")
    add("4. **Structural program induction.** Generate typed relational programs from constraints and reusable subprograms; infer structures and morphisms before enumerating value transformations.")
    add("5. **Trace semantics.** Every hypothesis denotes an executable distribution over observation/action traces. Likelihood is computed from predicted observations and outcomes, not from descriptive similarity.")
    add("6. **Active experiment selection.** Choose actions by expected information gain, expected goal progress, risk, reversibility, and action cost.")
    add("7. **Hierarchical memory.** Preserve episodes, inferred entities, laws, programs, failed hypotheses, uncertainty, and cross-level abstractions with provenance.")
    add("8. **Planning and control.** Search over belief states and temporally extended programs, replan after prediction error, and optimize completion plus action efficiency.")
    add("9. **Verification.** Replay complete traces, test predictions on withheld observations, distinguish mathematical consistency from empirical evidence, and reject task-specific leakage.")
    add("10. **Open-ended extension.** Propose new typed operators only as compositions or conservative extensions with executable semantics, provenance, and measured transfer.")
    add("")
    add("A suitable joint hypothesis is:")
    add("")
    add("```text")
    add("H = (headset, ontology, latent_state, transition_law, goal, program_library, policy)")
    add("P(H | trace) ∝ P(trace | H) · P(H)")
    add("s[t+1] ~ T_H(s[t], a[t])")
    add("o[t]     ~ O_H(s[t])")
    add("a[t]     ~ π_H(belief[t], goal_H)")
    add("```")
    add("")
    add("Here `O_H` is part of the inferred headset. It cannot be a post-hoc renderer built after a program has already chosen an answer. The Markov recursion operates over the joint latent state and observer interface; observed pixels alone need not form a closed Markov process.")
    add("")
    add("## Consequence for the current failure")
    add("")
    add("Task `136b0064` is a concrete carrier-level diagnostic. Its examples map a 15-column observation to a 7-column action carrier. A hypothesis space containing only whole-grid and product-level-set domains cannot express the required observer-relative subworld before value induction. Adding another task operator would hide the defect. SOMA needs a posterior over carrier decompositions and typed morphisms, then conditional value/program induction inside the selected carrier.")
    add("")
    add("The same correction is required for ARC-AGI-3: a 64×64 frame is an external boundary value, while the operative world may consist of persistent objects, controls, resources, rules, and hidden state. The headset must construct that world and expose intervention morphisms; the transition model and planner then operate on it.")
    add("")
    add("## What this inventory does not establish")
    add("")
    add("- It does not identify the unique intended solution for every ARC-AGI-2 task.")
    add("- It does not solve or score any ARC-AGI-3 environment.")
    add("- It does not prove that the listed vocabulary is complete for private or future tasks.")
    add("- It does not demonstrate general intelligence or consciousness.")
    add("- It does provide a complete, reproducible inventory of the locally available ARC-AGI-2 corpus and the currently public ARC-AGI-3 set, with evidence types kept explicit.")
    add("")
    add("## Reproduction")
    add("")
    add("```bash")
    add("python3 scripts/analyze_arc_features.py")
    add("```")
    add("")
    add("Machine-readable output is written to `artifacts/arc-feature-inventory.json`. External-file hashes and retrieval provenance are recorded under `data/external/ARC-AGI-3/PROVENANCE.json`.")
    add("")
    return "\n".join(lines)


def main() -> None:
    paths = sorted(ARC2.glob("*.json"))
    if len(paths) != 120:
        raise SystemExit(f"expected 120 ARC-AGI-2 tasks, found {len(paths)}")
    arc2 = [analyze_arc2_task(path) for path in paths]
    arc3 = analyze_arc3()
    if len(arc3) != 25:
        raise SystemExit(f"expected 25 public ARC-AGI-3 environments, found {len(arc3)}")
    payload = {
        "schema": "soma.arc-feature-inventory.v1",
        "method": "published-corpus characterization; not blind evaluation",
        "arc_agi_2": arc2,
        "arc_agi_3": arc3,
    }
    OUT_JSON.write_text(json.dumps(payload, indent=2) + "\n")
    OUT_MD.write_text(render(arc2, arc3))
    print(f"wrote {OUT_MD} ({len(arc2)} ARC-AGI-2 tasks, {len(arc3)} ARC-AGI-3 environments)")


if __name__ == "__main__":
    main()
