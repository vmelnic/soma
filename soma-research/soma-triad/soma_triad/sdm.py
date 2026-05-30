"""
Sparse Distributed Memory — pure content-addressable store.

Zero learnable parameters. No query projection. Pure cosine similarity.
Write episodes at their encoded address, read by similarity-weighted blend.

Based on: Kanerva, "Sparse Distributed Memory" (MIT Press, 1988).
"""

import numpy as np


class SDM:
    """Content-addressable memory: write by address, read by similarity."""

    def __init__(self, dim: int = 10000, top_k: int = 8):
        self.dim = dim
        self.top_k = top_k
        self._addresses: list[np.ndarray] = []
        self._data: list[np.ndarray] = []
        self._labels: list[str] = []
        self._write_counts: list[int] = []

    def write(self, address: np.ndarray, data: np.ndarray, label: str = "") -> None:
        """Store an entry. If label exists, reinforce via running average."""
        address = self._normalize(address)

        if label:
            for i, existing_label in enumerate(self._labels):
                if existing_label == label:
                    n = self._write_counts[i]
                    new_n = n + 1
                    self._data[i] = (self._data[i] * n + data) / new_n
                    self._addresses[i] = self._normalize(
                        (self._addresses[i] * n + address) / new_n
                    )
                    self._write_counts[i] = new_n
                    return

        self._addresses.append(address)
        self._data.append(data.copy())
        self._labels.append(label)
        self._write_counts.append(1)

    def read(self, query: np.ndarray, top_k: int | None = None) -> list[tuple[np.ndarray, float, str]]:
        """Return top-k entries sorted by descending similarity. Each: (data, similarity, label)."""
        if not self._addresses:
            return []

        k = top_k or self.top_k
        query = self._normalize(query)

        similarities = []
        for addr in self._addresses:
            sim = float(np.dot(query, addr))
            similarities.append(sim)

        indices = np.argsort(similarities)[::-1][:k]
        return [
            (self._data[i], similarities[i], self._labels[i])
            for i in indices
        ]

    def read_blended(self, query: np.ndarray, top_k: int | None = None) -> np.ndarray | None:
        """Return similarity-weighted average of top-k entries. The core SDM generalization."""
        matches = self.read(query, top_k)
        if not matches:
            return None

        sims = np.array([m[1] for m in matches])
        max_sim = sims.max()
        exp_sims = np.exp((sims - max_sim) * 10.0)
        weights = exp_sims / exp_sims.sum()

        blended = np.zeros(self.dim, dtype=np.float32)
        for (data, _, _), w in zip(matches, weights):
            blended += data * w
        return blended

    def count(self) -> int:
        return len(self._addresses)

    def clear(self) -> None:
        self._addresses.clear()
        self._data.clear()
        self._labels.clear()
        self._write_counts.clear()

    def decay(self, factor: float) -> int:
        """Decay all write counts. Remove entries that reach zero. Return number removed."""
        survivors = []
        for i in range(len(self._write_counts)):
            new_count = int(self._write_counts[i] * factor)
            if new_count > 0:
                self._write_counts[i] = new_count
                survivors.append(i)

        removed = len(self._addresses) - len(survivors)
        if removed > 0:
            self._addresses = [self._addresses[i] for i in survivors]
            self._data = [self._data[i] for i in survivors]
            self._labels = [self._labels[i] for i in survivors]
            self._write_counts = [self._write_counts[i] for i in survivors]
        return removed

    def _normalize(self, v: np.ndarray) -> np.ndarray:
        mag = float(np.linalg.norm(v))
        if mag > 0:
            return (v / mag).astype(np.float32)
        return v.astype(np.float32)
