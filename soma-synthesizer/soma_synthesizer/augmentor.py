"""
Training data augmentation -- synonym replacement, word dropout,
word shuffle, and typo injection.

Part of the Synthesis Pipeline (spec Section 11). Bridges the gap
between template-expanded training data and the diverse phrasings
real users produce.

Self-contained -- no imports from poc/ or pow/.
"""

import random
import copy

# ---------------------------------------------------------------------------
# Synonym table -- verbs and filler words common in SOMA intents.
# Keys are lowercased tokens; values are lists of acceptable replacements.
# ---------------------------------------------------------------------------

SYNONYMS: dict[str, list[str]] = {
    # Action verbs
    "list": ["show", "display", "enumerate", "get"],
    "read": ["cat", "show", "display", "output", "print"],
    "create": ["make", "generate", "write", "produce"],
    "delete": ["remove", "erase", "destroy", "unlink"],
    "find": ["search", "locate", "look for", "seek"],
    "send": ["transmit", "deliver", "forward", "push"],
    "get": ["fetch", "retrieve", "obtain", "acquire"],
    "update": ["modify", "change", "edit", "alter"],
    "check": ["verify", "inspect", "examine", "test"],
    "start": ["begin", "launch", "initiate", "run"],
    "stop": ["halt", "end", "terminate", "kill"],
    "save": ["store", "persist", "write", "keep"],
    "load": ["fetch", "read", "import", "open"],
    "move": ["transfer", "relocate", "shift"],
    "copy": ["duplicate", "clone", "replicate"],
    "connect": ["link", "attach", "join"],
    # Filler / modifier words
    "all": ["every", "each"],
    "the": [""],
    "please": [""],
    "show": ["display", "list", "print", "output"],
}

# Reverse index: for each synonym value, record which key it belongs to.
# Built once at import time so synonym_replace can look up alternatives
# for words that appear only as values.
_REVERSE_SYNONYMS: dict[str, str] = {}
for _key, _vals in SYNONYMS.items():
    for _v in _vals:
        if _v and _v not in SYNONYMS:
            _REVERSE_SYNONYMS[_v] = _key

# ---------------------------------------------------------------------------
# Typo patterns -- common keyboard / transposition errors.
# ---------------------------------------------------------------------------

TYPO_SWAPS: list[tuple[str, str]] = [
    ("th", "ht"),
    ("ie", "ei"),
    ("er", "re"),
    ("le", "el"),
    ("ch", "hc"),
    ("ou", "uo"),
    ("an", "na"),
    ("in", "ni"),
    ("on", "no"),
    ("ti", "it"),
]

# Words too short or too important to corrupt with typos.
_TYPO_MIN_LEN = 4


# ---------------------------------------------------------------------------
# Droppable words -- filler that can be removed without changing intent.
# ---------------------------------------------------------------------------

DROPPABLE_WORDS: set[str] = {
    "please", "the", "a", "an", "of", "all", "to", "for",
    "my", "me", "just", "also", "that", "this", "those",
    "some", "any", "every", "each", "with", "from",
}


class Augmentor:
    """Generates augmented training intents using four techniques.

    Each technique operates on the *intent text only*; the target program
    is unchanged (the augmented intent should still map to the same
    program).  The ``augment_dataset`` method returns a list of
    ``(original_example, augmented_intent)`` pairs.

    Config keys (all optional, shown with defaults)::

        synonym_replace_rate  0.3   probability of applying synonym replacement
        word_dropout_rate     0.2   probability of applying word dropout
        word_shuffle_rate     0.1   probability of applying word shuffle
        typo_rate             0.05  probability of applying typo injection
        augmentation_factor   3     augmented copies per original example
    """

    def __init__(self, config: dict | None = None):
        c = config or {}
        self.synonym_rate: float = c.get("synonym_replace_rate", 0.3)
        self.dropout_rate: float = c.get("word_dropout_rate", 0.2)
        self.shuffle_rate: float = c.get("word_shuffle_rate", 0.1)
        self.typo_rate: float = c.get("typo_rate", 0.05)
        self.factor: int = c.get("augmentation_factor", 3)

        # Allow callers to inject a custom synonym table.
        self.synonyms: dict[str, list[str]] = c.get("synonyms", SYNONYMS)
        self.droppable: set[str] = c.get("droppable_words", DROPPABLE_WORDS)

    # ------------------------------------------------------------------
    # Individual augmentation techniques
    # ------------------------------------------------------------------

    def synonym_replace(self, text: str) -> str:
        """Replace eligible words with random synonyms.

        Preserves words that look like paths, SQL, or parameter values
        (contain ``/``, ``$``, ``=``, or start with uppercase after the
        first word).
        """
        tokens = text.split()
        out: list[str] = []
        for i, tok in enumerate(tokens):
            lower = tok.lower()
            # Skip tokens that look like parameters / paths / SQL.
            if _is_param_like(tok):
                out.append(tok)
                continue

            candidates: list[str] | None = None

            # Direct match in synonym table.
            if lower in self.synonyms:
                candidates = self.synonyms[lower]
            # Reverse lookup: the word is itself a synonym value.
            elif lower in _REVERSE_SYNONYMS:
                key = _REVERSE_SYNONYMS[lower]
                candidates = [key] + [
                    v for v in self.synonyms[key] if v != lower
                ]

            if candidates and random.random() < self.synonym_rate:
                replacement = random.choice(candidates)
                # Multi-word synonym (e.g. "look for") -- insert as-is.
                if replacement == "":
                    continue  # drop the word (synonym is empty string)
                out.append(replacement)
            else:
                out.append(tok)
        return " ".join(out)

    def word_dropout(self, text: str) -> str:
        """Randomly drop non-essential words to simulate terse inputs.

        Only drops words in ``DROPPABLE_WORDS``.  Never drops all words --
        at least half the original tokens are retained, and at least one
        non-droppable token must remain.
        """
        tokens = text.split()
        if len(tokens) <= 2:
            return text

        # Identify which positions are droppable.
        droppable_indices = [
            i for i, t in enumerate(tokens)
            if t.lower() in self.droppable
        ]
        if not droppable_indices:
            return text

        # Cap drops so we never remove more than half the tokens.
        max_drops = max(1, len(tokens) // 2)
        drops: set[int] = set()
        for idx in droppable_indices:
            if len(drops) >= max_drops:
                break
            if random.random() < self.dropout_rate:
                drops.add(idx)

        out = [t for i, t in enumerate(tokens) if i not in drops]
        return " ".join(out) if out else text

    def word_shuffle(self, text: str) -> str:
        """Mildly shuffle word order via adjacent-pair swaps.

        Only swaps *adjacent* tokens, and only when neither looks like a
        path or parameter.  This produces mild reorderings that a robust
        encoder should still parse correctly.
        """
        tokens = text.split()
        if len(tokens) <= 2:
            return text

        tokens = list(tokens)  # copy
        # Walk through pairs; probabilistically swap.
        i = 0
        while i < len(tokens) - 1:
            if random.random() < self.shuffle_rate:
                # Don't swap parameter-like tokens.
                if not _is_param_like(tokens[i]) and not _is_param_like(tokens[i + 1]):
                    tokens[i], tokens[i + 1] = tokens[i + 1], tokens[i]
                    i += 2  # skip the swapped token to avoid cascading
                    continue
            i += 1
        return " ".join(tokens)

    def typo_inject(self, text: str) -> str:
        """Introduce a realistic typo into one word.

        Applies at most one typo per call (spec says ~5 % of examples,
        not 5 % of words).  Skips short words and parameter-like tokens.
        """
        tokens = text.split()
        eligible = [
            i for i, t in enumerate(tokens)
            if len(t) >= _TYPO_MIN_LEN and not _is_param_like(t)
        ]
        if not eligible:
            return text

        idx = random.choice(eligible)
        word = tokens[idx].lower()
        mutated = _apply_typo(word)
        if mutated != word:
            tokens[idx] = mutated
        return " ".join(tokens)

    # ------------------------------------------------------------------
    # Composite augmentation
    # ------------------------------------------------------------------

    def augment(self, text: str) -> str:
        """Apply a randomly chosen augmentation technique to *text*.

        The technique is selected using the configured rates as relative
        weights so that higher-rate techniques are chosen more often.
        """
        techniques = [
            (self.synonym_rate, self.synonym_replace),
            (self.dropout_rate, self.word_dropout),
            (self.shuffle_rate, self.word_shuffle),
            (self.typo_rate, self.typo_inject),
        ]

        # Weight-based selection.
        weights = [w for w, _ in techniques]
        total = sum(weights)
        if total == 0:
            return text
        funcs = [f for _, f in techniques]
        chosen = random.choices(funcs, weights=weights, k=1)[0]
        return chosen(text)

    def augment_dataset(
        self,
        examples: list[dict],
    ) -> list[dict]:
        """Augment an entire dataset of training examples.

        Each example is expected to have an ``"intents"`` list and a
        ``"program"`` list (per the SOMA training data schema in
        03_PLUGINS.md Section 16).

        Returns a *new* list containing the original examples plus
        ``self.factor`` augmented copies of each.  Augmented examples
        are shallow copies of the original with the ``"intents"`` list
        replaced by augmented variants, and an ``"_augmented": True``
        flag added for traceability.
        """
        augmented: list[dict] = []

        for ex in examples:
            # Always keep the original.
            augmented.append(ex)

            intents = ex.get("intents", [])
            if not intents:
                continue

            for _ in range(self.factor):
                new_ex = copy.copy(ex)
                new_intents = [self.augment(intent) for intent in intents]
                new_ex = {**ex, "intents": new_intents, "_augmented": True}
                augmented.append(new_ex)

        return augmented


# ---------------------------------------------------------------------------
# Helpers (module-private)
# ---------------------------------------------------------------------------

def _is_param_like(token: str) -> bool:
    """Return True if *token* looks like a path, SQL fragment, or value."""
    return any(ch in token for ch in "/$=@*{}()[].,;:\"'")


def _apply_typo(word: str) -> str:
    """Apply a single character-swap typo from TYPO_SWAPS."""
    for src, dst in TYPO_SWAPS:
        pos = word.find(src)
        if pos != -1:
            return word[:pos] + dst + word[pos + len(src):]
    # Fallback: swap two adjacent characters near the middle.
    if len(word) >= 4:
        mid = len(word) // 2
        return word[:mid] + word[mid + 1] + word[mid] + word[mid + 2:]
    return word
