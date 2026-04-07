"""Synthesis configuration from TOML files.

Loads and merges TOML configuration with sensible defaults.
All architecture, training, augmentation, and LoRA parameters
are specified here per spec Section 4.3.

Self-contained — no imports from poc/ or pow/.
"""

from dataclasses import dataclass, field

try:
    import tomllib  # Python 3.11+ stdlib
except ModuleNotFoundError:
    import toml as _toml_compat  # fallback for Python 3.10


@dataclass
class ArchitectureConfig:
    # type: "bilstm_gru" (default) or "transformer" (future)
    type: str = "bilstm_gru"
    embed_dim: int = 64
    hidden_dim: int = 128
    decoder_dim: int = 256
    num_encoder_layers: int = 2
    num_decoder_layers: int = 1
    dropout: float = 0.3
    max_program_steps: int = 16
    opcode_embed_dim: int = 32


@dataclass
class TrainingConfig:
    epochs: int = 200
    batch_size: int = 32
    learning_rate: float = 1e-3
    weight_decay: float = 1e-2
    patience: int = 30
    scheduler_patience: int = 10
    scheduler_factor: float = 0.5
    gradient_clip: float = 1.0
    train_split: float = 0.8
    val_split: float = 0.1
    test_split: float = 0.1


@dataclass
class AugmentationConfig:
    enabled: bool = True
    synonym_replace_rate: float = 0.3
    word_dropout_rate: float = 0.2
    word_shuffle_rate: float = 0.1
    typo_rate: float = 0.05
    augmentation_factor: int = 3


@dataclass
class LoRAConfig:
    rank: int = 8
    alpha: float = 2.0
    epochs: int = 40
    learning_rate: float = 2e-3
    target_modules: list = field(
        default_factory=lambda: ["op_head", "gru", "a0t_head", "a1t_head"]
    )


@dataclass
class SynthesisConfig:
    architecture: ArchitectureConfig = field(default_factory=ArchitectureConfig)
    training: TrainingConfig = field(default_factory=TrainingConfig)
    augmentation: AugmentationConfig = field(default_factory=AugmentationConfig)
    lora: LoRAConfig = field(default_factory=LoRAConfig)
    version: str = "0.1.0"


def _merge_section(dc_instance, overrides: dict) -> None:
    """Merge a dict of overrides into a dataclass instance in-place.

    Only sets attributes that already exist on the dataclass.
    Ignores unknown keys so forward-compatible TOML files don't
    break older synthesizer versions.
    """
    for key, value in overrides.items():
        if hasattr(dc_instance, key):
            setattr(dc_instance, key, value)


def load_config(path: str) -> SynthesisConfig:
    """Load config from TOML file, merge with defaults.

    Any section or key missing from the TOML file falls back to the
    default value defined in the dataclass.  Extra keys in the TOML
    are silently ignored for forward compatibility.

    TOML sections map 1:1 to config dataclasses::

        [architecture]   -> ArchitectureConfig
        [training]       -> TrainingConfig
        [augmentation]   -> AugmentationConfig
        [lora]           -> LoRAConfig

    A top-level ``version`` key is also recognised.
    """
    try:
        # Python 3.11+ stdlib
        with open(path, "rb") as f:
            raw = tomllib.load(f)
    except NameError:
        # Fallback for Python 3.10
        raw = _toml_compat.load(path)

    config = SynthesisConfig()

    if "architecture" in raw:
        _merge_section(config.architecture, raw["architecture"])

    if "training" in raw:
        _merge_section(config.training, raw["training"])

    if "augmentation" in raw:
        _merge_section(config.augmentation, raw["augmentation"])

    if "lora" in raw:
        _merge_section(config.lora, raw["lora"])

    if "version" in raw:
        config.version = raw["version"]

    return config
