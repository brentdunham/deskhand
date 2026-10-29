//! Agent runtime: provider adapters (Anthropic Messages API, OpenAI-compatible)
//! and the tool-use loop over [`deskhand_tools::Toolbox`].
//!
//! This crate must not depend on Tauri, so the agent can later run in a
//! headless daemon. The UI connects through [`AgentHost`].

mod agent;
pub mod anthropic;
mod message;
pub mod openai;
mod provider;
mod sse;

pub use agent::{
    Agent, AgentConfig, AgentError, AgentEvent, AgentHost, ConfirmationRequest,
    DEFAULT_SYSTEM_PROMPT, Session,
};
pub use message::{Block, ChatRequest, Message, Role, StopReason, StreamEvent, ToolCall, Usage};
pub use provider::{EventStream, Provider, ProviderError};
