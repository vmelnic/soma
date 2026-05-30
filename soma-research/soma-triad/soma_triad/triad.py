"""
Triad — the orchestrator that wires LTC + HDC + SDM into a reasoning loop.

The controller (LTC) decides operations. HDC performs composition. SDM stores/retrieves knowledge.
The triad runs until the controller emits EMIT or exhausts the step budget.
"""

import numpy as np
import torch

from soma_triad.hdc import HDC
from soma_triad.sdm import SDM
from soma_triad.controller import Controller, Operation


class TriadStep:
    """Record of one step in the reasoning trace."""

    def __init__(self, op: int, op_name: str, result_sim: float = 0.0, detail: str = ""):
        self.op = op
        self.op_name = op_name
        self.result_sim = result_sim
        self.detail = detail

    def __repr__(self) -> str:
        return f"Step({self.op_name}, sim={self.result_sim:.3f}, {self.detail})"


class Triad:
    """
    LTC + HDC + SDM reasoning system.

    Usage:
        triad = Triad(dim=10000)
        # Populate SDM with episodes
        triad.store_episode(world_state_fields, outcome_fields, label)
        # Reason about a new situation
        result = triad.reason(query_fields, max_steps=10)
    """

    def __init__(self, dim: int = 10000, hidden_size: int = 128, seed: int = 42):
        self.dim = dim
        self.hdc = HDC(dim=dim, seed=seed)
        self.sdm = SDM(dim=dim)
        self.controller = Controller(state_size=dim, hidden_size=hidden_size)
        self._trace: list[TriadStep] = []

    def store_episode(
        self, context: dict[str, str], outcome: dict[str, str], label: str = ""
    ) -> None:
        """Store an episode in SDM. Context becomes the address, outcome becomes the data."""
        addr = self.hdc.encode_record(context)
        data = self.hdc.encode_record(outcome)
        self.sdm.write(addr, data, label)

    def reason(
        self,
        query: dict[str, str],
        max_steps: int = 10,
        temperature: float = 0.0,
    ) -> dict:
        """
        Run the reasoning loop.

        The controller sequences operations over SDM and HDC until it emits EMIT
        or exhausts the step budget.

        Returns dict with:
            - result: the final state vector
            - best_match: (name, similarity) of closest codebook entry to result
            - trace: list of TriadSteps
            - steps: number of steps taken
        """
        self._trace = []
        state = self.hdc.encode_record(query)
        state_t = torch.from_numpy(state).unsqueeze(0)
        hidden = None

        for step_idx in range(max_steps):
            op_logits, arg_vec, hidden = self.controller(state_t, hidden)
            op = self.controller.select_op(op_logits.squeeze(0), temperature)

            arg_np = arg_vec.squeeze(0).detach().numpy()

            if op == Operation.EMIT:
                self._trace.append(TriadStep(op, "EMIT", detail=f"step {step_idx}"))
                break

            elif op == Operation.SDM_READ:
                matches = self.sdm.read(state)
                if matches:
                    best_data, best_sim, best_label = matches[0]
                    state = best_data
                    self._trace.append(TriadStep(
                        op, "SDM_READ", result_sim=best_sim, detail=best_label
                    ))
                else:
                    self._trace.append(TriadStep(op, "SDM_READ", detail="empty"))

            elif op == Operation.HDC_BIND:
                state = self.hdc.bind(state, arg_np)
                self._trace.append(TriadStep(op, "HDC_BIND"))

            elif op == Operation.HDC_UNBIND:
                state = self.hdc.unbind(state, arg_np)
                self._trace.append(TriadStep(op, "HDC_UNBIND"))

            elif op == Operation.HDC_BUNDLE:
                state = self.hdc.bundle([state, arg_np])
                self._trace.append(TriadStep(op, "HDC_BUNDLE"))

            elif op == Operation.HDC_BEST_MATCH:
                name, sim = self.hdc.best_match(state)
                self._trace.append(TriadStep(
                    op, "HDC_BEST_MATCH", result_sim=sim, detail=name
                ))

            else:
                self._trace.append(TriadStep(op, f"OP_{op}", detail="passthrough"))

            state_t = torch.from_numpy(state).unsqueeze(0)

        name, sim = self.hdc.best_match(state)
        return {
            "result": state,
            "best_match": (name, sim),
            "trace": self._trace,
            "steps": len(self._trace),
        }

    @property
    def trace(self) -> list[TriadStep]:
        return self._trace

    def param_count(self) -> int:
        """Total learnable parameters (only in the controller)."""
        return self.controller.param_count()

    def knowledge_count(self) -> int:
        """Number of episodes stored in SDM."""
        return self.sdm.count()
