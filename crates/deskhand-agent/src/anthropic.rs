//! Anthropic Messages API adapter with streaming.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::message::{Block, ChatRequest, Message, Role, StopReason, StreamEvent, ToolCall, Usage};
use crate::provider::{
    Decoder, EventStream, Provider, ProviderError, parse_tool_input, sse_stream,
};
use crate::sse::SseEvent;

pub const DEFAULT_MODEL: &str = "claude-sonnet-5-5";
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";

#[derive(Clone)]
pub struct AnthropicConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub max_tokens: u32,
}

impl AnthropicConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: DEFAULT_MODEL.into(),
            base_url: DEFAULT_BASE_URL.into(),
            max_tokens: 4096,
        }
    }
}

// Never print the key.
impl std::fmt::Debug for AnthropicConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicConfig")
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("max_tokens", &self.max_tokens)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct Anthropic {
    http: reqwest::Client,
    config: AnthropicConfig,
}

impl Anthropic {
    pub fn new(http: reqwest::Client, config: AnthropicConfig) -> Self {
        Self { http, config }
    }
}

impl Provider for Anthropic {
    fn stream(&self, request: &ChatRequest) -> EventStream<'_> {
        let url = format!("{}/v1/messages", self.config.base_url.trim_end_matches('/'));
        let builder = self
            .http
            .post(url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", API_VERSION)
            .json(&request_body(&self.config, request));
        sse_stream(builder, AnthropicDecoder::default())
    }
}

pub(crate) fn request_body(config: &AnthropicConfig, request: &ChatRequest) -> Value {
    let mut body = json!({
        "model": config.model,
        "max_tokens": config.max_tokens,
        "system": request.system,
        "stream": true,
        "messages": request.messages.iter().map(message_json).collect::<Vec<_>>(),
    });
    if !request.tools.is_empty() {
        body["tools"] = json!(request.tools);
    }
    body
}

fn message_json(m: &Message) -> Value {
    let role = match m.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    let content: Vec<Value> = m
        .content
        .iter()
        .map(|b| match b {
            Block::Text(t) => json!({ "type": "text", "text": t }),
            Block::Image(img) => json!({
                "type": "image",
                "source": { "type": "base64", "media_type": img.media_type, "data": img.base64 },
            }),
            Block::ToolUse(c) => {
                json!({ "type": "tool_use", "id": c.id, "name": c.name, "input": c.input })
            }
            Block::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": content,
                "is_error": is_error,
            }),
        })
        .collect();
    json!({ "role": role, "content": content })
}

enum Partial {
    Text,
    Tool {
        id: String,
        name: String,
        json: String,
    },
}

#[derive(Default)]
pub(crate) struct AnthropicDecoder {
    blocks: HashMap<u64, Partial>,
    stop: Option<StopReason>,
    usage: Usage,
    done: bool,
}

fn index(v: &Value) -> Result<u64, ProviderError> {
    v["index"]
        .as_u64()
        .ok_or_else(|| ProviderError::Protocol("content block event without index".into()))
}

fn stop_reason(s: &str) -> StopReason {
    match s {
        "end_turn" | "stop_sequence" => StopReason::EndTurn,
        "tool_use" => StopReason::ToolUse,
        "max_tokens" => StopReason::MaxTokens,
        other => StopReason::Other(other.to_owned()),
    }
}

impl Decoder for AnthropicDecoder {
    fn decode(&mut self, event: SseEvent) -> Result<Vec<StreamEvent>, ProviderError> {
        let v: Value = serde_json::from_str(&event.data)
            .map_err(|e| ProviderError::Protocol(format!("invalid event JSON: {e}")))?;
        let mut out = Vec::new();
        match v["type"].as_str().unwrap_or_default() {
            "message_start" => {
                if let Some(n) = v["message"]["usage"]["input_tokens"].as_u64() {
                    self.usage.input_tokens = n;
                }
            }
            "content_block_start" => {
                let i = index(&v)?;
                let block = &v["content_block"];
                match block["type"].as_str() {
                    Some("tool_use") => {
                        self.blocks.insert(
                            i,
                            Partial::Tool {
                                id: block["id"].as_str().unwrap_or_default().to_owned(),
                                name: block["name"].as_str().unwrap_or_default().to_owned(),
                                json: String::new(),
                            },
                        );
                    }
                    Some("text") => {
                        if let Some(t) = block["text"].as_str().filter(|t| !t.is_empty()) {
                            out.push(StreamEvent::TextDelta(t.to_owned()));
                        }
                        self.blocks.insert(i, Partial::Text);
                    }
                    // Other block types (e.g. thinking) are not surfaced.
                    _ => {}
                }
            }
            "content_block_delta" => {
                let i = index(&v)?;
                let delta = &v["delta"];
                match (delta["type"].as_str(), self.blocks.get_mut(&i)) {
                    (Some("text_delta"), _) => {
                        if let Some(t) = delta["text"].as_str() {
                            out.push(StreamEvent::TextDelta(t.to_owned()));
                        }
                    }
                    (Some("input_json_delta"), Some(Partial::Tool { json, .. })) => {
                        json.push_str(delta["partial_json"].as_str().unwrap_or_default());
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                if let Some(Partial::Tool { id, name, json }) = self.blocks.remove(&index(&v)?) {
                    out.push(StreamEvent::ToolUse(ToolCall {
                        id,
                        name,
                        input: parse_tool_input(&json)?,
                    }));
                }
            }
            "message_delta" => {
                if let Some(s) = v["delta"]["stop_reason"].as_str() {
                    self.stop = Some(stop_reason(s));
                }
                if let Some(n) = v["usage"]["output_tokens"].as_u64() {
                    self.usage.output_tokens = n;
                }
            }
            "message_stop" => {
                self.done = true;
                out.push(StreamEvent::Done {
                    stop: self.stop.take().unwrap_or(StopReason::EndTurn),
                    usage: Some(self.usage),
                });
            }
            "error" => {
                return Err(ProviderError::Api {
                    kind: v["error"]["type"].as_str().unwrap_or("error").to_owned(),
                    message: v["error"]["message"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                });
            }
            _ => {}
        }
        Ok(out)
    }

    fn finish(&mut self) -> Result<Vec<StreamEvent>, ProviderError> {
        if self.done {
            Ok(Vec::new())
        } else {
            Err(ProviderError::Protocol(
                "stream ended before message_stop".into(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sse::SseParser;

    fn decode_all(raw: &str) -> Result<Vec<StreamEvent>, ProviderError> {
        let mut parser = SseParser::new();
        let mut d = AnthropicDecoder::default();
        let mut out = Vec::new();
        for e in parser.push(raw.as_bytes()) {
            out.extend(d.decode(e)?);
        }
        out.extend(d.finish()?);
        Ok(out)
    }

    const TOOL_STREAM: &str = "\
event: message_start
data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":120}}}

event: content_block_start
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}

event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Snapping \"}}

event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"it.\"}}

event: content_block_stop
data: {\"type\":\"content_block_stop\",\"index\":0}

event: content_block_start
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"snap_window\",\"input\":{}}}

event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"window_id\\\": 42, \"}}

event: ping
data: {\"type\":\"ping\"}

event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"position\\\": \\\"left_half\\\"}\"}}

event: content_block_stop
data: {\"type\":\"content_block_stop\",\"index\":1}

event: message_delta
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":35}}

event: message_stop
data: {\"type\":\"message_stop\"}

";

    #[test]
    fn decodes_text_and_tool_use() {
        let events = decode_all(TOOL_STREAM).unwrap();
        assert_eq!(
            events,
            [
                StreamEvent::TextDelta("Snapping ".into()),
                StreamEvent::TextDelta("it.".into()),
                StreamEvent::ToolUse(ToolCall {
                    id: "toolu_1".into(),
                    name: "snap_window".into(),
                    input: json!({ "window_id": 42, "position": "left_half" }),
                }),
                StreamEvent::Done {
                    stop: StopReason::ToolUse,
                    usage: Some(Usage {
                        input_tokens: 120,
                        output_tokens: 35
                    }),
                },
            ]
        );
    }

    #[test]
    fn tool_with_no_arguments_gets_empty_object() {
        let raw = "\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"t\",\"name\":\"list_windows\",\"input\":{}}}

data: {\"type\":\"content_block_stop\",\"index\":0}

data: {\"type\":\"message_stop\"}

";
        let events = decode_all(raw).unwrap();
        assert!(matches!(&events[0], StreamEvent::ToolUse(c) if c.input == json!({})));
    }

    #[test]
    fn surfaces_stream_errors() {
        let raw = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n";
        assert_eq!(
            decode_all(raw),
            Err(ProviderError::Api {
                kind: "overloaded_error".into(),
                message: "Overloaded".into()
            })
        );
    }

    #[test]
    fn truncated_stream_is_an_error() {
        let raw = "data: {\"type\":\"message_start\",\"message\":{}}\n\n";
        assert!(matches!(decode_all(raw), Err(ProviderError::Protocol(_))));
    }

    #[test]
    fn request_body_maps_all_block_types() {
        let config = AnthropicConfig::new("k");
        let request = ChatRequest {
            system: "sys".into(),
            messages: vec![
                Message::user_text("hi"),
                Message {
                    role: Role::Assistant,
                    content: vec![Block::ToolUse(ToolCall {
                        id: "t1".into(),
                        name: "list_windows".into(),
                        input: json!({}),
                    })],
                },
                Message {
                    role: Role::User,
                    content: vec![Block::ToolResult {
                        tool_use_id: "t1".into(),
                        content: "[]".into(),
                        is_error: false,
                    }],
                },
            ],
            tools: vec![],
        };
        let body = request_body(&config, &request);
        assert_eq!(body["model"], DEFAULT_MODEL);
        assert_eq!(body["stream"], true);
        assert!(body.get("tools").is_none());
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "t1");
    }

    #[test]
    fn maps_images_to_base64_sources() {
        let body = request_body(
            &AnthropicConfig::new("k"),
            &ChatRequest {
                system: String::new(),
                messages: vec![Message {
                    role: Role::User,
                    content: vec![
                        Block::Text("what is this".into()),
                        Block::Image(crate::message::ImageData {
                            media_type: "image/png".into(),
                            base64: "iVBOR".into(),
                        }),
                    ],
                }],
                tools: vec![],
            },
        );
        let image = &body["messages"][0]["content"][1];
        assert_eq!(image["type"], "image");
        assert_eq!(image["source"]["type"], "base64");
        assert_eq!(image["source"]["media_type"], "image/png");
        assert_eq!(image["source"]["data"], "iVBOR");
    }

    #[test]
    fn debug_hides_api_key() {
        let s = format!("{:?}", AnthropicConfig::new("sk-ant-secret"));
        assert!(!s.contains("secret"));
    }
}
