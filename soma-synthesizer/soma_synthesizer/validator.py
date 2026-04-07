"""
Training data validation -- checks for errors, conflicts, coverage,
duplicate examples, and invalid references.

Part of the Synthesis Pipeline (spec Section 13). Run before training
to catch data quality issues early:

    soma-synthesize validate --plugins ./plugins

Self-contained -- no imports from poc/ or pow/.
"""

from collections import Counter


class ValidationResult:
    """Accumulates errors (fatal) and warnings (non-fatal) from checks."""

    def __init__(self):
        self.errors: list[str] = []
        self.warnings: list[str] = []

    @property
    def valid(self) -> bool:
        """True when there are zero errors (warnings are acceptable)."""
        return len(self.errors) == 0

    def error(self, msg: str) -> None:
        self.errors.append(msg)

    def warn(self, msg: str) -> None:
        self.warnings.append(msg)

    def summary(self) -> str:
        """Human-readable summary suitable for CLI output."""
        lines: list[str] = []
        for e in self.errors:
            lines.append(f"  ERROR: {e}")
        for w in self.warnings:
            lines.append(f"  WARNING: {w}")
        status = "PASS" if self.valid else "FAIL"
        lines.append(
            f"  --- {status}: {len(self.errors)} error(s), "
            f"{len(self.warnings)} warning(s)"
        )
        return "\n".join(lines)


class TrainingDataValidator:
    """Validates a list of training examples against a convention catalog.

    Parameters
    ----------
    catalog : list[dict]
        Convention catalog as built by the data collector.  Each entry
        must have at least ``"full_name"`` (e.g. ``"postgres.query"``)
        and ``"catalog_id"``.  Built-in opcodes ``EMIT`` and ``STOP``
        should also appear.
    config : dict, optional
        Validation thresholds::

            max_imbalance_ratio   5     max ratio between largest and
                                        smallest plugin example count
            max_program_steps     16    programs longer than this are
                                        un-learnable
            max_intent_tokens     100   intents longer than this may
                                        exceed model capacity
    """

    def __init__(self, catalog: list[dict], config: dict | None = None):
        self.catalog = catalog

        # Set of valid convention names for quick lookup.
        self._valid_conventions: set[str] = {
            entry["full_name"] for entry in catalog
        }

        c = config or {}
        self.max_imbalance: float = c.get("max_imbalance_ratio", 5.0)
        self.max_steps: int = c.get("max_program_steps", 16)
        self.max_intent_tokens: int = c.get("max_intent_tokens", 100)

    # ------------------------------------------------------------------
    # Public interface
    # ------------------------------------------------------------------

    def validate(self, examples: list[dict]) -> ValidationResult:
        """Run all checks on *examples* and return a ValidationResult."""
        result = ValidationResult()
        self._check_conventions_exist(examples, result)
        self._check_conflicts(examples, result)
        self._check_refs(examples, result)
        self._check_coverage(examples, result)
        self._check_duplicates(examples, result)
        self._check_program_length(examples, result)
        self._check_intent_length(examples, result)
        return result

    # ------------------------------------------------------------------
    # Individual checks
    # ------------------------------------------------------------------

    def _check_conventions_exist(
        self, examples: list[dict], result: ValidationResult
    ) -> None:
        """Every convention referenced in programs must exist in the catalog."""
        for ex in examples:
            ex_id = ex.get("id", "<no id>")
            for step_idx, step in enumerate(ex.get("program", [])):
                conv = step.get("convention", "")
                if conv and conv not in self._valid_conventions:
                    result.error(
                        f"Example {ex_id} step {step_idx}: convention "
                        f"'{conv}' not found in catalog"
                    )

    def _check_conflicts(
        self, examples: list[dict], result: ValidationResult
    ) -> None:
        """Flag cases where the *same* intent text maps to *different* programs.

        Two examples conflict when they share at least one identical
        intent string but their programs differ.  This confuses the
        model during training.
        """
        # Map each intent string to (example_id, program_fingerprint).
        intent_map: dict[str, list[tuple[str, str]]] = {}
        for ex in examples:
            ex_id = ex.get("id", "<no id>")
            fp = _program_fingerprint(ex.get("program", []))
            for intent in ex.get("intents", []):
                key = intent.strip().lower()
                intent_map.setdefault(key, []).append((ex_id, fp))

        for intent_text, entries in intent_map.items():
            fingerprints = {fp for _, fp in entries}
            if len(fingerprints) > 1:
                ids = sorted({eid for eid, _ in entries})
                result.error(
                    f"CONFLICT: intent '{intent_text}' maps to different "
                    f"programs in examples {', '.join(ids)}"
                )

    def _check_refs(
        self, examples: list[dict], result: ValidationResult
    ) -> None:
        """Validate ref indices: must point to a *previous* step (no self-ref, no forward-ref)."""
        for ex in examples:
            ex_id = ex.get("id", "<no id>")
            program = ex.get("program", [])
            for step_idx, step in enumerate(program):
                for arg in step.get("args", []):
                    if arg.get("type") != "ref":
                        continue
                    ref_step = arg.get("step")
                    if ref_step is None:
                        result.error(
                            f"Example {ex_id} step {step_idx}: ref arg "
                            f"missing 'step' field"
                        )
                        continue
                    if not isinstance(ref_step, int):
                        result.error(
                            f"Example {ex_id} step {step_idx}: ref step "
                            f"must be an integer, got {type(ref_step).__name__}"
                        )
                        continue
                    if ref_step < 0:
                        result.error(
                            f"Example {ex_id} step {step_idx}: ref step "
                            f"{ref_step} is negative"
                        )
                    elif ref_step >= step_idx:
                        result.error(
                            f"Example {ex_id} step {step_idx}: ref step "
                            f"{ref_step} is not a previous step "
                            f"(self-ref or forward-ref)"
                        )

                    # Also check fallback_step if present.
                    fb = arg.get("fallback_step")
                    if fb is not None:
                        if not isinstance(fb, int) or fb < 0 or fb >= step_idx:
                            result.error(
                                f"Example {ex_id} step {step_idx}: "
                                f"fallback_step {fb} is invalid"
                            )

    def _check_coverage(
        self, examples: list[dict], result: ValidationResult
    ) -> None:
        """Check that training data is reasonably balanced across plugins.

        Also warns about conventions that have zero training examples.
        """
        # Count examples per plugin.
        plugin_counts: Counter[str] = Counter()
        covered_conventions: set[str] = set()

        for ex in examples:
            plugin = ex.get("plugin", _infer_plugin(ex))
            plugin_counts[plugin] += 1
            for step in ex.get("program", []):
                conv = step.get("convention", "")
                if conv:
                    covered_conventions.add(conv)

        # Imbalance check.
        if plugin_counts:
            counts = [c for c in plugin_counts.values() if c > 0]
            if counts:
                max_c = max(counts)
                min_c = min(counts)
                if min_c > 0 and max_c / min_c > self.max_imbalance:
                    largest = plugin_counts.most_common(1)[0]
                    smallest = plugin_counts.most_common()[-1]
                    result.warn(
                        f"IMBALANCE: {largest[0]} has {largest[1]} examples, "
                        f"{smallest[0]} has {smallest[1]} examples "
                        f"({largest[1] / smallest[1]:.0f}:1 ratio, "
                        f"recommend <={self.max_imbalance:.0f}:1)"
                    )

        # Zero-example conventions.
        # Exclude built-in opcodes from this check.
        builtins = {"EMIT", "STOP"}
        for entry in self.catalog:
            name = entry["full_name"]
            if name in builtins:
                continue
            if name not in covered_conventions:
                result.warn(
                    f"LOW COVERAGE: {name} has 0 training examples"
                )

    def _check_duplicates(
        self, examples: list[dict], result: ValidationResult
    ) -> None:
        """Detect identical (intents, program) pairs -- redundant data."""
        seen: dict[str, str] = {}  # fingerprint -> first example id
        for ex in examples:
            ex_id = ex.get("id", "<no id>")
            intents_sorted = tuple(
                sorted(i.strip().lower() for i in ex.get("intents", []))
            )
            prog_fp = _program_fingerprint(ex.get("program", []))
            key = f"{intents_sorted}|{prog_fp}"
            if key in seen:
                result.warn(
                    f"DUPLICATE: {ex_id} is identical to {seen[key]} "
                    f"after normalization (redundant)"
                )
            else:
                seen[key] = ex_id

    def _check_program_length(
        self, examples: list[dict], result: ValidationResult
    ) -> None:
        """Warn when a program has more steps than the model can learn."""
        for ex in examples:
            program = ex.get("program", [])
            if len(program) > self.max_steps:
                ex_id = ex.get("id", "<no id>")
                result.warn(
                    f"Example {ex_id}: program has {len(program)} steps "
                    f"(max learnable is {self.max_steps})"
                )

    def _check_intent_length(
        self, examples: list[dict], result: ValidationResult
    ) -> None:
        """Warn when an intent has more tokens than the model capacity."""
        for ex in examples:
            ex_id = ex.get("id", "<no id>")
            for intent in ex.get("intents", []):
                tokens = intent.split()
                if len(tokens) > self.max_intent_tokens:
                    result.warn(
                        f"Example {ex_id}: intent has {len(tokens)} tokens "
                        f"(>{self.max_intent_tokens}): "
                        f"'{intent[:60]}...'"
                    )


# ---------------------------------------------------------------------------
# Helpers (module-private)
# ---------------------------------------------------------------------------

def _program_fingerprint(program: list[dict]) -> str:
    """Produce a deterministic string fingerprint for a program.

    Two programs that are semantically identical (same conventions, same
    arg types and values in the same order) will have the same
    fingerprint.
    """
    parts: list[str] = []
    for step in program:
        conv = step.get("convention", "?")
        args_parts: list[str] = []
        for arg in step.get("args", []):
            atype = arg.get("type", "?")
            aval = str(arg.get("value", arg.get("step", arg.get("extract", ""))))
            args_parts.append(f"{atype}={aval}")
        parts.append(f"{conv}({','.join(args_parts)})")
    return "|".join(parts)


def _infer_plugin(example: dict) -> str:
    """Best-effort plugin name from an example's program conventions."""
    for step in example.get("program", []):
        conv = step.get("convention", "")
        if "." in conv:
            return conv.split(".")[0]
    return "_unknown"
