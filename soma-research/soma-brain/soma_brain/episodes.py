"""
Episode distillation — learn from the body's operational experience.

Loads episodes from soma-next JSON files and converts them into training
data for the structured output heads (skill selection, bindings, confidence).

The body teaches the brain: episodes where an LLM brain made decisions
become training signal for the native brain to replicate those decisions.
"""

import json
from dataclasses import dataclass
from pathlib import Path

import torch


@dataclass
class DistillationSample:
    query_embedding: torch.Tensor
    skill_index: int
    outcome_score: float
    step_context: str


class EpisodeLoader:

    def __init__(self, skill_registry: list[str]):
        self.skill_registry = skill_registry
        self.skill_to_idx = {s: i for i, s in enumerate(skill_registry)}

    def load_episodes(self, path: str | Path) -> list[dict]:
        path = Path(path)
        episodes = []
        if path.is_dir():
            for f in sorted(path.glob("*.json")):
                episodes.extend(self._load_file(f))
        elif path.is_file():
            episodes.extend(self._load_file(path))
        return episodes

    def _load_file(self, path: Path) -> list[dict]:
        try:
            data = json.loads(path.read_text())
            if isinstance(data, list):
                return data
            if isinstance(data, dict) and "episodes" in data:
                return data["episodes"]
            if isinstance(data, dict) and "steps" in data:
                return [data]
            return []
        except (json.JSONDecodeError, UnicodeDecodeError):
            return []

    def extract_samples(self, episodes: list[dict], embedder=None) -> list[DistillationSample]:
        """Convert episodes into training samples for the output heads."""
        samples = []
        for episode in episodes:
            success = episode.get("success", False)
            outcome = episode.get("outcome", "")
            base_score = 1.0 if success else -0.5
            if outcome == "PartialSuccess":
                base_score = 0.5

            steps = episode.get("steps", [])
            for step in steps:
                skill_id = step.get("selected_skill", "")
                if skill_id not in self.skill_to_idx:
                    continue

                context = json.dumps(step.get("belief_summary", {}), default=str)[:512]
                progress = step.get("progress_delta", 0.0)
                score = base_score * (0.5 + progress)

                embedding = None
                if "embedding" in step:
                    embedding = torch.tensor(step["embedding"], dtype=torch.float32)

                if embedding is None and embedder is not None:
                    embedding = embedder.embed_one(context)

                if embedding is not None:
                    samples.append(DistillationSample(
                        query_embedding=embedding,
                        skill_index=self.skill_to_idx[skill_id],
                        outcome_score=score,
                        step_context=context,
                    ))

        return samples

    @staticmethod
    def collate(samples: list[DistillationSample]) -> dict[str, torch.Tensor]:
        """Batch samples into tensors for training."""
        return {
            "embeddings": torch.stack([s.query_embedding for s in samples]),
            "skill_targets": torch.tensor([s.skill_index for s in samples], dtype=torch.long),
            "outcome_scores": torch.tensor([s.outcome_score for s in samples], dtype=torch.float32),
        }


def generate_synthetic_episodes(skill_ids: list[str], n_episodes: int = 100) -> list[dict]:
    """Generate synthetic episodes for bootstrapping when real episodes don't exist yet."""
    import random
    episodes = []
    for _ in range(n_episodes):
        n_steps = random.randint(2, 8)
        success = random.random() > 0.3
        steps = []
        for i in range(n_steps):
            steps.append({
                "step_index": i,
                "selected_skill": random.choice(skill_ids),
                "belief_summary": {"step": i, "context": f"synthetic step {i}"},
                "progress_delta": random.uniform(0.0, 0.3) if success else random.uniform(-0.1, 0.1),
                "critic_decision": "continue",
            })
        episodes.append({
            "success": success,
            "outcome": "Success" if success else "Failure",
            "steps": steps,
        })
    return episodes
