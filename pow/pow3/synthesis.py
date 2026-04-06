"""
SOMA Synthesizer — POW 1.

Generates training data mapping intents to libc call sequences.
The model learns which libc functions to call from the discovered catalog.

Usage:
    python -m pow.pow3.synthesis
"""

import json
import os
import random
import sys
import time

import torch
import torch.nn as nn
from torch.utils.data import DataLoader, Dataset

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(__file__))))

from pow.pow3.tokenizer import Tokenizer, find_span, NULL_IDX
from pow.pow3.discovery import discover_body, EMIT_ID, STOP_ID
from pow.pow3.mind import SomaMind, ARG_NONE, ARG_SPAN, ARG_REF

PATHS = ["/tmp", ".", "..", "~/Desktop", "~/Documents", "~/Downloads",
         "/var/log", "/etc", "/Users/vm/Projects", "/opt", "~/Pictures",
         "/home", "~", "/Users/vm"]
FILENAMES = ["hello.txt", "test.txt", "readme.md", "data.csv", "notes.txt",
             "output.log", "config.json", "temp.txt", "report.txt", "todo.txt",
             "index.html", "main.py", "app.js", "style.css", "info.txt"]
NOVEL = ["alpha.txt", "bravo.md", "charlie.log", "delta.json", "echo.py",
         "foxtrot.csv", "golf.html", "hotel.js", "india.xml", "juliet.txt",
         "kilo.md", "lima.log", "mike.json", "november.py", "oscar.csv",
         "papa.txt", "quebec.md", "romeo.log", "sierra.json", "tango.py",
         "uniform.txt", "victor.md", "whiskey.log", "xray.json", "yankee.py",
         "zulu.txt", "archive.tar", "backup.zip", "draft.doc", "spec.pdf"]
CONTENTS = ["hello", "hello world", "test content", "foo bar", "some data",
            "important notes", "first line", "sample text", "test", "ok",
            "this is a test", "readme", "data here", "my content", "done"]
ALL_FILES = FILENAMES + NOVEL
ALL_PATHS = PATHS + ALL_FILES


def _s(conv_id, a0="none", a1="none"):
    return (conv_id, a0, a1)


def build_task_types(catalog):
    by_name = {c.name: c.id for c in catalog}
    OR, CR, RD, WR, CL = by_name["open_read"], by_name["create_file"], by_name["read_content"], by_name["write_content"], by_name["close_fd"]
    OD, RDE, CD = by_name["open_dir"], by_name["read_dir_entries"], by_name["close_dir"]
    DEL, MK, REN, ACC, ST = by_name["delete_file"], by_name["create_dir"], by_name["rename_path"], by_name["check_access"], by_name["file_stat"]
    CWD, TM, UN = by_name["get_cwd"], by_name["get_time"], by_name["get_uname"]

    SEND = by_name["send_signal"]
    PEER_NAMES = ["soma-b", "soma-a", "sensor", "gateway", "server", "monitor"]

    return {
        "read_file": {"params": [("path", ALL_FILES)], "templates": [
            "read {path}", "cat {path}", "show me {path}", "display {path}",
            "read the file {path}", "show contents of {path}", "read file {path}",
            "print {path}", "view {path}", "get contents of {path}",
            "display file {path}", "read the contents of {path}",
            "can you read {path}", "let me see file {path}",
            "show the content of file {path}", "what does {path} contain",
            "dump {path}", "type {path}", "output the contents of {path}",
            "show {path} contents", "please show me file {path}",
            "what's in file {path}", "read out {path}", "open {path}",
        ], "program": [_s(OR,"span:path"),_s(RD,"ref:0"),_s(CL,"ref:0"),_s("EMIT","ref:1"),_s("STOP")]},

        "create_file_content": {"params": [("path", FILENAMES), ("content", CONTENTS)], "templates": [
            "create a file called {path} with content {content}",
            "make a file {path} containing {content}",
            "create {path} with content {content}",
            "make a new file {path} with {content}",
            "create file {path} content {content}",
            "save {content} to file {path}",
            "new file {path} with text {content}",
            "write a file named {path} with the content {content}",
            "please create {path} containing {content}",
            "create a new file {path} and write {content}",
            "store {content} in file {path}",
            "write file {path} with content {content}",
            "create a text file {path} with {content}",
            "write the text {content} to a file called {path}",
        ], "program": [_s(CR,"span:path"),_s(WR,"ref:0","span:content"),_s(CL,"ref:0"),_s("EMIT","ref:2"),_s("STOP")]},

        "create_file_empty": {"params": [("path", ALL_FILES)], "templates": [
            "create file {path}", "create {path}", "make file {path}",
            "touch {path}", "create a file {path}", "create a file called {path}",
            "make a file called {path}", "create an empty file {path}",
            "new file {path}", "make a new file {path}", "please create file {path}",
            "generate file {path}", "create a new file called {path}",
            "make a new file called {path}", "add a file {path}",
        ], "program": [_s(CR,"span:path"),_s(CL,"ref:0"),_s("EMIT","ref:1"),_s("STOP")]},

        "delete_file": {"params": [("path", ALL_FILES)], "templates": [
            "delete {path}", "remove {path}", "rm {path}",
            "delete the file {path}", "remove file {path}",
            "erase {path}", "get rid of {path}", "trash {path}",
            "delete file {path}", "please delete {path}",
            "can you remove {path}", "destroy {path}",
            "throw away {path}", "discard {path}",
            "unlink {path}", "wipe {path}", "remove the file {path}",
        ], "program": [_s(DEL,"span:path"),_s("EMIT","ref:0"),_s("STOP")]},

        "list_dir": {"params": [("path", ALL_PATHS)], "templates": [
            "list files in {path}", "show files in {path}",
            "show me the files in {path}", "what files are in {path}",
            "ls {path}", "what's in {path}", "list the contents of {path}",
            "show me what's in {path}", "get files from {path}",
            "list directory {path}", "dir {path}", "files in {path}",
            "please show files in {path}", "display files in {path}",
            "show me everything in {path}", "check out files in {path}",
            "browse {path}", "explore {path}", "look at files in {path}",
            "see what is in {path}", "what do we have in {path}",
            "show directory listing for {path}", "can you list files in {path}",
            "enumerate all files in {path}", "display contents of {path}",
            "what exists in {path}", "get directory contents of {path}",
        ], "program": [_s(OD,"span:path"),_s(RDE,"ref:0"),_s(CD,"ref:0"),_s("EMIT","ref:1"),_s("STOP")]},

        "file_info": {"params": [("path", ALL_PATHS)], "templates": [
            "info about {path}", "file info {path}", "details of {path}",
            "stat {path}", "get info on {path}", "show details of {path}",
            "file details {path}", "metadata for {path}",
            "what is the size of {path}", "when was {path} modified",
            "show file information for {path}", "tell me about file {path}",
            "properties of {path}", "describe file {path}",
        ], "program": [_s(ST,"span:path"),_s("EMIT","ref:0"),_s("STOP")]},

        "make_dir": {"params": [("path", FILENAMES+["projects","output","backup","logs","temp"])], "templates": [
            "create directory {path}", "make directory {path}", "mkdir {path}",
            "create a folder {path}", "make a folder called {path}",
            "new directory {path}", "create folder {path}", "make folder {path}",
            "please create directory {path}", "add a directory {path}",
            "create a new folder {path}", "make a new directory called {path}",
            "new folder {path}", "set up directory {path}",
        ], "program": [_s(MK,"span:path"),_s("EMIT","ref:0"),_s("STOP")]},

        "file_exists": {"params": [("path", ALL_PATHS)], "templates": [
            "does {path} exist", "check if {path} exists",
            "is there a file {path}", "file exists {path}",
            "does file {path} exist", "check {path} exists",
            "is {path} there", "see if {path} exists",
            "verify {path} exists", "check for {path}",
            "tell me if {path} exists", "does the file {path} exist",
            "is {path} present", "test if {path} exists",
        ], "program": [_s(ACC,"span:path"),_s("EMIT","ref:0"),_s("STOP")]},

        "rename_file": {"params": [("source", FILENAMES), ("destination", FILENAMES)], "templates": [
            "move {source} to {destination}", "rename {source} to {destination}",
            "mv {source} {destination}", "move file {source} to {destination}",
            "rename file {source} to {destination}",
            "relocate {source} to {destination}",
            "please move {source} to {destination}",
            "transfer {source} to {destination}",
            "move the file {source} to {destination}",
            "rename the file {source} as {destination}",
        ], "program": [_s(REN,"span:source","span:destination"),_s("EMIT","ref:0"),_s("STOP")]},

        "current_dir": {"params": [], "templates": [
            "where am i", "current directory", "pwd",
            "what directory am i in", "print working directory",
            "show current directory", "current path",
            "what is the current directory", "show me the current path",
            "what folder am i in", "get current directory", "where are we",
            "current location", "what's the working directory",
            "where am i right now", "what is my current location",
            "show my location", "present working directory",
        ], "program": [_s(CWD),_s("EMIT","ref:0"),_s("STOP")]},

        "system_info": {"params": [], "templates": [
            "system info", "system information", "show system info",
            "what system is this", "uname", "tell me about this system",
            "what os is this", "operating system info",
            "what machine is this", "system details", "get system information",
            "what platform am i on", "show machine info", "computer info",
            "give me system information", "describe this computer",
            "show computer details", "os details",
        ], "program": [_s(UN),_s("EMIT","ref:0"),_s("STOP")]},

        "current_time": {"params": [], "templates": [
            "what time is it", "current time", "tell me the time",
            "show the time", "time now", "what is the time",
            "get current time", "show current date", "what day is it",
            "current date and time", "give me the time", "tell me the date",
            "date", "what's the time", "time please",
            "whats the current time", "display the time", "show me the time",
            "what's the date today", "clock",
        ], "program": [_s(TM),_s("EMIT","ref:0"),_s("STOP")]},

        # COMPOSITIONAL
        "read_then_delete": {"params": [("path", ALL_FILES)], "templates": [
            "read {path} and then delete it", "read {path} and remove it",
            "show me {path} and then delete it", "cat {path} then delete it",
            "read {path} then remove the file", "read file {path} and then get rid of it",
            "open {path} then delete it", "read {path} and erase it",
            "view {path} then throw it away", "display {path} and delete it afterwards",
        ], "program": [_s(OR,"span:path"),_s(RD,"ref:0"),_s(CL,"ref:0"),_s(DEL,"span:path"),_s("EMIT","ref:1"),_s("STOP")]},

        "create_then_read": {"params": [("path", FILENAMES), ("content", CONTENTS)], "templates": [
            "create {path} with content {content} and read it back",
            "write {content} to {path} then read it",
            "create file {path} with {content} and show its contents",
            "make {path} with {content} and then read it",
            "create {path} containing {content} then display it",
            "save {content} to {path} then cat it",
            "create {path} with {content} and verify the content",
            "make file {path} with {content} then read it back",
        ], "program": [_s(CR,"span:path"),_s(WR,"ref:0","span:content"),_s(CL,"ref:0"),_s(OR,"span:path"),_s(RD,"ref:3"),_s(CL,"ref:3"),_s("EMIT","ref:4"),_s("STOP")]},

        "read_and_save": {"params": [("source", ALL_FILES), ("destination", ALL_FILES)], "templates": [
            "read {source} and save it to {destination}",
            "read {source} and write it to {destination}",
            "read file {source} and save to {destination}",
            "cat {source} and save to {destination}",
            "read {source} then save the content to {destination}",
            "open {source} and write its content to {destination}",
            "read {source} and store it in {destination}",
            "get contents of {source} and save to {destination}",
        ], "program": [_s(OR,"span:source"),_s(RD,"ref:0"),_s(CL,"ref:0"),_s(CR,"span:destination"),_s(WR,"ref:3","ref:1"),_s(CL,"ref:3"),_s("EMIT","ref:5"),_s("STOP")]},

        # ===== SYNAPTIC PROTOCOL TASKS (POW 3) =====
        # These use SEND instead of EMIT — data goes to a peer SOMA

        "list_and_send": {"params": [("path", ALL_PATHS), ("peer", PEER_NAMES)], "templates": [
            "list files in {path} and send to {peer}",
            "list files in {path} and send the result to {peer}",
            "show files in {path} and forward to {peer}",
            "ls {path} and send to {peer}",
            "list directory {path} and send result to {peer}",
            "get files from {path} and send to {peer}",
            "list files in {path} and transmit to {peer}",
            "show me what's in {path} and send to {peer}",
            "list files in {path} and relay to {peer}",
            "enumerate files in {path} and send to {peer}",
        ], "program": [_s(OD,"span:path"),_s(RDE,"ref:0"),_s(CD,"ref:0"),_s(SEND,"span:peer","ref:1"),_s("STOP")]},

        "read_and_send": {"params": [("path", ALL_FILES), ("peer", PEER_NAMES)], "templates": [
            "read {path} and send to {peer}",
            "read {path} and send the content to {peer}",
            "cat {path} and forward to {peer}",
            "read file {path} and send to {peer}",
            "read {path} and transmit to {peer}",
            "get contents of {path} and send to {peer}",
            "read {path} and relay to {peer}",
            "show {path} and send to {peer}",
            "read {path} and forward it to {peer}",
            "read {path} and pass to {peer}",
        ], "program": [_s(OR,"span:path"),_s(RD,"ref:0"),_s(CL,"ref:0"),_s(SEND,"span:peer","ref:1"),_s("STOP")]},

        "time_and_send": {"params": [("peer", PEER_NAMES)], "templates": [
            "get the time and send to {peer}",
            "what time is it and send to {peer}",
            "check the time and forward to {peer}",
            "get current time and send to {peer}",
            "time and send to {peer}",
            "send the current time to {peer}",
            "tell {peer} what time it is",
            "send time to {peer}",
        ], "program": [_s(TM),_s(SEND,"span:peer","ref:0"),_s("STOP")]},

        "sysinfo_and_send": {"params": [("peer", PEER_NAMES)], "templates": [
            "get system info and send to {peer}",
            "send system information to {peer}",
            "uname and send to {peer}",
            "tell {peer} about this system",
            "send system details to {peer}",
            "forward system info to {peer}",
            "send machine info to {peer}",
            "system info and send to {peer}",
        ], "program": [_s(UN),_s(SEND,"span:peer","ref:0"),_s("STOP")]},

        "delegate_intent": {"params": [("peer", PEER_NAMES)], "templates": [
            "ask {peer} to list files in /tmp",
            "tell {peer} to check the time",
            "delegate to {peer} to get system info",
            "ask {peer} for system information",
            "have {peer} list files in /tmp",
            "request {peer} to show the time",
            "send an intent to {peer} to list files",
            "delegate system info to {peer}",
        ], "program": [_s(SEND,"span:peer","none"),_s("EMIT","ref:0"),_s("STOP")]},
    }


def _resolve(program_template, pnames, pvals, tokens, tok, emit_id, stop_id):
    steps = []
    for cid, a0d, a1d in program_template:
        oid = emit_id if cid == "EMIT" else (stop_id if cid == "STOP" else cid)
        a0t, a0s, a0e, a0r = ARG_NONE, -1, -1, -1
        a1t, a1s, a1e, a1r = ARG_NONE, -1, -1, -1
        for desc, is_a0 in [(a0d, True), (a1d, False)]:
            if desc == "none": continue
            if desc.startswith("span:"):
                idx = pnames.index(desc[5:])
                sp = find_span(tokens, tok.tokenize(pvals[idx]))
                if sp is None: return None
                s, e = sp[0]+1, sp[1]+1
                if is_a0: a0t, a0s, a0e = ARG_SPAN, s, e
                else: a1t, a1s, a1e = ARG_SPAN, s, e
            elif desc.startswith("ref:"):
                ri = int(desc[4:])
                if is_a0: a0t, a0r = ARG_REF, ri
                else: a1t, a1r = ARG_REF, ri
        steps.append({"opcode": oid, "a0_type": a0t, "a0_span_s": a0s, "a0_span_e": a0e, "a0_ref": a0r,
                       "a1_type": a1t, "a1_span_s": a1s, "a1_span_e": a1e, "a1_ref": a1r})
    return steps


def generate_training_data(catalog, seed=42):
    rng = random.Random(seed)
    tok = Tokenizer()
    nc = len(catalog)
    emit_id, stop_id, ms = nc, nc+1, 8
    tasks = build_task_types(catalog)
    raw = []
    for tname, t in tasks.items():
        tmpls, ps, prog = t["templates"], t["params"], t["program"]
        np_ = len(ps)
        if np_ == 0:
            for tmpl in tmpls:
                for _ in range(8):
                    raw.append({"text": tmpl, "pn": [], "pv": [], "prog": prog})
        elif np_ == 1:
            pn, pool = ps[0]
            for tmpl in tmpls:
                for v in rng.sample(pool, min(len(pool), 10)):
                    raw.append({"text": tmpl.format(**{pn: v}), "pn": [pn], "pv": [v], "prog": prog})
        elif np_ == 2:
            p0n, p0 = ps[0]; p1n, p1 = ps[1]
            for tmpl in tmpls:
                for _ in range(12):
                    v0, v1 = rng.choice(p0), rng.choice(p1)
                    if v0 != v1:
                        raw.append({"text": tmpl.format(**{p0n: v0, p1n: v1}), "pn": [p0n, p1n], "pv": [v0, v1], "prog": prog})

    tok.build_vocab([e["text"] for e in raw])
    examples, skip = [], 0
    for e in raw:
        tokens = tok.tokenize(e["text"])
        ids = [NULL_IDX] + tok.encode(e["text"])
        steps = _resolve(e["prog"], e["pn"], e["pv"], tokens, tok, emit_id, stop_id)
        if steps is None: skip += 1; continue
        while len(steps) < ms:
            steps.append({"opcode": stop_id, "a0_type": ARG_NONE, "a0_span_s": -1, "a0_span_e": -1, "a0_ref": -1,
                           "a1_type": ARG_NONE, "a1_span_s": -1, "a1_span_e": -1, "a1_ref": -1})
        examples.append({"token_ids": ids, "length": len(ids), "steps": steps[:ms]})
    rng.shuffle(examples)
    return examples, tok, skip


class DS(Dataset):
    def __init__(self, x): self.x = x
    def __len__(self): return len(self.x)
    def __getitem__(self, i): return self.x[i]

def collate(batch):
    ml = max(b["length"] for b in batch)
    d = {"input_ids": torch.tensor([b["token_ids"]+[0]*(ml-b["length"]) for b in batch], dtype=torch.long),
         "lengths": torch.tensor([b["length"] for b in batch], dtype=torch.long)}
    for k in ["opcode","a0_type","a1_type","a0_span_s","a0_span_e","a1_span_s","a1_span_e","a0_ref","a1_ref"]:
        d[k] = torch.tensor([[s[k] for s in b["steps"]] for b in batch], dtype=torch.long)
    return d

def _mce(logits, targets, ign=-1):
    m = targets != ign
    return nn.functional.cross_entropy(logits[m], targets[m]) if m.any() else torch.tensor(0.0, device=logits.device)


def train_soma(save_dir="pow/pow3/artifacts", epochs=200, batch_size=32,
               lr=1e-3, patience=30, seed=42):
    torch.manual_seed(seed); random.seed(seed)
    dev = torch.device("cpu")

    print("[Discovery] Scanning target body...")
    catalog, libc = discover_body()
    print(f"  Found {len(catalog)} calling conventions")

    print("\n[Synthesis] Generating training data...")
    examples, tok, skip = generate_training_data(catalog, seed)
    print(f"  Examples: {len(examples)} (skipped {skip}), Vocab: {tok.vocab_size}")

    n = len(examples); nt = int(0.8*n); nv = int(0.1*n)
    tr, va, tst = examples[:nt], examples[nt:nt+nv], examples[nt+nv:]
    print(f"  Train: {len(tr)}, Val: {len(va)}, Test: {len(tst)}")

    model = SomaMind(tok.vocab_size, len(catalog)).to(dev)
    print(f"\n[Synthesis] Model: {sum(p.numel() for p in model.parameters()):,} params")

    opt = torch.optim.AdamW(model.parameters(), lr=lr, weight_decay=1e-2)
    sch = torch.optim.lr_scheduler.ReduceLROnPlateau(opt, factor=0.5, patience=10)
    tl = DataLoader(DS(tr), batch_size, shuffle=True, collate_fn=collate)
    vl = DataLoader(DS(va), batch_size, shuffle=False, collate_fn=collate)

    print(f"\n[Synthesis] Training up to {epochs} epochs (patience={patience})...")
    bl, be, ni, bs = float("inf"), 0, 0, None
    t0 = time.time()

    for ep in range(1, epochs+1):
        model.train(); tls, tN = 0.0, 0
        for b in tl:
            ids = b["input_ids"].to(dev); lens = b["lengths"].to(dev); tgt = b["opcode"].to(dev)
            B, S = tgt.shape; out = model(ids, lens, tgt)
            loss = nn.functional.cross_entropy(out["op"].reshape(B*S,-1), tgt.reshape(B*S))
            loss = loss + nn.functional.cross_entropy(out["a0t"].reshape(B*S,-1), b["a0_type"].to(dev).reshape(B*S))
            loss = loss + nn.functional.cross_entropy(out["a1t"].reshape(B*S,-1), b["a1_type"].to(dev).reshape(B*S))
            for ks,ke,ts,te in [("s0s","s0e","a0_span_s","a0_span_e"),("s1s","s1e","a1_span_s","a1_span_e")]:
                loss = loss + _mce(out[ks].reshape(B*S,-1), b[ts].to(dev).reshape(B*S))
                loss = loss + _mce(out[ke].reshape(B*S,-1), b[te].to(dev).reshape(B*S))
            for kr,tr_ in [("r0","a0_ref"),("r1","a1_ref")]:
                loss = loss + _mce(out[kr].reshape(B*S,-1), b[tr_].to(dev).reshape(B*S))
            opt.zero_grad(); loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0); opt.step()
            tls += loss.item()*B; tN += B
        tls /= tN

        model.eval(); vls, vok, vN = 0.0, 0, 0
        with torch.no_grad():
            for b in vl:
                ids = b["input_ids"].to(dev); lens = b["lengths"].to(dev); tgt = b["opcode"].to(dev)
                B, S = tgt.shape; out = model(ids, lens, tgt)
                loss = nn.functional.cross_entropy(out["op"].reshape(B*S,-1), tgt.reshape(B*S))
                loss = loss + nn.functional.cross_entropy(out["a0t"].reshape(B*S,-1), b["a0_type"].to(dev).reshape(B*S))
                loss = loss + nn.functional.cross_entropy(out["a1t"].reshape(B*S,-1), b["a1_type"].to(dev).reshape(B*S))
                for ks,ke,ts,te in [("s0s","s0e","a0_span_s","a0_span_e"),("s1s","s1e","a1_span_s","a1_span_e")]:
                    loss = loss + _mce(out[ks].reshape(B*S,-1), b[ts].to(dev).reshape(B*S))
                    loss = loss + _mce(out[ke].reshape(B*S,-1), b[te].to(dev).reshape(B*S))
                for kr,tr_ in [("r0","a0_ref"),("r1","a1_ref")]:
                    loss = loss + _mce(out[kr].reshape(B*S,-1), b[tr_].to(dev).reshape(B*S))
                vls += loss.item()*B; pred = out["op"].argmax(-1)
                vok += (pred == tgt).all(-1).sum().item(); vN += B
        vls /= vN; sch.step(vls)
        if ep % 10 == 0 or ep == 1:
            print(f"  Epoch {ep:3d} | Train={tls:.4f} | Val={vls:.4f} prog={vok/vN:.3f}")
        if vls < bl: bl, be, bs, ni = vls, ep, {k: v.cpu().clone() for k,v in model.state_dict().items()}, 0
        else:
            ni += 1
            if ni >= patience: print(f"  Early stop at {ep} (best={be})"); break

    print(f"\n[Synthesis] Done in {time.time()-t0:.1f}s (best epoch: {be})")
    model.load_state_dict(bs)

    model.eval(); test_l = DataLoader(DS(tst), batch_size, shuffle=False, collate_fn=collate)
    tok_, tN_ = 0, 0
    with torch.no_grad():
        for b in test_l:
            pred = model(b["input_ids"].to(dev), b["lengths"].to(dev), b["opcode"].to(dev))["op"].argmax(-1)
            tok_ += (pred == b["opcode"].to(dev)).all(-1).sum().item(); tN_ += b["input_ids"].size(0)
    print(f"\n[Test] Program exact match: {tok_/tN_:.3f}")

    os.makedirs(save_dir, exist_ok=True)
    torch.save(bs, os.path.join(save_dir, "soma_mind.pt"))
    tok.save(os.path.join(save_dir, "vocab.json"))
    with open(os.path.join(save_dir, "meta.json"), "w") as f:
        json.dump({"num_conventions": len(catalog), "vocab_size": tok.vocab_size}, f)
    print(f"[Synthesis] Saved to {save_dir}/")
    print("[Synthesis] SOMA drives libc directly. No dispatch table.")


if __name__ == "__main__":
    train_soma()
