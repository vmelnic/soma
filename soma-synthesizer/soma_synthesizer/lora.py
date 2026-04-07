"""
SOMA LoRA — Low-Rank Adaptation training for per-plugin knowledge.

Part of the Synthesizer (spec Section 6). Trains lightweight LoRA
adapters on top of a frozen base Mind, allowing per-plugin
specialisation without retraining the full model.

Each plugin gets its own LoRA weights that can be loaded/unloaded at
runtime. LoRA metadata is exported as a ``.lora.json`` sidecar file.

Self-contained — no imports from poc/ or pow/.
"""

import json
import os
import time

import torch
import torch.nn as nn
import torch.nn.functional as F


# -----------------------------------------------------------------------
# LoRA linear layer
# -----------------------------------------------------------------------

class LoRALinear(nn.Module):
    """Low-Rank Adaptation layer wrapping an existing ``nn.Linear``.

    Freezes the base weight and bias, then adds a low-rank decomposition
    ``B @ A`` scaled by ``alpha / rank``.  During forward, the output is
    ``base(x) + (x @ A^T) @ B^T * scale``.

    Args:
        base: The ``nn.Linear`` layer to adapt.
        rank: Rank of the low-rank decomposition.
        alpha: Scaling factor (effective scale = alpha / rank).
    """

    def __init__(self, base: nn.Linear, rank: int = 8, alpha: float = 2.0):
        super().__init__()
        self.base = base
        self.base.weight.requires_grad_(False)
        if self.base.bias is not None:
            self.base.bias.requires_grad_(False)

        in_f = base.in_features
        out_f = base.out_features
        self.rank = rank
        self.alpha = alpha

        # A: (rank, in_features) — initialised with small random values
        self.A = nn.Parameter(torch.randn(rank, in_f) * 0.01)
        # B: (out_features, rank) — initialised to zero so LoRA starts
        # as identity (no change to base output)
        self.B = nn.Parameter(torch.zeros(out_f, rank))
        self.scale = alpha / rank

    def forward(self, x):
        base_out = self.base(x)
        lora_out = (x @ self.A.T) @ self.B.T * self.scale
        return base_out + lora_out

    def merge(self):
        """Merge LoRA weights into the base layer permanently.

        After merging, this layer behaves identically to a plain
        ``nn.Linear`` with updated weights.  This is useful for
        exporting a merged model without LoRA overhead.
        """
        with torch.no_grad():
            self.base.weight.add_(
                (self.B @ self.A) * self.scale
            )
        self.A.data.zero_()
        self.B.data.zero_()

    @property
    def lora_params(self):
        """Return LoRA parameters (A and B) for optimiser construction."""
        return [self.A, self.B]


# -----------------------------------------------------------------------
# Default target modules for the BiLSTM+GRU Mind architecture
# -----------------------------------------------------------------------

DEFAULT_TARGET_MODULES = [
    "init_h",
    "ctx_q",
    "op_head",
    "a0t_head",
    "a1t_head",
    "s0s_q",
    "s0e_q",
    "s1s_q",
    "s1e_q",
    "r0q",
    "r0k",
    "r1q",
    "r1k",
]


# -----------------------------------------------------------------------
# Apply / remove LoRA
# -----------------------------------------------------------------------

def apply_lora(model, rank=8, alpha=2.0, target_modules=None):
    """Freeze the base model and apply LoRA to target Linear layers.

    Walks the model's named modules and replaces each ``nn.Linear``
    whose name matches one of *target_modules* with a ``LoRALinear``
    wrapper.  All non-LoRA parameters are frozen.

    Args:
        model: The SomaMind (or any nn.Module) to adapt.
        rank: LoRA rank.
        alpha: LoRA scaling factor.
        target_modules: List of module name suffixes to target.
            Defaults to ``DEFAULT_TARGET_MODULES``.

    Returns:
        Dict mapping module path -> LoRALinear instance, for use
        with ``save_lora`` and ``get_lora_params``.
    """
    if target_modules is None:
        target_modules = DEFAULT_TARGET_MODULES

    # Freeze all base parameters
    for param in model.parameters():
        param.requires_grad_(False)

    # Replace target Linear layers with LoRALinear wrappers
    lora_layers = {}
    for name, module in list(model.named_modules()):
        # Check if this module's name ends with one of the targets
        short_name = name.split(".")[-1] if "." in name else name
        if short_name in target_modules and isinstance(module, nn.Linear):
            lora = LoRALinear(module, rank=rank, alpha=alpha)
            # Replace the module in the parent
            parts = name.split(".")
            parent = model
            for part in parts[:-1]:
                parent = getattr(parent, part)
            setattr(parent, parts[-1], lora)
            lora_layers[name] = lora

    return lora_layers


def remove_lora(model, lora_layers):
    """Remove LoRA wrappers and restore the original Linear layers.

    Args:
        model: The model with LoRA layers applied.
        lora_layers: Dict from ``apply_lora``.
    """
    for name, lora in lora_layers.items():
        parts = name.split(".")
        parent = model
        for part in parts[:-1]:
            parent = getattr(parent, part)
        setattr(parent, parts[-1], lora.base)


def get_lora_params(lora_layers):
    """Collect all trainable LoRA parameters for optimiser construction.

    Args:
        lora_layers: Dict from ``apply_lora``.

    Returns:
        List of nn.Parameter objects (A and B from each LoRALinear).
    """
    params = []
    for lora in lora_layers.values():
        params.extend(lora.lora_params)
    return params


# -----------------------------------------------------------------------
# Save / load LoRA weights
# -----------------------------------------------------------------------

def save_lora(model, path, metadata):
    """Save LoRA weights and metadata to disk.

    Writes two files:
      - ``<path>`` — PyTorch state dict containing only LoRA A/B weights.
      - ``<path>.json`` — LoRA metadata sidecar (plugin info, architecture,
        training stats).

    Args:
        model: The model with LoRA layers applied (or a dict of
            LoRALinear layers from ``apply_lora``).
        path: Output file path (e.g. ``lora/postgres.lora``).
        metadata: Dict with LoRA metadata. Expected keys:
            plugin, mind_architecture, mind_version, hidden_dim,
            decoder_dim, target_layers, rank, alpha, training_stats.
    """
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)

    # Extract LoRA weights
    lora_state = {}
    if isinstance(model, dict):
        # model is lora_layers dict from apply_lora
        for name, lora in model.items():
            lora_state[f"{name}.A"] = lora.A.detach().cpu()
            lora_state[f"{name}.B"] = lora.B.detach().cpu()
    else:
        # Walk the model to find LoRALinear modules
        for name, module in model.named_modules():
            if isinstance(module, LoRALinear):
                lora_state[f"{name}.A"] = module.A.detach().cpu()
                lora_state[f"{name}.B"] = module.B.detach().cpu()

    torch.save(lora_state, path)

    # Write metadata sidecar
    json_path = path + ".json"
    with open(json_path, "w") as f:
        json.dump(metadata, f, indent=2)

    return path, json_path


def load_lora(model, path, strict=True):
    """Load LoRA weights from disk and apply them to the model.

    The model must already have LoRA layers applied (via ``apply_lora``)
    with matching layer names and dimensions.

    Args:
        model: The model with LoRA layers applied.
        path: Path to the ``.lora`` weights file.
        strict: If True, raise on missing/unexpected keys.

    Returns:
        LoRA metadata dict loaded from the sidecar JSON (or None
        if the sidecar does not exist).
    """
    lora_state = torch.load(path, map_location="cpu", weights_only=True)

    # Apply loaded weights to matching LoRALinear modules
    applied = set()
    for name, module in model.named_modules():
        if isinstance(module, LoRALinear):
            a_key = f"{name}.A"
            b_key = f"{name}.B"
            if a_key in lora_state:
                module.A.data.copy_(lora_state[a_key])
                applied.add(a_key)
            elif strict:
                raise KeyError(f"Missing LoRA weight: {a_key}")
            if b_key in lora_state:
                module.B.data.copy_(lora_state[b_key])
                applied.add(b_key)
            elif strict:
                raise KeyError(f"Missing LoRA weight: {b_key}")

    if strict:
        unexpected = set(lora_state.keys()) - applied
        if unexpected:
            raise KeyError(f"Unexpected LoRA weights: {unexpected}")

    # Load metadata sidecar if present
    json_path = path + ".json"
    metadata = None
    if os.path.exists(json_path):
        with open(json_path, "r") as f:
            metadata = json.load(f)

    return metadata


# -----------------------------------------------------------------------
# LoRA training
# -----------------------------------------------------------------------

def train_plugin_lora(base_model, plugin_data, config):
    """Train LoRA on a single plugin's training data.

    Freezes the base model, applies LoRA adapters, and trains only
    the LoRA parameters on the plugin's (intent, program) pairs.

    Args:
        base_model: Trained SomaMind instance (will be frozen).
        plugin_data: Dict with training data for one plugin::

            {
                "plugin_name": "postgres",
                "train_loader": DataLoader,  # training batches
                "val_loader": DataLoader,    # validation batches (optional)
                "loss_fn": callable,         # loss function(model_output, batch)
            }

        config: Dict with training configuration::

            {
                "rank": 8,
                "alpha": 2.0,
                "target_modules": [...],  # optional, defaults to all heads
                "epochs": 50,
                "lr": 1e-3,
                "weight_decay": 1e-2,
                "gradient_clip": 1.0,
                "output_dir": "lora/",
                "mind_version": "0.1.0",
                "hidden_dim": 128,
                "decoder_dim": 256,
            }

    Returns:
        Dict with training results::

            {
                "lora_path": str,
                "metadata_path": str,
                "best_val_loss": float,
                "best_epoch": int,
                "training_time": float,
            }
    """
    plugin_name = plugin_data["plugin_name"]
    train_loader = plugin_data["train_loader"]
    val_loader = plugin_data.get("val_loader")
    loss_fn = plugin_data["loss_fn"]

    rank = config.get("rank", 8)
    alpha = config.get("alpha", 2.0)
    target_modules = config.get("target_modules", None)
    epochs = config.get("epochs", 50)
    lr = config.get("lr", 1e-3)
    weight_decay = config.get("weight_decay", 1e-2)
    gradient_clip = config.get("gradient_clip", 1.0)
    output_dir = config.get("output_dir", "lora")

    # Apply LoRA
    lora_layers = apply_lora(
        base_model, rank=rank, alpha=alpha,
        target_modules=target_modules,
    )
    lora_params = get_lora_params(lora_layers)

    optimizer = torch.optim.AdamW(lora_params, lr=lr, weight_decay=weight_decay)
    scheduler = torch.optim.lr_scheduler.ReduceLROnPlateau(
        optimizer, mode="min", patience=5, factor=0.5,
    )

    best_val_loss = float("inf")
    best_epoch = 0
    best_state = None
    start_time = time.time()

    for epoch in range(1, epochs + 1):
        # --- Training ---
        base_model.train()
        train_loss_sum = 0.0
        train_batches = 0

        for batch in train_loader:
            optimizer.zero_grad()
            output = base_model(*batch[:-1])  # all inputs except targets
            loss = loss_fn(output, batch)
            loss.backward()

            if gradient_clip > 0:
                torch.nn.utils.clip_grad_norm_(lora_params, gradient_clip)

            optimizer.step()
            train_loss_sum += loss.item()
            train_batches += 1

        train_loss = train_loss_sum / max(train_batches, 1)

        # --- Validation ---
        val_loss = train_loss  # fallback if no val_loader
        if val_loader is not None:
            base_model.eval()
            val_loss_sum = 0.0
            val_batches = 0
            with torch.no_grad():
                for batch in val_loader:
                    output = base_model(*batch[:-1])
                    loss = loss_fn(output, batch)
                    val_loss_sum += loss.item()
                    val_batches += 1
            val_loss = val_loss_sum / max(val_batches, 1)

        scheduler.step(val_loss)

        if val_loss < best_val_loss:
            best_val_loss = val_loss
            best_epoch = epoch
            # Snapshot best LoRA state
            best_state = {}
            for name, lora in lora_layers.items():
                best_state[f"{name}.A"] = lora.A.detach().cpu().clone()
                best_state[f"{name}.B"] = lora.B.detach().cpu().clone()

    elapsed = time.time() - start_time

    # Restore best state
    if best_state is not None:
        for name, lora in lora_layers.items():
            lora.A.data.copy_(best_state[f"{name}.A"])
            lora.B.data.copy_(best_state[f"{name}.B"])

    # Build metadata
    target_names = list(lora_layers.keys())
    metadata = {
        "plugin": plugin_name,
        "mind_architecture": config.get("mind_architecture", "bilstm_gru"),
        "mind_version": config.get("mind_version", "0.1.0"),
        "hidden_dim": config.get("hidden_dim", 128),
        "decoder_dim": config.get("decoder_dim", 256),
        "target_layers": target_names,
        "rank": rank,
        "alpha": alpha,
        "training_stats": {
            "epochs": epochs,
            "best_epoch": best_epoch,
            "best_val_loss": best_val_loss,
            "training_time_seconds": elapsed,
        },
    }

    # Save
    os.makedirs(output_dir, exist_ok=True)
    lora_path = os.path.join(output_dir, f"{plugin_name}.lora")
    lora_path, meta_path = save_lora(lora_layers, lora_path, metadata)

    # Clean up — remove LoRA wrappers to restore base model
    remove_lora(base_model, lora_layers)

    # Unfreeze base model parameters (caller may want to continue using it)
    for param in base_model.parameters():
        param.requires_grad_(True)

    return {
        "lora_path": lora_path,
        "metadata_path": meta_path,
        "best_val_loss": best_val_loss,
        "best_epoch": best_epoch,
        "training_time": elapsed,
    }
