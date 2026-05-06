"""
Consolidation loop — body teaches brain continuously.

Three consolidation paths:
  1. TTT → SDM: successful session patterns write back to long-term memory
  2. Episode → training: new episodes queue brain training steps
  3. Routine → SDM: compiled routines become retrievable knowledge

The confidence head tracks how often the native brain agrees with the
LLM brain. As training accumulates, confidence rises and the brain
handles more decisions autonomously.
"""

import torch
import torch.nn.functional as F


class ConsolidationLoop:

    def __init__(self, model, embedder=None, ttt_consolidation_threshold: float = 0.1):
        self.model = model
        self.embedder = embedder
        self.ttt_consolidation_threshold = ttt_consolidation_threshold
        self.session_log: list[dict] = []

    def on_session_complete(self, episode: dict) -> dict:
        """Called when a soma-next session completes. Returns consolidation stats."""
        stats = {"ttt_writebacks": 0, "ttt_consolidated": 0, "sdm_new_entries": 0}

        if not episode.get("success", False):
            self.model.ttt.reset_state()
            return stats

        ttt_stats = self.consolidate_ttt()
        stats["ttt_consolidated"] = ttt_stats["entries_written"]

        steps = episode.get("steps", [])
        for step in steps:
            context = step.get("belief_summary", {})
            text = str(context)[:512]

            if self.embedder is not None:
                emb = self.embedder.embed_one(text).unsqueeze(0).to(
                    next(self.model.parameters()).device
                )
                self.model.ingest(emb, text=text)
                stats["sdm_new_entries"] += 1

        stats["ttt_writebacks"] = len(steps)
        self.session_log.append({"episode_success": True, "stats": stats})
        return stats

    def consolidate_ttt(self) -> dict:
        """Flush TTT fast memory into SDM long-term store.

        The TTT layer accumulates weight deltas during inference. When a
        session succeeds, those deltas represent learned patterns not yet
        in SDM. This method:
          1. Gets the session inputs the TTT layer saw
          2. Runs them through the full reasoning pipeline to get refined embeddings
          3. Checks novelty against existing SDM content
          4. Writes novel patterns to SDM
          5. Resets TTT state for next session
        """
        stats = {"entries_written": 0, "inputs_seen": 0, "skipped_redundant": 0}

        session_inputs = self.model.ttt.get_session_inputs()
        stats["inputs_seen"] = len(session_inputs)

        if not session_inputs:
            self.model.ttt.reset_state()
            return stats

        device = next(self.model.parameters()).device

        for x_t in session_inputs:
            query = x_t.unsqueeze(0) if x_t.dim() == 1 else x_t

            if self.model.sdm.num_locations > 0:
                _, scores = self.model.sdm.read_topk(query.to(device))
                max_sim = scores.max().item()
                if max_sim > (1.0 - self.ttt_consolidation_threshold):
                    stats["skipped_redundant"] += 1
                    continue

            self.model.sdm.write(query.to(device), query.to(device))
            stats["entries_written"] += 1

        self.model.ttt.reset_state()
        return stats

    def on_routine_compiled(self, routine: dict) -> dict:
        """Called when the body compiles a new routine. Write it to SDM."""
        stats = {"sdm_new_entries": 0}

        description = routine.get("description", "")
        steps_text = " -> ".join(
            step.get("skill_id", "unknown") for step in routine.get("steps", [])
        )
        text = f"{description}: {steps_text}"

        if self.embedder is not None:
            device = next(self.model.parameters()).device
            emb = self.embedder.embed_one(text).unsqueeze(0).to(device)
            self.model.ingest(emb, text=text)
            stats["sdm_new_entries"] = 1

        return stats

    def measure_confidence(self, test_embeddings: torch.Tensor) -> float:
        """Measure average confidence across a batch of queries."""
        result = self.model.reason(test_embeddings)
        return result.confidence.mean().item()

    def get_stats(self) -> dict:
        sessions = len(self.session_log)
        successful = sum(1 for s in self.session_log if s.get("episode_success"))
        total_writebacks = sum(s["stats"]["sdm_new_entries"] for s in self.session_log)
        total_ttt = sum(s["stats"].get("ttt_consolidated", 0) for s in self.session_log)
        return {
            "sessions_observed": sessions,
            "successful_sessions": successful,
            "total_sdm_writebacks": total_writebacks,
            "total_ttt_consolidated": total_ttt,
            "sdm_size": self.model.sdm.num_locations,
        }
