#!/usr/bin/env python3
"""Translate measured intervention probes into SOMA's generic trace boundary.

The script does not infer game mechanics or fabricate rollouts. Every emitted
transition is a direct edge present in the provenance-bearing probe artifact.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "artifacts/arc3-observable-probes.json"
OUTPUT = ROOT / "artifacts/observed-intervention-traces.json"
MANIFEST = ROOT / "manifests/observed-intervention-traces.json"


def outcome(record: dict) -> tuple[int, int, bool]:
    changed = int(record.get("delta", {}).get("changed_cells", 0))
    levels = int(record.get("levels_completed", 0))
    terminal = record.get("state") not in (None, "NOT_FINISHED")
    return changed, levels, terminal


def append(events: list[dict], sequence: str, ordinal: int, state: str,
           action: str, next_state: str, record: dict) -> None:
    changed, levels, terminal = outcome(record)
    events.append({
        "sequence": sequence,
        "ordinal": ordinal,
        "state": state,
        "action": action,
        "next_state": next_state,
        "changed_cells": changed,
        "levels_completed": levels,
        "terminal": terminal,
        "action_cost": 1,
    })


def main() -> None:
    probes = json.loads(SOURCE.read_text())
    events: list[dict] = []
    for environment in probes["environments"]:
        game = environment["game_id"]
        initial = environment["initial_digest"]
        for action, record in sorted(environment.get("one_step", {}).items()):
            append(events, f"{game}:one:{action}", 0, initial, action,
                   record["digest"], record)
        for action, item in sorted(environment.get("repeat", {}).items()):
            record = item.get("two_step")
            first = environment.get("one_step", {}).get(action)
            if not record or not first:
                continue
            sequence = f"{game}:repeat:{action}"
            append(events, sequence, 0, initial, action, first["digest"], first)
            append(events, sequence, 1, first["digest"], action,
                   record["digest"], record)
        click_probe = environment.get("click_probe") or {}
        for index, item in enumerate(click_probe.get("examples", [])):
            record = item.get("outcome", item)
            action = json.dumps(item.get("action", item.get("point")), sort_keys=True)
            if isinstance(record, dict) and record.get("digest"):
                append(events, f"{game}:coordinate:{index}", 0, initial,
                       action, record["digest"], record)
    payload = {
        "schema": "soma.observed-transition-traces.v1",
        "source_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
        "events": events,
    }
    OUTPUT.write_text(json.dumps(payload, indent=2) + "\n")
    digest = hashlib.sha256(OUTPUT.read_bytes()).hexdigest()
    manifest = {
        "schema": "soma.rtl.instrument.v1",
        "source": {
            "id": "public-observed-intervention-traces",
            "path": "../artifacts/observed-intervention-traces.json",
            "sha256": digest,
            "provenance_url": "local:artifacts/arc3-observable-probes.json",
            "citation": "Runtime-observed public intervention probes; exact local provenance is embedded in the source artifact.",
            "role": "observed_interaction_trace",
        },
        "protocol": {
            "kind": "trace_json",
            "events_pointer": "/events",
            "sequence_field": "sequence",
            "ordinal_field": "ordinal",
            "state_field": "state",
            "action_field": "action",
            "next_state_field": "next_state",
            "outcome_fields": ["changed_cells", "levels_completed"],
            "terminal_field": "terminal",
            "cost_field": "action_cost",
        },
    }
    MANIFEST.write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"exported {len(events)} observed transitions")


if __name__ == "__main__":
    main()
