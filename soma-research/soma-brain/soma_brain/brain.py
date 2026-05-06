"""
SOMA Brain — reasoning core + diffusion decoder.

Architecture:
  Text -> Pretrained Embedder -> [ReasoningBlock �� N] -> TTT -> Structured Result
  Hidden state -> DiffusionDecoder -> Generated text (parallel, iterative)

Each ReasoningBlock = liquid ODE step + SDM retrieval + memory attention.
N blocks = N hops of retrieval refinement.

Knowledge lives in SDM (RAM). The core reasons over it and produces:
  - Structured decisions (skill scores, bindings, confidence)
  - Generated text via masked diffusion (not autoregressive)
"""

from dataclasses import dataclass, field

import torch
import torch.nn as nn
import torch.nn.functional as F

from .config import BrainConfig
from .sdm import SparseDistributedMemory
from .ttt import TTTLayer
from .memory_attn import ReasoningBlock
from .ar_decoder import ARDecoder
from .span_extractor import SpanExtractor
from .tokenizer import Tokenizer


@dataclass
class ReasoningResult:
    hidden: torch.Tensor
    retrieved_entries: torch.Tensor
    retrieved_scores: torch.Tensor
    skill_scores: torch.Tensor
    bindings: torch.Tensor
    confidence: torch.Tensor
    sources: list[tuple[str, float]] = field(default_factory=list)


class SomaBrain(nn.Module):

    def __init__(self, config: BrainConfig):
        super().__init__()
        self.config = config

        self.input_proj = nn.Linear(config.embed_dim, config.hidden_size)
        if config.embed_dim == config.hidden_size:
            nn.init.eye_(self.input_proj.weight)
            nn.init.zeros_(self.input_proj.bias)

        self.sdm = SparseDistributedMemory(
            address_size=config.hidden_size,
            data_size=config.hidden_size,
            top_k=config.sdm_top_k,
        )

        self.blocks = nn.ModuleList()
        for _ in range(config.num_layers):
            self.blocks.append(ReasoningBlock(
                config.hidden_size, config.ode_steps, config.num_attn_heads,
            ))

        self.ttt = TTTLayer(config.hidden_size, config.ttt_inner_size, config.ttt_lr)

        self.output_proj = nn.Linear(config.hidden_size, config.embed_dim)
        if config.embed_dim == config.hidden_size:
            nn.init.eye_(self.output_proj.weight)
            nn.init.zeros_(self.output_proj.bias)

        self.skill_head = nn.Sequential(
            nn.Linear(config.hidden_size, config.hidden_size // 2),
            nn.SiLU(),
            nn.Linear(config.hidden_size // 2, config.max_skills),
        )
        self.binding_head = nn.Sequential(
            nn.Linear(config.hidden_size, config.hidden_size // 2),
            nn.SiLU(),
            nn.Linear(config.hidden_size // 2, config.binding_dim),
        )
        self.confidence_head = nn.Sequential(
            nn.Linear(config.hidden_size, config.hidden_size // 4),
            nn.SiLU(),
            nn.Linear(config.hidden_size // 4, 1),
        )

        self.skill_registry: list[str] = []

        self.tokenizer = Tokenizer()
        vocab_size = config.decoder_vocab_size or self.tokenizer.vocab_size
        self.decoder = ARDecoder(
            vocab_size=vocab_size,
            hidden_size=config.decoder_hidden,
            num_layers=config.decoder_layers,
            cond_size=config.hidden_size,
            max_seq_len=config.decoder_max_seq,
        )

        self.span_extractor = SpanExtractor(
            hidden_size=config.decoder_hidden,
            cond_size=config.hidden_size,
            num_heads=config.decoder_heads,
            num_encoder_layers=config.decoder_layers,
        )

        self.source_texts: list[str] = []
        self.source_embeddings: list[torch.Tensor] = []

    def forward_train(self, embedding: torch.Tensor) -> dict[str, torch.Tensor | list[torch.Tensor]]:
        """Training forward pass. Returns dict with all outputs for loss computation."""
        x = self.input_proj(embedding).unsqueeze(1)
        layer_outputs = [x.squeeze(1)]

        for block in self.blocks:
            x, _ = block(x, self.sdm)
            layer_outputs.append(x.squeeze(1))

        x = self.ttt(x, update=False)
        h = x.squeeze(1)

        return {
            "reconstructed": self.output_proj(h),
            "layer_outputs": layer_outputs,
            "skill_logits": self.skill_head(h),
            "bindings": self.binding_head(h),
            "confidence": torch.sigmoid(self.confidence_head(h)),
        }

    @torch.no_grad()
    def reason(self, embedding: torch.Tensor, top_k_sources: int = 5) -> ReasoningResult:
        """Embedding → retrieve from SDM → reason → structured result.

        embedding: (batch, embed_dim) from pretrained embedder.
        """
        x = self.input_proj(embedding).unsqueeze(1)

        for block in self.blocks:
            x, _ = block(x, self.sdm)

        x = self.ttt(x, update=True)
        h = x.squeeze(1)

        entries, scores = self.sdm.read_topk(h)

        sources = []
        if self.source_embeddings:
            refined = self.output_proj(h)
            if isinstance(self.source_embeddings, torch.Tensor):
                source_matrix = self.source_embeddings.to(embedding.device)
            else:
                source_matrix = torch.stack(self.source_embeddings).to(embedding.device)
            query_norm = F.normalize(refined[0:1], dim=-1)
            source_norm = F.normalize(source_matrix, dim=-1)
            sims = torch.matmul(query_norm, source_norm.T).squeeze(0)
            top_vals, top_idx = torch.topk(sims, min(top_k_sources, len(self.source_texts)))
            for val, idx in zip(top_vals.tolist(), top_idx.tolist()):
                sources.append((self.source_texts[idx], val))
        skill_scores = self.skill_head(h)
        bindings = self.binding_head(h)
        confidence = torch.sigmoid(self.confidence_head(h))

        return ReasoningResult(
            hidden=h,
            retrieved_entries=entries,
            retrieved_scores=scores,
            skill_scores=skill_scores,
            bindings=bindings,
            confidence=confidence,
            sources=sources,
        )

    @torch.no_grad()
    def ingest(self, embedding: torch.Tensor, text: str | None = None) -> int:
        """Write embedding into SDM.

        embedding: (batch, embed_dim) from pretrained embedder.
        """
        projected = self.input_proj(embedding)
        self.sdm.write(projected, projected)
        if text is not None:
            self.source_texts.append(text)
            self.source_embeddings.append(embedding.squeeze(0).cpu())
        return embedding.shape[0]

    @torch.no_grad()
    def generate(self, embedding: torch.Tensor, max_len: int = 128, steps: int = 16) -> list[str]:
        """Embedding → reason → diffusion decode → text."""
        x = self.input_proj(embedding).unsqueeze(1)
        for block in self.blocks:
            x, _ = block(x, self.sdm)
        x = self.ttt(x, update=True)
        token_ids = self.decoder.generate(x, seq_len=max_len, steps=steps)
        results = []
        for b in range(token_ids.shape[0]):
            text = self.tokenizer.decode(token_ids[b].tolist())
            results.append(text)
        return results

    def register_skills(self, skill_ids: list[str]) -> None:
        """Register skill IDs so the brain can map skill_head output to skill names."""
        self.skill_registry = list(skill_ids)

    def decode_skill(self, skill_scores: torch.Tensor, top_k: int = 5) -> list[tuple[str, float]]:
        """Map skill_head output to (skill_id, score) pairs."""
        if not self.skill_registry:
            return []
        n = min(len(self.skill_registry), skill_scores.shape[-1])
        scores = skill_scores[0, :n]
        probs = F.softmax(scores, dim=-1)
        top_vals, top_idx = torch.topk(probs, min(top_k, n))
        return [(self.skill_registry[i], v) for v, i in zip(top_vals.tolist(), top_idx.tolist())]

    def count_parameters(self) -> dict[str, int]:
        def count(module):
            return sum(p.numel() for p in module.parameters())

        return {
            "input_proj": count(self.input_proj),
            "sdm_query_proj": count(self.sdm),
            "reasoning_blocks": sum(count(b) for b in self.blocks),
            "ttt": count(self.ttt),
            "total": sum(p.numel() for p in self.parameters()),
            "total_unique": sum(p.numel() for p in set(self.parameters())),
        }
