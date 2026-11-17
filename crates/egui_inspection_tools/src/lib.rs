//! Agent tools that drive a live egui app over the [`egui_inspection`] protocol.
//!
//! The tools read the app's `AccessKit` tree (`widget_tree`, `get_widget`, `wait_for`), take a
//! `screenshot`, and synthesize real input (`click`, `hover`, `scroll`, `drag`, `type_text`,
//! `press_key`, `resize`, `batch`).
//!
//! There is no MCP and no async runtime here, so the tools can run in-process, e.g. for an
//! agent built into the app itself, and on `wasm32`. The host supplies two things:
//!
//! - a [`Transport`] that carries one [`egui_inspection::protocol::Request`] to the app and
//!   returns its response, and
//! - a [`Sleep`], the host's timer, which `wait_for` polls with.
//!
//! [`tools()`] lists every tool with its JSON Schema; [`call`] runs one by name.
//! `egui_mcp` serves these same tools over MCP.

mod args;
mod output;
mod tools;
pub mod transport;
pub mod tree;

#[cfg(test)]
mod dispatch_tests;
#[cfg(test)]
mod real_app_tests;

pub use args::*;
pub use output::{Content, ToolOutput};
pub use tools::{
    Effect, JsonObject, ToolDef, call, check_arguments, input_schema, run_batch, tool, tools,
};
pub use transport::{BoxFuture, MaybeSend, MaybeSync, Sleep, Transport, TreeSnapshot};
