"""
SOMA Tokenizer — Word-level tokenizer with vocabulary management.

Part of Layer 1 (Intent Reception). Converts raw human text into
token indices that the Mind can process.

Self-contained — no imports from poc/ or pow/.
"""

import json


# ---------------------------------------------------------------------------
# Special tokens: PAD for padding, UNK for unknown words, NULL for empty
# parameter spans (used by encode_with_null to prepend a null marker).
# ---------------------------------------------------------------------------
PAD_TOKEN = "<PAD>"
UNK_TOKEN = "<UNK>"
NULL_TOKEN = "<NULL>"
SPECIAL_TOKENS = [PAD_TOKEN, UNK_TOKEN, NULL_TOKEN]

PAD_IDX = 0
UNK_IDX = 1
NULL_IDX = 2


class Tokenizer:
    """Word-level tokenizer with special-token support.

    Usage::

        tok = Tokenizer()
        tok.build_vocab(["create file foo.txt", "delete file bar.txt"])
        ids = tok.encode("create file foo.txt")
        words = tok.decode_indices(ids)
    """

    def __init__(self, max_vocab_size=None):
        self.max_vocab_size = max_vocab_size
        self.word2idx: dict[str, int] = {}
        self.idx2word: dict[int, str] = {}
        self._vocab_built = False

    # ------------------------------------------------------------------
    # Vocabulary construction
    # ------------------------------------------------------------------

    def build_vocab(self, corpus: list[str]) -> None:
        """Build vocabulary from a list of intent strings.

        Special tokens are always placed at indices 0-2 (PAD, UNK, NULL).
        Remaining words are assigned incrementing indices in order of first
        appearance across the corpus.
        """
        self.word2idx = {}
        self.idx2word = {}

        # Reserve special tokens at fixed indices
        for i, tok in enumerate(SPECIAL_TOKENS):
            self.word2idx[tok] = i
            self.idx2word[i] = tok

        idx = len(SPECIAL_TOKENS)
        for text in corpus:
            for token in self.tokenize(text):
                if token not in self.word2idx:
                    self.word2idx[token] = idx
                    self.idx2word[idx] = token
                    idx += 1

        # Embedded tokenizer: prune to max_vocab_size (Sec 10.3)
        if self.max_vocab_size and len(self.word2idx) > self.max_vocab_size:
            # Count token frequencies
            freq = {}
            for text in corpus:
                for token in self.tokenize(text):
                    freq[token] = freq.get(token, 0) + 1
            # Keep special tokens + most frequent
            keep = set(SPECIAL_TOKENS)
            sorted_tokens = sorted(freq.items(), key=lambda x: -x[1])
            for tok, _ in sorted_tokens:
                if len(keep) >= self.max_vocab_size:
                    break
                keep.add(tok)
            # Rebuild
            self.word2idx = {t: i for i, t in enumerate(sorted(keep, key=lambda t: self.word2idx.get(t, 999999)))}
            self.idx2word = {v: k for k, v in self.word2idx.items()}

        self._vocab_built = True

    # ------------------------------------------------------------------
    # Tokenisation & encoding
    # ------------------------------------------------------------------

    def tokenize(self, text: str) -> list[str]:
        """Split text into word tokens.

        Lowercases and splits on whitespace. Preserves paths
        (e.g. ``/Users/vm/docs``) and other contiguous strings as single
        tokens.
        """
        return text.lower().split()

    def encode(self, text: str) -> list[int]:
        """Convert *text* to a list of token indices.

        Unknown words map to ``UNK_IDX`` (1).
        """
        return [self.word2idx.get(t, UNK_IDX) for t in self.tokenize(text)]

    def encode_with_null(self, text: str) -> list[int]:
        """Encode *text* and prepend a ``NULL`` token (index 2).

        Useful as a sentinel/start marker for decoder inputs or for
        representing empty parameter slots.
        """
        return [NULL_IDX] + self.encode(text)

    def decode_indices(self, indices: list[int]) -> list[str]:
        """Convert token indices back to word strings.

        Unknown indices map to the ``UNK_TOKEN`` string.
        """
        return [self.idx2word.get(i, UNK_TOKEN) for i in indices]

    # ------------------------------------------------------------------
    # Properties
    # ------------------------------------------------------------------

    @property
    def vocab_size(self) -> int:
        """Number of entries in the vocabulary (including special tokens)."""
        return len(self.word2idx)

    # ------------------------------------------------------------------
    # Persistence
    # ------------------------------------------------------------------

    def save(self, path: str) -> None:
        """Save the word→index mapping to a JSON file."""
        with open(path, "w") as f:
            json.dump(self.word2idx, f, indent=2)

    def load(self, path: str) -> None:
        """Load a word→index mapping from a JSON file.

        Rebuilds the reverse index automatically.
        """
        with open(path, "r") as f:
            self.word2idx = json.load(f)
        self.idx2word = {int(v): k for k, v in self.word2idx.items()}
        self._vocab_built = True


class BPETokenizer:
    """Byte-Pair Encoding tokenizer with character-level fallback (Spec 10.2).

    Handles: natural language, SQL strings, file paths, URLs, multilingual text.
    Trains on the training corpus to learn common subword merges.
    """

    def __init__(self, vocab_size=4000):
        self.vocab_size = vocab_size
        self.merges = []  # list of (pair, merged) tuples
        self.word2idx = {}
        self.idx2word = {}

    def train(self, corpus):
        """Train BPE from a list of texts.
        1. Start with character-level vocabulary
        2. Repeatedly merge most frequent adjacent pair
        3. Stop when vocab_size reached
        """
        # Build initial char-level vocab
        self.word2idx = {}
        for i, tok in enumerate(SPECIAL_TOKENS):
            self.word2idx[tok] = i

        # Collect all unique characters
        chars = set()
        for text in corpus:
            chars.update(text.lower())

        idx = len(SPECIAL_TOKENS)
        for c in sorted(chars):
            if c not in self.word2idx:
                self.word2idx[c] = idx
                idx += 1

        # Tokenize corpus to char sequences
        word_freqs = {}
        for text in corpus:
            tokens = tuple(text.lower())
            word_freqs[tokens] = word_freqs.get(tokens, 0) + 1

        # BPE merge loop
        while len(self.word2idx) < self.vocab_size:
            # Count pair frequencies
            pair_freqs = {}
            for word, freq in word_freqs.items():
                for i in range(len(word) - 1):
                    pair = (word[i], word[i + 1])
                    pair_freqs[pair] = pair_freqs.get(pair, 0) + freq

            if not pair_freqs:
                break

            best_pair = max(pair_freqs, key=pair_freqs.get)
            merged = best_pair[0] + best_pair[1]
            self.merges.append((best_pair, merged))

            if merged not in self.word2idx:
                self.word2idx[merged] = len(self.word2idx)

            # Apply merge to all words
            new_word_freqs = {}
            for word, freq in word_freqs.items():
                new_word = []
                i = 0
                while i < len(word):
                    if i < len(word) - 1 and (word[i], word[i + 1]) == best_pair:
                        new_word.append(merged)
                        i += 2
                    else:
                        new_word.append(word[i])
                        i += 1
                new_word_freqs[tuple(new_word)] = freq
            word_freqs = new_word_freqs

        self.idx2word = {v: k for k, v in self.word2idx.items()}

    def tokenize(self, text):
        """Tokenize text using learned BPE merges."""
        tokens = list(text.lower())
        for (a, b), merged in self.merges:
            new_tokens = []
            i = 0
            while i < len(tokens):
                if i < len(tokens) - 1 and tokens[i] == a and tokens[i + 1] == b:
                    new_tokens.append(merged)
                    i += 2
                else:
                    new_tokens.append(tokens[i])
                    i += 1
            tokens = new_tokens
        return tokens

    def encode(self, text):
        return [self.word2idx.get(t, UNK_IDX) for t in self.tokenize(text)]

    def encode_with_null(self, text):
        return [NULL_IDX] + self.encode(text)

    @property
    def vocab_size_actual(self):
        return len(self.word2idx)

    def save(self, path):
        data = {"word2idx": self.word2idx, "merges": [(list(p), m) for p, m in self.merges]}
        with open(path, "w") as f:
            json.dump(data, f)

    def load(self, path):
        with open(path) as f:
            data = json.load(f)
        self.word2idx = data["word2idx"]
        self.merges = [(tuple(p), m) for p, m in data["merges"]]
        self.idx2word = {int(v): k for k, v in self.word2idx.items()}


# -----------------------------------------------------------------------
# Span search utility
# -----------------------------------------------------------------------

def find_span(tokens: list[str], param_tokens: list[str]) -> tuple[int, int] | None:
    """Find the start/end indices of *param_tokens* within *tokens*.

    Returns ``(start, end)`` inclusive, or ``None`` if not found.  This is
    used to locate argument spans in the encoder's token sequence so the
    model can learn to point at them.
    """
    if not param_tokens:
        return None
    n = len(param_tokens)
    for i in range(len(tokens) - n + 1):
        if tokens[i : i + n] == param_tokens:
            return (i, i + n - 1)
    return None
