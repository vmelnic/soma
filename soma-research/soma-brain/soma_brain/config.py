from dataclasses import dataclass


@dataclass
class BrainConfig:
    embed_dim: int = 768
    hidden_size: int = 1024
    num_layers: int = 8
    ode_steps: int = 4

    sdm_top_k: int = 8
    num_attn_heads: int = 4

    ttt_inner_size: int | None = None
    ttt_lr: float = 0.01

    max_skills: int = 512
    binding_dim: int = 256

    decoder_hidden: int = 512
    decoder_layers: int = 6
    decoder_heads: int = 8
    decoder_max_seq: int = 256
    decoder_vocab_size: int | None = None

    def __post_init__(self):
        if self.ttt_inner_size is None:
            self.ttt_inner_size = self.hidden_size // 2

    @classmethod
    def tiny(cls):
        """For unit tests."""
        return cls(
            embed_dim=128,
            hidden_size=256,
            num_layers=4,
            ode_steps=2,
            sdm_top_k=4,
            num_attn_heads=4,
            ttt_inner_size=128,
            decoder_hidden=128,
            decoder_layers=2,
            decoder_heads=4,
            decoder_max_seq=64,
        )

    @classmethod
    def medium(cls):
        """Runs on Apple Silicon / single consumer GPU."""
        return cls(
            embed_dim=1024,
            hidden_size=1024,
            num_layers=4,
            ode_steps=2,
            sdm_top_k=8,
            num_attn_heads=8,
            decoder_vocab_size=256,
            decoder_hidden=512,
            decoder_layers=4,
            decoder_heads=8,
            decoder_max_seq=128,
        )

    @classmethod
    def small(cls):
        """Single consumer GPU."""
        return cls()

    @classmethod
    def base(cls):
        return cls(
            embed_dim=768,
            hidden_size=2048,
            num_layers=12,
            ode_steps=6,
        )

    @classmethod
    def large(cls):
        """Target architecture."""
        return cls(
            embed_dim=768,
            hidden_size=4096,
            num_layers=24,
            ode_steps=8,
            sdm_top_k=16,
            num_attn_heads=16,
        )
