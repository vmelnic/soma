"""
SOMA Brain port — serves the brain over MCP as a SOMA port.

The runtime calls invoke_port("brain", {...}) and gets back structured
results: skill scores, bindings, confidence, retrieved sources, and
optionally generated text.

The confidence head gates autonomy:
  - confidence > threshold → brain's decision is authoritative
  - confidence < threshold → defer to LLM brain
"""

import json

import torch

from .brain import SomaBrain, ReasoningResult
from .embedder import Embedder
from .consolidation import ConsolidationLoop


class BrainPort:
    """Wraps SomaBrain as a callable port for the SOMA runtime."""

    def __init__(self, model: SomaBrain, embedder: Embedder, confidence_threshold: float = 0.7):
        self.model = model
        self.embedder = embedder
        self.confidence_threshold = confidence_threshold
        self.consolidation = ConsolidationLoop(model, embedder)
        self.device = next(model.parameters()).device

    def invoke(self, capability: str, params: dict) -> dict:
        """Handle a port invocation from the runtime."""
        handlers = {
            "reason": self._handle_reason,
            "generate": self._handle_generate,
            "ingest": self._handle_ingest,
            "consolidate_episode": self._handle_consolidate_episode,
            "consolidate_routine": self._handle_consolidate_routine,
            "consolidate_ttt": self._handle_consolidate_ttt,
            "status": self._handle_status,
        }
        handler = handlers.get(capability)
        if handler is None:
            return {"error": f"unknown capability: {capability}", "success": False}
        return handler(params)

    def _handle_reason(self, params: dict) -> dict:
        query = params.get("query", "")
        top_k = params.get("top_k_sources", 5)

        emb = self.embedder.embed_one(query).unsqueeze(0).to(self.device)
        result = self.model.reason(emb, top_k_sources=top_k)

        conf = result.confidence.item()
        is_authoritative = conf >= self.confidence_threshold

        response = {
            "success": True,
            "confidence": conf,
            "authoritative": is_authoritative,
            "sources": [
                {"text": text[:500], "score": score}
                for text, score in result.sources
            ],
        }

        if self.model.skill_registry:
            skills = self.model.decode_skill(result.skill_scores, top_k=5)
            response["skill_recommendations"] = [
                {"skill_id": sid, "score": score} for sid, score in skills
            ]

        return response

    def _handle_generate(self, params: dict) -> dict:
        query = params.get("query", "")
        max_len = params.get("max_len", 128)
        steps = params.get("steps", 16)

        emb = self.embedder.embed_one(query).unsqueeze(0).to(self.device)
        texts = self.model.generate(emb, max_len=max_len, steps=steps)

        return {
            "success": True,
            "generated_text": texts[0] if texts else "",
        }

    def _handle_ingest(self, params: dict) -> dict:
        text = params.get("text", "")
        if not text:
            return {"success": False, "error": "no text provided"}

        emb = self.embedder.embed_one(text).unsqueeze(0).to(self.device)
        n = self.model.ingest(emb, text=text)
        return {"success": True, "entries_added": n, "sdm_size": self.model.sdm.num_locations}

    def _handle_consolidate_episode(self, params: dict) -> dict:
        episode = params.get("episode", {})
        stats = self.consolidation.on_session_complete(episode)
        return {"success": True, **stats}

    def _handle_consolidate_routine(self, params: dict) -> dict:
        routine = params.get("routine", {})
        stats = self.consolidation.on_routine_compiled(routine)
        return {"success": True, **stats}

    def _handle_consolidate_ttt(self, params: dict) -> dict:
        stats = self.consolidation.consolidate_ttt()
        return {"success": True, **stats}

    def _handle_status(self, params: dict) -> dict:
        cons_stats = self.consolidation.get_stats()
        brain_params = self.model.count_parameters()
        return {
            "success": True,
            "sdm_entries": self.model.sdm.num_locations,
            "source_texts": len(self.model.source_texts),
            "skills_registered": len(self.model.skill_registry),
            "confidence_threshold": self.confidence_threshold,
            "parameters": brain_params,
            "consolidation": cons_stats,
        }

    def manifest(self) -> dict:
        """Return port manifest for soma-next registration."""
        return {
            "port_id": "brain",
            "version": "0.1.0",
            "capabilities": [
                {
                    "capability_id": "reason",
                    "description": "Query the brain — retrieve from SDM, reason, return structured result",
                    "inputs": {"type": "object", "properties": {
                        "query": {"type": "string"},
                        "top_k_sources": {"type": "integer", "default": 5},
                    }, "required": ["query"]},
                },
                {
                    "capability_id": "generate",
                    "description": "Generate text from the brain's reasoning over a query",
                    "inputs": {"type": "object", "properties": {
                        "query": {"type": "string"},
                        "max_len": {"type": "integer", "default": 128},
                        "steps": {"type": "integer", "default": 16},
                    }, "required": ["query"]},
                },
                {
                    "capability_id": "ingest",
                    "description": "Add knowledge to SDM",
                    "inputs": {"type": "object", "properties": {
                        "text": {"type": "string"},
                    }, "required": ["text"]},
                },
                {
                    "capability_id": "consolidate_episode",
                    "description": "Consolidate a completed episode into long-term memory",
                    "inputs": {"type": "object", "properties": {
                        "episode": {"type": "object"},
                    }, "required": ["episode"]},
                },
                {
                    "capability_id": "consolidate_routine",
                    "description": "Write a compiled routine into SDM",
                    "inputs": {"type": "object", "properties": {
                        "routine": {"type": "object"},
                    }, "required": ["routine"]},
                },
                {
                    "capability_id": "consolidate_ttt",
                    "description": "Flush TTT fast memory into SDM long-term store",
                    "inputs": {"type": "object", "properties": {}},
                },
                {
                    "capability_id": "status",
                    "description": "Get brain status: SDM size, parameters, consolidation stats",
                    "inputs": {"type": "object", "properties": {}},
                },
            ],
        }
