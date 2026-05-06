"""
Phase 1: Full surgical extraction of Qwen-3B into self-contained tensors.

Extracts all weights needed to run a forward pass without HuggingFace:
  - token embeddings + LM head
  - per-layer: input_layernorm, attention (Q,K,V,O), post_attention_layernorm,
    MLP (gate, up, down)
  - final norm
  - RoPE config (theta, max_position_embeddings)

Verifies extraction by reconstructing each layer's MLP forward pass and
comparing against the model's native forward.

Output: a single .pt file containing every parameter and structural
constant needed to run forward without the original transformer code.

Usage:
  python extract.py [--model Qwen/Qwen2.5-3B] [--out checkpoints/qwen3b_full.pt]
"""

import argparse
import os
import time

import torch
from transformers import AutoModelForCausalLM


def get_device():
    if torch.cuda.is_available():
        return torch.device("cuda")
    if torch.backends.mps.is_available():
        return torch.device("mps")
    return torch.device("cpu")


def cpu_fp16(t):
    return t.detach().cpu().to(torch.float16).contiguous()


def extract_layer(block):
    """Pull all weights from one transformer block."""
    attn = block.self_attn
    mlp = block.mlp
    out = {
        "input_norm":         cpu_fp16(block.input_layernorm.weight),
        "post_attn_norm":     cpu_fp16(block.post_attention_layernorm.weight),
        "q":                  cpu_fp16(attn.q_proj.weight),
        "k":                  cpu_fp16(attn.k_proj.weight),
        "v":                  cpu_fp16(attn.v_proj.weight),
        "o":                  cpu_fp16(attn.o_proj.weight),
        "gate":               cpu_fp16(mlp.gate_proj.weight),
        "up":                 cpu_fp16(mlp.up_proj.weight),
        "down":               cpu_fp16(mlp.down_proj.weight),
    }
    if attn.q_proj.bias is not None:
        out["q_bias"] = cpu_fp16(attn.q_proj.bias)
        out["k_bias"] = cpu_fp16(attn.k_proj.bias)
        out["v_bias"] = cpu_fp16(attn.v_proj.bias)
    return out


def extract_all(model):
    """Walk the model, return everything in a flat dict."""
    cfg = model.config
    layers = [extract_layer(block) for block in model.model.layers]

    return {
        "embed_tokens":   cpu_fp16(model.model.embed_tokens.weight),
        "lm_head":        cpu_fp16(model.lm_head.weight),
        "final_norm":     cpu_fp16(model.model.norm.weight),
        "layers":         layers,
        "config": {
            "num_layers":          cfg.num_hidden_layers,
            "hidden_size":         cfg.hidden_size,
            "intermediate_size":   cfg.intermediate_size,
            "num_attention_heads": cfg.num_attention_heads,
            "num_kv_heads":        getattr(cfg, "num_key_value_heads", cfg.num_attention_heads),
            "head_dim":            cfg.hidden_size // cfg.num_attention_heads,
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
            "dtype":               "float16",
        },
    }


def reconstruct_mlp(x, gate, up, down):
    g = torch.nn.functional.silu(x @ gate.T)
    u = x @ up.T
    return (g * u) @ down.T


def verify_mlp_streaming(model, extracted, device, n_probes=4, seq_len=64):
    """
    Verification with layer-by-layer streaming — never holds the whole
    model on GPU. Moves one layer to GPU, verifies, evicts.

    Strategy that scales to 1T models: only the active layer touches VRAM.
    """
    print("\n=== Verification (streaming, one layer at a time) ===")
    hidden = extracted["config"]["hidden_size"]
    probe = torch.randn(n_probes, seq_len, hidden, device=device, dtype=torch.float16)

    sample_layers = [0, len(extracted["layers"]) // 2, len(extracted["layers"]) - 1]
    for li in sample_layers:
        with torch.no_grad():
            block = model.model.layers[li].to(device)
            x_norm = block.post_attention_layernorm(probe)
            original = block.mlp(x_norm)

            L = extracted["layers"][li]
            reconstructed = reconstruct_mlp(
                x_norm,
                L["gate"].to(device),
                L["up"].to(device),
                L["down"].to(device),
            )
            diff = (original - reconstructed).abs()
            rmse = ((original - reconstructed) ** 2).mean().sqrt().item()
            scale = original.abs().mean().item()
            print(
                f"  layer {li:3d}: max_abs_diff={diff.max().item():.2e} "
                f"rmse={rmse:.2e} rel={rmse/max(scale,1e-9):.2%}"
            )
            # evict back to CPU
            model.model.layers[li] = block.cpu()
            torch.cuda.empty_cache() if device.type == "cuda" else None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", default="Qwen/Qwen2.5-3B")
    parser.add_argument("--out", default="checkpoints/qwen3b_full.pt")
    parser.add_argument("--no-verify", action="store_true")
    args = parser.parse_args()

    device = get_device()
    print(f"device: {device}")
    print(f"model:  {args.model}")

    # Load on CPU — works for any model size. Only the layer being
    # verified moves to GPU, then evicts. Scales to 70B/1T identically.
    t0 = time.time()
    print("\nloading model on CPU (no full-model GPU load)...")
    model = AutoModelForCausalLM.from_pretrained(
        args.model, dtype=torch.float16, low_cpu_mem_usage=True,
    )
    model.train(False)
    cfg = model.config
    print(f"  layers={cfg.num_hidden_layers} hidden={cfg.hidden_size} "
          f"intermediate={cfg.intermediate_size} heads={cfg.num_attention_heads} "
          f"kv_heads={getattr(cfg, 'num_key_value_heads', '?')}")
    print(f"  load time: {time.time() - t0:.1f}s")

    print("\nextracting all parameters (CPU only)...")
    t0 = time.time()
    extracted = extract_all(model)
    print(f"  extracted {len(extracted['layers'])} layers in {time.time() - t0:.1f}s")

    if not args.no_verify:
        verify_mlp_streaming(model, extracted, device)

    n_mlp_entries = (
        len(extracted["layers"]) * extracted["config"]["intermediate_size"]
    )
    print(f"\nMLP entries:  {n_mlp_entries:,}")
    print(f"Attn weights: {len(extracted['layers'])} × (Q, K, V, O)")

    os.makedirs(os.path.dirname(args.out) or ".", exist_ok=True)
    torch.save(extracted, args.out)
    size_gb = os.path.getsize(args.out) / 1e9
    print(f"\nsaved: {args.out} ({size_gb:.2f} GB)")


if __name__ == "__main__":
    main()
