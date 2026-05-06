"""
Benchmark SOMA brain on SQuAD 2.0.

Three benchmarks:
  1. Retrieval — does SDM return passages containing the gold answer?
  2. Span extraction — EM and F1 on extracted answer spans vs gold
  3. TTT adaptation — accuracy improvement within question clusters

Usage:
  python benchmark.py [--checkpoint checkpoints/brain.pt]
"""

import argparse
import os
import re
import time
from collections import Counter, defaultdict

import torch

from soma_brain import SomaBrain, BrainConfig


def get_device():
    if torch.backends.mps.is_available():
        return torch.device("mps")
    if torch.cuda.is_available():
        return torch.device("cuda")
    return torch.device("cpu")


def load_checkpoint(path, device):
    data = torch.load(path, map_location=device, weights_only=False)
    config = data["config"]
    model = SomaBrain(config).to(device)
    model.load_state_dict(data["state_dict"], strict=False)

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

    return model, config


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


def extract_answer_text(context_ids, start, end):
    span = context_ids[start : end + 1].tolist()
    return bytes(b for b in span if 0 < b < 256).decode("utf-8", errors="replace")


def bench_retrieval(model, val_data, device, max_samples=1000):
    """Does SDM retrieve passages containing the gold answer?"""
    print("=== Retrieval Accuracy ===")
    n = min(len(val_data["answers"]), max_samples)

    hits = {1: 0, 5: 0, 8: 0}

    with torch.no_grad():
        for i in range(n):
            emb = val_data["embeddings"][i : i + 1].to(device)
            gold = val_data["answers"][i].lower()

            result = model.reason(emb, top_k_sources=8)

            for rank, (text, _score) in enumerate(result.sources):
                if gold in text.lower():
                    for k in hits:
                        if rank < k:
                            hits[k] += 1
                    break

            if (i + 1) % 200 == 0:
                print(f"  {i + 1}/{n}...")

    results = {f"hit@{k}": v / n for k, v in hits.items()}
    print(
        f"  hit@1: {results['hit@1']:.3f} | "
        f"hit@5: {results['hit@5']:.3f} | "
        f"hit@8: {results['hit@8']:.3f}"
    )
    print(f"  ({n} questions)\n")
    return results


def bench_span_extraction(
    model, val_data, device, context_seq_len=2048, max_samples=500
):
    """Extract answer spans and measure EM / F1."""
    print("=== Span Extraction ===")
    n = min(len(val_data["answers"]), max_samples)

    total_em = 0
    total_f1 = 0.0
    examples = []
    batch_size = 8

    with torch.no_grad():
        for i in range(0, n, batch_size):
            end = min(i + batch_size, n)
            embeddings = val_data["embeddings"][i:end].to(device)
            contexts = [val_data["contexts"][j] for j in range(i, end)]

            out = model.forward_train(embeddings)
            h = torch.stack(out["layer_outputs"], dim=1)

            context_ids, context_lengths = encode_bytes(
                contexts, context_seq_len, device,
            )
            spans = model.span_extractor.extract(
                context_ids, context_lengths, h,
            )

            for b in range(end - i):
                gold = val_data["answers"][i + b]
                s, e = spans[b]
                pred = extract_answer_text(context_ids[b], s, e)

                em = int(normalize_answer(pred) == normalize_answer(gold))
                f1 = compute_f1(pred, gold)
                total_em += em
                total_f1 += f1

                if len(examples) < 10:
                    examples.append(
                        (val_data["questions"][i + b], gold, pred, em, f1)
                    )

            if (i + batch_size) % 100 < batch_size:
                print(f"  {min(i + batch_size, n)}/{n}...")

    em_score = total_em / n
    f1_score = total_f1 / n
    print(f"  EM: {em_score:.3f} | F1: {f1_score:.3f}")
    print(f"  ({n} questions)\n")

    print("  examples:")
    for q, gold, pred, em, f1 in examples[:5]:
        print(f"    Q: {q[:70]}")
        print(f"    gold: {gold}")
        print(f"    pred: {pred[:80]}")
        print(f"    EM={em} F1={f1:.2f}\n")

    return {"em": em_score, "f1": f1_score}


def bench_ttt_adaptation(
    model, val_data, device, context_seq_len=2048, max_clusters=50,
):
    """Test TTT online adaptation within question clusters."""
    print("=== TTT Adaptation ===")

    clusters = defaultdict(list)
    for i in range(len(val_data["answers"])):
        ctx = val_data["contexts"][i]
        clusters[ctx].append(i)

    multi = [
        (ctx, idxs)
        for ctx, idxs in clusters.items()
        if len(idxs) >= 3
    ]
    multi = multi[:max_clusters]

    if not multi:
        print("  no multi-question clusters found\n")
        return {}

    first_half_f1 = []
    second_half_f1 = []

    with torch.no_grad():
        for ctx_text, idxs in multi:
            model.ttt.reset_state()
            cluster_f1 = []

            context_ids, context_lengths = encode_bytes(
                [ctx_text], context_seq_len, device,
            )

            for idx in idxs:
                emb = val_data["embeddings"][idx : idx + 1].to(device)
                gold = val_data["answers"][idx]

                x = model.input_proj(emb).unsqueeze(1)
                for block in model.blocks:
                    x, _ = block(x, model.sdm)
                x = model.ttt(x, update=True)

                h = x
                spans = model.span_extractor.extract(
                    context_ids, context_lengths, h,
                )
                s, e = spans[0]
                pred = extract_answer_text(context_ids[0], s, e)
                cluster_f1.append(compute_f1(pred, gold))

            mid = len(cluster_f1) // 2
            first_half_f1.extend(cluster_f1[:mid])
            second_half_f1.extend(cluster_f1[mid:])

    first_avg = sum(first_half_f1) / len(first_half_f1) if first_half_f1 else 0
    second_avg = (
        sum(second_half_f1) / len(second_half_f1) if second_half_f1 else 0
    )
    delta = second_avg - first_avg

    print(f"  clusters: {len(multi)}")
    print(f"  first-half F1:  {first_avg:.3f} ({len(first_half_f1)} questions)")
    print(f"  second-half F1: {second_avg:.3f} ({len(second_half_f1)} questions)")
    print(f"  TTT delta:      {delta:+.3f}")
    if delta > 0:
        print(f"  -> TTT adaptation detected (+{delta:.3f})\n")
    else:
        print(f"  -> no TTT adaptation detected\n")

    return {
        "first_half_f1": first_avg,
        "second_half_f1": second_avg,
        "ttt_delta": delta,
    }


def main():
    parser = argparse.ArgumentParser(description="Benchmark SOMA brain on SQuAD")
    parser.add_argument("--checkpoint", default="checkpoints/brain.pt")
    parser.add_argument("--val-data", default="data/val_qa.pt")
    parser.add_argument("--retrieval-samples", type=int, default=1000)
    parser.add_argument("--gen-samples", type=int, default=500)
    parser.add_argument("--ttt-clusters", type=int, default=50)
    parser.add_argument("--context-seq-len", type=int, default=1024)
    args = parser.parse_args()

    if not os.path.exists(args.checkpoint):
        print(f"error: {args.checkpoint} not found — run train.py first")
        raise SystemExit(1)

    device = get_device()
    print(f"device: {device}")

    model, config = load_checkpoint(args.checkpoint, device)
    model.train(False)
    print(f"SDM: {model.sdm.num_locations} entries | sources: {len(model.source_texts)}")

    params = model.count_parameters()
    print(f"model: {params['total_unique']:,} params\n")

    val_data = torch.load(args.val_data, map_location="cpu", weights_only=False)
    print(f"val: {len(val_data['answers'])} pairs\n")
    t0 = time.time()

    retrieval = bench_retrieval(model, val_data, device, args.retrieval_samples)
    extraction = bench_span_extraction(
        model, val_data, device, args.context_seq_len, args.gen_samples,
    )
    ttt = bench_ttt_adaptation(
        model, val_data, device, args.context_seq_len, args.ttt_clusters,
    )

    elapsed = time.time() - t0
    print("=" * 60)
    print("SUMMARY")
    print("=" * 60)
    print(
        f"  retrieval: hit@1={retrieval.get('hit@1', 0):.3f} "
        f"hit@5={retrieval.get('hit@5', 0):.3f} "
        f"hit@8={retrieval.get('hit@8', 0):.3f}"
    )
    print(
        f"  span extraction: EM={extraction.get('em', 0):.3f} "
        f"F1={extraction.get('f1', 0):.3f}"
    )
    if ttt:
        print(f"  TTT delta: {ttt.get('ttt_delta', 0):+.3f}")
    print(
        f"  model: {params['total_unique']:,} params | "
        f"SDM: {model.sdm.num_locations} entries"
    )
    print(f"  benchmark time: {elapsed:.0f}s")
    print("=" * 60)


if __name__ == "__main__":
    main()
