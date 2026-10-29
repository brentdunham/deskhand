//! Tool layer: the [`Toolbox`] registry and the built-in tools.
//!
//! The toolbox is the only path from a model's tool call to execution:
//! [`Toolbox::authorize`] asks the policy engine, [`Toolbox::execute`] runs a
//! permitted call and records it, and [`Toolbox::undo`] reverses it. Every
//! call, including denied and declined ones, lands in the action log.
//!
//! MCP exposure (server and client) is deferred to v0.2; tool definitions are
//! already shaped for it (name, description, JSON Schema input).

mod toolbox;
#[cfg(windows)]
pub mod window_tools;

pub use toolbox::{
    CallOutcome, Tool, ToolDefinition, ToolError, ToolOutput, Toolbox, ToolboxError,
};
