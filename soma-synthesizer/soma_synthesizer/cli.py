"""SOMA Synthesizer CLI — soma-synthesize command.

Provides the ``soma-synthesize`` entry point defined in pyproject.toml.
Each subcommand drives a complete pipeline stage: train, train-lora,
export, validate, test, benchmark.

Self-contained — no imports from poc/ or pow/.
"""

import argparse
import json
import os
import sys
import time


# ---------------------------------------------------------------------------
# Subcommands
# ---------------------------------------------------------------------------


def cmd_train(args):
    """Train a new Mind from plugin training data.

    Pipeline: load config -> collect data -> validate -> augment ->
    build vocab -> create model -> train -> export.
    """
    import torch

    from soma_synthesizer.config import load_config, SynthesisConfig
    from soma_synthesizer.data import collect_training_data, build_catalog, expand_examples
    from soma_synthesizer.tokenizer import Tokenizer, NULL_IDX
    from soma_synthesizer.validator import TrainingDataValidator
    from soma_synthesizer.augmentor import Augmentor
    from soma_synthesizer.model import SomaMind
    from soma_synthesizer.trainer import train_mind
    from soma_synthesizer.exporter import export_onnx, export_metadata

    # 1. Load configuration
    config = _load_config_or_default(args.config)

    # 2. Collect training data from plugins
    print("\n[Synthesis] Collecting training data...")
    plugin_dirs = _discover_plugin_dirs(args.plugins)
    examples = collect_training_data(plugin_dirs, domain_file=args.domain)
    catalog = build_catalog(plugin_dirs)
    num_conventions = len(catalog)
    print(f"  Plugins: {len(plugin_dirs)}")
    print(f"  Total conventions: {num_conventions}")
    print(f"  Raw examples: {len(examples)}")

    # 3. Validate training data (abort on errors)
    print("\n[Synthesis] Validating training data...")
    validator = TrainingDataValidator(catalog)
    report = validator.validate(examples)
    _print_validation_report(report)
    if report.errors:
        print("\n[Synthesis] ABORTING — fix validation errors before training.")
        sys.exit(1)

    # 4. Expand template examples into concrete (intent, program) pairs
    pairs = expand_examples(examples, catalog)
    print(f"  Expanded training pairs: {len(pairs)}")

    # 5. Augment data
    if config.augmentation.enabled:
        print("\n[Synthesis] Augmenting training data...")
        aug = Augmentor(config.augmentation.__dict__ if hasattr(config, 'augmentation') else {})
        pairs = aug.augment_dataset(pairs)
        print(f"  After augmentation: {len(pairs)}")

    # 6. Build tokenizer vocabulary
    tokenizer = Tokenizer()
    tokenizer.build_vocab([p["intent"] for p in pairs])
    print(f"  Vocabulary: {tokenizer.vocab_size} tokens")

    # 7. Create model
    model = SomaMind(
        vocab_size=tokenizer.vocab_size,
        num_conventions=num_conventions,
        embed_dim=config.architecture.embed_dim,
        hidden_dim=config.architecture.hidden_dim,
        decoder_dim=config.architecture.decoder_dim,
        opcode_embed_dim=config.architecture.opcode_embed_dim,
        num_layers=config.architecture.num_encoder_layers,
        dropout=config.architecture.dropout,
        max_program_steps=config.architecture.max_program_steps,
    )
    total_params = sum(p.numel() for p in model.parameters())
    print(f"\n[Synthesis] Model: {config.architecture.type}, {total_params:,} parameters")

    # 8. Train
    training_stats = train_mind(
        model=model,
        pairs=pairs,
        tokenizer=tokenizer,
        catalog=catalog,
        config=config,
    )

    # 9. Export
    output_dir = args.output
    os.makedirs(output_dir, exist_ok=True)

    if args.target in ("server", "both"):
        server_dir = os.path.join(output_dir, "server")
        os.makedirs(server_dir, exist_ok=True)
        print(f"\n[Synthesis] Exporting ONNX to {server_dir}/...")
        export_onnx(model, server_dir, config)

    if args.target in ("embedded", "both"):
        from soma_synthesizer.exporter import export_embedded
        embedded_dir = os.path.join(output_dir, "embedded")
        os.makedirs(embedded_dir, exist_ok=True)
        print(f"\n[Synthesis] Exporting .soma-model to {embedded_dir}/...")
        export_embedded(model, embedded_dir, config)

    # Save tokenizer, catalog, and metadata
    vocab_path = os.path.join(output_dir, "vocab.json")
    tokenizer.save(vocab_path)
    print(f"  -> {vocab_path}")

    catalog_path = os.path.join(output_dir, "catalog.json")
    with open(catalog_path, "w") as f:
        json.dump(catalog, f, indent=2)
    print(f"  -> {catalog_path}")

    # Save model checkpoint for later LoRA / re-export
    model_path = os.path.join(output_dir, "soma_mind.pt")
    torch.save(model.state_dict(), model_path)
    print(f"  -> {model_path}")

    meta_path = os.path.join(output_dir, "meta.json")
    export_metadata(config, training_stats, tokenizer, catalog, meta_path)
    print(f"  -> {meta_path}")

    print("\n[Synthesis] SOMA synthesis complete.")


def cmd_train_lora(args):
    """Train LoRA weights for a specific plugin."""
    import torch

    from soma_synthesizer.config import load_config
    from soma_synthesizer.data import collect_training_data, build_catalog, expand_examples
    from soma_synthesizer.tokenizer import Tokenizer
    from soma_synthesizer.model import SomaMind
    from soma_synthesizer.lora import apply_lora, train_lora, save_lora, save_lora_metadata

    # 1. Load config + base model metadata
    config = _load_config_or_default(args.config)

    base_dir = args.base_model
    meta_path = os.path.join(base_dir, "meta.json")
    with open(meta_path) as f:
        meta = json.load(f)

    # 2. Load tokenizer from base model
    tokenizer = Tokenizer()
    vocab_path = os.path.join(base_dir, "vocab.json")
    tokenizer.load(vocab_path)

    # 3. Load catalog from base model
    catalog_path = os.path.join(base_dir, "catalog.json")
    with open(catalog_path) as f:
        catalog = json.load(f)

    # 4. Reconstruct and load base model
    model = SomaMind(
        vocab_size=meta["vocab_size"],
        num_conventions=meta["num_conventions"],
        embed_dim=meta.get("embed_dim", config.architecture.embed_dim),
        hidden_dim=meta.get("hidden_dim", config.architecture.hidden_dim),
        decoder_dim=meta.get("decoder_dim", config.architecture.decoder_dim),
        opcode_embed_dim=meta.get("opcode_embed_dim", config.architecture.opcode_embed_dim),
        num_layers=meta.get("num_encoder_layers", config.architecture.num_encoder_layers),
        dropout=0.0,  # no dropout at inference / LoRA training
        max_program_steps=meta.get("max_steps", config.architecture.max_program_steps),
    )
    model_path = os.path.join(base_dir, "soma_mind.pt")
    model.load_state_dict(torch.load(model_path, map_location="cpu", weights_only=True))
    print(f"[LoRA] Loaded base model from {model_path}")

    # 5. Collect plugin-specific training data
    plugin_name = args.plugin
    plugin_dir = _find_plugin_dir(base_dir, plugin_name)
    if plugin_dir is None:
        # Fall back: look relative to the base model for a plugins/ sibling
        plugin_dir = _find_plugin_dir(os.path.dirname(base_dir), plugin_name)
    if plugin_dir is None:
        print(f"[LoRA] ERROR: Cannot find plugin directory for '{plugin_name}'.")
        sys.exit(1)

    examples = collect_training_data([plugin_dir])
    pairs = expand_examples(examples, catalog)
    print(f"[LoRA] Plugin '{plugin_name}': {len(pairs)} training pairs")

    # 6. Freeze base weights and attach LoRA
    for param in model.parameters():
        param.requires_grad_(False)

    lora_layers = apply_lora(
        model,
        rank=config.lora.rank,
        alpha=config.lora.alpha,
        target_modules=config.lora.target_modules,
    )
    trainable = sum(p.numel() for p in model.parameters() if p.requires_grad)
    print(f"[LoRA] Trainable LoRA parameters: {trainable:,}")

    # 7. Train LoRA
    train_lora(
        model=model,
        pairs=pairs,
        tokenizer=tokenizer,
        catalog=catalog,
        config=config,
    )

    # 8. Save LoRA weights + metadata
    output_dir = args.output
    os.makedirs(output_dir, exist_ok=True)
    lora_path = os.path.join(output_dir, f"{plugin_name}.lora")
    save_lora(lora_layers, lora_path)
    print(f"[LoRA] Saved LoRA weights -> {lora_path}")

    lora_meta_path = os.path.join(output_dir, f"{plugin_name}.lora.json")
    save_lora_metadata(lora_meta_path, {
        "plugin": plugin_name,
        "mind_architecture": config.architecture.type,
        "mind_version": config.version,
        "hidden_dim": config.architecture.hidden_dim,
        "decoder_dim": config.architecture.decoder_dim,
        "target_layers": config.lora.target_modules,
        "rank": config.lora.rank,
        "alpha": config.lora.alpha,
    })
    print(f"[LoRA] Saved LoRA metadata -> {lora_meta_path}")
    print("[LoRA] Plugin LoRA training complete.")


def cmd_export(args):
    """Export a trained model to ONNX / .soma-model."""
    import torch

    from soma_synthesizer.config import load_config, SynthesisConfig
    from soma_synthesizer.model import SomaMind
    from soma_synthesizer.exporter import export_onnx, export_embedded, export_metadata

    # Load model metadata
    model_dir = args.model
    meta_path = os.path.join(model_dir, "meta.json")
    if not os.path.isfile(meta_path):
        print(f"[Export] ERROR: meta.json not found in {model_dir}")
        sys.exit(1)

    with open(meta_path) as f:
        meta = json.load(f)

    # Reconstruct model
    config = SynthesisConfig()  # defaults for anything not in meta
    model = SomaMind(
        vocab_size=meta["vocab_size"],
        num_conventions=meta["num_conventions"],
        embed_dim=meta.get("embed_dim", config.architecture.embed_dim),
        hidden_dim=meta.get("hidden_dim", config.architecture.hidden_dim),
        decoder_dim=meta.get("decoder_dim", config.architecture.decoder_dim),
        opcode_embed_dim=meta.get("opcode_embed_dim", config.architecture.opcode_embed_dim),
        num_layers=meta.get("num_encoder_layers", config.architecture.num_encoder_layers),
        dropout=0.0,
        max_program_steps=meta.get("max_steps", config.architecture.max_program_steps),
    )
    model_path = os.path.join(model_dir, "soma_mind.pt")
    model.load_state_dict(torch.load(model_path, map_location="cpu", weights_only=True))
    model.set_to_inference_mode()
    print(f"[Export] Loaded model from {model_path}")

    output_dir = args.output
    os.makedirs(output_dir, exist_ok=True)

    if args.target in ("server", "both"):
        server_dir = os.path.join(output_dir, "server")
        os.makedirs(server_dir, exist_ok=True)
        print(f"[Export] Exporting ONNX to {server_dir}/...")
        export_onnx(model, server_dir, config)
        _list_dir_sizes(server_dir)

    if args.target in ("embedded", "both"):
        embedded_dir = os.path.join(output_dir, "embedded")
        os.makedirs(embedded_dir, exist_ok=True)
        print(f"[Export] Exporting .soma-model to {embedded_dir}/...")
        export_embedded(
            model, embedded_dir, config,
            ram_budget=args.embedded_ram,
            flash_budget=args.embedded_flash,
        )
        _list_dir_sizes(embedded_dir)

    print("[Export] Export complete.")


def cmd_validate(args):
    """Validate plugin training data and print a report."""
    from soma_synthesizer.data import collect_training_data, build_catalog
    from soma_synthesizer.validator import TrainingDataValidator

    plugin_dirs = _discover_plugin_dirs(args.plugins)
    examples = collect_training_data(plugin_dirs, domain_file=args.domain)
    catalog = build_catalog(plugin_dirs)

    print(f"\n[Validate] Loaded {len(examples)} examples from {len(plugin_dirs)} plugins")
    print(f"[Validate] Convention catalog: {len(catalog)} entries")

    validator = TrainingDataValidator(catalog)
    report = validator.validate(examples)
    _print_validation_report(report)

    if report.errors:
        print(f"\n[Validate] FAILED — {len(report.errors)} error(s), {len(report.warnings)} warning(s)")
        sys.exit(1)
    else:
        print(f"\n[Validate] PASSED — 0 errors, {len(report.warnings)} warning(s)")


def cmd_test(args):
    """Test a trained model on held-out intents."""
    import torch

    from soma_synthesizer.config import SynthesisConfig
    from soma_synthesizer.data import collect_training_data, build_catalog, expand_examples
    from soma_synthesizer.tokenizer import Tokenizer
    from soma_synthesizer.model import SomaMind
    from soma_synthesizer.trainer import run_test_suite

    # Load model
    model_dir = args.model
    meta_path = os.path.join(model_dir, "meta.json")
    with open(meta_path) as f:
        meta = json.load(f)

    config = SynthesisConfig()
    model = SomaMind(
        vocab_size=meta["vocab_size"],
        num_conventions=meta["num_conventions"],
        embed_dim=meta.get("embed_dim", config.architecture.embed_dim),
        hidden_dim=meta.get("hidden_dim", config.architecture.hidden_dim),
        decoder_dim=meta.get("decoder_dim", config.architecture.decoder_dim),
        opcode_embed_dim=meta.get("opcode_embed_dim", config.architecture.opcode_embed_dim),
        num_layers=meta.get("num_encoder_layers", config.architecture.num_encoder_layers),
        dropout=0.0,
        max_program_steps=meta.get("max_steps", config.architecture.max_program_steps),
    )
    model_path = os.path.join(model_dir, "soma_mind.pt")
    model.load_state_dict(torch.load(model_path, map_location="cpu", weights_only=True))
    model.set_to_inference_mode()
    print(f"[Test] Loaded model from {model_path}")

    # Load tokenizer
    tokenizer = Tokenizer()
    tokenizer.load(os.path.join(model_dir, "vocab.json"))

    # Load catalog
    with open(os.path.join(model_dir, "catalog.json")) as f:
        catalog = json.load(f)

    # Collect test data from plugins
    plugin_dirs = _discover_plugin_dirs(args.plugins)
    examples = collect_training_data(plugin_dirs)
    pairs = expand_examples(examples, catalog)
    print(f"[Test] Test pairs: {len(pairs)}")

    # Run test suite
    results = run_test_suite(model, pairs, tokenizer, catalog, config)

    print("\n[Test] Results:")
    print(f"  Op Accuracy:       {results['op_accuracy']:.3f}")
    print(f"  Program Exact:     {results['program_exact_match']:.3f}")
    print(f"  Span Accuracy:     {results.get('span_accuracy', 0.0):.3f}")
    print(f"  Ref Accuracy:      {results.get('ref_accuracy', 0.0):.3f}")
    print(f"  End-to-End:        {results['end_to_end_accuracy']:.3f}")


def cmd_benchmark(args):
    """Benchmark inference speed of a trained model."""
    import torch

    from soma_synthesizer.config import SynthesisConfig
    from soma_synthesizer.tokenizer import Tokenizer
    from soma_synthesizer.model import SomaMind

    # Load model
    model_dir = args.model
    meta_path = os.path.join(model_dir, "meta.json")
    with open(meta_path) as f:
        meta = json.load(f)

    config = SynthesisConfig()
    model = SomaMind(
        vocab_size=meta["vocab_size"],
        num_conventions=meta["num_conventions"],
        embed_dim=meta.get("embed_dim", config.architecture.embed_dim),
        hidden_dim=meta.get("hidden_dim", config.architecture.hidden_dim),
        decoder_dim=meta.get("decoder_dim", config.architecture.decoder_dim),
        opcode_embed_dim=meta.get("opcode_embed_dim", config.architecture.opcode_embed_dim),
        num_layers=meta.get("num_encoder_layers", config.architecture.num_encoder_layers),
        dropout=0.0,
        max_program_steps=meta.get("max_steps", config.architecture.max_program_steps),
    )
    model_path = os.path.join(model_dir, "soma_mind.pt")
    model.load_state_dict(torch.load(model_path, map_location="cpu", weights_only=True))
    model.set_to_inference_mode()

    # Load tokenizer
    tokenizer = Tokenizer()
    tokenizer.load(os.path.join(model_dir, "vocab.json"))

    # Benchmark intents of varying lengths
    test_intents = [
        "list files",
        "read the file hello.txt",
        "create a file called test.txt with content hello world",
        "copy report.txt to backup.txt and delete the original",
        "find all providers within 10km who offer plumbing services and are available next Thursday",
    ]

    iterations = args.iterations
    max_steps = meta.get("max_steps", config.architecture.max_program_steps)
    total_params = sum(p.numel() for p in model.parameters())

    print(f"[Benchmark] Model: {total_params:,} parameters")
    print(f"[Benchmark] Max program steps: {max_steps}")
    print(f"[Benchmark] Iterations per intent: {iterations}")
    print()

    overall_times = []

    with torch.no_grad():
        for intent in test_intents:
            token_ids = tokenizer.encode(intent)
            input_ids = torch.tensor([token_ids], dtype=torch.long)
            lengths = torch.tensor([len(token_ids)], dtype=torch.long)

            # Warmup
            for _ in range(5):
                model.encode(input_ids, lengths)

            # Benchmark encode
            t0 = time.perf_counter()
            for _ in range(iterations):
                encoder_out, enc_mask, pooled = model.encode(input_ids, lengths)
            encode_elapsed = time.perf_counter() - t0
            encode_avg_us = (encode_elapsed / iterations) * 1_000_000

            # Benchmark full forward pass (encode + decode all steps)
            # Use a dummy target_opcodes for teacher-forced timing
            dummy_ops = torch.zeros(1, max_steps, dtype=torch.long)
            t0 = time.perf_counter()
            for _ in range(iterations):
                model(input_ids, lengths, dummy_ops)
            full_elapsed = time.perf_counter() - t0
            full_avg_us = (full_elapsed / iterations) * 1_000_000

            overall_times.append(full_avg_us)

            intent_display = intent[:50] + ("..." if len(intent) > 50 else "")
            print(f"  \"{intent_display}\"  ({len(token_ids)} tokens)")
            print(f"    Encode:  {encode_avg_us:8.1f} us")
            print(f"    Full:    {full_avg_us:8.1f} us")
            print(f"    Decode:  {full_avg_us - encode_avg_us:8.1f} us")
            print()

    avg_us = sum(overall_times) / len(overall_times)
    print(f"[Benchmark] Average full inference: {avg_us:.1f} us ({avg_us / 1000:.2f} ms)")
    print(f"[Benchmark] Throughput: {1_000_000 / avg_us:.0f} inferences/sec")


def cmd_export_experience(args):
    """Export successful experiences from a SOMA checkpoint as training data (Sec 9.3).

    Reads the checkpoint's decisions and execution history,
    extracts successful (intent, program) pairs, and writes them
    as training JSON compatible with the synthesizer's collect_training_data format.
    """
    import json

    # Read SOMA checkpoint (same format as soma-core checkpoint.rs)
    with open(args.checkpoint, "rb") as f:
        magic = f.read(4)
        if magic != b"SOMA":
            print(f"Error: not a SOMA checkpoint (magic: {magic})")
            return
        version = int.from_bytes(f.read(4), "little")
        body = json.loads(f.read())

    # Extract successful execution records
    executions = body.get("recent_executions", [])
    decisions = body.get("decisions", [])

    successful = [e for e in executions if e.get("success", False)]

    # Convert to training format
    examples = []
    for i, ex in enumerate(successful):
        intent = ex.get("intent", "")
        if not intent:
            continue
        examples.append({
            "id": f"exp_{i:04d}",
            "intents": [intent],
            "program": [],  # program steps not stored in execution history (only summary)
            "source": "experience",
            "confidence": ex.get("confidence", 0.0),
            "execution_time_ms": ex.get("execution_time_ms", 0),
        })

    output = {
        "schema_version": "1.0",
        "plugin": "_experience",
        "source_checkpoint": args.checkpoint,
        "soma_id": body.get("soma_id", "unknown"),
        "examples": examples,
    }

    with open(args.output, "w") as f:
        json.dump(output, f, indent=2)

    print(f"[Experience] Exported {len(examples)} successful executions to {args.output}")
    print(f"  Source: {args.checkpoint}")
    print(f"  Total executions: {len(executions)}, successful: {len(successful)}")
    if decisions:
        print(f"  Decisions in checkpoint: {len(decisions)}")


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _load_config_or_default(config_path: str):
    """Load config from a TOML file, or return defaults if file doesn't exist."""
    from soma_synthesizer.config import load_config, SynthesisConfig

    if os.path.isfile(config_path):
        print(f"[Config] Loading {config_path}")
        return load_config(config_path)
    else:
        print(f"[Config] No config file at {config_path}, using defaults")
        return SynthesisConfig()


def _discover_plugin_dirs(plugins_path: str) -> list[str]:
    """Find all plugin directories under the given path.

    Each subdirectory that contains a manifest.toml is treated as a
    plugin directory.
    """
    if not os.path.isdir(plugins_path):
        print(f"[Error] Plugin directory not found: {plugins_path}")
        sys.exit(1)

    plugin_dirs = []
    for entry in sorted(os.listdir(plugins_path)):
        candidate = os.path.join(plugins_path, entry)
        if os.path.isdir(candidate):
            manifest = os.path.join(candidate, "manifest.toml")
            if os.path.isfile(manifest):
                plugin_dirs.append(candidate)

    if not plugin_dirs:
        print(f"[Warning] No plugins found in {plugins_path} "
              "(looking for subdirs with manifest.toml)")

    return plugin_dirs


def _find_plugin_dir(base_path: str, plugin_name: str) -> str | None:
    """Locate a plugin directory by name under base_path."""
    if not os.path.isdir(base_path):
        return None

    # Direct match
    candidate = os.path.join(base_path, plugin_name)
    if os.path.isdir(candidate) and os.path.isfile(os.path.join(candidate, "manifest.toml")):
        return candidate

    # Search under plugins/ subdirectory
    plugins_dir = os.path.join(base_path, "plugins")
    if os.path.isdir(plugins_dir):
        candidate = os.path.join(plugins_dir, plugin_name)
        if os.path.isdir(candidate) and os.path.isfile(os.path.join(candidate, "manifest.toml")):
            return candidate

    return None


def _print_validation_report(report) -> None:
    """Pretty-print validation report to stdout."""
    for warning in report.warnings:
        print(f"  [WARN] {warning}")

    for error in report.errors:
        print(f"  [ERROR] {error}")


def _list_dir_sizes(directory: str) -> None:
    """Print file names and sizes for all files in a directory."""
    for name in sorted(os.listdir(directory)):
        path = os.path.join(directory, name)
        if os.path.isfile(path):
            size = os.path.getsize(path)
            if size >= 1024 * 1024:
                print(f"  -> {name} ({size / (1024 * 1024):.1f} MB)")
            else:
                print(f"  -> {name} ({size / 1024:.0f} KB)")


# ---------------------------------------------------------------------------
# Main entry point
# ---------------------------------------------------------------------------


def main():
    parser = argparse.ArgumentParser(
        prog="soma-synthesize",
        description="SOMA Synthesizer — train and export Mind models",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    # train
    p_train = subparsers.add_parser(
        "train", help="Train a new Mind from plugin training data",
    )
    p_train.add_argument("--config", default="synthesis_config.toml")
    p_train.add_argument("--plugins", required=True, help="Plugin directory")
    p_train.add_argument("--domain", help="Domain-specific training data")
    p_train.add_argument("--output", default="./models", help="Output directory")
    p_train.add_argument(
        "--target", choices=["server", "embedded", "both"], default="server",
    )

    # train-lora
    p_lora = subparsers.add_parser(
        "train-lora", help="Train LoRA for a specific plugin",
    )
    p_lora.add_argument("--plugin", required=True)
    p_lora.add_argument("--base-model", required=True)
    p_lora.add_argument("--output", default="./lora")
    p_lora.add_argument("--config", default="synthesis_config.toml")

    # export
    p_export = subparsers.add_parser(
        "export", help="Export model to ONNX / .soma-model",
    )
    p_export.add_argument("--model", required=True)
    p_export.add_argument("--output", default="./models")
    p_export.add_argument(
        "--target", choices=["server", "embedded", "both"], default="server",
    )
    p_export.add_argument("--embedded-ram", type=int, default=256000)
    p_export.add_argument("--embedded-flash", type=int, default=4000000)

    # validate
    p_val = subparsers.add_parser(
        "validate", help="Validate plugin training data",
    )
    p_val.add_argument("--plugins", required=True)
    p_val.add_argument("--domain", help="Domain training data")

    # test
    p_test = subparsers.add_parser(
        "test", help="Test model on held-out intents",
    )
    p_test.add_argument("--model", required=True)
    p_test.add_argument("--plugins", required=True)

    # benchmark
    p_bench = subparsers.add_parser(
        "benchmark", help="Benchmark inference speed",
    )
    p_bench.add_argument("--model", required=True)
    p_bench.add_argument("--iterations", type=int, default=100)

    # export-experience
    p_exp = subparsers.add_parser(
        "export-experience",
        help="Export experience from SOMA checkpoint for re-synthesis",
    )
    p_exp.add_argument(
        "--checkpoint", required=True, help="SOMA checkpoint file path",
    )
    p_exp.add_argument(
        "--output", default="./experience.json",
        help="Output training data JSON",
    )

    args = parser.parse_args()

    commands = {
        "train": cmd_train,
        "train-lora": cmd_train_lora,
        "export": cmd_export,
        "validate": cmd_validate,
        "test": cmd_test,
        "benchmark": cmd_benchmark,
        "export-experience": cmd_export_experience,
    }
    commands[args.command](args)


if __name__ == "__main__":
    main()
