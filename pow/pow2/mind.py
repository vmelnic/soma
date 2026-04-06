"""
SOMA Mind — POW 1: Drives libc directly.

Same seq2seq as v0.3 but outputs CATALOG function IDs from the
discovered body. No hand-written opcodes. The model learns which
libc functions to call during synthesis from the catalog.
"""

import torch
import torch.nn as nn
import torch.nn.functional as F

from pow.pow2.discovery import EMIT_ID, STOP_ID
from pow.pow2.bridge import ProgramStep

ARG_NONE = 0
ARG_SPAN = 1
ARG_REF = 2


class SomaMind(nn.Module):

    def __init__(self, vocab_size, num_conventions, max_steps=8,
                 embed_dim=64, hidden_dim=128, decoder_dim=256,
                 opcode_embed_dim=32, num_layers=2, dropout=0.3):
        super().__init__()
        self.max_steps = max_steps
        self.num_output_ids = num_conventions + 2  # +EMIT +STOP
        self.emit_out = num_conventions
        self.stop_out = num_conventions + 1
        self.start_tok = num_conventions + 2
        enc_dim = hidden_dim * 2

        self.embedding = nn.Embedding(vocab_size, embed_dim, padding_idx=0)
        self.encoder = nn.LSTM(embed_dim, hidden_dim, num_layers,
                               bidirectional=True, batch_first=True,
                               dropout=dropout if num_layers > 1 else 0.0)

        self.op_emb = nn.Embedding(num_conventions + 3, opcode_embed_dim)
        self.init_h = nn.Linear(enc_dim, decoder_dim)
        self.ctx_q = nn.Linear(decoder_dim, enc_dim)
        self.gru = nn.GRUCell(opcode_embed_dim + enc_dim, decoder_dim)

        self.op_head = nn.Linear(decoder_dim, self.num_output_ids)
        self.a0t_head = nn.Linear(decoder_dim, 3)
        self.a1t_head = nn.Linear(decoder_dim, 3)

        self.s0s_q = nn.Linear(decoder_dim, enc_dim)
        self.s0e_q = nn.Linear(decoder_dim, enc_dim)
        self.s1s_q = nn.Linear(decoder_dim, enc_dim)
        self.s1e_q = nn.Linear(decoder_dim, enc_dim)

        rd = 64
        self.r0q = nn.Linear(decoder_dim, rd)
        self.r0k = nn.Linear(decoder_dim, rd)
        self.r1q = nn.Linear(decoder_dim, rd)
        self.r1k = nn.Linear(decoder_dim, rd)

    def encode(self, ids, lens):
        B, L = ids.shape
        emb = self.embedding(ids)
        pk = nn.utils.rnn.pack_padded_sequence(emb, lens.cpu(), batch_first=True, enforce_sorted=False)
        out, _ = self.encoder(pk)
        out, _ = nn.utils.rnn.pad_packed_sequence(out, batch_first=True, total_length=L)
        pos = torch.arange(L, device=ids.device).unsqueeze(0)
        mask = pos < lens.unsqueeze(1)
        mf = mask.unsqueeze(-1).float()
        pooled = (out * mf).sum(1) / lens.unsqueeze(1).float()
        return out, mask, pooled

    def _ctx(self, h, eo, em):
        q = self.ctx_q(h)
        s = torch.bmm(q.unsqueeze(1), eo.transpose(1, 2)).squeeze(1).masked_fill(~em, -1e9)
        return torch.bmm(F.softmax(s, -1).unsqueeze(1), eo).squeeze(1)

    def _span(self, h, eo, em, ql):
        return torch.bmm(ql(h).unsqueeze(1), eo.transpose(1, 2)).squeeze(1).masked_fill(~em, -1e9)

    def _ref(self, h, ph, ql, kl):
        B, d, ms = h.size(0), h.device, self.max_steps
        t = len(ph)
        if t == 0:
            return torch.full((B, ms), -1e9, device=d)
        keys = torch.stack(ph, 1)
        sc = torch.bmm(ql(h).unsqueeze(1), kl(keys).transpose(1, 2)).squeeze(1)
        if t < ms:
            sc = torch.cat([sc, torch.full((B, ms - t), -1e9, device=d)], 1)
        return sc

    def forward(self, ids, lens, tgt_ops):
        B = ids.size(0)
        eo, em, pooled = self.encode(ids, lens)
        h = torch.tanh(self.init_h(pooled))
        dev = ids.device
        ph = []
        outs = {k: [] for k in ["op", "a0t", "a1t", "s0s", "s0e", "s1s", "s1e", "r0", "r1"]}

        for t in range(self.max_steps):
            pid = torch.full((B,), self.start_tok, dtype=torch.long, device=dev) if t == 0 else tgt_ops[:, t-1]
            h = self.gru(torch.cat([self.op_emb(pid), self._ctx(h, eo, em)], -1), h)
            ph.append(h)
            outs["op"].append(self.op_head(h))
            outs["a0t"].append(self.a0t_head(h))
            outs["a1t"].append(self.a1t_head(h))
            outs["s0s"].append(self._span(h, eo, em, self.s0s_q))
            outs["s0e"].append(self._span(h, eo, em, self.s0e_q))
            outs["s1s"].append(self._span(h, eo, em, self.s1s_q))
            outs["s1e"].append(self._span(h, eo, em, self.s1e_q))
            outs["r0"].append(self._ref(h, ph[:-1], self.r0q, self.r0k))
            outs["r1"].append(self._ref(h, ph[:-1], self.r1q, self.r1k))

        return {k: torch.stack(v, 1) for k, v in outs.items()}

    @torch.no_grad()
    def predict(self, ids, lens, tokens, catalog):
        self.eval()
        dev = ids.device
        eo, em, pooled = self.encode(ids, lens)
        h = torch.tanh(self.init_h(pooled))
        pid = torch.tensor([self.start_tok], dtype=torch.long, device=dev)
        ph, steps = [], []
        conf = 0.0

        for t in range(self.max_steps):
            h = self.gru(torch.cat([self.op_emb(pid), self._ctx(h, eo, em)], -1), h)
            ph.append(h)
            logits = self.op_head(h)
            probs = F.softmax(logits, -1)
            pred = logits.argmax(-1).item()
            if t == 0:
                conf = probs.max().item()

            if pred == self.stop_out:
                steps.append(ProgramStep(STOP_ID, [], []))
                break
            elif pred == self.emit_out:
                r = self._ref(h, ph[:-1], self.r0q, self.r0k)
                steps.append(ProgramStep(EMIT_ID, ["ref"], [r.argmax(-1).item()]))
            else:
                a0t = self.a0t_head(h).argmax(-1).item()
                a1t = self.a1t_head(h).argmax(-1).item()
                a0ty, a0v = self._resolve(a0t, h, eo, em, ph[:-1], self.s0s_q, self.s0e_q, self.r0q, self.r0k, tokens)
                a1ty, a1v = self._resolve(a1t, h, eo, em, ph[:-1], self.s1s_q, self.s1e_q, self.r1q, self.r1k, tokens)
                types = [a0ty] + ([a1ty] if a1ty != "none" else [])
                vals = [a0v] + ([a1v] if a1ty != "none" else [])
                steps.append(ProgramStep(pred, types, vals))

            pid = torch.tensor([pred], dtype=torch.long, device=dev)

        return steps, conf

    def _resolve(self, tid, h, eo, em, ph, sq, eq, rq, rk, tokens):
        if tid == ARG_NONE:
            return "none", None
        if tid == ARG_SPAN:
            s = self._span(h, eo, em, sq).argmax(-1).item()
            e = self._span(h, eo, em, eq).argmax(-1).item()
            e = max(e, s)
            if s == 0 and e == 0:
                return "span", ""
            return "span", " ".join(tokens[s-1:e])
        if tid == ARG_REF:
            return "ref", self._ref(h, ph, rq, rk).argmax(-1).item()
        return "none", None
