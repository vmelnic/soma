"""
SOMA Synthesizer — Training data generation and training loop.

This is the "compiler" that produces a functioning Mind for a specific Body.
It takes the Body's operation manifest and generates training data (intent-action
pairs), then trains the neural architecture to map intents to operations.

Usage:
    python -m poc.synthesis
"""

import json
import os
import random
import time
from pathlib import Path

import torch
import torch.nn as nn
from torch.utils.data import DataLoader, Dataset

from poc.body import OPERATIONS, NUM_OPERATIONS, MAX_PARAM_SLOTS
from poc.mind import SomaMind
from poc.tokenizer import Tokenizer, find_span, NULL_IDX

# ============================================================================
# Training Data Templates
# ============================================================================

PATHS = [
    "/tmp", ".", "..", "~/Desktop", "~/Documents", "~/Downloads",
    "/var/log", "/etc", "/Users/vm/Projects", "/opt", "/usr/local/bin",
    "~/Pictures", "/home", "~", "/Users/vm",
]

FILENAMES = [
    "hello.txt", "test.txt", "readme.md", "data.csv", "notes.txt",
    "output.log", "config.json", "temp.txt", "report.txt", "todo.txt",
    "index.html", "main.py", "app.js", "style.css", "info.txt",
]

CONTENTS = [
    "hello", "hello world", "test content", "foo bar", "some data",
    "important notes", "first line", "sample text", "test", "ok",
    "this is a test", "readme", "data here", "my content", "done",
]

PATTERNS = [
    "*.txt", "*.py", "*.md", "*.log", "*.json", "*.csv", "*.js",
    "*.html", "*.css", "test*", "readme*", "*.xml", "config*",
]

TEMPLATES: dict[int, list[str]] = {
    # 0: LIST_DIR
    0: [
        "list files in {path}",
        "show files in {path}",
        "show me the files in {path}",
        "what files are in {path}",
        "display contents of {path}",
        "ls {path}",
        "what's in {path}",
        "list the contents of {path}",
        "show me what's in {path}",
        "get files from {path}",
        "list directory {path}",
        "dir {path}",
        "files in {path}",
        "please show files in {path}",
        "i want to see files in {path}",
        "what do we have in {path}",
        "show directory listing for {path}",
        "get directory contents of {path}",
        "can you list files in {path}",
        "display files in {path}",
        "show me everything in {path}",
        "check out files in {path}",
        "browse {path}",
        "explore {path}",
        "look at files in {path}",
        "see what is in {path}",
        "check what files are in {path}",
        "give me a listing of {path}",
        "what exists in {path}",
        "enumerate all files in {path}",
    ],
    # 1: CREATE_FILE
    1: [
        "create a file called {path} with content {content}",
        "make a file {path} containing {content}",
        "create {path} with content {content}",
        "make a new file {path} with {content}",
        "create file {path} content {content}",
        "save {content} to file {path}",
        "new file {path} with text {content}",
        "put {content} into a new file called {path}",
        "write a file named {path} with the content {content}",
        "please create {path} containing {content}",
        "make file {path} with {content} inside",
        "generate a file {path} with {content}",
        "create a new file {path} and write {content}",
        "store {content} in file {path}",
        "write file {path} with content {content}",
        "create a file {path} and put {content} in it",
        "write to file {path} the content {content}",
        "create a text file {path} with {content}",
        "save content {content} into file {path}",
        "write the text {content} to a file called {path}",
    ],
    # 2: READ_FILE
    2: [
        "read {path}",
        "show me {path}",
        "cat {path}",
        "display {path}",
        "open {path}",
        "read the file {path}",
        "show contents of {path}",
        "what's in file {path}",
        "read file {path}",
        "print {path}",
        "view {path}",
        "show {path} contents",
        "get contents of {path}",
        "display file {path}",
        "read the contents of {path}",
        "please show me file {path}",
        "can you read {path}",
        "let me see file {path}",
        "output the contents of {path}",
        "type {path}",
        "show the content of file {path}",
        "print out {path}",
        "dump {path}",
        "what does {path} contain",
        "read out {path}",
    ],
    # 3: DELETE_FILE
    3: [
        "delete {path}",
        "remove {path}",
        "rm {path}",
        "delete the file {path}",
        "remove file {path}",
        "erase {path}",
        "get rid of {path}",
        "trash {path}",
        "delete file {path}",
        "please delete {path}",
        "can you remove {path}",
        "destroy {path}",
        "wipe {path}",
        "unlink {path}",
        "remove the file {path}",
        "eliminate {path}",
        "throw away {path}",
        "discard {path}",
    ],
    # 4: MAKE_DIR
    4: [
        "create directory {path}",
        "make directory {path}",
        "mkdir {path}",
        "create a folder {path}",
        "make a folder called {path}",
        "new directory {path}",
        "create folder {path}",
        "make folder {path}",
        "please create directory {path}",
        "add a directory {path}",
        "create a new folder {path}",
        "set up directory {path}",
        "make a new directory called {path}",
        "create a new directory {path}",
        "add directory {path}",
        "new folder {path}",
    ],
    # 5: FILE_INFO
    5: [
        "info about {path}",
        "file info {path}",
        "details of {path}",
        "stat {path}",
        "get info on {path}",
        "show details of {path}",
        "file details {path}",
        "metadata for {path}",
        "what is the size of {path}",
        "when was {path} modified",
        "show file information for {path}",
        "tell me about file {path}",
        "properties of {path}",
        "get file details for {path}",
        "file properties {path}",
        "describe file {path}",
    ],
    # 6: CURRENT_DIR
    6: [
        "where am i",
        "current directory",
        "pwd",
        "what directory am i in",
        "print working directory",
        "show current directory",
        "which directory",
        "current path",
        "what is the current directory",
        "show me the current path",
        "what folder am i in",
        "get current directory",
        "where are we",
        "current location",
        "what's the working directory",
        "where am i right now",
        "what is my current location",
        "what path am i in",
        "show my location",
        "present working directory",
    ],
    # 7: SYSTEM_INFO
    7: [
        "system info",
        "system information",
        "show system info",
        "what system is this",
        "uname",
        "tell me about this system",
        "what os is this",
        "operating system info",
        "show os information",
        "what machine is this",
        "system details",
        "get system information",
        "what platform am i on",
        "show machine info",
        "computer info",
        "give me system information",
        "describe this computer",
        "what hardware is this",
        "show computer details",
        "os details",
    ],
    # 8: CURRENT_TIME
    8: [
        "what time is it",
        "current time",
        "tell me the time",
        "show the time",
        "time now",
        "what is the time",
        "get current time",
        "show current date",
        "what day is it",
        "current date and time",
        "give me the time",
        "tell me the date",
        "date",
        "what's the time",
        "time please",
        "whats the current time",
        "what is the current time right now",
        "display the time",
        "show me the time",
        "what's the date today",
        "what is today's date",
        "current time and date",
        "right now what time is it",
        "clock",
    ],
    # 9: DISK_USAGE
    9: [
        "disk usage",
        "disk space",
        "how much disk space",
        "show disk usage",
        "df",
        "storage space",
        "how much space is left",
        "check disk space",
        "available disk space",
        "free space",
        "how much storage",
        "show storage info",
        "how much free space do i have",
        "storage left",
        "remaining disk space",
        "how full is the disk",
        "space available",
        "check storage",
        "disk free space",
        "how much room is left",
    ],
    # 10: PROCESS_LIST
    10: [
        "list processes",
        "show processes",
        "running processes",
        "ps",
        "what's running",
        "show running processes",
        "process list",
        "active processes",
        "list running processes",
        "what processes are running",
        "top processes",
        "show active processes",
        "what processes are active",
        "current processes",
        "show me all processes",
        "what is currently running",
        "display running processes",
        "tasks running",
        "show all tasks",
        "what tasks are active",
    ],
    # 11: MOVE_FILE
    11: [
        "move {source} to {destination}",
        "rename {source} to {destination}",
        "mv {source} {destination}",
        "move file {source} to {destination}",
        "rename file {source} to {destination}",
        "relocate {source} to {destination}",
        "move {source} into {destination}",
        "please move {source} to {destination}",
        "transfer {source} to {destination}",
        "can you move {source} to {destination}",
        "move the file {source} to {destination}",
        "rename the file {source} as {destination}",
    ],
    # 12: COPY_FILE
    12: [
        "copy {source} to {destination}",
        "cp {source} {destination}",
        "duplicate {source} to {destination}",
        "copy file {source} to {destination}",
        "make a copy of {source} as {destination}",
        "clone {source} to {destination}",
        "copy {source} into {destination}",
        "please copy {source} to {destination}",
        "replicate {source} as {destination}",
        "can you copy {source} to {destination}",
        "copy the file {source} to {destination}",
        "duplicate file {source} as {destination}",
        "back up {source} to {destination}",
        "create a copy of {source} called {destination}",
    ],
    # 13: FIND_FILE
    13: [
        "find {pattern}",
        "search for {pattern}",
        "find files matching {pattern}",
        "look for {pattern}",
        "search {pattern}",
        "find all {pattern} files",
        "locate {pattern}",
        "where are the {pattern} files",
        "find files named {pattern}",
        "search for files matching {pattern}",
        "please find {pattern}",
        "can you find {pattern}",
        "look for files named {pattern}",
        "hunt for {pattern}",
        "scan for {pattern}",
    ],
    # 14: FILE_EXISTS
    14: [
        "does {path} exist",
        "check if {path} exists",
        "is there a file {path}",
        "file exists {path}",
        "does file {path} exist",
        "check {path} exists",
        "is {path} there",
        "see if {path} exists",
        "does {path} exist on disk",
        "verify {path} exists",
        "is there a {path}",
        "check for {path}",
        "tell me if {path} exists",
        "does the file {path} exist",
        "is {path} present",
        "look if {path} exists",
        "test if {path} exists",
    ],
}

# Templates for CREATE_FILE without content (single param: path only)
CREATE_FILE_NO_CONTENT = [
    "create file {path}",
    "create {path}",
    "make file {path}",
    "touch {path}",
    "create a file {path}",
    "create a file called {path}",
    "make a file called {path}",
    "create an empty file {path}",
    "new file {path}",
    "make a new file {path}",
    "please create file {path}",
    "generate file {path}",
    "create a new file called {path}",
    "make a new file called {path}",
    "add a file {path}",
    "create a blank file {path}",
]


def _get_param_pool(param_type: str, param_name: str) -> list[str]:
    if param_name == "pattern":
        return PATTERNS
    if param_name == "content":
        return CONTENTS
    if param_name in ("source", "destination"):
        return FILENAMES
    if param_type == "path":
        return PATHS + FILENAMES
    return FILENAMES


def generate_training_data(seed: int = 42) -> list[dict]:
    """Generate synthetic training examples from templates."""
    rng = random.Random(seed)
    examples = []

    for op in OPERATIONS:
        templates = TEMPLATES[op.opcode]
        num_params = len(op.params)

        if num_params == 0:
            # Oversample zero-param ops to balance against 1/2-param ops
            for template in templates:
                for _ in range(8):
                    examples.append({
                        "text": template,
                        "op_id": op.opcode,
                        "param_values": [None, None],
                    })

        if num_params == 1:
            pool = _get_param_pool(op.params[0].type, op.params[0].name)
            for template in templates:
                sampled = rng.sample(pool, min(len(pool), 7))
                for val in sampled:
                    text = template.format(
                        path=val, pattern=val, source=val, destination=val
                    )
                    examples.append({
                        "text": text,
                        "op_id": op.opcode,
                        "param_values": [val, None],
                    })

        # CREATE_FILE also has single-param templates (no content)
        if op.opcode == 1:
            pool = _get_param_pool("path", "path")
            for template in CREATE_FILE_NO_CONTENT:
                sampled = rng.sample(pool, min(len(pool), 7))
                for val in sampled:
                    text = template.format(path=val)
                    examples.append({
                        "text": text,
                        "op_id": op.opcode,
                        "param_values": [val, None],
                    })

        if num_params == 2:
            pool0 = _get_param_pool(op.params[0].type, op.params[0].name)
            pool1 = _get_param_pool(op.params[1].type, op.params[1].name)
            for template in templates:
                pairs = []
                for _ in range(12):
                    v0 = rng.choice(pool0)
                    v1 = rng.choice(pool1)
                    if v0 != v1:
                        pairs.append((v0, v1))
                for v0, v1 in pairs:
                    text = template.format(
                        path=v0, content=v1,
                        source=v0, destination=v1,
                    )
                    examples.append({
                        "text": text,
                        "op_id": op.opcode,
                        "param_values": [v0, v1],
                    })

    rng.shuffle(examples)
    return examples


def prepare_example(example: dict, tokenizer: Tokenizer) -> dict | None:
    """Convert raw example to training format with span positions.
    Prepends <NULL> token so null spans point at index 0."""
    tokens = tokenizer.tokenize(example["text"])
    token_ids = [NULL_IDX] + tokenizer.encode(example["text"])
    length = len(token_ids)

    spans_start = []
    spans_end = []
    for param_val in example["param_values"]:
        if param_val is None:
            spans_start.append(0)
            spans_end.append(0)
        else:
            param_tokens = tokenizer.tokenize(param_val)
            span = find_span(tokens, param_tokens)
            if span is None:
                return None
            spans_start.append(span[0] + 1)
            spans_end.append(span[1] + 1)

    return {
        "token_ids": token_ids,
        "length": length,
        "op_id": example["op_id"],
        "span_starts": spans_start,
        "span_ends": spans_end,
    }


class SomaDataset(Dataset):
    def __init__(self, examples: list[dict]):
        self.examples = examples

    def __len__(self):
        return len(self.examples)

    def __getitem__(self, idx):
        return self.examples[idx]


def collate_fn(batch: list[dict]) -> dict:
    max_len = max(b["length"] for b in batch)
    padded_ids = [
        b["token_ids"] + [0] * (max_len - b["length"])
        for b in batch
    ]
    return {
        "input_ids": torch.tensor(padded_ids, dtype=torch.long),
        "lengths": torch.tensor([b["length"] for b in batch], dtype=torch.long),
        "op_ids": torch.tensor([b["op_id"] for b in batch], dtype=torch.long),
        "span_starts": torch.tensor([b["span_starts"] for b in batch], dtype=torch.long),
        "span_ends": torch.tensor([b["span_ends"] for b in batch], dtype=torch.long),
    }


def train_soma(
    save_dir: str = "poc/artifacts",
    epochs: int = 120,
    batch_size: int = 32,
    lr: float = 2e-3,
    patience: int = 20,
    seed: int = 42,
):
    """Synthesize a SOMA Mind onto the Body."""
    torch.manual_seed(seed)
    random.seed(seed)
    device = torch.device("cpu")
    print(f"Device: {device}")

    # --- Generate training data ---
    print("\n[Synthesis] Generating training data...")
    raw_examples = generate_training_data(seed=seed)
    print(f"  Raw examples: {len(raw_examples)}")

    tokenizer = Tokenizer()
    tokenizer.build_vocab([ex["text"] for ex in raw_examples])
    print(f"  Vocabulary size: {tokenizer.vocab_size}")

    prepared = []
    skipped = 0
    for ex in raw_examples:
        p = prepare_example(ex, tokenizer)
        if p is not None:
            prepared.append(p)
        else:
            skipped += 1
    print(f"  Prepared examples: {len(prepared)} (skipped {skipped})")

    n = len(prepared)
    n_train = int(0.8 * n)
    n_val = int(0.1 * n)
    train_data = prepared[:n_train]
    val_data = prepared[n_train:n_train + n_val]
    test_data = prepared[n_train + n_val:]
    print(f"  Train: {len(train_data)}, Val: {len(val_data)}, Test: {len(test_data)}")

    op_counts = [0] * NUM_OPERATIONS
    for ex in train_data:
        op_counts[ex["op_id"]] += 1

    weights = torch.zeros(NUM_OPERATIONS)
    for i, c in enumerate(op_counts):
        weights[i] = 1.0 / max(c, 1)
    weights = weights / weights.sum() * NUM_OPERATIONS

    # --- Create model ---
    model = SomaMind(vocab_size=tokenizer.vocab_size).to(device)
    total_params = sum(p.numel() for p in model.parameters())
    print(f"\n[Synthesis] Model: {total_params:,} parameters")

    criterion_op = nn.CrossEntropyLoss(weight=weights.to(device))
    criterion_span = nn.CrossEntropyLoss()
    optimizer = torch.optim.AdamW(model.parameters(), lr=lr, weight_decay=1e-2)
    scheduler = torch.optim.lr_scheduler.ReduceLROnPlateau(
        optimizer, mode="min", factor=0.5, patience=5
    )

    train_loader = DataLoader(
        SomaDataset(train_data), batch_size=batch_size,
        shuffle=True, collate_fn=collate_fn,
    )
    val_loader = DataLoader(
        SomaDataset(val_data), batch_size=batch_size,
        shuffle=False, collate_fn=collate_fn,
    )

    # --- Training ---
    print(f"\n[Synthesis] Training for up to {epochs} epochs (patience={patience})...")
    best_val_loss = float("inf")
    best_epoch = 0
    no_improve = 0
    best_state = None

    t0 = time.time()
    for epoch in range(1, epochs + 1):
        model.train()
        train_loss = 0.0
        train_correct_op = 0
        train_total = 0

        for batch in train_loader:
            input_ids = batch["input_ids"].to(device)
            lengths = batch["lengths"].to(device)
            op_ids = batch["op_ids"].to(device)
            span_starts = batch["span_starts"].to(device)
            span_ends = batch["span_ends"].to(device)

            op_logits, span_logits = model(input_ids, lengths)
            loss_op = criterion_op(op_logits, op_ids)

            loss_span = torch.tensor(0.0, device=device)
            for slot_idx, (s_logits, e_logits) in enumerate(span_logits):
                loss_span = loss_span + criterion_span(s_logits, span_starts[:, slot_idx])
                loss_span = loss_span + criterion_span(e_logits, span_ends[:, slot_idx])

            loss = loss_op + loss_span

            optimizer.zero_grad()
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
            optimizer.step()

            train_loss += loss.item() * input_ids.size(0)
            train_correct_op += (op_logits.argmax(dim=-1) == op_ids).sum().item()
            train_total += input_ids.size(0)

        train_loss /= train_total
        train_acc = train_correct_op / train_total

        # Validate
        model.eval()
        val_loss = 0.0
        val_correct_op = 0
        val_correct_span = 0
        val_correct_e2e = 0
        val_total = 0

        with torch.no_grad():
            for batch in val_loader:
                input_ids = batch["input_ids"].to(device)
                lengths = batch["lengths"].to(device)
                op_ids = batch["op_ids"].to(device)
                span_starts = batch["span_starts"].to(device)
                span_ends = batch["span_ends"].to(device)

                op_logits, span_logits = model(input_ids, lengths)
                loss_op = criterion_op(op_logits, op_ids)
                loss_span = torch.tensor(0.0, device=device)
                for slot_idx, (s_logits, e_logits) in enumerate(span_logits):
                    loss_span = loss_span + criterion_span(s_logits, span_starts[:, slot_idx])
                    loss_span = loss_span + criterion_span(e_logits, span_ends[:, slot_idx])
                loss = loss_op + loss_span

                val_loss += loss.item() * input_ids.size(0)
                pred_ops = op_logits.argmax(dim=-1)
                op_correct = (pred_ops == op_ids)
                val_correct_op += op_correct.sum().item()

                all_spans_correct = torch.ones(input_ids.size(0), dtype=torch.bool, device=device)
                for slot_idx, (s_logits, e_logits) in enumerate(span_logits):
                    pred_s = s_logits.argmax(dim=-1)
                    pred_e = e_logits.argmax(dim=-1)
                    s_ok = (pred_s == span_starts[:, slot_idx])
                    e_ok = (pred_e == span_ends[:, slot_idx])
                    all_spans_correct = all_spans_correct & s_ok & e_ok

                val_correct_span += all_spans_correct.sum().item()
                val_correct_e2e += (op_correct & all_spans_correct).sum().item()
                val_total += input_ids.size(0)

        val_loss /= val_total
        val_op_acc = val_correct_op / val_total
        val_span_acc = val_correct_span / val_total
        val_e2e_acc = val_correct_e2e / val_total

        scheduler.step(val_loss)

        if epoch % 10 == 0 or epoch == 1:
            print(
                f"  Epoch {epoch:3d} | "
                f"Train loss={train_loss:.4f} acc={train_acc:.3f} | "
                f"Val loss={val_loss:.4f} op={val_op_acc:.3f} "
                f"span={val_span_acc:.3f} e2e={val_e2e_acc:.3f}"
            )

        if val_loss < best_val_loss:
            best_val_loss = val_loss
            best_epoch = epoch
            best_state = {k: v.cpu().clone() for k, v in model.state_dict().items()}
            no_improve = 0
        else:
            no_improve += 1
            if no_improve >= patience:
                print(f"  Early stopping at epoch {epoch} (best={best_epoch})")
                break

    elapsed = time.time() - t0
    print(f"\n[Synthesis] Training complete in {elapsed:.1f}s (best epoch: {best_epoch})")

    model.load_state_dict(best_state)

    # --- Test set ---
    print("\n[Synthesis] Test set results:")
    model.eval()
    test_correct_op = 0
    test_correct_span = 0
    test_correct_e2e = 0
    test_total = 0

    test_loader = DataLoader(
        SomaDataset(test_data), batch_size=batch_size,
        shuffle=False, collate_fn=collate_fn,
    )

    with torch.no_grad():
        for batch in test_loader:
            input_ids = batch["input_ids"].to(device)
            lengths = batch["lengths"].to(device)
            op_ids = batch["op_ids"].to(device)
            span_starts = batch["span_starts"].to(device)
            span_ends = batch["span_ends"].to(device)

            op_logits, span_logits = model(input_ids, lengths)
            pred_ops = op_logits.argmax(dim=-1)
            op_correct = (pred_ops == op_ids)
            test_correct_op += op_correct.sum().item()

            all_spans_correct = torch.ones(input_ids.size(0), dtype=torch.bool, device=device)
            for slot_idx, (s_logits, e_logits) in enumerate(span_logits):
                pred_s = s_logits.argmax(dim=-1)
                pred_e = e_logits.argmax(dim=-1)
                s_ok = (pred_s == span_starts[:, slot_idx])
                e_ok = (pred_e == span_ends[:, slot_idx])
                all_spans_correct = all_spans_correct & s_ok & e_ok

            test_correct_span += all_spans_correct.sum().item()
            test_correct_e2e += (op_correct & all_spans_correct).sum().item()
            test_total += input_ids.size(0)

    test_op_acc = test_correct_op / test_total
    test_span_acc = test_correct_span / test_total
    test_e2e_acc = test_correct_e2e / test_total

    print(f"  Op Accuracy:   {test_op_acc:.3f}")
    print(f"  Span Accuracy: {test_span_acc:.3f}")
    print(f"  E2E Accuracy:  {test_e2e_acc:.3f}")

    # --- Save ---
    os.makedirs(save_dir, exist_ok=True)
    model_path = os.path.join(save_dir, "soma_mind.pt")
    vocab_path = os.path.join(save_dir, "vocab.json")

    torch.save(best_state, model_path)
    tokenizer.save(vocab_path)
    print(f"\n[Synthesis] Saved model -> {model_path}")
    print(f"[Synthesis] Saved vocab -> {vocab_path}")
    print("[Synthesis] SOMA synthesis complete.")

    return {
        "test_op_acc": test_op_acc,
        "test_span_acc": test_span_acc,
        "test_e2e_acc": test_e2e_acc,
        "best_epoch": best_epoch,
        "training_time": elapsed,
    }


if __name__ == "__main__":
    train_soma()
