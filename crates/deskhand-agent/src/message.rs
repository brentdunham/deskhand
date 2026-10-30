//! Provider-neutral conversation and streaming types.

use deskhand_tools::ToolDefinition;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub role: Role,
    pub content: Vec<Block>,
}

impl Message {
    pub fn user_text(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![Block::Text(text.into())],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Text(String),
    /// Only in user messages.
    Image(ImageData),
    ToolUse(ToolCall),
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// An image sent to the model.
#[derive(Clone, PartialEq, Eq)]
pub struct ImageData {
    /// e.g. `image/png`.
    pub media_type: String,
    /// Base64 without a `data:` prefix.
    pub base64: String,
}

// Base64 payloads are large; keep them out of debug output.
impl std::fmt::Debug for ImageData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageData")
            .field("media_type", &self.media_type)
            .field("base64_len", &self.base64.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Other(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Normalized streaming output. Adapters assemble tool calls internally and
/// emit each one complete, after its arguments have fully arrived.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    TextDelta(String),
    ToolUse(ToolCall),
    Done {
        stop: StopReason,
        usage: Option<Usage>,
    },
}
