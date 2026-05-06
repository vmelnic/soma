"""Defaults for qwen-knn. Override via CLI flags."""
from dataclasses import dataclass


@dataclass
class Config:
    model_id: str = "google/gemma-4-26B-A4B-it"
    dtype: str = "float16"
    device: str = "cuda"
    max_seq_len: int = 2048
    hidden_layer: int = -1
    k: int = 16
    lam: float = 0.25
    temperature: float = 1.0
    faiss_nlist: int = 4096
    faiss_nprobe: int = 32
    faiss_pq_m: int = 64
    faiss_pq_nbits: int = 8


MODEL_PRESETS = {
    "gemma4-e2b": "google/gemma-4-E2B-it",
    "gemma4-e4b": "google/gemma-4-E4B-it",
    "gemma4-26b-a4b": "google/gemma-4-26B-A4B-it",
    "gemma4-26b-a4b-base": "google/gemma-4-26B-A4B",
    "gemma3-4b": "google/gemma-3-4b-it",
    "gemma3-12b": "google/gemma-3-12b-it",
    "qwen-coder-7b": "Qwen/Qwen2.5-Coder-7B",
    "qwen-coder-1.5b": "Qwen/Qwen2.5-Coder-1.5B",
    "qwen3-4b": "Qwen/Qwen3-4B",
    "granite4-1b": "ibm-granite/granite-4.0-h-1b",
}
