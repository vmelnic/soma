"""
Train SOMA brain on extractive QA: question -> retrieve from SDM -> extract span.

Requires: python prepare_data.py (creates checkpoint with SDM + QA data)

Training signals:
  - span: cross-entropy on start/end positions in passage
  - pc: predictive coding — bidirectional layer coherence (regularizer)
  - recon: reconstruction — embedding identity preservation (regularizer)

Usage:
  python train.py [--steps 10000] [--val-every 500] [--patience 5]
"""

import argparse
import os
import re
import sys
import time
from collections import Counter

import torch
import torch.nn.functional as F

from soma_brain import SomaBrain, BrainConfig, PredictiveCodingLoss


def get_device():
    if torch.backends.mps.is_available():
        return torch.device("mps")
    if torch.cuda.is_available():
        return torch.device("cuda")
    return torch.device("cpu")


def load_checkpoint(path, device, decoder_overrides=None):
    if not os.path.exists(path):
        print(f"error: {path} not found — run prepare_data.py first")
        raise SystemExit(1)

    data = torch.load(path, map_location=device, weights_only=False)
    config = data["config"]
    config.decoder_vocab_size = 256
    if decoder_overrides:
        for k, v in decoder_overrides.items():
            if v is not None:
                setattr(config, k, v)
    model = SomaBrain(config).to(device)
    ckpt_state = data["state_dict"]
    model_state = model.state_dict()
    filtered = {
        k: v for k, v in ckpt_state.items()
        if k in model_state and v.shape == model_state[k].shape
    }
    model.load_state_dict(filtered, strict=False)

    if "sdm_addresses" in data:
        addrs = data["sdm_addresses"]
        entries = data["sdm_entries"]
        if isinstance(addrs, list):
            model.sdm._addr_buf = torch.stack(addrs)
            model.sdm._data_buf = torch.stack(entries)
            model.sdm._count = len(addrs)
        else:
            model.sdm._addr_buf = addrs
            model.sdm._data_buf = entries
            model.sdm._count = addrs.shape[0]
    if "source_texts" in data:
        model.source_texts = data["source_texts"]
    if "source_embeddings" in data:
        se = data["source_embeddings"]
        if isinstance(se, torch.Tensor) and se.dim() == 2:
            model.source_embeddings = list(se)
        elif isinstance(se, list):
            model.source_embeddings = se

    pc = PredictiveCodingLoss(config.hidden_size, config.num_layers).to(device)
    if "pc_loss_state" in data:
        pc.load_state_dict(data["pc_loss_state"], strict=False)

    return model, config, pc


def save_checkpoint(model, config, pc_loss_fn, path):
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    torch.save(
        {
            "config": config,
            "state_dict": model.state_dict(),
            "sdm_addresses": model.sdm._addr_buf[: model.sdm._count].cpu(),
            "sdm_entries": model.sdm._data_buf[: model.sdm._count].cpu(),
            "source_texts": model.source_texts,
            "source_embeddings": (
                torch.stack(model.source_embeddings).cpu()
                if model.source_embeddings
                else []
            ),
            "pc_loss_state": pc_loss_fn.state_dict(),
        },
        path,
    )


def encode_bytes(texts, max_len, device):
    ids_list = []
    lengths = []
    for text in texts:
        b = list(text.encode("utf-8"))[:max_len]
        lengths.append(len(b))
        b = b + [0] * (max_len - len(b))
        ids_list.append(b)
    return (
        torch.tensor(ids_list, dtype=torch.long, device=device),
        torch.tensor(lengths, dtype=torch.long, device=device),
    )


def find_answer_spans(contexts, answers, max_ctx_len):
    """Find byte-level start positions of answers within contexts."""
    starts = []
    ends = []
    for ctx, ans in zip(contexts, answers):
        ctx_bytes = ctx.encode("utf-8")[:max_ctx_len]
        ans_bytes = ans.encode("utf-8")
        pos = ctx_bytes.find(ans_bytes)
        if pos >= 0:
            starts.append(pos)
            ends.append(pos + len(ans_bytes) - 1)
        else:
            ans_lower = ans.lower().encode("utf-8")
            ctx_lower = ctx.lower().encode("utf-8")[:max_ctx_len]
            pos = ctx_lower.find(ans_lower)
            if pos >= 0:
                starts.append(pos)
                ends.append(pos + len(ans_lower) - 1)
            else:
                starts.append(0)
                ends.append(0)
    return starts, ends


def normalize_answer(text):
    text = text.lower().strip()
    text = re.sub(r"\b(a|an|the)\b", " ", text)
    text = re.sub(r"[^\w\s]", "", text)
    return " ".join(text.split())


def compute_f1(pred, gold):
    pred_tokens = normalize_answer(pred).split()
    gold_tokens = normalize_answer(gold).split()
    if not pred_tokens and not gold_tokens:
        return 1.0
    if not pred_tokens or not gold_tokens:
        return 0.0
    common = Counter(pred_tokens) & Counter(gold_tokens)
    n_common = sum(common.values())
    if n_common == 0:
        return 0.0
    precision = n_common / len(pred_tokens)
    recall = n_common / len(gold_tokens)
    return 2 * precision * recall / (precision + recall)


def extract_answer_text(context_ids, start, end):
    """Extract answer bytes from context IDs given start/end positions."""
    span = context_ids[start : end + 1].tolist()
    return bytes(b for b in span if 0 < b < 256).decode("utf-8", errors="replace")


def run_validation(model, val_data, context_seq_len, device, batch_size, max_samples):
    model.train(False)
    import random

    n = min(len(val_data["answers"]), max_samples)
    indices = random.sample(range(len(val_data["answers"])), n)

    total_loss = 0.0
    total_em = 0
    total_f1 = 0.0
    n_batches = 0
    examples = []

    with torch.no_grad():
        for i in range(0, n, batch_size):
            batch_idx = indices[i : i + batch_size]
            embeddings = val_data["embeddings"][batch_idx].to(device)
            answers = [val_data["answers"][j] for j in batch_idx]
            contexts = [val_data["contexts"][j] for j in batch_idx]

            out = model.forward_train(embeddings)
            h = torch.stack(out["layer_outputs"], dim=1)

            context_ids, context_lengths = encode_bytes(
                contexts, context_seq_len, device,
            )
            gold_starts, gold_ends = find_answer_spans(
                contexts, answers, context_seq_len,
            )
            start_targets = torch.tensor(gold_starts, device=device)
            end_targets = torch.tensor(gold_ends, device=device)

            start_logits, end_logits = model.span_extractor(
                context_ids, context_lengths, h,
            )
            loss = (
                F.cross_entropy(start_logits, start_targets)
                + F.cross_entropy(end_logits, end_targets)
            ) / 2
            total_loss += loss.item()
            n_batches += 1

            spans = model.span_extractor.extract(
                context_ids, context_lengths, h,
            )
            for b, gold in enumerate(answers):
                s, e = spans[b]
                pred = extract_answer_text(context_ids[b], s, e)
                total_em += int(normalize_answer(pred) == normalize_answer(gold))
                total_f1 += compute_f1(pred, gold)
                if len(examples) < 5:
                    examples.append((gold, pred))

    model.train()
    if examples:
        for gold, pred in examples[:3]:
            print(f"    gold: {gold[:60]}  |  pred: {pred[:60]}")
    return {
        "loss": total_loss / max(n_batches, 1),
        "em": total_em / n,
        "f1": total_f1 / n,
    }


def main():
    parser = argparse.ArgumentParser(description="Train SOMA brain on QA")
    parser.add_argument("--checkpoint", default="checkpoints/brain_base.pt")
    parser.add_argument("--save", default="checkpoints/brain.pt")
    parser.add_argument("--train-data", default="data/train_qa.pt")
    parser.add_argument("--val-data", default="data/val_qa.pt")
    parser.add_argument("--steps", type=int, default=50000)
    parser.add_argument("--batch-size", type=int, default=16)
    parser.add_argument("--lr", type=float, default=3e-4)
    parser.add_argument("--span-weight", type=float, default=1.0)
    parser.add_argument("--pc-weight", type=float, default=0.05)
    parser.add_argument("--recon-weight", type=float, default=0.1)
    parser.add_argument("--context-seq-len", type=int, default=1024)
    parser.add_argument("--decoder-hidden", type=int, default=None)
    parser.add_argument("--decoder-layers", type=int, default=None)
    parser.add_argument("--decoder-heads", type=int, default=None)
    parser.add_argument("--val-every", type=int, default=1000)
    parser.add_argument("--val-samples", type=int, default=500)
    parser.add_argument("--patience", type=int, default=10)
    parser.add_argument("--log-every", type=int, default=50)
    parser.add_argument("--log-file", default=None)
    args = parser.parse_args()

    if args.log_file:
        sys.stdout = open(args.log_file, "w", buffering=1, encoding="utf-8")
        sys.stderr = sys.stdout

    device = get_device()
    print(f"device: {device}")

    train_data = torch.load(args.train_data, map_location="cpu", weights_only=False)
    val_data = torch.load(args.val_data, map_location="cpu", weights_only=False)
    n_train = len(train_data["answers"])
    n_val = len(val_data["answers"])
    print(f"train: {n_train} pairs | val: {n_val} pairs")

    decoder_overrides = {
        "decoder_hidden": args.decoder_hidden,
        "decoder_layers": args.decoder_layers,
        "decoder_heads": args.decoder_heads,
    }
    model, config, pc_loss_fn = load_checkpoint(
        args.checkpoint, device,
        decoder_overrides=decoder_overrides,
    )
    print(f"span extractor: hidden={config.decoder_hidden} layers={config.decoder_layers}")
    print(f"SDM: {model.sdm.num_locations} entries")

    all_params = (
        list(model.parameters()) + list(pc_loss_fn.parameters())
    )
    optimizer = torch.optim.AdamW(all_params, lr=args.lr, weight_decay=0.01)
    scheduler = torch.optim.lr_scheduler.CosineAnnealingLR(optimizer, args.steps)

    total_params = sum(p.numel() for p in all_params)
    print(f"trainable: {total_params:,}")
    print(f"steps: {args.steps} | batch: {args.batch_size} | lr: {args.lr}")
    print(
        f"weights — span: {args.span_weight} | pc: {args.pc_weight} "
        f"| recon: {args.recon_weight}"
    )
    print(
        f"context seq_len: {args.context_seq_len} | val every: {args.val_every} "
        f"| patience: {args.patience}"
    )
    print()

    model.train()
    best_val_f1 = 0.0
    patience_counter = 0
    t0 = time.time()
    step = 0

    for step in range(1, args.steps + 1):
        idx = torch.randint(0, n_train, (args.batch_size,))
        embeddings = train_data["embeddings"][idx].to(device)
        answers = [train_data["answers"][i] for i in idx]
        contexts = [train_data["contexts"][i] for i in idx]

        out = model.forward_train(embeddings)
        losses = {}

        h = torch.stack(out["layer_outputs"], dim=1)
        context_ids, context_lengths = encode_bytes(
            contexts, args.context_seq_len, device,
        )
        gold_starts, gold_ends = find_answer_spans(
            contexts, answers, args.context_seq_len,
        )
        start_targets = torch.tensor(gold_starts, device=device)
        end_targets = torch.tensor(gold_ends, device=device)

        start_logits, end_logits = model.span_extractor(
            context_ids, context_lengths, h,
        )
        losses["span"] = args.span_weight * (
            F.cross_entropy(start_logits, start_targets)
            + F.cross_entropy(end_logits, end_targets)
        ) / 2

        td_fe, bu_fe = pc_loss_fn(out["layer_outputs"])
        losses["pc"] = args.pc_weight * (td_fe + bu_fe)

        losses["recon"] = args.recon_weight * (
            1.0
            - F.cosine_similarity(out["reconstructed"], embeddings, dim=-1).mean()
        )

        loss = sum(losses.values())
        optimizer.zero_grad()
        loss.backward()
        torch.nn.utils.clip_grad_norm_(all_params, 1.0)
        optimizer.step()
        scheduler.step()

        if step % args.log_every == 0 or step == 1:
            elapsed = time.time() - t0
            rate = step / elapsed
            eta = (args.steps - step) / rate if rate > 0 else 0
            lr = scheduler.get_last_lr()[0]
            parts = " ".join(f"{k}:{v.item():.4f}" for k, v in losses.items())
            print(
                f"  step {step}/{args.steps} | loss {loss.item():.4f} ({parts}) "
                f"| lr {lr:.2e} | {rate:.1f} step/s | eta {eta:.0f}s"
            )

        if step % args.val_every == 0:
            metrics = run_validation(
                model,
                val_data,
                args.context_seq_len,
                device,
                args.batch_size,
                args.val_samples,
            )
            print(
                f"  VAL step {step} | loss {metrics['loss']:.4f} "
                f"| EM {metrics['em']:.3f} | F1 {metrics['f1']:.3f}"
            )

            if metrics["f1"] > best_val_f1:
                best_val_f1 = metrics["f1"]
                patience_counter = 0
                save_checkpoint(model, config, pc_loss_fn, args.save)
                print(f"  ^ saved best (F1={best_val_f1:.3f})")
            else:
                patience_counter += 1
                if patience_counter >= args.patience:
                    print(
                        f"  early stop: no improvement for "
                        f"{args.patience} validations"
                    )
                    break

    elapsed = time.time() - t0
    print(f"\ndone: {step} steps in {elapsed:.1f}s | best val F1: {best_val_f1:.3f}")
    print(f"checkpoint: {args.save}")


if __name__ == "__main__":
    main()
