"""
Extract MLP weights from a transformer's safetensors files into a
flat SDM. No GPU needed — purely I/O bound (read safetensors, repack).

For a 70B model on SSD: ~5-10 minutes (bandwidth-limited).

Output: checkpoints/<model>_sdm.pt with stacked (gate, up, down)
across all layers + metadata.
"""

import argparse
import os
import time

import torch
from transformers import AutoModelForCausalLM


def cpu_dtype(t, dtype=torch.float16):
    return t.detach().cpu().to(dtype).contiguous()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True, help="HF model id, e.g. meta-llama/Llama-3.1-70B")
    parser.add_argument("--out", required=True, help="Output .pt path")
    parser.add_argument("--dtype", default="float16", choices=["float16", "bfloat16", "int8"])
    args = parser.parse_args()

    out_dtype = {"float16": torch.float16, "bfloat16": torch.bfloat16,
                 "int8": torch.int8}[args.dtype]

    print(f"loading {args.model} on CPU (no GPU)...")
    t0 = time.time()
    model = AutoModelForCausalLM.from_pretrained(
        args.model, dtype=torch.float16, low_cpu_mem_usage=True,
    )
    model.train(False)
    cfg = model.config
    L = cfg.num_hidden_layers
    H = cfg.hidden_size
    I = cfg.intermediate_size
    print(f"  layers={L} hidden={H} intermediate={I} ({time.time() - t0:.1f}s)")

    print("extracting MLPs...")
    t0 = time.time()
    layers = []
    norms = {
        "embed_tokens": cpu_dtype(model.model.embed_tokens.weight, torch.float16),
        "final_norm":   cpu_dtype(model.model.norm.weight, torch.float16),
    }
    if not getattr(cfg, "tie_word_embeddings", False):
        norms["lm_head"] = cpu_dtype(model.lm_head.weight, torch.float16)

    for i, block in enumerate(model.model.layers):
        layers.append({
            "gate": cpu_dtype(block.mlp.gate_proj.weight, out_dtype),
            "up":   cpu_dtype(block.mlp.up_proj.weight,   out_dtype),
            "down": cpu_dtype(block.mlp.down_proj.weight, out_dtype),
            "input_norm":     cpu_dtype(block.input_layernorm.weight, torch.float16),
            "post_attn_norm": cpu_dtype(block.post_attention_layernorm.weight, torch.float16),
        })
        if (i + 1) % 8 == 0 or i == L - 1:
            print(f"  layer {i+1}/{L}")

    print(f"  extraction time: {time.time() - t0:.1f}s")

    # Stack MLPs into single tensors for fast loading
    print("stacking SDM tensors...")
    gate_all = torch.stack([w["gate"] for w in layers])
    up_all   = torch.stack([w["up"]   for w in layers])
    down_all = torch.stack([w["down"] for w in layers])

    config_meta = {
        "num_layers":          L,
        "hidden_size":         H,
        "intermediate_size":   I,
        "num_attention_heads": cfg.num_attention_heads,
        "num_kv_heads":        getattr(cfg, "num_key_value_heads", cfg.num_attention_heads),
        "head_dim":            H // cfg.num_attention_heads,
        "vocab_size":          cfg.vocab_size,
        "rms_norm_eps":        cfg.rms_norm_eps,
        "rope_theta": (
            getattr(cfg, "rope_theta", None)
            or (cfg.rope_parameters.get("rope_theta")
                if hasattr(cfg, "rope_parameters") and cfg.rope_parameters
                else None)
            or 10000.0
        ),
        "max_position":        getattr(cfg, "max_position_embeddings", 32768),
        "tie_word_embeddings": getattr(cfg, "tie_word_embeddings", False),
        "model_type":          cfg.model_type,
        "model_id":            args.model,
        "dtype":               args.dtype,
    }

    sdm = {
        "gate_all": gate_all,
        "up_all":   up_all,
        "down_all": down_all,
        "norms":    norms,
        "layer_norms": [
            {"input_norm": w["input_norm"], "post_attn_norm": w["post_attn_norm"]}
            for w in layers
        ],
        "config":   config_meta,
    }

    os.makedirs(os.path.dirname(args.out) or ".", exist_ok=True)
    print(f"saving to {args.out}...")
    t0 = time.time()
    torch.save(sdm, args.out)
    size_gb = os.path.getsize(args.out) / 1e9
    print(f"  save time: {time.time() - t0:.1f}s")
    print(f"  size: {size_gb:.2f} GB")
    print(f"  MLP entries: {L * I:,}")


if __name__ == "__main__":
    main()
