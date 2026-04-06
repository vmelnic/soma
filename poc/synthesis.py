"""
SOMA Synthesizer — Phase 2: Program Sequence Training.

Generates (intent, program) pairs and trains the Mind to output
multi-step programs with data dependencies.

Usage:
    python -m poc.synthesis
"""

import os
import random
import time

import torch
import torch.nn as nn
from torch.utils.data import DataLoader, Dataset

from poc.body import (
    Prim, NUM_PRIMITIVES, MAX_PROGRAM_STEPS, OPCODE_SCHEMA, OPCODE_NAMES,
)
from poc.mind import SomaMind
from poc.tokenizer import Tokenizer, find_span, NULL_IDX

# ============================================================================
# Parameter pools
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

# Novel filenames that appear rarely in training — forces the model to
# rely on context (the verb) rather than memorizing specific filenames.
# These become near-UNK during training, teaching generalization.
NOVEL_FILENAMES = [
    "alpha.txt", "bravo.md", "charlie.log", "delta.json", "echo.py",
    "foxtrot.csv", "golf.html", "hotel.js", "india.xml", "juliet.txt",
    "kilo.md", "lima.log", "mike.json", "november.py", "oscar.csv",
    "papa.txt", "quebec.md", "romeo.log", "sierra.json", "tango.py",
    "uniform.txt", "victor.md", "whiskey.log", "xray.json", "yankee.py",
    "zulu.txt", "archive.tar", "backup.zip", "draft.doc", "spec.pdf",
]

ALL_PATHS = PATHS + FILENAMES + NOVEL_FILENAMES

# ============================================================================
# Task types: (templates, program generator)
# Each task maps intent templates to a program sequence.
# ============================================================================

# Program step shorthand:
# S(op, "span:name", "none") or S(op, "ref:N", "span:name")
def _s(op, a0="none", a1="none"):
    """Build a program step descriptor."""
    return (op, a0, a1)


TASK_TYPES = {
    "read_file": {
        "params": [("path", FILENAMES + NOVEL_FILENAMES)],
        "templates": [
            "read {path}", "cat {path}", "show me {path}", "display {path}",
            "open {path}", "read the file {path}", "show contents of {path}",
            "read file {path}", "print {path}", "view {path}",
            "get contents of {path}", "display file {path}",
            "read the contents of {path}", "can you read {path}",
            "let me see file {path}", "show the content of file {path}",
            "what does {path} contain", "dump {path}", "type {path}",
            "output the contents of {path}", "print out {path}",
            "read out {path}", "show {path} contents",
            "please show me file {path}", "what's in file {path}",
        ],
        "program": [
            _s(Prim.FILE_OPEN_R, "span:path"),
            _s(Prim.FILE_READ, "ref:0"),
            _s(Prim.FILE_CLOSE, "ref:0"),
            _s(Prim.EMIT, "ref:1"),
            _s(Prim.STOP),
        ],
    },
    "create_file_content": {
        "params": [("path", FILENAMES), ("content", CONTENTS)],
        "templates": [
            "create a file called {path} with content {content}",
            "make a file {path} containing {content}",
            "create {path} with content {content}",
            "make a new file {path} with {content}",
            "create file {path} content {content}",
            "save {content} to file {path}",
            "new file {path} with text {content}",
            "write a file named {path} with the content {content}",
            "please create {path} containing {content}",
            "make file {path} with {content} inside",
            "create a new file {path} and write {content}",
            "store {content} in file {path}",
            "write file {path} with content {content}",
            "create a text file {path} with {content}",
            "write the text {content} to a file called {path}",
        ],
        "program": [
            _s(Prim.FILE_OPEN_W, "span:path"),
            _s(Prim.FILE_WRITE, "ref:0", "span:content"),
            _s(Prim.FILE_CLOSE, "ref:0"),
            _s(Prim.EMIT, "ref:2"),
            _s(Prim.STOP),
        ],
    },
    "create_file_empty": {
        "params": [("path", FILENAMES)],
        "templates": [
            "create file {path}", "create {path}", "make file {path}",
            "touch {path}", "create a file {path}",
            "create a file called {path}", "make a file called {path}",
            "create an empty file {path}", "new file {path}",
            "make a new file {path}", "please create file {path}",
            "generate file {path}", "create a new file called {path}",
            "make a new file called {path}", "add a file {path}",
        ],
        "program": [
            _s(Prim.FILE_OPEN_W, "span:path"),
            _s(Prim.FILE_CLOSE, "ref:0"),
            _s(Prim.EMIT, "ref:1"),
            _s(Prim.STOP),
        ],
    },
    "delete_file": {
        "params": [("path", ALL_PATHS)],
        "templates": [
            "delete {path}", "remove {path}", "rm {path}",
            "delete the file {path}", "remove file {path}",
            "erase {path}", "get rid of {path}", "trash {path}",
            "delete file {path}", "please delete {path}",
            "can you remove {path}", "destroy {path}", "wipe {path}",
            "unlink {path}", "throw away {path}", "discard {path}",
            "eliminate {path}", "remove the file {path}",
        ],
        "program": [
            _s(Prim.FILE_DELETE, "span:path"),
            _s(Prim.EMIT, "ref:0"),
            _s(Prim.STOP),
        ],
    },
    "list_dir": {
        "params": [("path", ALL_PATHS)],
        "templates": [
            "list files in {path}", "show files in {path}",
            "show me the files in {path}", "what files are in {path}",
            "ls {path}", "what's in {path}", "list the contents of {path}",
            "show me what's in {path}", "get files from {path}",
            "list directory {path}", "dir {path}", "files in {path}",
            "please show files in {path}", "display files in {path}",
            "show me everything in {path}", "check out files in {path}",
            "browse {path}", "explore {path}", "look at files in {path}",
            "see what is in {path}", "what do we have in {path}",
            "show directory listing for {path}", "get directory contents of {path}",
            "can you list files in {path}", "enumerate all files in {path}",
            "display contents of {path}", "check what files are in {path}",
            "give me a listing of {path}", "what exists in {path}",
        ],
        "program": [
            _s(Prim.DIR_LIST, "span:path"),
            _s(Prim.EMIT, "ref:0"),
            _s(Prim.STOP),
        ],
    },
    "file_info": {
        "params": [("path", ALL_PATHS)],
        "templates": [
            "info about {path}", "file info {path}", "details of {path}",
            "stat {path}", "get info on {path}", "show details of {path}",
            "file details {path}", "metadata for {path}",
            "what is the size of {path}", "when was {path} modified",
            "show file information for {path}", "tell me about file {path}",
            "properties of {path}", "get file details for {path}",
            "describe file {path}", "file properties {path}",
        ],
        "program": [
            _s(Prim.FILE_STAT, "span:path"),
            _s(Prim.EMIT, "ref:0"),
            _s(Prim.STOP),
        ],
    },
    "make_dir": {
        "params": [("path", FILENAMES + ["projects", "output", "backup", "logs", "temp", "archive"])],
        "templates": [
            "create directory {path}", "make directory {path}", "mkdir {path}",
            "create a folder {path}", "make a folder called {path}",
            "new directory {path}", "create folder {path}", "make folder {path}",
            "please create directory {path}", "add a directory {path}",
            "create a new folder {path}", "set up directory {path}",
            "make a new directory called {path}", "create a new directory {path}",
            "add directory {path}", "new folder {path}",
        ],
        "program": [
            _s(Prim.DIR_CREATE, "span:path"),
            _s(Prim.EMIT, "ref:0"),
            _s(Prim.STOP),
        ],
    },
    "file_exists": {
        "params": [("path", ALL_PATHS)],
        "templates": [
            "does {path} exist", "check if {path} exists",
            "is there a file {path}", "file exists {path}",
            "does file {path} exist", "check {path} exists",
            "is {path} there", "see if {path} exists",
            "verify {path} exists", "is there a {path}",
            "check for {path}", "tell me if {path} exists",
            "does the file {path} exist", "is {path} present",
            "test if {path} exists", "look if {path} exists",
        ],
        "program": [
            _s(Prim.FILE_EXISTS, "span:path"),
            _s(Prim.EMIT, "ref:0"),
            _s(Prim.STOP),
        ],
    },
    "rename_file": {
        "params": [("source", FILENAMES), ("destination", FILENAMES)],
        "templates": [
            "move {source} to {destination}", "rename {source} to {destination}",
            "mv {source} {destination}", "move file {source} to {destination}",
            "rename file {source} to {destination}",
            "relocate {source} to {destination}",
            "move {source} into {destination}",
            "please move {source} to {destination}",
            "transfer {source} to {destination}",
            "can you move {source} to {destination}",
            "move the file {source} to {destination}",
            "rename the file {source} as {destination}",
        ],
        "program": [
            _s(Prim.FILE_RENAME, "span:source", "span:destination"),
            _s(Prim.EMIT, "ref:0"),
            _s(Prim.STOP),
        ],
    },
    "copy_file": {
        "params": [("source", FILENAMES), ("destination", FILENAMES)],
        "templates": [
            "copy {source} to {destination}", "cp {source} {destination}",
            "duplicate {source} to {destination}",
            "copy file {source} to {destination}",
            "make a copy of {source} as {destination}",
            "clone {source} to {destination}",
            "copy {source} into {destination}",
            "please copy {source} to {destination}",
            "can you copy {source} to {destination}",
            "copy the file {source} to {destination}",
            "duplicate file {source} as {destination}",
            "back up {source} to {destination}",
            "create a copy of {source} called {destination}",
        ],
        "program": [
            _s(Prim.FILE_COPY, "span:source", "span:destination"),
            _s(Prim.EMIT, "ref:0"),
            _s(Prim.STOP),
        ],
    },
    "current_dir": {
        "params": [],
        "templates": [
            "where am i", "current directory", "pwd",
            "what directory am i in", "print working directory",
            "show current directory", "which directory", "current path",
            "what is the current directory", "show me the current path",
            "what folder am i in", "get current directory", "where are we",
            "current location", "what's the working directory",
            "where am i right now", "what is my current location",
            "what path am i in", "show my location",
            "present working directory",
        ],
        "program": [
            _s(Prim.SYS_CWD),
            _s(Prim.EMIT, "ref:0"),
            _s(Prim.STOP),
        ],
    },
    "system_info": {
        "params": [],
        "templates": [
            "system info", "system information", "show system info",
            "what system is this", "uname", "tell me about this system",
            "what os is this", "operating system info",
            "show os information", "what machine is this",
            "system details", "get system information",
            "what platform am i on", "show machine info", "computer info",
            "give me system information", "describe this computer",
            "what hardware is this", "show computer details", "os details",
        ],
        "program": [
            _s(Prim.SYS_INFO),
            _s(Prim.EMIT, "ref:0"),
            _s(Prim.STOP),
        ],
    },
    "current_time": {
        "params": [],
        "templates": [
            "what time is it", "current time", "tell me the time",
            "show the time", "time now", "what is the time",
            "get current time", "show current date", "what day is it",
            "current date and time", "give me the time", "tell me the date",
            "date", "what's the time", "time please",
            "whats the current time", "what is the current time right now",
            "display the time", "show me the time", "what's the date today",
            "what is today's date", "clock", "right now what time is it",
        ],
        "program": [
            _s(Prim.SYS_TIME),
            _s(Prim.EMIT, "ref:0"),
            _s(Prim.STOP),
        ],
    },
    "disk_usage": {
        "params": [],
        "templates": [
            "disk usage", "disk space", "how much disk space",
            "show disk usage", "df", "storage space",
            "how much space is left", "check disk space",
            "available disk space", "free space", "how much storage",
            "show storage info", "how much free space do i have",
            "storage left", "remaining disk space", "how full is the disk",
            "space available", "check storage", "disk free space",
            "how much room is left",
        ],
        "program": [
            _s(Prim.SYS_DISK),
            _s(Prim.EMIT, "ref:0"),
            _s(Prim.STOP),
        ],
    },
    "process_list": {
        "params": [],
        "templates": [
            "list processes", "show processes", "running processes", "ps",
            "what's running", "show running processes", "process list",
            "active processes", "list running processes",
            "what processes are running", "top processes",
            "show active processes", "what processes are active",
            "current processes", "show me all processes",
            "what is currently running", "display running processes",
            "tasks running", "show all tasks", "what tasks are active",
        ],
        "program": [
            _s(Prim.SYS_PROCS),
            _s(Prim.EMIT, "ref:0"),
            _s(Prim.STOP),
        ],
    },

    # ===== COMPOSITIONAL TASKS (v0.3) =====

    "read_then_delete": {
        "params": [("path", FILENAMES + NOVEL_FILENAMES)],
        "templates": [
            "read {path} and then delete it",
            "read {path} and remove it",
            "show me {path} and then delete it",
            "cat {path} then delete it",
            "read {path} then remove the file",
            "display {path} and delete it afterwards",
            "read file {path} and then get rid of it",
            "open {path} then delete it",
            "show {path} and remove it after",
            "read {path} and erase it",
            "view {path} then throw it away",
            "read the file {path} and then remove it",
        ],
        "program": [
            _s(Prim.FILE_OPEN_R, "span:path"),
            _s(Prim.FILE_READ, "ref:0"),
            _s(Prim.FILE_CLOSE, "ref:0"),
            _s(Prim.FILE_DELETE, "span:path"),
            _s(Prim.EMIT, "ref:1"),
            _s(Prim.STOP),
        ],
    },
    "copy_then_delete": {
        "params": [("source", FILENAMES), ("destination", FILENAMES)],
        "templates": [
            "copy {source} to {destination} and delete the original",
            "copy {source} to {destination} then delete {source}",
            "back up {source} to {destination} and remove {source}",
            "duplicate {source} as {destination} then erase {source}",
            "copy {source} to {destination} and remove the original",
            "cp {source} {destination} then rm {source}",
            "copy file {source} to {destination} and delete {source}",
            "save {source} to {destination} then throw away {source}",
            "copy {source} into {destination} and delete the source",
            "back up {source} as {destination} then delete it",
        ],
        "program": [
            _s(Prim.FILE_COPY, "span:source", "span:destination"),
            _s(Prim.FILE_DELETE, "span:source"),
            _s(Prim.EMIT, "ref:0"),
            _s(Prim.STOP),
        ],
    },
    "create_then_verify": {
        "params": [("path", FILENAMES), ("content", CONTENTS)],
        "templates": [
            "create {path} with content {content} and verify it exists",
            "create file {path} with {content} and check it exists",
            "make {path} with {content} then verify it was created",
            "write {content} to {path} and check it exists",
            "create {path} containing {content} and confirm it exists",
            "make file {path} with {content} and verify",
            "create a file {path} with {content} then check if it exists",
            "write file {path} with content {content} then verify",
            "create {path} with content {content} and see if it exists",
            "save {content} to {path} and check the file exists",
        ],
        "program": [
            _s(Prim.FILE_OPEN_W, "span:path"),
            _s(Prim.FILE_WRITE, "ref:0", "span:content"),
            _s(Prim.FILE_CLOSE, "ref:0"),
            _s(Prim.FILE_EXISTS, "span:path"),
            _s(Prim.EMIT, "ref:3"),
            _s(Prim.STOP),
        ],
    },
    "create_then_read": {
        "params": [("path", FILENAMES), ("content", CONTENTS)],
        "templates": [
            "create {path} with content {content} and read it back",
            "write {content} to {path} then read it",
            "create file {path} with {content} and show its contents",
            "make {path} with {content} and then read it",
            "create {path} containing {content} then display it",
            "write {content} to {path} and read it back",
            "save {content} to {path} then cat it",
            "create {path} with {content} and verify the content",
            "make file {path} with {content} then read it back",
            "create a file {path} with {content} and show what was written",
        ],
        "program": [
            _s(Prim.FILE_OPEN_W, "span:path"),
            _s(Prim.FILE_WRITE, "ref:0", "span:content"),
            _s(Prim.FILE_CLOSE, "ref:0"),
            _s(Prim.FILE_OPEN_R, "span:path"),
            _s(Prim.FILE_READ, "ref:3"),
            _s(Prim.FILE_CLOSE, "ref:3"),
            _s(Prim.EMIT, "ref:4"),
            _s(Prim.STOP),
        ],
    },
    "read_and_save": {
        "params": [("source", FILENAMES + NOVEL_FILENAMES), ("destination", FILENAMES + NOVEL_FILENAMES)],
        "templates": [
            "read {source} and save it to {destination}",
            "read {source} and write it to {destination}",
            "read file {source} and save to {destination}",
            "cat {source} and save to {destination}",
            "read {source} then save the content to {destination}",
            "open {source} and write its content to {destination}",
            "read {source} and store it in {destination}",
            "get contents of {source} and save to {destination}",
            "read {source} and put it in {destination}",
            "read {source} and copy its content to {destination}",
        ],
        "program": [
            _s(Prim.FILE_OPEN_R, "span:source"),
            _s(Prim.FILE_READ, "ref:0"),
            _s(Prim.FILE_CLOSE, "ref:0"),
            _s(Prim.FILE_OPEN_W, "span:destination"),
            _s(Prim.FILE_WRITE, "ref:3", "ref:1"),  # CROSS-OP DATA FLOW: data from READ
            _s(Prim.FILE_CLOSE, "ref:3"),
            _s(Prim.EMIT, "ref:5"),
            _s(Prim.STOP),
        ],
    },
}


# ============================================================================
# Training data generation
# ============================================================================

# Arg type constants (must match mind.py)
_ARG_NONE = 0
_ARG_SPAN = 1
_ARG_REF = 2


def _resolve_program(program_template, param_names, param_values, tokens, tokenizer):
    """Convert program template to training targets with resolved span/ref indices and types."""
    steps = []
    for op, a0_desc, a1_desc in program_template:
        a0s, a0e, a0r, a0t = -1, -1, -1, _ARG_NONE
        a1s, a1e, a1r, a1t = -1, -1, -1, _ARG_NONE

        for desc, setter in [(a0_desc, "a0"), (a1_desc, "a1")]:
            if desc == "none":
                continue
            elif desc.startswith("span:"):
                name = desc[5:]
                idx = param_names.index(name)
                val = param_values[idx]
                val_tokens = tokenizer.tokenize(val)
                span = find_span(tokens, val_tokens)
                if span is None:
                    return None
                s, e = span[0] + 1, span[1] + 1
                if setter == "a0":
                    a0s, a0e, a0t = s, e, _ARG_SPAN
                else:
                    a1s, a1e, a1t = s, e, _ARG_SPAN
            elif desc.startswith("ref:"):
                ref_idx = int(desc[4:])
                if setter == "a0":
                    a0r, a0t = ref_idx, _ARG_REF
                else:
                    a1r, a1t = ref_idx, _ARG_REF

        steps.append({
            "opcode": int(op),
            "a0_type": a0t, "a0_span_s": a0s, "a0_span_e": a0e, "a0_ref": a0r,
            "a1_type": a1t, "a1_span_s": a1s, "a1_span_e": a1e, "a1_ref": a1r,
        })
    return steps


def generate_training_data(seed: int = 42) -> list[dict]:
    """Generate (intent, program) training examples."""
    rng = random.Random(seed)
    tokenizer = Tokenizer()
    examples_raw = []

    for task_name, task in TASK_TYPES.items():
        templates = task["templates"]
        params_spec = task["params"]
        program_template = task["program"]
        num_params = len(params_spec)

        if num_params == 0:
            for template in templates:
                for _ in range(8):  # oversample zero-param tasks
                    examples_raw.append({
                        "text": template,
                        "param_names": [],
                        "param_values": [],
                        "program_template": program_template,
                    })

        elif num_params == 1:
            pname, pool = params_spec[0]
            for template in templates:
                sampled = rng.sample(pool, min(len(pool), 10))
                for val in sampled:
                    text = template.format(**{pname: val})
                    examples_raw.append({
                        "text": text,
                        "param_names": [pname],
                        "param_values": [val],
                        "program_template": program_template,
                    })

        elif num_params == 2:
            pname0, pool0 = params_spec[0]
            pname1, pool1 = params_spec[1]
            for template in templates:
                for _ in range(12):
                    v0 = rng.choice(pool0)
                    v1 = rng.choice(pool1)
                    if v0 != v1:
                        text = template.format(**{pname0: v0, pname1: v1})
                        examples_raw.append({
                            "text": text,
                            "param_names": [pname0, pname1],
                            "param_values": [v0, v1],
                            "program_template": program_template,
                        })

    # Build tokenizer vocab from all texts
    tokenizer.build_vocab([ex["text"] for ex in examples_raw])

    # Resolve programs
    examples = []
    skipped = 0
    for ex in examples_raw:
        tokens = tokenizer.tokenize(ex["text"])
        token_ids = [NULL_IDX] + tokenizer.encode(ex["text"])
        steps = _resolve_program(
            ex["program_template"], ex["param_names"], ex["param_values"],
            tokens, tokenizer,
        )
        if steps is None:
            skipped += 1
            continue

        # Pad to MAX_PROGRAM_STEPS
        while len(steps) < MAX_PROGRAM_STEPS:
            steps.append({
                "opcode": int(Prim.STOP),
                "a0_type": _ARG_NONE, "a0_span_s": -1, "a0_span_e": -1, "a0_ref": -1,
                "a1_type": _ARG_NONE, "a1_span_s": -1, "a1_span_e": -1, "a1_ref": -1,
            })

        examples.append({
            "token_ids": token_ids,
            "length": len(token_ids),
            "steps": steps[:MAX_PROGRAM_STEPS],
        })

    rng.shuffle(examples)
    return examples, tokenizer, skipped


# ============================================================================
# Dataset and DataLoader
# ============================================================================

class SomaDataset(Dataset):
    def __init__(self, examples):
        self.examples = examples
    def __len__(self):
        return len(self.examples)
    def __getitem__(self, idx):
        return self.examples[idx]


def collate_fn(batch):
    max_len = max(b["length"] for b in batch)
    padded = [b["token_ids"] + [0] * (max_len - b["length"]) for b in batch]

    opcodes = []
    a0t, a1t = [], []
    a0ss, a0se, a1ss, a1se, a0r, a1r = [], [], [], [], [], []
    for b in batch:
        opcodes.append([s["opcode"] for s in b["steps"]])
        a0t.append([s["a0_type"] for s in b["steps"]])
        a1t.append([s["a1_type"] for s in b["steps"]])
        a0ss.append([s["a0_span_s"] for s in b["steps"]])
        a0se.append([s["a0_span_e"] for s in b["steps"]])
        a1ss.append([s["a1_span_s"] for s in b["steps"]])
        a1se.append([s["a1_span_e"] for s in b["steps"]])
        a0r.append([s["a0_ref"] for s in b["steps"]])
        a1r.append([s["a1_ref"] for s in b["steps"]])

    return {
        "input_ids": torch.tensor(padded, dtype=torch.long),
        "lengths": torch.tensor([b["length"] for b in batch], dtype=torch.long),
        "target_opcodes": torch.tensor(opcodes, dtype=torch.long),
        "target_a0_type": torch.tensor(a0t, dtype=torch.long),
        "target_a1_type": torch.tensor(a1t, dtype=torch.long),
        "target_a0_span_s": torch.tensor(a0ss, dtype=torch.long),
        "target_a0_span_e": torch.tensor(a0se, dtype=torch.long),
        "target_a1_span_s": torch.tensor(a1ss, dtype=torch.long),
        "target_a1_span_e": torch.tensor(a1se, dtype=torch.long),
        "target_a0_ref": torch.tensor(a0r, dtype=torch.long),
        "target_a1_ref": torch.tensor(a1r, dtype=torch.long),
    }


# ============================================================================
# Training
# ============================================================================

def _masked_ce(logits, targets, ignore_val=-1):
    """CrossEntropy loss only where targets != ignore_val."""
    mask = targets != ignore_val
    if not mask.any():
        return torch.tensor(0.0, device=logits.device)
    return nn.functional.cross_entropy(logits[mask], targets[mask])


def train_soma(
    save_dir: str = "poc/artifacts",
    epochs: int = 200,
    batch_size: int = 32,
    lr: float = 1e-3,
    patience: int = 30,
    seed: int = 42,
):
    torch.manual_seed(seed)
    random.seed(seed)
    device = torch.device("cpu")
    print(f"Device: {device}")

    # Generate data
    print("\n[Synthesis] Generating training data...")
    examples, tokenizer, skipped = generate_training_data(seed=seed)
    print(f"  Examples: {len(examples)} (skipped {skipped})")
    print(f"  Vocabulary: {tokenizer.vocab_size}")

    n = len(examples)
    n_train = int(0.8 * n)
    n_val = int(0.1 * n)
    train_data = examples[:n_train]
    val_data = examples[n_train:n_train + n_val]
    test_data = examples[n_train + n_val:]
    print(f"  Train: {len(train_data)}, Val: {len(val_data)}, Test: {len(test_data)}")

    # Model
    model = SomaMind(vocab_size=tokenizer.vocab_size).to(device)
    total_params = sum(p.numel() for p in model.parameters())
    print(f"\n[Synthesis] Model: {total_params:,} parameters")

    optimizer = torch.optim.AdamW(model.parameters(), lr=lr, weight_decay=1e-2)
    scheduler = torch.optim.lr_scheduler.ReduceLROnPlateau(
        optimizer, mode="min", factor=0.5, patience=10,
    )

    train_loader = DataLoader(SomaDataset(train_data), batch_size=batch_size,
                              shuffle=True, collate_fn=collate_fn)
    val_loader = DataLoader(SomaDataset(val_data), batch_size=batch_size,
                            shuffle=False, collate_fn=collate_fn)

    print(f"\n[Synthesis] Training for up to {epochs} epochs (patience={patience})...")
    best_val_loss = float("inf")
    best_epoch = 0
    no_improve = 0
    best_state = None

    t0 = time.time()
    for epoch in range(1, epochs + 1):
        model.train()
        train_loss = 0.0
        train_total = 0

        for batch in train_loader:
            ids = batch["input_ids"].to(device)
            lens = batch["lengths"].to(device)
            t_ops = batch["target_opcodes"].to(device)

            out = model(ids, lens, t_ops)
            B, S, _ = out["op_logits"].shape

            # Opcode loss
            loss_op = nn.functional.cross_entropy(
                out["op_logits"].reshape(B * S, -1),
                t_ops.reshape(B * S),
            )

            # Arg type losses
            loss_type = nn.functional.cross_entropy(
                out["a0t"].reshape(B * S, -1), batch["target_a0_type"].to(device).reshape(B * S),
            ) + nn.functional.cross_entropy(
                out["a1t"].reshape(B * S, -1), batch["target_a1_type"].to(device).reshape(B * S),
            )

            # Span losses (masked)
            loss_span = torch.tensor(0.0, device=device)
            for key_s, key_e, tgt_s, tgt_e in [
                ("s0s", "s0e", "target_a0_span_s", "target_a0_span_e"),
                ("s1s", "s1e", "target_a1_span_s", "target_a1_span_e"),
            ]:
                ts = batch[tgt_s].to(device)
                te = batch[tgt_e].to(device)
                loss_span = loss_span + _masked_ce(out[key_s].reshape(B * S, -1), ts.reshape(B * S))
                loss_span = loss_span + _masked_ce(out[key_e].reshape(B * S, -1), te.reshape(B * S))

            # Ref losses (masked)
            loss_ref = torch.tensor(0.0, device=device)
            for key_r, tgt_r in [("r0", "target_a0_ref"), ("r1", "target_a1_ref")]:
                tr = batch[tgt_r].to(device)
                loss_ref = loss_ref + _masked_ce(out[key_r].reshape(B * S, -1), tr.reshape(B * S))

            loss = loss_op + loss_type + loss_span + loss_ref

            optimizer.zero_grad()
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
            optimizer.step()

            train_loss += loss.item() * B
            train_total += B

        train_loss /= train_total

        # Validate
        model.eval()
        val_loss = 0.0
        val_prog_correct = 0
        val_total = 0

        with torch.no_grad():
            for batch in val_loader:
                ids = batch["input_ids"].to(device)
                lens = batch["lengths"].to(device)
                t_ops = batch["target_opcodes"].to(device)
                B = ids.size(0)
                S = MAX_PROGRAM_STEPS

                out = model(ids, lens, t_ops)

                loss_op = nn.functional.cross_entropy(
                    out["op_logits"].reshape(B * S, -1), t_ops.reshape(B * S),
                )
                loss_type = nn.functional.cross_entropy(
                    out["a0t"].reshape(B * S, -1), batch["target_a0_type"].to(device).reshape(B * S),
                ) + nn.functional.cross_entropy(
                    out["a1t"].reshape(B * S, -1), batch["target_a1_type"].to(device).reshape(B * S),
                )
                loss_span = torch.tensor(0.0, device=device)
                for key_s, key_e, tgt_s, tgt_e in [
                    ("s0s", "s0e", "target_a0_span_s", "target_a0_span_e"),
                    ("s1s", "s1e", "target_a1_span_s", "target_a1_span_e"),
                ]:
                    ts = batch[tgt_s].to(device)
                    te = batch[tgt_e].to(device)
                    loss_span = loss_span + _masked_ce(out[key_s].reshape(B * S, -1), ts.reshape(B * S))
                    loss_span = loss_span + _masked_ce(out[key_e].reshape(B * S, -1), te.reshape(B * S))

                loss_ref = torch.tensor(0.0, device=device)
                for key_r, tgt_r in [("r0", "target_a0_ref"), ("r1", "target_a1_ref")]:
                    tr = batch[tgt_r].to(device)
                    loss_ref = loss_ref + _masked_ce(out[key_r].reshape(B * S, -1), tr.reshape(B * S))

                loss = loss_op + loss_type + loss_span + loss_ref
                val_loss += loss.item() * B

                # Program exact match (opcode sequence)
                pred_ops = out["op_logits"].argmax(dim=-1)
                val_prog_correct += (pred_ops == t_ops).all(dim=-1).sum().item()
                val_total += B

        val_loss /= val_total
        val_prog_acc = val_prog_correct / val_total
        scheduler.step(val_loss)

        if epoch % 10 == 0 or epoch == 1:
            print(
                f"  Epoch {epoch:3d} | "
                f"Train loss={train_loss:.4f} | "
                f"Val loss={val_loss:.4f} prog_match={val_prog_acc:.3f}"
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

    # Test
    print("\n[Synthesis] Test set results:")
    model.eval()
    test_loader = DataLoader(SomaDataset(test_data), batch_size=batch_size,
                             shuffle=False, collate_fn=collate_fn)
    test_prog_correct = 0
    test_op_correct = 0
    test_total = 0
    test_steps_total = 0

    with torch.no_grad():
        for batch in test_loader:
            ids = batch["input_ids"].to(device)
            lens = batch["lengths"].to(device)
            t_ops = batch["target_opcodes"].to(device)
            B = ids.size(0)

            out = model(ids, lens, t_ops)
            pred_ops = out["op_logits"].argmax(dim=-1)
            test_prog_correct += (pred_ops == t_ops).all(dim=-1).sum().item()
            test_op_correct += (pred_ops == t_ops).sum().item()
            test_total += B
            test_steps_total += B * MAX_PROGRAM_STEPS

    print(f"  Program Exact Match: {test_prog_correct / test_total:.3f}")
    print(f"  Per-Step Op Accuracy: {test_op_correct / test_steps_total:.3f}")

    # Save
    os.makedirs(save_dir, exist_ok=True)
    model_path = os.path.join(save_dir, "soma_mind.pt")
    vocab_path = os.path.join(save_dir, "vocab.json")
    torch.save(best_state, model_path)
    tokenizer.save(vocab_path)
    print(f"\n[Synthesis] Saved model -> {model_path}")
    print(f"[Synthesis] Saved vocab -> {vocab_path}")
    print("[Synthesis] SOMA synthesis complete.")


if __name__ == "__main__":
    train_soma()
