"""Tokenizer wrapper around tiktoken for SOMA Brain."""

import tiktoken


class Tokenizer:
    def __init__(self, encoding_name: str = "cl100k_base"):
        self.enc = tiktoken.get_encoding(encoding_name)
        self.vocab_size = self.enc.n_vocab
        self._orig_to_restricted = None
        self._restricted_to_orig = None

    def restrict(self, allowed_ids: list[int]) -> "Tokenizer":
        """Restrict output vocab to a subset of token IDs."""
        allowed_ids = sorted(set(allowed_ids))
        self._restricted_to_orig = allowed_ids
        self._orig_to_restricted = {orig: i for i, orig in enumerate(allowed_ids)}
        self.vocab_size = len(allowed_ids)
        return self

    def encode(self, text: str) -> list[int]:
        ids = self.enc.encode(text, allowed_special="all")
        if self._orig_to_restricted is not None:
            ids = [self._orig_to_restricted.get(i, 0) for i in ids]
        return ids

    def decode(self, tokens: list[int]) -> str:
        if self._restricted_to_orig is not None:
            tokens = [
                self._restricted_to_orig[t]
                if t < len(self._restricted_to_orig)
                else 0
                for t in tokens
            ]
        return self.enc.decode(tokens)

    def chunk(self, text: str, max_len: int) -> list[list[int]]:
        """Tokenize and split into fixed-length chunks."""
        ids = self.encode(text)
        return [ids[i : i + max_len] for i in range(0, len(ids), max_len)]
