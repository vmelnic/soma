"""Training data collection, expansion, and PyTorch Dataset.

Collects training examples from plugin training/*.json files,
builds a unified convention catalog, expands template x param pool
combinations into concrete (intent, program) pairs, and provides
a PyTorch Dataset for the training loop.

Self-contained: imports only from soma_synthesizer package + stdlib + torch.
"""

import json
import os
import random
from itertools import product

import torch
from torch.utils.data import Dataset


# ---------------------------------------------------------------------------
# Arg-type constants (must match model output head indices)
# ---------------------------------------------------------------------------
ARG_NONE = 0
ARG_SPAN = 1
ARG_REF = 2
ARG_LITERAL = 3

_ARG_TYPE_MAP = {
    "none": ARG_NONE,
    "span": ARG_SPAN,
    "ref": ARG_REF,
    "literal": ARG_LITERAL,
}

# Special token indices (must match tokenizer conventions)
PAD_IDX = 0
NULL_IDX = 2  # prepended to every token-id sequence as position-0 anchor


# ---------------------------------------------------------------------------
# Convention Catalog
# ---------------------------------------------------------------------------

class ConventionCatalog:
    """Merged convention catalog built from all target plugins.

    Each plugin contributes its conventions; after all plugins are added,
    ``finalize()`` appends the built-in EMIT and STOP control opcodes.

    Attributes:
        entries: ordered list of dicts with at least ``full_name`` and
                 ``catalog_id`` keys.
        name_to_id: maps ``"plugin.convention"`` to catalog_id.
        emit_id / stop_id: ids for EMIT and STOP (set after finalize).
        num_opcodes: total number of opcodes (conventions + EMIT + STOP).
    """

    def __init__(self):
        self.entries: list[dict] = []
        self.name_to_id: dict[str, int] = {}
        self.emit_id: int = -1
        self.stop_id: int = -1
        self.num_opcodes: int = 0

    # -- build ---------------------------------------------------------------

    def add_plugin(self, plugin_name: str, conventions: list[dict]):
        """Register all conventions exported by *plugin_name*.

        Each element of *conventions* must be a dict with at least a
        ``"name"`` key (e.g. ``"query"``).  Extra keys (description,
        args spec, ...) are preserved in the catalog entry.
        """
        for conv in conventions:
            full_name = f"{plugin_name}.{conv['name']}"
            cid = len(self.entries)
            entry = {"full_name": full_name, "catalog_id": cid, "plugin": plugin_name}
            entry.update(conv)
            self.entries.append(entry)
            self.name_to_id[full_name] = cid

    def finalize(self):
        """Append EMIT and STOP control opcodes and freeze the catalog."""
        self.emit_id = len(self.entries)
        self.entries.append({"full_name": "EMIT", "catalog_id": self.emit_id})
        self.name_to_id["EMIT"] = self.emit_id

        self.stop_id = len(self.entries)
        self.entries.append({"full_name": "STOP", "catalog_id": self.stop_id})
        self.name_to_id["STOP"] = self.stop_id

        self.num_opcodes = len(self.entries)

    # -- query ---------------------------------------------------------------

    def __len__(self) -> int:
        """Total number of opcodes (conventions + EMIT + STOP)."""
        return self.num_opcodes

    @property
    def num_conventions(self) -> int:
        """Number of *plugin* conventions (excludes EMIT/STOP)."""
        if self.emit_id < 0:
            return len(self.entries)
        return self.emit_id  # EMIT is first non-convention id

    def resolve(self, convention_name: str) -> int:
        """Return the catalog_id for a convention name.

        Accepts both ``"EMIT"`` / ``"STOP"`` and qualified names like
        ``"postgres.query"``.
        """
        if convention_name not in self.name_to_id:
            raise KeyError(f"Unknown convention: {convention_name!r}")
        return self.name_to_id[convention_name]

    def save(self, path: str):
        """Serialize the catalog to JSON."""
        with open(path, "w") as f:
            json.dump({
                "entries": self.entries,
                "emit_id": self.emit_id,
                "stop_id": self.stop_id,
            }, f, indent=2)

    @classmethod
    def load(cls, path: str) -> "ConventionCatalog":
        """Deserialize a catalog from JSON."""
        with open(path) as f:
            data = json.load(f)
        cat = cls()
        cat.entries = data["entries"]
        cat.emit_id = data["emit_id"]
        cat.stop_id = data["stop_id"]
        cat.num_opcodes = len(cat.entries)
        cat.name_to_id = {e["full_name"]: e["catalog_id"] for e in cat.entries}
        return cat


def build_catalog(plugin_dirs: list[str]) -> ConventionCatalog:
    """Build a convention catalog from plugin manifest files.

    Scans each directory for ``manifest.json`` and registers all
    conventions.  Returns a finalized catalog with EMIT/STOP appended.
    """
    catalog = ConventionCatalog()
    for plugin_dir in plugin_dirs:
        manifest_path = os.path.join(plugin_dir, "manifest.json")
        plugin_name = os.path.basename(plugin_dir)
        conventions: list[dict] = []

        if os.path.exists(manifest_path):
            with open(manifest_path) as f:
                manifest = json.load(f)
            plugin_name = manifest.get("plugin", {}).get("name", plugin_name)
            conventions = manifest.get("conventions", [])

        catalog.add_plugin(plugin_name, conventions)

    catalog.finalize()
    return catalog


# ---------------------------------------------------------------------------
# Training data collection
# ---------------------------------------------------------------------------

def _validate_conventions(data: dict, plugin_dir: str, available: set[str] | None = None):
    """Check that all conventions referenced in examples exist.

    *available* is the set of full convention names known so far.  If
    ``None``, validation is skipped (useful when collecting across
    plugins where cross-references are resolved later).
    """
    if available is None:
        return
    for ex in data.get("examples", []):
        for step in ex.get("program", []):
            conv = step.get("convention", "")
            if conv in ("EMIT", "STOP"):
                continue
            if conv not in available:
                raise ValueError(
                    f"Example {ex.get('id','?')} in {plugin_dir} references "
                    f"unknown convention {conv!r}"
                )


def collect_training_data(
    plugin_dirs: list[str],
    domain_file: str | None = None,
) -> list[dict]:
    """Collect training examples from all plugins and optional domain data.

    Args:
        plugin_dirs: directories, each containing a plugin with
            ``training/examples.json`` and optionally a ``manifest.json``.
        domain_file: optional path to a JSON file with domain-specific
            ``{"examples": [...]}`` training data.

    Returns:
        Flat list of example dicts in the format defined in
        03_PLUGINS.md Section 16.
    """
    all_examples: list[dict] = []

    for plugin_dir in plugin_dirs:
        plugin_name = os.path.basename(plugin_dir)
        manifest_path = os.path.join(plugin_dir, "manifest.json")
        if os.path.exists(manifest_path):
            with open(manifest_path) as f:
                manifest = json.load(f)
            plugin_name = manifest.get("plugin", {}).get("name", plugin_name)

        # --- training examples ---------------------------------------------
        training_file = os.path.join(plugin_dir, "training", "examples.json")
        if os.path.exists(training_file):
            with open(training_file) as f:
                data = json.load(f)
            # Tag each example with its source plugin
            for ex in data.get("examples", []):
                ex.setdefault("_plugin", data.get("plugin", plugin_name))
            all_examples.extend(data.get("examples", []))

    # --- domain-specific data (optional) -----------------------------------
    if domain_file and os.path.exists(domain_file):
        with open(domain_file) as f:
            domain_data = json.load(f)
        for ex in domain_data.get("examples", []):
            ex.setdefault("_plugin", "_domain")
        all_examples.extend(domain_data.get("examples", []))

    return all_examples


# ---------------------------------------------------------------------------
# Span finder (token-level subsequence match)
# ---------------------------------------------------------------------------

def find_span(tokens: list[str], param_tokens: list[str]) -> tuple[int, int] | None:
    """Find *param_tokens* as a contiguous subsequence of *tokens*.

    Returns ``(start, end)`` **inclusive** (0-based into *tokens*), or
    ``None`` if not found.
    """
    if not param_tokens:
        return None
    n = len(param_tokens)
    for i in range(len(tokens) - n + 1):
        if tokens[i:i + n] == param_tokens:
            return (i, i + n - 1)
    return None


# ---------------------------------------------------------------------------
# Template x param-pool expansion
# ---------------------------------------------------------------------------

def _product_of_pools(params: dict) -> list[dict]:
    """Cartesian product of named parameter pools.

    ``params`` maps param-name to ``{"pool": [...], ...}`` (or a plain list).
    Returns a list of dicts mapping param-name to a concrete value.
    """
    if not params:
        return [{}]

    names = list(params.keys())
    pools = []
    for name in names:
        spec = params[name]
        pool = spec["pool"] if isinstance(spec, dict) else spec
        pools.append(pool)

    combos = []
    for vals in product(*pools):
        combos.append(dict(zip(names, vals)))
    return combos


def _resolve_program(
    program_template: list[dict],
    param_values: dict,
    catalog: ConventionCatalog,
    intent_tokens: list[str],
    tokenize_fn,
) -> list[dict] | None:
    """Resolve a single program template into concrete training targets.

    Each step becomes a dict with keys:
        opcode, a0_type, a0_span_s, a0_span_e, a0_ref, a0_lit,
                a1_type, a1_span_s, a1_span_e, a1_ref, a1_lit

    Returns ``None`` if a required span cannot be found (skip this pair).
    """
    steps: list[dict] = []
    for step_def in program_template:
        conv_name = step_def["convention"]
        opcode = catalog.resolve(conv_name)

        resolved = {
            "opcode": opcode,
            "a0_type": ARG_NONE, "a0_span_s": -1, "a0_span_e": -1, "a0_ref": -1, "a0_lit": -1,
            "a1_type": ARG_NONE, "a1_span_s": -1, "a1_span_e": -1, "a1_ref": -1, "a1_lit": -1,
        }

        args = step_def.get("args", [])
        for arg_idx, arg in enumerate(args[:2]):  # at most 2 args per step
            prefix = f"a{arg_idx}_"
            arg_type = arg.get("type", "none")
            resolved[prefix + "type"] = _ARG_TYPE_MAP.get(arg_type, ARG_NONE)

            if arg_type == "span":
                # Extract the parameter name or literal span value
                extract = arg.get("extract", arg.get("value", ""))
                # Substitute param values if extract references a param
                if extract in param_values:
                    extract = str(param_values[extract])
                span_tokens = tokenize_fn(extract)
                sp = find_span(intent_tokens, span_tokens)
                if sp is None:
                    return None  # cannot locate span in this intent
                # +1 offset because token_ids get a NULL_IDX prepended at pos 0
                resolved[prefix + "span_s"] = sp[0] + 1
                resolved[prefix + "span_e"] = sp[1] + 1

            elif arg_type == "ref":
                resolved[prefix + "ref"] = arg.get("step", 0)

            elif arg_type == "literal":
                # Literal values are encoded as vocab token indices.
                # Store the raw value; encoding happens in SynthesisDataset.
                resolved[prefix + "lit"] = arg.get("value", "")

        steps.append(resolved)
    return steps


def expand_examples(
    examples: list[dict],
    catalog: ConventionCatalog,
    tokenize_fn=None,
    max_per_template: int | None = None,
    seed: int = 42,
) -> list[dict]:
    """Expand template x param-pool into concrete (intent, program) pairs.

    Args:
        examples: raw examples from ``collect_training_data``.
        catalog: finalized ``ConventionCatalog``.
        tokenize_fn: callable ``str -> list[str]`` for tokenizing intents
            (default: ``str.lower().split()``).
        max_per_template: if set, cap the number of param combinations
            sampled per intent template (useful for huge pools).
        seed: RNG seed for sampling when *max_per_template* is used.

    Returns:
        List of dicts with keys ``"intent"``, ``"tokens"``, ``"program"``,
        ``"_plugin"``.
    """
    if tokenize_fn is None:
        tokenize_fn = lambda s: s.lower().split()

    rng = random.Random(seed)
    pairs: list[dict] = []

    for ex in examples:
        param_spec = ex.get("params", {})
        combos = _product_of_pools(param_spec)

        # Optional cap on combinatorial explosion
        if max_per_template and len(combos) > max_per_template:
            combos = rng.sample(combos, max_per_template)

        intents = ex.get("intents", [])
        program_template = ex.get("program", [])
        plugin = ex.get("_plugin", "unknown")

        for intent_template in intents:
            for param_values in combos:
                # Substitute parameters into the intent string
                try:
                    intent = intent_template.format(**param_values)
                except KeyError:
                    continue

                tokens = tokenize_fn(intent)
                program = _resolve_program(
                    program_template, param_values, catalog, tokens, tokenize_fn,
                )
                if program is None:
                    continue

                pairs.append({
                    "intent": intent,
                    "tokens": tokens,
                    "program": program,
                    "_plugin": plugin,
                })

    return pairs


# ---------------------------------------------------------------------------
# Oversampling for zero-param conventions (Spec 5.2 balancing)
# ---------------------------------------------------------------------------

def oversample_zero_param(pairs: list[dict], factor: int = 8) -> list[dict]:
    """Duplicate examples whose programs have no span/literal args.

    Zero-param conventions (system info, time, ...) tend to have far fewer
    expanded examples.  Oversampling brings them closer to parity with
    parameterized conventions.
    """
    out: list[dict] = []
    for p in pairs:
        has_param = any(
            s["a0_type"] in (ARG_SPAN, ARG_LITERAL) or
            s["a1_type"] in (ARG_SPAN, ARG_LITERAL)
            for s in p["program"]
        )
        copies = 1 if has_param else factor
        for _ in range(copies):
            out.append(p)
    return out


# ---------------------------------------------------------------------------
# PyTorch Dataset
# ---------------------------------------------------------------------------

def _encode_literal(value, vocab: dict[str, int], unk_idx: int = 1) -> int:
    """Encode a literal value to a vocab index.

    If the value is a string, look it up in *vocab* (case-insensitive).
    Numeric / other types are converted to string first.
    """
    s = str(value).lower()
    return vocab.get(s, unk_idx)


class SynthesisDataset(Dataset):
    """PyTorch Dataset for SOMA Mind training.

    Each item is a dict with:
        - ``input_ids``:  LongTensor ``[L]`` — token indices (NULL prepended)
        - ``length``:     int — true length of input_ids
        - ``opcode``:     LongTensor ``[S]`` — target opcodes per step
        - ``a0_type``:    LongTensor ``[S]`` — arg0 type per step
        - ``a1_type``:    LongTensor ``[S]`` — arg1 type per step
        - ``a0_span_s``:  LongTensor ``[S]`` — arg0 span start (-1 = ignore)
        - ``a0_span_e``:  LongTensor ``[S]`` — arg0 span end
        - ``a1_span_s``:  LongTensor ``[S]`` — arg1 span start
        - ``a1_span_e``:  LongTensor ``[S]`` — arg1 span end
        - ``a0_ref``:     LongTensor ``[S]`` — arg0 ref pointer (-1 = ignore)
        - ``a1_ref``:     LongTensor ``[S]`` — arg1 ref pointer
        - ``a0_lit``:     LongTensor ``[S]`` — arg0 literal vocab index (-1 = ignore)
        - ``a1_lit``:     LongTensor ``[S]`` — arg1 literal vocab index (-1 = ignore)
    """

    STEP_KEYS = (
        "opcode", "a0_type", "a1_type",
        "a0_span_s", "a0_span_e", "a1_span_s", "a1_span_e",
        "a0_ref", "a1_ref", "a0_lit", "a1_lit",
    )

    def __init__(
        self,
        pairs: list[dict],
        encode_fn,
        max_steps: int = 16,
        stop_id: int | None = None,
        vocab: dict[str, int] | None = None,
    ):
        """
        Args:
            pairs: expanded (intent, program) pairs from ``expand_examples``.
            encode_fn: callable ``str -> list[int]`` that encodes an intent
                string into token indices.
            max_steps: maximum program length (shorter programs are STOP-padded,
                longer ones are truncated).
            stop_id: catalog_id for the STOP opcode (used for padding).
            vocab: word-to-index mapping for encoding literal arg values.
        """
        super().__init__()
        self.max_steps = max_steps
        self.stop_id = stop_id if stop_id is not None else 0
        self.vocab = vocab or {}
        self.items: list[dict] = []

        for pair in pairs:
            token_ids = [NULL_IDX] + encode_fn(pair["intent"])
            length = len(token_ids)

            # Resolve literal arg values to vocab indices
            steps = []
            for s in pair["program"][:max_steps]:
                step = dict(s)
                for prefix in ("a0_", "a1_"):
                    lit_key = prefix + "lit"
                    if step[prefix + "type"] == ARG_LITERAL and step[lit_key] != -1:
                        step[lit_key] = _encode_literal(step[lit_key], self.vocab)
                steps.append(step)

            # Pad to max_steps with STOP
            while len(steps) < max_steps:
                steps.append({
                    "opcode": self.stop_id,
                    "a0_type": ARG_NONE, "a0_span_s": -1, "a0_span_e": -1,
                    "a0_ref": -1, "a0_lit": -1,
                    "a1_type": ARG_NONE, "a1_span_s": -1, "a1_span_e": -1,
                    "a1_ref": -1, "a1_lit": -1,
                })

            self.items.append({
                "token_ids": token_ids,
                "length": length,
                "steps": steps,
            })

    def __len__(self) -> int:
        return len(self.items)

    def __getitem__(self, idx: int) -> dict:
        return self.items[idx]


# ---------------------------------------------------------------------------
# Collation (batching with dynamic padding)
# ---------------------------------------------------------------------------

def collate_fn(batch: list[dict]) -> dict:
    """Collate a list of dataset items into a padded batch dict.

    Returns a dict of tensors suitable for ``SomaMind.forward`` and
    ``SomaTrainer.compute_loss``.
    """
    max_len = max(item["length"] for item in batch)

    input_ids = torch.tensor(
        [item["token_ids"] + [PAD_IDX] * (max_len - item["length"]) for item in batch],
        dtype=torch.long,
    )
    lengths = torch.tensor([item["length"] for item in batch], dtype=torch.long)

    result = {"input_ids": input_ids, "lengths": lengths}

    for key in SynthesisDataset.STEP_KEYS:
        result[key] = torch.tensor(
            [[step[key] for step in item["steps"]] for item in batch],
            dtype=torch.long,
        )

    return result


# ---------------------------------------------------------------------------
# Data split utility
# ---------------------------------------------------------------------------

def split_data(
    pairs: list[dict],
    train_frac: float = 0.8,
    val_frac: float = 0.1,
    seed: int = 42,
) -> tuple[list[dict], list[dict], list[dict]]:
    """Shuffle and split expanded pairs into train / val / test."""
    rng = random.Random(seed)
    shuffled = list(pairs)
    rng.shuffle(shuffled)
    n = len(shuffled)
    n_train = int(n * train_frac)
    n_val = int(n * val_frac)
    return shuffled[:n_train], shuffled[n_train:n_train + n_val], shuffled[n_train + n_val:]
