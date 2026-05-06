"""
Chat with the standalone LiquidCore — no transformer at inference.

Loads:
  - extracted SDM (qwen3b_full.pt — frozen substrate)
  - distilled LiquidCore weights (or bootstrap if missing)
"""

import argparse
import time
import os

import torch
import torch.nn.functional as F
from transformers import AutoTokenizer

from ltc import bootstrap_from_extraction


@torch.no_grad()
def generate(core, input_ids, max_new=128, temperature=0.7, top_p=0.9, eos_ids=None):
    eos_ids = eos_ids or []
    out_ids = input_ids.clone()
    for _ in range(max_new):
        logits = core(out_ids)[:, -1, :]
        if temperature > 0:
            logits = logits / temperature
            probs = F.softmax(logits.float(), dim=-1)
            sorted_probs, sorted_idx = probs.sort(descending=True, dim=-1)
            cum = sorted_probs.cumsum(dim=-1)
            mask = cum > top_p
            mask[..., 0] = False
            sorted_probs[mask] = 0
            sorted_probs = sorted_probs / sorted_probs.sum(dim=-1, keepdim=True)
            choice = torch.multinomial(sorted_probs, 1)
            next_id = sorted_idx.gather(-1, choice)
        else:
            next_id = logits.argmax(-1, keepdim=True)
        out_ids = torch.cat([out_ids, next_id], dim=-1)
        tok_id = next_id.item()
        if tok_id in eos_ids:
            break
        yield tok_id


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--ckpt", default="checkpoints/qwen3b_full.pt")
    parser.add_argument("--core", default="checkpoints/liquid_core.pt")
    parser.add_argument("--model", default="Qwen/Qwen2.5-3B")
    parser.add_argument("--n-steps", type=int, default=8)
    parser.add_argument("--top-k", type=int, default=128)
    parser.add_argument("--max-new", type=int, default=128)
    parser.add_argument("--temperature", type=float, default=0.7)
    parser.add_argument("--prompt", default=None)
    args = parser.parse_args()

    device = torch.device("cuda" if torch.cuda.is_available()
                          else "mps" if torch.backends.mps.is_available()
                          else "cpu")
    dtype = torch.bfloat16 if device.type == "cuda" else torch.float16

    print(f"loading extraction from {args.ckpt}...")
    extracted = torch.load(args.ckpt, map_location="cpu", weights_only=False)

    print(f"building LiquidCore...")
    core = bootstrap_from_extraction(
        extracted, n_steps=args.n_steps, top_k=args.top_k, dtype=dtype,
    ).to(device)

    if os.path.exists(args.core):
        ckpt = torch.load(args.core, map_location=device, weights_only=False)
        core.load_state_dict(ckpt["ltc_state"], strict=False)
        print(f"  loaded distilled weights (step {ckpt.get('step', '?')})")
    else:
        print("  (no distilled weights — bootstrap only, expect noise)")

    tok = AutoTokenizer.from_pretrained(args.model)
    eos_ids = [tok.eos_token_id]
    for s in ["<|im_end|>", "<|endoftext|>"]:
        tid = tok.convert_tokens_to_ids(s)
        if tid is not None and tid != tok.unk_token_id:
            eos_ids.append(tid)

    def chat(messages):
        prompt = tok.apply_chat_template(messages, tokenize=False, add_generation_prompt=True)
        ids = tok(prompt, return_tensors="pt").input_ids.to(device)
        print(f"\n[input tokens: {ids.shape[1]}]\nassistant: ", end="", flush=True)
        t0 = time.time()
        n = 0
        for tid in generate(core, ids, max_new=args.max_new,
                            temperature=args.temperature, eos_ids=eos_ids):
            print(tok.decode([tid], skip_special_tokens=True), end="", flush=True)
            n += 1
        elapsed = time.time() - t0
        print(f"\n[{n} tokens, {n/elapsed:.1f} tok/s]")

    if args.prompt:
        chat([{"role": "user", "content": args.prompt}])
    else:
        history = []
        print("\n/quit /reset\n")
        while True:
            try:
                user = input("you: ")
            except (EOFError, KeyboardInterrupt):
                break
            if user.strip() == "/quit":
                break
            if user.strip() == "/reset":
                history = []
                print("(history cleared)")
                continue
            if not user.strip():
                continue
            history.append({"role": "user", "content": user})
            chat(history)


if __name__ == "__main__":
    main()
