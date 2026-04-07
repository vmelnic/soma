//! MCP (Model Context Protocol) server — the bridge between LLMs and SOMA
//! (Whitepaper Section 8, Milestone 3).
//!
//! JSON-RPC 2.0 over stdio. Exposes state tools (query what exists) and
//! action tools (do things). At this point, an LLM can drive SOMA.

pub mod auth;
pub mod server;
pub mod tools;
