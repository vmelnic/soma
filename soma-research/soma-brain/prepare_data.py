"""
Prepare SQuAD 2.0 for SOMA brain training.

Downloads the dataset, ingests context passages into SDM, pre-computes
question embeddings. Run once before train.py.

Usage:
  pip install 'soma-brain[train]'
  python prepare_data.py [--size medium]

Outputs:
  checkpoints/brain_base.pt  — fresh model with SDM populated from passages
  data/train_qa.pt           — training (embeddings, answers, contexts)
  data/val_qa.pt             — validation (embeddings, answers, contexts)
"""

import argparse
import os
import time
from collections import OrderedDict

import torch

from soma_brain import SomaBrain, BrainConfig, Embedder


def get_device():
    if torch.backends.mps.is_available():
        return torch.device("mps")
    if torch.cuda.is_available():
        return torch.device("cuda")
    return torch.device("cpu")


def main():
    parser = argparse.ArgumentParser(description="Prepare SQuAD 2.0 for SOMA brain")
    parser.add_argument("--size", choices=["tiny", "medium", "small", "base"], default="medium")
    parser.add_argument("--save-checkpoint", default="checkpoints/brain_base.pt")
    parser.add_argument("--data-dir", default="data")
    parser.add_argument("--embed-batch", type=int, default=32)
    parser.add_argument("--max-passages", type=int, default=None)
    args = parser.parse_args()

    device = get_device()
    print(f"device: {device}")

    print("loading embedder...")
    embedder = Embedder()

    print("downloading SQuAD 2.0...")
    from datasets import load_dataset
    ds = load_dataset("rajpurkar/squad_v2")

    passages = OrderedDict()
    for split in ["train", "validation"]:
        for item in ds[split]:
            ctx = item["context"]
            if ctx not in passages:
                passages[ctx] = len(passages)

    passage_list = list(passages.keys())
    if args.max_passages:
        passage_list = passage_list[:args.max_passages]
        passages = {p: i for i, p in enumerate(passage_list)}
    print(f"unique passages: {len(passage_list)}")

    config = getattr(BrainConfig, args.size)()
    config.embed_dim = embedder.embed_dim
    model = SomaBrain(config).to(device)
    params = model.count_parameters()
    print(f"model: {params['total_unique']:,} params ({args.size})")

    print(f"\ningesting {len(passage_list)} passages into SDM...")
    t0 = time.time()
    for i in range(0, len(passage_list), args.embed_batch):
        batch = passage_list[i : i + args.embed_batch]
        embeds = embedder.embed(batch).to(device)
        for j, text in enumerate(batch):
            model.ingest(embeds[j : j + 1], text=text)
        done = min(i + args.embed_batch, len(passage_list))
        if done % 500 < args.embed_batch or done == len(passage_list):
            elapsed = time.time() - t0
            rate = done / elapsed if elapsed > 0 else 0
            print(f"  {done}/{len(passage_list)} | {rate:.0f}/s | {elapsed:.0f}s")
    print(f"SDM: {model.sdm.num_locations} entries in {time.time() - t0:.0f}s")

    os.makedirs(os.path.dirname(args.save_checkpoint) or ".", exist_ok=True)
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
        },
        args.save_checkpoint,
    )
    print(f"saved: {args.save_checkpoint}")

    os.makedirs(args.data_dir, exist_ok=True)

    for split_name, ds_key in [("train", "train"), ("val", "validation")]:
        questions = []
        answers = []
        contexts = []

        for item in ds[ds_key]:
            ans_texts = item["answers"]["text"]
            if not ans_texts:
                continue
            ctx = item["context"]
            if ctx not in passages:
                continue
            questions.append(item["question"])
            answers.append(ans_texts[0])
            contexts.append(ctx)

        print(f"\n{split_name}: {len(questions)} answerable QA pairs")
        print(f"embedding questions...")
        t0 = time.time()
        all_embeds = []
        for i in range(0, len(questions), args.embed_batch):
            batch = questions[i : i + args.embed_batch]
            embeds = embedder.embed(batch).cpu()
            all_embeds.append(embeds)
            done = min(i + args.embed_batch, len(questions))
            if done % 5000 < args.embed_batch or done == len(questions):
                elapsed = time.time() - t0
                rate = done / elapsed if elapsed > 0 else 0
                print(f"  {done}/{len(questions)} | {rate:.0f}/s | {elapsed:.0f}s")

        all_embeds = torch.cat(all_embeds, dim=0)
        out_path = os.path.join(args.data_dir, f"{split_name}_qa.pt")
        torch.save(
            {
                "embeddings": all_embeds,
                "questions": questions,
                "answers": answers,
                "contexts": contexts,
            },
            out_path,
        )
        print(f"saved: {out_path} ({all_embeds.shape[0]} pairs, {all_embeds.shape[1]}-dim)")

    print("\ndone — next: python train.py")


if __name__ == "__main__":
    main()
