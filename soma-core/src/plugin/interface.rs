//! Plugin trait — the contract every SOMA plugin implements (Whitepaper Section 6).
//!
//! All types are defined in the `soma-plugin-sdk` crate and re-exported here.
//! This allows external plugins to depend only on the lightweight SDK crate
//! instead of all of soma-core.

// Re-export everything from the SDK
pub use soma_plugin_sdk::*;
