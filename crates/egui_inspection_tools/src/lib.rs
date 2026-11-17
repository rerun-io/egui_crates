//! Agent tools that drive a live egui app over the `egui_inspection` protocol.
//!
//! This crate holds the tool logic with no MCP and no async runtime, so it can run in-process
//! and on `wasm32`. `egui_mcp` serves these same tools over MCP.

pub mod tree;

#[cfg(test)]
mod real_app_tests;
