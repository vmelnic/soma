//! Memory system — experience tracking, checkpointing, and consolidation
//! (Spec Sections 6 + 17).
//!
// Future: Diffuse memory tier — synaptic queries to peer SOMAs (Section 6.1).
// When local inference fails or confidence is low, query peers for knowledge.

pub mod experience;
pub mod checkpoint;
pub mod consolidation;
