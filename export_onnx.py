"""
Export SOMA Mind to ONNX for Rust runtime.

Exports:
  models/encoder.onnx     -- intent encoding
  models/decoder.onnx     -- single decoder step (autoregressive)
  models/tokenizer.json   -- vocabulary
  models/meta.json        -- model metadata + catalog

Usage:
    python export_onnx.py
"""

import json
import os
import shutil

import torch
import torch.nn as nn
import torch.nn.functional as F


class EncoderForExport(nn.Module):
    """ONNX-compatible encoder. No pack_padded_sequence."""

    def __init__(self, mind):
        super().__init__()
        self.embedding = mind.embedding
        self.encoder = mind.encoder
        self.init_h = mind.init_h

    def forward(self, input_ids, mask_float):
        emb = self.embedding(input_ids)
        encoder_out, _ = self.encoder(emb)
        mask_3d = mask_float.unsqueeze(-1)
        length = mask_float.sum(dim=1, keepdim=True).clamp(min=1)
        pooled = (encoder_out * mask_3d).sum(dim=1) / length
        init_hidden = torch.tanh(self.init_h(pooled))
        return encoder_out, pooled, init_hidden


class DecoderStepForExport(nn.Module):
    """ONNX-compatible single decoder step."""

    def __init__(self, mind):
        super().__init__()
        self.op_emb = mind.op_emb
        self.ctx_q = mind.ctx_q
        self.gru = mind.gru
        self.op_head = mind.op_head
        self.a0t_head = mind.a0t_head
        self.a1t_head = mind.a1t_head
        self.s0s_q = mind.s0s_q
        self.s0e_q = mind.s0e_q
        self.s1s_q = mind.s1s_q
        self.s1e_q = mind.s1e_q
        self.r0q = mind.r0q
        self.r0k = mind.r0k
        self.r1q = mind.r1q
        self.r1k = mind.r1k

    def forward(self, prev_op_id, hidden, encoder_out, enc_mask_float,
                prev_hiddens_flat, num_prev_scalar):
        # Context attention
        prev_emb = self.op_emb(prev_op_id)
        ctx_query = self.ctx_q(hidden)
        scores = torch.bmm(ctx_query.unsqueeze(1),
                           encoder_out.transpose(1, 2)).squeeze(1)
        enc_mask_bool = enc_mask_float > 0.5
        scores = scores.masked_fill(~enc_mask_bool, -1e9)
        weights = F.softmax(scores, dim=-1)
        context = torch.bmm(weights.unsqueeze(1), encoder_out).squeeze(1)

        # GRU step
        gru_input = torch.cat([prev_emb, context], dim=-1)
        new_hidden = self.gru(gru_input, hidden)

        # Opcode + type logits
        op_logits = self.op_head(new_hidden)
        a0t_logits = self.a0t_head(new_hidden)
        a1t_logits = self.a1t_head(new_hidden)

        # Span logits (4 heads)
        def _span(ql):
            q = ql(new_hidden)
            lg = torch.bmm(q.unsqueeze(1), encoder_out.transpose(1, 2)).squeeze(1)
            return lg.masked_fill(~enc_mask_bool, -1e9)

        s0s, s0e = _span(self.s0s_q), _span(self.s0e_q)
        s1s, s1e = _span(self.s1s_q), _span(self.s1e_q)

        # Ref logits -- use num_prev to mask
        # prev_hiddens_flat is (1, max_steps, 256)
        max_steps = prev_hiddens_flat.size(1)
        # Create ref mask
        ref_indices = torch.arange(max_steps).unsqueeze(0).float()
        ref_mask = ref_indices < num_prev_scalar.unsqueeze(1).float()

        def _ref(ql, kl):
            q = ql(new_hidden).unsqueeze(1)
            k = kl(prev_hiddens_flat)
            sc = torch.bmm(q, k.transpose(1, 2)).squeeze(1)
            return sc.masked_fill(~ref_mask, -1e9)

        r0, r1 = _ref(self.r0q, self.r0k), _ref(self.r1q, self.r1k)

        return (new_hidden, op_logits, a0t_logits, a1t_logits,
                s0s, s0e, s1s, s1e, r0, r1)


def main():
    from pow.pow1.discovery import discover_body
    from pow.pow1.mind import SomaMind

    artifacts = "pow/pow1/artifacts"
    out_dir = "models"
    os.makedirs(out_dir, exist_ok=True)

    with open(os.path.join(artifacts, "meta.json")) as f:
        meta = json.load(f)
    mind = SomaMind(meta["vocab_size"], meta["num_conventions"])
    mind.load_state_dict(torch.load(
        os.path.join(artifacts, "soma_mind.pt"),
        map_location="cpu", weights_only=True))
    mind.eval()
    print(f"Model: vocab={meta['vocab_size']}, conv={meta['num_conventions']}, "
          f"params={sum(p.numel() for p in mind.parameters()):,}")

    catalog, _ = discover_body()

    # Fixed sequence length — pad inputs to this size
    # Avoids dynamic shape issues with tract ONNX runtime
    MAX_SEQ_LEN = 20

    # Export encoder
    print(f"\nExporting encoder.onnx (fixed seq_len={MAX_SEQ_LEN})...")
    encoder = EncoderForExport(mind)
    encoder.eval()
    torch.onnx.export(
        encoder,
        (torch.randint(0, meta["vocab_size"], (1, MAX_SEQ_LEN)),
         torch.ones(1, MAX_SEQ_LEN)),
        os.path.join(out_dir, "encoder.onnx"),
        input_names=["input_ids", "mask"],
        output_names=["encoder_out", "pooled", "init_hidden"],
        opset_version=17,
        dynamo=False)
    print("  OK")

    # Export decoder
    print("Exporting decoder.onnx...")
    decoder = DecoderStepForExport(mind)
    decoder.eval()
    max_steps = 8
    torch.onnx.export(
        decoder,
        (torch.tensor([0], dtype=torch.long),
         torch.randn(1, 256), torch.randn(1, MAX_SEQ_LEN, 256),
         torch.ones(1, MAX_SEQ_LEN),
         torch.zeros(1, max_steps, 256),
         torch.tensor([0], dtype=torch.long)),
        os.path.join(out_dir, "decoder.onnx"),
        input_names=["prev_op", "hidden", "encoder_out", "enc_mask",
                     "prev_hiddens", "num_prev"],
        output_names=["new_hidden", "op_logits", "a0t_logits", "a1t_logits",
                       "s0s", "s0e", "s1s", "s1e", "r0", "r1"],
        opset_version=17,
        dynamo=False)
    print("  OK")

    # Export tokenizer + meta
    shutil.copy(os.path.join(artifacts, "vocab.json"),
                os.path.join(out_dir, "tokenizer.json"))

    catalog_info = [{"id": c.id, "name": c.name, "function": c.function,
                     "call_pattern": c.call_pattern, "var_args": c.var_args,
                     "description": c.description} for c in catalog]
    export_meta = {
        "vocab_size": meta["vocab_size"], "num_conventions": meta["num_conventions"],
        "max_steps": max_steps, "max_seq_len": MAX_SEQ_LEN, "decoder_dim": 256,
        "emit_id": meta["num_conventions"],
        "stop_id": meta["num_conventions"] + 1,
        "start_token": meta["num_conventions"] + 2,
        "catalog": catalog_info,
    }
    with open(os.path.join(out_dir, "meta.json"), "w") as f:
        json.dump(export_meta, f, indent=2)

    print(f"\nExported to {out_dir}/:")
    for f_name in os.listdir(out_dir):
        size = os.path.getsize(os.path.join(out_dir, f_name))
        print(f"  {f_name}: {size / 1024:.0f} KB")

    # Verify
    print("\nVerifying with onnxruntime...")
    try:
        import onnxruntime as ort
        enc_s = ort.InferenceSession(os.path.join(out_dir, "encoder.onnx"))
        dec_s = ort.InferenceSession(os.path.join(out_dir, "decoder.onnx"))
        print(f"  Encoder: {[i.name for i in enc_s.get_inputs()]}")
        print(f"  Decoder: {[i.name for i in dec_s.get_inputs()]}")
        print("  ONNX verification OK")
    except ImportError:
        print("  onnxruntime not installed, skipping verification")

    print("\nDone. Models ready for Rust runtime.")


if __name__ == "__main__":
    main()
