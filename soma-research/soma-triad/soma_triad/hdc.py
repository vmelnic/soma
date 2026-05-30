"""
Hyperdimensional Computing — pure algebraic operations over high-dimensional vectors.

Zero learnable parameters. The codebook is generated deterministically from a seed.
Operations: bind, bundle, permute, unbind, sequence encoding, role-filler structures.

Based on: Kanerva (2009), Rahimi et al. (2020), Kleyko et al. (2023).
"""

import numpy as np


class HDC:
    """Hyperdimensional computing algebra with a fixed codebook."""

    def __init__(self, dim: int = 10000, seed: int = 42):
        self.dim = dim
        self.rng = np.random.default_rng(seed)
        self._codebook: dict[str, np.ndarray] = {}

    def random_hv(self) -> np.ndarray:
        """Generate a random bipolar hypervector (+1/-1)."""
        return self.rng.choice([-1.0, 1.0], size=self.dim).astype(np.float32)

    def get_symbol(self, name: str) -> np.ndarray:
        """Get or create a deterministic hypervector for a named symbol."""
        if name not in self._codebook:
            self._codebook[name] = self.random_hv()
        return self._codebook[name]

    def bind(self, a: np.ndarray, b: np.ndarray) -> np.ndarray:
        """Bind two hypervectors (element-wise multiply). Result is dissimilar to both inputs."""
        return a * b

    def unbind(self, bound: np.ndarray, key: np.ndarray) -> np.ndarray:
        """Unbind: recover the other vector from a binding. For bipolar, bind is its own inverse."""
        return bound * key

    def bundle(self, vectors: list[np.ndarray]) -> np.ndarray:
        """Bundle multiple hypervectors (element-wise sum + sign). Result is similar to all inputs."""
        summed = np.sum(vectors, axis=0)
        result = np.sign(summed)
        result[result == 0] = 1.0
        return result.astype(np.float32)

    def permute(self, v: np.ndarray, shifts: int = 1) -> np.ndarray:
        """Permute a hypervector (circular shift). Creates a dissimilar but reversible vector."""
        return np.roll(v, shifts).astype(np.float32)

    def inverse_permute(self, v: np.ndarray, shifts: int = 1) -> np.ndarray:
        """Reverse a permutation."""
        return np.roll(v, -shifts).astype(np.float32)

    def encode_sequence(self, symbols: list[str]) -> np.ndarray:
        """Encode an ordered sequence as: permute(s0, n-1) * permute(s1, n-2) * ... * sn bundled."""
        n = len(symbols)
        if n == 0:
            return np.zeros(self.dim, dtype=np.float32)
        positional = []
        for i, sym in enumerate(symbols):
            hv = self.get_symbol(sym)
            shifted = self.permute(hv, n - 1 - i)
            positional.append(shifted)
        return self.bundle(positional)

    def encode_record(self, fields: dict[str, str]) -> np.ndarray:
        """Encode a role-filler structure: bundle of bind(role, filler) pairs."""
        pairs = []
        for role, filler in fields.items():
            role_hv = self.get_symbol(f"role:{role}")
            filler_hv = self.get_symbol(filler)
            pairs.append(self.bind(role_hv, filler_hv))
        return self.bundle(pairs)

    def query_record(self, record: np.ndarray, role: str) -> np.ndarray:
        """Query a record for a specific role's filler. Unbind the role, then match against codebook."""
        role_hv = self.get_symbol(f"role:{role}")
        return self.unbind(record, role_hv)

    def similarity(self, a: np.ndarray, b: np.ndarray) -> float:
        """Cosine similarity between two hypervectors."""
        dot = float(np.dot(a, b))
        mag_a = float(np.linalg.norm(a))
        mag_b = float(np.linalg.norm(b))
        if mag_a == 0 or mag_b == 0:
            return 0.0
        return dot / (mag_a * mag_b)

    def best_match(self, query: np.ndarray, candidates: list[str] | None = None) -> tuple[str, float]:
        """Find the codebook entry most similar to the query vector."""
        search_space = candidates if candidates else list(self._codebook.keys())
        best_name = ""
        best_sim = -1.0
        for name in search_space:
            if name not in self._codebook:
                continue
            sim = self.similarity(query, self._codebook[name])
            if sim > best_sim:
                best_sim = sim
                best_name = name
        return best_name, best_sim

    @property
    def codebook_size(self) -> int:
        return len(self._codebook)
