"""Text embedder using pretrained sentence-transformers."""

import torch
from sentence_transformers import SentenceTransformer


class Embedder:
    def __init__(self, model_name: str = "BAAI/bge-m3"):
        self.model = SentenceTransformer(model_name, trust_remote_code=True)
        self.embed_dim = self.model.get_embedding_dimension()

    def embed(self, texts: list[str]) -> torch.Tensor:
        return self.model.encode(texts, convert_to_tensor=True)

    def embed_one(self, text: str) -> torch.Tensor:
        return self.embed([text])[0]

    @staticmethod
    def chunk_text(text: str, max_chars: int = 512) -> list[str]:
        """Split text into chunks by character count."""
        chunks = []
        i = 0
        while i < len(text):
            end = min(i + max_chars, len(text))
            if end < len(text):
                nl = text.rfind("\n", i, end)
                if nl > i:
                    end = nl + 1
            chunks.append(text[i:end])
            i = end
        return [c for c in chunks if c.strip()]
