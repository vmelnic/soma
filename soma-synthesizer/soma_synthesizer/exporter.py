"""
SOMA Exporter — ONNX export, .soma-model binary, metadata, catalog.

Part of the Synthesizer (spec Section 7). Exports trained Mind models
into formats consumed by the Rust SOMA Core at runtime.

Export targets:
  - ONNX (server/desktop): encoder.onnx + decoder.onnx
  - .soma-model (embedded): quantized binary with SOMA magic header
  - tokenizer.json: vocabulary for intent tokenisation
  - catalog.json: convention catalog (separate from meta.json)
  - meta.json: model metadata with SHA-256 model hash

Self-contained — no imports from poc/ or pow/.
"""

import hashlib
import json
import os
import shutil
import struct
from datetime import datetime, timezone

import torch
import torch.nn as nn
import torch.nn.functional as F


# -----------------------------------------------------------------------
# Architecture constants — match spec Section 7.2
# -----------------------------------------------------------------------

SOMA_MAGIC = b"SOMA"
FORMAT_VERSION = 1

QUANT_F32 = 0
QUANT_F16 = 1
QUANT_INT8 = 2

ARCH_BILSTM_GRU = 0


# -----------------------------------------------------------------------
# ONNX export wrappers
# -----------------------------------------------------------------------

class EncoderForExport(nn.Module):
    """ONNX-compatible encoder wrapper.

    Replaces ``pack_padded_sequence`` with explicit masking so the graph
    is fully traceable by the ONNX exporter.  Inputs use a float mask
    instead of integer lengths for the same reason.
    """

    def __init__(self, mind):
        super().__init__()
        self.embedding = mind.embedding
        self.encoder = mind.encoder
        self.init_h = mind.init_h

    def forward(self, input_ids, mask_float):
        """
        Args:
            input_ids: (B, L) token indices.
            mask_float: (B, L) float mask — 1.0 for real tokens, 0.0 for pad.

        Returns:
            encoder_out: (B, L, enc_dim) encoder hidden states.
            pooled: (B, enc_dim) mean-pooled encoder output.
            init_hidden: (B, decoder_dim) initial decoder hidden state.
        """
        emb = self.embedding(input_ids)
        encoder_out, _ = self.encoder(emb)
        mask_3d = mask_float.unsqueeze(-1)
        length = mask_float.sum(dim=1, keepdim=True).clamp(min=1)
        pooled = (encoder_out * mask_3d).sum(dim=1) / length
        init_hidden = torch.tanh(self.init_h(pooled))
        return encoder_out, pooled, init_hidden


class DecoderStepForExport(nn.Module):
    """ONNX-compatible single decoder step.

    Wraps all decoder heads (opcode, arg types, span pointers, ref
    pointers) into a single forward pass suitable for autoregressive
    inference in the Rust runtime.
    """

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
        """
        Args:
            prev_op_id: (B,) previous opcode index.
            hidden: (B, decoder_dim) current decoder hidden state.
            encoder_out: (B, L, enc_dim) encoder outputs.
            enc_mask_float: (B, L) encoder mask (float).
            prev_hiddens_flat: (B, max_steps, decoder_dim) stacked
                previous decoder hidden states.
            num_prev_scalar: (B,) number of valid previous steps.

        Returns:
            Tuple of (new_hidden, op_logits, a0t_logits, a1t_logits,
            s0s, s0e, s1s, s1e, r0, r1).
        """
        # Context attention over encoder outputs
        prev_emb = self.op_emb(prev_op_id)
        ctx_query = self.ctx_q(hidden)
        scores = torch.bmm(
            ctx_query.unsqueeze(1), encoder_out.transpose(1, 2)
        ).squeeze(1)
        enc_mask_bool = enc_mask_float > 0.5
        scores = scores.masked_fill(~enc_mask_bool, -1e9)
        weights = F.softmax(scores, dim=-1)
        context = torch.bmm(weights.unsqueeze(1), encoder_out).squeeze(1)

        # GRU step
        gru_input = torch.cat([prev_emb, context], dim=-1)
        new_hidden = self.gru(gru_input, hidden)

        # Opcode + arg-type logits
        op_logits = self.op_head(new_hidden)
        a0t_logits = self.a0t_head(new_hidden)
        a1t_logits = self.a1t_head(new_hidden)

        # Span logits — pointer over encoder positions
        def _span(query_layer):
            q = query_layer(new_hidden)
            logits = torch.bmm(
                q.unsqueeze(1), encoder_out.transpose(1, 2)
            ).squeeze(1)
            return logits.masked_fill(~enc_mask_bool, -1e9)

        s0s = _span(self.s0s_q)
        s0e = _span(self.s0e_q)
        s1s = _span(self.s1s_q)
        s1e = _span(self.s1e_q)

        # Ref logits — pointer over previous decoder steps
        max_steps = prev_hiddens_flat.size(1)
        ref_indices = torch.arange(max_steps).unsqueeze(0).float()
        ref_mask = ref_indices < num_prev_scalar.unsqueeze(1).float()

        def _ref(query_layer, key_layer):
            q = query_layer(new_hidden).unsqueeze(1)
            k = key_layer(prev_hiddens_flat)
            sc = torch.bmm(q, k.transpose(1, 2)).squeeze(1)
            return sc.masked_fill(~ref_mask, -1e9)

        r0 = _ref(self.r0q, self.r0k)
        r1 = _ref(self.r1q, self.r1k)

        return (new_hidden, op_logits, a0t_logits, a1t_logits,
                s0s, s0e, s1s, s1e, r0, r1)


# -----------------------------------------------------------------------
# ONNX export
# -----------------------------------------------------------------------

def export_onnx(model, tokenizer, catalog, output_dir,
                max_seq_len=20, max_steps=8, opset_version=17):
    """Export encoder and decoder to ONNX format.

    Produces ``encoder.onnx`` and ``decoder.onnx`` in *output_dir*,
    suitable for tract / onnxruntime inference in the Rust runtime.

    Args:
        model: Trained SomaMind instance.
        tokenizer: Tokenizer with ``vocab_size`` attribute.
        catalog: List of convention dicts (or objects with .id/.name attrs).
        output_dir: Directory to write ONNX files into.
        max_seq_len: Fixed input sequence length (padded).
        max_steps: Maximum decoder steps.
        opset_version: ONNX opset version.

    Returns:
        Tuple of (encoder_path, decoder_path).
    """
    os.makedirs(output_dir, exist_ok=True)

    vocab_size = tokenizer.vocab_size
    decoder_dim = model.init_h.out_features
    enc_dim = model.init_h.in_features  # encoder output dim

    # --- Encoder ---
    encoder = EncoderForExport(model)
    encoder.eval()

    dummy_ids = torch.randint(0, vocab_size, (1, max_seq_len))
    dummy_mask = torch.ones(1, max_seq_len)

    encoder_path = os.path.join(output_dir, "encoder.onnx")
    torch.onnx.export(
        encoder,
        (dummy_ids, dummy_mask),
        encoder_path,
        input_names=["input_ids", "mask"],
        output_names=["encoder_out", "pooled", "init_hidden"],
        opset_version=opset_version,
        dynamo=False,
    )

    # --- Decoder ---
    decoder = DecoderStepForExport(model)
    decoder.eval()

    dummy_prev_op = torch.tensor([0], dtype=torch.long)
    dummy_hidden = torch.randn(1, decoder_dim)
    dummy_enc_out = torch.randn(1, max_seq_len, enc_dim)
    dummy_enc_mask = torch.ones(1, max_seq_len)
    dummy_prev_hiddens = torch.zeros(1, max_steps, decoder_dim)
    dummy_num_prev = torch.tensor([0], dtype=torch.long)

    decoder_path = os.path.join(output_dir, "decoder.onnx")
    torch.onnx.export(
        decoder,
        (dummy_prev_op, dummy_hidden, dummy_enc_out, dummy_enc_mask,
         dummy_prev_hiddens, dummy_num_prev),
        decoder_path,
        input_names=["prev_op", "hidden", "encoder_out", "enc_mask",
                     "prev_hiddens", "num_prev"],
        output_names=["new_hidden", "op_logits", "a0t_logits", "a1t_logits",
                      "s0s", "s0e", "s1s", "s1e", "r0", "r1"],
        opset_version=opset_version,
        dynamo=False,
    )

    return encoder_path, decoder_path


# -----------------------------------------------------------------------
# Tokenizer / catalog / metadata export
# -----------------------------------------------------------------------

def export_tokenizer(tokenizer, output_dir):
    """Export tokenizer vocabulary to ``tokenizer.json``.

    If the tokenizer has a ``save`` method (file-based), it writes
    directly.  Otherwise copies from a source path if provided.

    Args:
        tokenizer: Tokenizer instance (must have a ``save`` method)
            or a str path to an existing tokenizer JSON file.
        output_dir: Directory to write into.

    Returns:
        Path to the exported tokenizer file.
    """
    os.makedirs(output_dir, exist_ok=True)
    dest = os.path.join(output_dir, "tokenizer.json")

    if isinstance(tokenizer, str):
        # Path to existing tokenizer file — copy it
        shutil.copy(tokenizer, dest)
    else:
        # Tokenizer object with save()
        tokenizer.save(dest)

    return dest


def export_catalog(catalog, output_dir):
    """Export convention catalog to ``catalog.json`` (separate file).

    Each catalog entry contains at minimum: id, name.  Optional fields
    (function, call_pattern, var_args, description, plugin) are
    preserved if present.

    Args:
        catalog: List of convention dicts or objects with attributes.
        output_dir: Directory to write into.

    Returns:
        Path to the exported catalog file.
    """
    os.makedirs(output_dir, exist_ok=True)
    dest = os.path.join(output_dir, "catalog.json")

    catalog_list = []
    for entry in catalog:
        if isinstance(entry, dict):
            catalog_list.append(entry)
        else:
            # Object with attributes — extract known fields
            info = {}
            for attr in ("id", "catalog_id", "name", "full_name", "function",
                         "call_pattern", "var_args", "description", "plugin"):
                val = getattr(entry, attr, None)
                if val is not None:
                    info[attr] = val
            catalog_list.append(info)

    with open(dest, "w") as f:
        json.dump(catalog_list, f, indent=2)

    return dest


def _sha256_files(*paths):
    """Compute a single SHA-256 hex digest over one or more files."""
    h = hashlib.sha256()
    for path in sorted(paths):
        with open(path, "rb") as f:
            while True:
                chunk = f.read(1 << 16)
                if not chunk:
                    break
                h.update(chunk)
    return h.hexdigest()


def export_metadata(model, catalog, training_stats, output_dir,
                    max_seq_len=20, max_steps=8, plugins=None):
    """Export ``meta.json`` with model metadata and SHA-256 hash.

    The catalog is NOT embedded in meta.json — it lives in a separate
    ``catalog.json`` (per spec Section 7).

    Args:
        model: Trained SomaMind instance.
        catalog: Convention catalog (list).
        training_stats: Dict with training statistics, e.g.
            ``{"total_examples", "best_epoch", "test_e2e", "elapsed"}``.
        output_dir: Directory containing the ONNX files and where
            meta.json will be written.
        max_seq_len: Fixed sequence length used during export.
        max_steps: Maximum decoder steps.
        plugins: Optional list of plugin names included in this build.

    Returns:
        Path to the exported meta.json file.
    """
    os.makedirs(output_dir, exist_ok=True)

    # Derive architecture dimensions from model parameters
    vocab_size = model.embedding.num_embeddings
    embed_dim = model.embedding.embedding_dim
    hidden_dim = model.encoder.hidden_size
    num_layers = model.encoder.num_layers
    decoder_dim = model.init_h.out_features
    num_conventions = len(catalog)
    num_output_ids = num_conventions + 2  # +EMIT +STOP
    param_count = sum(p.numel() for p in model.parameters())

    # Compute SHA-256 over exported ONNX files if they exist
    model_hash = None
    encoder_path = os.path.join(output_dir, "encoder.onnx")
    decoder_path = os.path.join(output_dir, "decoder.onnx")
    onnx_files = [p for p in (encoder_path, decoder_path) if os.path.exists(p)]
    if onnx_files:
        model_hash = _sha256_files(*onnx_files)

    meta = {
        "soma_synthesizer_version": "0.1.0",
        "architecture": "bilstm_gru",
        "vocab_size": vocab_size,
        "embed_dim": embed_dim,
        "hidden_dim": hidden_dim,
        "decoder_dim": decoder_dim,
        "num_layers": num_layers,
        "num_conventions": num_conventions,
        "num_output_ids": num_output_ids,
        "max_steps": max_steps,
        "max_seq_len": max_seq_len,
        "emit_id": num_conventions,
        "stop_id": num_conventions + 1,
        "start_token": num_conventions + 2,
        "parameter_count": param_count,
        "plugins": plugins or [],
        "training": {
            "examples": training_stats.get("total_examples", 0),
            "epochs": training_stats.get("best_epoch", 0),
            "test_e2e_accuracy": training_stats.get("test_e2e", 0.0),
            "training_time_seconds": training_stats.get("elapsed", 0.0),
        },
        "export_timestamp": datetime.now(timezone.utc).isoformat(),
        "model_hash": model_hash,
    }

    dest = os.path.join(output_dir, "meta.json")
    with open(dest, "w") as f:
        json.dump(meta, f, indent=2)

    return dest


# -----------------------------------------------------------------------
# .soma-model binary export (embedded target)
# -----------------------------------------------------------------------

def _model_to_sections(state_dict):
    """Convert a state dict into a list of (name, tensor) pairs.

    Tensors are detached and moved to CPU.  The name uses the
    PyTorch parameter path (e.g. ``encoder.weight_ih_l0``).
    """
    sections = []
    for name, param in state_dict.items():
        tensor = param.detach().cpu()
        sections.append((name, tensor))
    return sections


def quantize_int8(model, calibration_data=None, method="asymmetric"):
    """Post-training int8 quantization with optional calibration.

    If *calibration_data* is provided, runs it through the model to
    observe activation ranges and computes per-layer scale/zero_point.
    Otherwise falls back to weight-only quantization using the weight
    tensor min/max directly.

    Args:
        model: Trained SomaMind instance (or any nn.Module).
        calibration_data: Optional iterable of (input_ids, lengths)
            batches for calibration.  100-500 examples recommended.
        method: ``"symmetric"`` or ``"asymmetric"``.

    Returns:
        Tuple of (quantized_state_dict, quant_params) where
        quantized_state_dict maps name -> int8 tensor and
        quant_params maps name -> {"scale": float, "zero_point": int}.
    """
    model_copy = model
    model_copy.eval()

    # Collect activation ranges via forward hooks (if calibration data given)
    activation_ranges = {}
    hooks = []
    if calibration_data is not None:
        def _make_hook(name):
            def hook_fn(module, inp, output):
                if isinstance(output, torch.Tensor):
                    t = output
                elif isinstance(output, tuple):
                    t = output[0] if isinstance(output[0], torch.Tensor) else None
                else:
                    t = None
                if t is not None:
                    prev = activation_ranges.get(name)
                    cur_min = t.min().item()
                    cur_max = t.max().item()
                    if prev is not None:
                        cur_min = min(prev[0], cur_min)
                        cur_max = max(prev[1], cur_max)
                    activation_ranges[name] = (cur_min, cur_max)
            return hook_fn

        for name, module in model_copy.named_modules():
            if isinstance(module, nn.Linear):
                h = module.register_forward_hook(_make_hook(name))
                hooks.append(h)

        with torch.no_grad():
            for batch in calibration_data:
                if isinstance(batch, (list, tuple)):
                    model_copy(*batch)
                else:
                    model_copy(batch)

        for h in hooks:
            h.remove()

    # Quantize weights
    quantized_state = {}
    quant_params = {}

    for name, param in model_copy.state_dict().items():
        tensor = param.detach().cpu().float()

        # Determine range — prefer activation range if available,
        # otherwise use weight tensor range
        layer_name = name.rsplit(".", 1)[0]
        if layer_name in activation_ranges:
            min_val, max_val = activation_ranges[layer_name]
        else:
            min_val = tensor.min().item()
            max_val = tensor.max().item()

        if method == "symmetric":
            abs_max = max(abs(min_val), abs(max_val), 1e-8)
            scale = abs_max / 127.0
            zero_point = 0
        else:
            rng = max(max_val - min_val, 1e-8)
            scale = rng / 255.0
            zero_point = int(round(-min_val / scale))
            zero_point = max(-128, min(127, zero_point))

        quantized = torch.round(tensor / scale) + zero_point
        quantized = quantized.clamp(-128, 127).to(torch.int8)

        quantized_state[name] = quantized
        quant_params[name] = {"scale": scale, "zero_point": zero_point}

    return quantized_state, quant_params


def export_soma_model(model, output_path, quantize="int8",
                      calibration_data=None, max_steps=8):
    """Export model to the ``.soma-model`` binary format (spec Section 7.2).

    Binary layout::

        magic:           "SOMA" (4 bytes)
        version:         u8
        quantization:    u8 (0=f32, 1=f16, 2=int8)
        architecture:    u8 (0=bilstm_gru)
        vocab_size:      u32 BE
        embed_dim:       u16 BE
        hidden_dim:      u16 BE
        decoder_dim:     u16 BE
        num_layers:      u8
        num_conventions: u16 BE
        max_steps:       u8
        section_count:   u16 BE
        [for each section]:
            name_len:    u8
            name:        UTF-8 bytes
            ndim:        u8
            dims:        ndim x u16 BE
            data:        raw bytes (f32/f16/int8 depending on quantization)

    For int8 quantization, each section is followed by per-tensor
    scale (float32, 4 bytes) and zero_point (int8, 1 byte).

    Args:
        model: Trained SomaMind instance.
        output_path: Path to write the .soma-model file.
        quantize: Quantization mode: ``"f32"``, ``"f16"``, or ``"int8"``.
        calibration_data: Calibration data for int8 quantization.
        max_steps: Maximum decoder steps.

    Returns:
        Path to the written file.
    """
    model.eval()

    # Derive architecture parameters
    vocab_size = model.embedding.num_embeddings
    embed_dim = model.embedding.embedding_dim
    hidden_dim = model.encoder.hidden_size
    num_layers = model.encoder.num_layers
    decoder_dim = model.init_h.out_features
    # num_conventions = op_head output - 2 (EMIT + STOP)
    num_conventions = model.op_head.out_features - 2

    quant_byte = {"f32": QUANT_F32, "f16": QUANT_F16, "int8": QUANT_INT8}[quantize]

    # Prepare weight sections
    if quantize == "int8":
        quant_state, quant_params = quantize_int8(
            model, calibration_data=calibration_data
        )
        sections = [(name, quant_state[name]) for name in quant_state]
    else:
        raw_state = model.state_dict()
        sections = _model_to_sections(raw_state)
        quant_params = {}

    os.makedirs(os.path.dirname(os.path.abspath(output_path)), exist_ok=True)

    with open(output_path, "wb") as f:
        # --- Header ---
        f.write(SOMA_MAGIC)
        f.write(struct.pack("B", FORMAT_VERSION))
        f.write(struct.pack("B", quant_byte))
        f.write(struct.pack("B", ARCH_BILSTM_GRU))

        f.write(struct.pack(">I", vocab_size))
        f.write(struct.pack(">H", embed_dim))
        f.write(struct.pack(">H", hidden_dim))
        f.write(struct.pack(">H", decoder_dim))
        f.write(struct.pack("B", num_layers))
        f.write(struct.pack(">H", num_conventions))
        f.write(struct.pack("B", max_steps))

        f.write(struct.pack(">H", len(sections)))

        # --- Weight sections ---
        for name, tensor in sections:
            name_bytes = name.encode("utf-8")
            f.write(struct.pack("B", len(name_bytes)))
            f.write(name_bytes)

            shape = tensor.shape
            f.write(struct.pack("B", len(shape)))
            for dim in shape:
                f.write(struct.pack(">H", dim))

            # Write tensor data
            if quantize == "f16":
                data = tensor.half().numpy().tobytes()
            elif quantize == "int8":
                data = tensor.numpy().tobytes()
            else:
                data = tensor.float().numpy().tobytes()
            f.write(data)

            # For int8, append per-tensor scale and zero_point
            if quantize == "int8":
                qp = quant_params.get(name, {"scale": 1.0, "zero_point": 0})
                f.write(struct.pack("<f", qp["scale"]))
                f.write(struct.pack("b", qp["zero_point"]))

    return output_path


# -----------------------------------------------------------------------
# Convenience: full export pipeline
# -----------------------------------------------------------------------

def export_all(model, tokenizer, catalog, output_dir,
               training_stats=None, plugins=None,
               max_seq_len=20, max_steps=8,
               embedded_path=None, quantize="int8",
               calibration_data=None):
    """Run the complete export pipeline.

    Exports ONNX models, tokenizer, catalog, and metadata to
    *output_dir*.  Optionally exports an embedded ``.soma-model``
    binary to *embedded_path*.

    Args:
        model: Trained SomaMind instance.
        tokenizer: Tokenizer instance or path to tokenizer JSON.
        catalog: List of convention entries.
        output_dir: Base output directory.
        training_stats: Dict with training statistics.
        plugins: List of plugin names.
        max_seq_len: Fixed sequence length for ONNX.
        max_steps: Maximum decoder steps.
        embedded_path: If set, also export .soma-model to this path.
        quantize: Quantization for .soma-model (``"f32"``/``"f16"``/``"int8"``).
        calibration_data: Calibration data for int8 quantization.

    Returns:
        Dict mapping export artifact names to their file paths.
    """
    if training_stats is None:
        training_stats = {}

    results = {}

    # ONNX
    enc_path, dec_path = export_onnx(
        model, tokenizer, catalog, output_dir,
        max_seq_len=max_seq_len, max_steps=max_steps,
    )
    results["encoder_onnx"] = enc_path
    results["decoder_onnx"] = dec_path

    # Tokenizer
    tok_path = export_tokenizer(tokenizer, output_dir)
    results["tokenizer"] = tok_path

    # Catalog (separate file, not in meta.json)
    cat_path = export_catalog(catalog, output_dir)
    results["catalog"] = cat_path

    # Metadata (must come after ONNX so hash can be computed)
    meta_path = export_metadata(
        model, catalog, training_stats, output_dir,
        max_seq_len=max_seq_len, max_steps=max_steps,
        plugins=plugins,
    )
    results["metadata"] = meta_path

    # Embedded binary (optional)
    if embedded_path is not None:
        soma_path = export_soma_model(
            model, embedded_path, quantize=quantize,
            calibration_data=calibration_data, max_steps=max_steps,
        )
        results["soma_model"] = soma_path

    return results
