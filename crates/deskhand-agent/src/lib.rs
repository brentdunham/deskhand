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
    DEFAULT_SYSTEM_PROMPT, ScreenContext, Session,
};
pub use message::{
    Block, ChatRequest, ImageData, Message, Role, StopReason, StreamEvent, ToolCall, Usage,
};
pub use provider::{EventStream, Provider, ProviderError};

/// The HTTP client type provider adapters take.
pub use reqwest::Client as HttpClient;

/// An HTTP client configured for model providers: identifies DeskHand and
/// fails fast on unreachable endpoints. There is no overall request timeout,
/// because streamed responses can legitimately run for minutes.
pub fn http_client() -> reqwest::Result<HttpClient> {
    reqwest::Client::builder()
        .user_agent(concat!("DeskHand/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
}
