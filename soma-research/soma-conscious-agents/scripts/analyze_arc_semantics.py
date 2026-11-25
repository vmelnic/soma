#!/usr/bin/env python3
"""Produce a two-pass semantic analysis of every public ARC-AGI-2 evaluation task.

The first pass proposes the smallest relational program consistent with every
published pair.  The second pass is a critic that must repair contradictions and
expose ambiguity.  This is architecture research over public answers, never a
blind benchmark evaluation.
"""
from __future__ import annotations

import concurrent.futures
import json
import os
import re
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "data/external/ARC-AGI-2/data/evaluation"
CACHE = ROOT / "artifacts/arc-semantic-analysis"
FINAL = ROOT / "artifacts/arc2-semantic-analysis.json"
ENV = ROOT.parents[1] / ".env"
BATCH_SIZE = 10
WORKERS = 3

SYSTEM = """You are a rigorous ARC program analyst helping design a universal
induction engine. Inspect every cell of every published input/output pair. Infer
the smallest typed relational program consistent with every pair. Separate the
observer carrier and decomposition from entities, relations, value rules, and
composition. Never claim a rule that contradicts an example. Mark underdetermined
parts explicitly. This analysis sees public answers and is not benchmark evidence.
Return JSON only, without Markdown fences."""

REQUIRED_KEYS = {
    "task_id", "semantic_family", "hypothesis", "carrier", "entities",
    "relations", "program", "required_capabilities", "alternatives",
    "confidence", "validation_notes",
}


def load_env() -> None:
    for line in ENV.read_text().splitlines():
        if "=" in line and not line.lstrip().startswith("#"):
            key, value = line.split("=", 1)
            os.environ.setdefault(key, value)


def extract_text(response: dict) -> str:
    return "".join(
        content.get("text", "")
        for output in response.get("output", [])
        for content in output.get("content", [])
        if content.get("type") == "output_text"
    )


def parse_json(text: str):
    text = text.strip()
    if text.startswith("```"):
        text = re.sub(r"^```(?:json)?\s*", "", text)
        text = re.sub(r"\s*```$", "", text)
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        start_candidates = [x for x in (text.find("["), text.find("{")) if x >= 0]
        if not start_candidates:
            raise
        start = min(start_candidates)
        end = max(text.rfind("]"), text.rfind("}"))
        return json.loads(text[start : end + 1])


def call_model(prompt: str, retries: int = 4):
    payload = {
        "model": os.environ["OPENAI_MODEL"],
        "input": [
            {"role": "system", "content": [{"type": "input_text", "text": SYSTEM}]},
            {"role": "user", "content": [{"type": "input_text", "text": prompt}]},
        ],
    }
    request = urllib.request.Request(
        os.environ["OPENAI_BASE_URL"].rstrip("/") + "/responses",
        data=json.dumps(payload).encode(),
        headers={
            "Content-Type": "application/json",
            "Authorization": "Bearer " + os.environ["OPENAI_API_KEY"],
        },
    )
    error = None
    for attempt in range(retries):
        try:
            with urllib.request.urlopen(request, timeout=600) as response:
                return parse_json(extract_text(json.load(response)))
        except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as exc:
            error = exc
            time.sleep(2 ** attempt)
    raise RuntimeError(f"model call failed after {retries} attempts: {error}")


def batches(paths: list[Path]):
    for i in range(0, len(paths), BATCH_SIZE):
        yield i // BATCH_SIZE, paths[i : i + BATCH_SIZE]


def task_payload(paths: list[Path]) -> list[dict]:
    return [{"task_id": p.stem, "pairs": json.loads(p.read_text())} for p in paths]


def validate(items, expected: list[str]) -> list[dict]:
    if isinstance(items, dict) and "tasks" in items:
        items = items["tasks"]
    if not isinstance(items, list):
        raise ValueError("response is not an array")
    by_id = {item.get("task_id"): item for item in items if isinstance(item, dict)}
    missing = set(expected) - set(by_id)
    extra = set(by_id) - set(expected)
    if missing or extra:
        raise ValueError(f"task mismatch: missing={sorted(missing)}, extra={sorted(extra)}")
    for task_id in expected:
        item = by_id[task_id]
        absent = REQUIRED_KEYS - set(item)
        if absent:
            raise ValueError(f"{task_id}: missing keys {sorted(absent)}")
        if not isinstance(item["required_capabilities"], list):
            raise ValueError(f"{task_id}: required_capabilities is not a list")
    return [by_id[x] for x in expected]


def draft_prompt(payload: list[dict]) -> str:
    return """Analyze every task below independently. Return one JSON array in the
same order. Every object must have exactly these semantic fields:
- task_id
- semantic_family: concise mechanism family
- hypothesis: the transformation in one precise sentence
- carrier: inferred input/output worlds, partitions, coordinate frames, and morphism
- entities: how task-relative entities are constructed
- relations: relations and invariants used
- program: ordered, compositional pseudocode without task-specific function names
- required_capabilities: array of general typed capabilities
- alternatives: plausible remaining ambiguity, or an empty string
- confidence: number in [0,1]
- validation_notes: how all examples constrain or challenge the hypothesis

Do not merely list pixel statistics. Explain the generative transformation. If a
unique exact rule cannot be inferred, say so rather than guessing.

TASKS:\n""" + json.dumps(payload, separators=(",", ":"))


def critic_prompt(payload: list[dict], proposals: list[dict]) -> str:
    return """Critique and repair the proposed semantic analyses against every
published pair. Return a corrected JSON array with the same task IDs, order, and
fields. Recompute the carrier, entities, relations, and program when needed. A
valid analysis must explain output dimensions, every changed and preserved value,
ordering, multiplicity, orientation, and composition. Reject accidental visual
similarities. `validation_notes` must state the strongest discriminating evidence
and any unresolved ambiguity. Keep capabilities domain-general and typed.

TASKS:\n""" + json.dumps(payload, separators=(",", ":")) + "\n\nPROPOSALS:\n" + json.dumps(proposals, separators=(",", ":"))


def run_batch(index: int, paths: list[Path], phase: str) -> Path:
    phase_dir = CACHE / phase
    phase_dir.mkdir(parents=True, exist_ok=True)
    out = phase_dir / f"batch-{index:02d}.json"
    expected = [p.stem for p in paths]
    if out.exists():
        validate(json.loads(out.read_text()), expected)
        return out
    payload = task_payload(paths)
    if phase == "draft":
        response = call_model(draft_prompt(payload))
    else:
        draft_path = CACHE / "draft" / f"batch-{index:02d}.json"
        proposals = validate(json.loads(draft_path.read_text()), expected)
        response = call_model(critic_prompt(payload, proposals))
    items = validate(response, expected)
    out.write_text(json.dumps(items, indent=2) + "\n")
    return out


def run_phase(paths: list[Path], phase: str) -> None:
    jobs = list(batches(paths))
    with concurrent.futures.ThreadPoolExecutor(max_workers=WORKERS) as pool:
        future_map = {
            pool.submit(run_batch, index, chunk, phase): index
            for index, chunk in jobs
        }
        for future in concurrent.futures.as_completed(future_map):
            index = future_map[future]
            path = future.result()
            print(f"{phase}: completed batch {index:02d} -> {path.relative_to(ROOT)}", flush=True)


def combine(paths: list[Path]) -> None:
    result = []
    for index, chunk in batches(paths):
        expected = [p.stem for p in chunk]
        result.extend(validate(json.loads((CACHE / "critic" / f"batch-{index:02d}.json").read_text()), expected))
    FINAL.write_text(json.dumps({
        "schema": "soma.arc2-semantic-analysis.v1",
        "method": "two-pass model-assisted analysis over all published input/output pairs; not blind evaluation",
        "task_count": len(result),
        "tasks": result,
    }, indent=2) + "\n")
    print(f"combined {len(result)} tasks -> {FINAL.relative_to(ROOT)}")


def main() -> None:
    load_env()
    paths = sorted(DATA.glob("*.json"))
    if len(paths) != 120:
        raise SystemExit(f"expected 120 tasks, found {len(paths)}")
    run_phase(paths, "draft")
    run_phase(paths, "critic")
    combine(paths)


if __name__ == "__main__":
    main()
