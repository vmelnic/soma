"""
Sparse Distributed Memory (SDM).

Content-addressable memory: read and write by similarity, not by address.
Knowledge lives in RAM as dense vectors. Dynamic allocation — one location
per ingested chunk, no fixed cap, no collisions.

Read activates only the nearest locations (sparse), retrieves their content,
and returns a weighted sum.

Based on: Kanerva, "Sparse Distributed Memory" (MIT Press, 1988).
"""

import torch
import torch.nn as nn
import torch.nn.functional as F


class SparseDistributedMemory(nn.Module):

    def __init__(
        self,
        address_size: int,
        data_size: int,
        top_k: int = 8,
    ):
        super().__init__()
        self.address_size = address_size
        self.data_size = data_size
        self.top_k = top_k

        self._addr_buf: torch.Tensor | None = None
        self._data_buf: torch.Tensor | None = None
        self._count: int = 0

        self.query_proj = nn.Linear(address_size, address_size)
        nn.init.eye_(self.query_proj.weight)
        nn.init.zeros_(self.query_proj.bias)

    @property
    def num_locations(self) -> int:
        return self._count

    @property
    def addresses(self) -> list[torch.Tensor]:
        if self._addr_buf is None:
            return []
        return [self._addr_buf[i] for i in range(self._count)]

    @addresses.setter
    def addresses(self, val: list[torch.Tensor]):
        if not val:
            self._addr_buf = None
            self._count = 0
        else:
            self._addr_buf = torch.stack(val)
            self._count = len(val)

    @property
    def entries(self) -> list[torch.Tensor]:
        if self._data_buf is None:
            return []
        return [self._data_buf[i] for i in range(self._count)]

    @entries.setter
    def entries(self, val: list[torch.Tensor]):
        if not val:
            self._data_buf = None
        else:
            self._data_buf = torch.stack(val)

    def _ensure_capacity(self, n: int) -> None:
        if self._addr_buf is not None and self._addr_buf.shape[0] >= n:
            return
        new_cap = max(n, (self._count or 1) * 2)
        new_addr = torch.zeros(new_cap, self.address_size)
        new_data = torch.zeros(new_cap, self.data_size)
        if self._addr_buf is not None and self._count > 0:
            new_addr[:self._count] = self._addr_buf[:self._count]
            new_data[:self._count] = self._data_buf[:self._count]
        self._addr_buf = new_addr
        self._data_buf = new_data

    def read(self, query: torch.Tensor) -> torch.Tensor:
        """Return weighted sum of top-k entries."""
        entries, scores = self.read_topk(query)
        if entries is None:
            return torch.zeros(query.shape[0], self.data_size, device=query.device)
        weights = F.softmax(scores * 10.0, dim=-1)
        return torch.einsum("bk,bkd->bd", weights, entries)

    def read_topk(self, query: torch.Tensor) -> tuple[torch.Tensor, torch.Tensor]:
        """Return top-k entries and their raw similarity scores."""
        if self._count == 0:
            k = self.top_k
            return (
                torch.zeros(query.shape[0], k, self.data_size, device=query.device),
                torch.zeros(query.shape[0], k, device=query.device),
            )

        addr_matrix = self._addr_buf[:self._count].to(query.device)
        data_matrix = self._data_buf[:self._count].to(query.device)

        q = F.normalize(self.query_proj(query), dim=-1)
        addr_norm = F.normalize(addr_matrix, dim=-1)
        sim = torch.matmul(q, addr_norm.T)

        k = min(self.top_k, self._count)
        topk_sim, topk_idx = torch.topk(sim, k, dim=-1)
        retrieved = data_matrix[topk_idx]

        if k < self.top_k:
            pad_k = self.top_k - k
            retrieved = F.pad(retrieved, (0, 0, 0, pad_k))
            topk_sim = F.pad(topk_sim, (0, pad_k))

        return retrieved, topk_sim

    @torch.no_grad()
    def write(self, address: torch.Tensor, data: torch.Tensor) -> None:
        """Append new entries. One location per write."""
        n = address.shape[0]
        self._ensure_capacity(self._count + n)
        for b in range(n):
            self._addr_buf[self._count] = F.normalize(address[b].detach().cpu(), dim=-1)
            self._data_buf[self._count] = data[b].detach().cpu()
            self._count += 1

    def clear(self) -> None:
        self._addr_buf = None
        self._data_buf = None
        self._count = 0
