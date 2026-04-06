"""
SOMA Tokenizer — Word-level tokenizer with vocabulary management.

Part of Layer 1 (Intent Reception). Converts raw human text into
token indices that the Mind can process.
"""

import json
import re


# Special tokens: PAD for padding, UNK for unknown words, NULL for empty parameter spans
PAD_TOKEN = "<PAD>"
UNK_TOKEN = "<UNK>"
NULL_TOKEN = "<NULL>"
SPECIAL_TOKENS = [PAD_TOKEN, UNK_TOKEN, NULL_TOKEN]

PAD_IDX = 0
UNK_IDX = 1
NULL_IDX = 2


class Tokenizer:

    def __init__(self):
        self.word2idx: dict[str, int] = {}
        self.idx2word: dict[int, str] = {}
        self._vocab_built = False

    def build_vocab(self, corpus: list[str]):
        """Build vocabulary from a list of intent strings."""
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

        self._vocab_built = True

    def tokenize(self, text: str) -> list[str]:
        """Split text into tokens. Lowercase, split on whitespace.
        Preserves paths (e.g. /Users/vm/docs) as single tokens."""
        return text.lower().split()

    def encode(self, text: str) -> list[int]:
        """Convert text to list of token indices."""
        return [self.word2idx.get(t, UNK_IDX) for t in self.tokenize(text)]

    def decode_indices(self, indices: list[int]) -> list[str]:
        """Convert token indices back to strings."""
        return [self.idx2word.get(i, UNK_TOKEN) for i in indices]

    @property
    def vocab_size(self) -> int:
        return len(self.word2idx)

    def save(self, path: str):
        """Save vocabulary to JSON."""
        with open(path, "w") as f:
            json.dump(self.word2idx, f)

    def load(self, path: str):
        """Load vocabulary from JSON."""
        with open(path, "r") as f:
            self.word2idx = json.load(f)
        self.idx2word = {int(v): k for k, v in self.word2idx.items()}
        self._vocab_built = True


def find_span(tokens: list[str], param_tokens: list[str]) -> tuple[int, int] | None:
    """Find the start/end indices of param_tokens within tokens.
    Returns (start, end) inclusive, or None if not found."""
    if not param_tokens:
        return None
    n = len(param_tokens)
    for i in range(len(tokens) - n + 1):
        if tokens[i:i + n] == param_tokens:
            return (i, i + n - 1)
    return None
