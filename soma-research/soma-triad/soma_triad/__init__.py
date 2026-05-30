"""soma-triad: LTC + HDC + SDM non-transformer reasoning architecture."""

from soma_triad.hdc import HDC
from soma_triad.sdm import SDM
from soma_triad.controller import Controller
from soma_triad.triad import Triad

__all__ = ["HDC", "SDM", "Controller", "Triad"]
