//! Three-tier memory system for runtime learning and persistence.
//!
//! - **Experience** — Ring buffer of successful inference outcomes, feeds `LoRA` adaptation.
//! - **Consolidation** — Merges high-magnitude `LoRA` deltas into permanent weight offsets.
//! - **Checkpoint** — Serializes full learned state (`LoRA`, experiences, plugin state) to disk.
//!
//! Implements Spec Sections 6 (Memory Architecture) and 17 (Experience Tracking).

pub mod experience;
pub mod checkpoint;
pub mod consolidation;
