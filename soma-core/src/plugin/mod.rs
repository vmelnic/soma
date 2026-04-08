//! Plugin system -- loads, routes, and manages SOMA plugins.
//!
//! Plugins expose named conventions (operations) that the Mind's generated programs invoke.
//! The [`manager::PluginManager`] routes convention calls by global ID, handles crash recovery,
//! timeouts, and topological dependency ordering. The built-in [`builtin::PosixPlugin`] provides
//! POSIX filesystem conventions. Dynamic loading via [`dynamic`] supports cdylib plugins with
//! Ed25519 signature verification.

pub mod interface;
pub mod manager;
pub mod builtin;
pub mod dynamic;
pub mod process;
