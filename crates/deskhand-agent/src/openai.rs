//! OpenAI-compatible Chat Completions adapter with streaming. Covers OpenAI,
//! OpenRouter, and local servers (Ollama, LM Studio, llama.cpp server).

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::message::{Block, ChatRequest, Message, Role, StopReason, StreamEvent, ToolCall, Usage};
use crate::provider::{
    Decoder, EventStream, Provider, ProviderError, parse_tool_input, sse_stream,
};
use crate::sse::SseEvent;

/// Ollama's OpenAI-compatible endpoint.
pub const OLLAMA_BASE_URL: &str = "http://localhost:11434/v1";

#[derive(Clone)]
pub struct OpenAiConfig {
    /// Including the version path, e.g. `https://api.openai.com/v1`.
    pub base_url: String,
    /// Optional: local servers usually need none.
    pub api_key: Option<String>,
    pub model: String,
    pub max_tokens: u32,
    /// Sent as `chat_template_kwargs.enable_thinking` when set. Unset sends
    /// nothing, so APIs that reject unknown parameters are unaffected.
    pub thinking: Option<bool>,
}

impl OpenAiConfig {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: None,
            model: model.into(),
            max_tokens: 4096,
            thinking: None,
        }
    }
}

// Never print the key.
impl std::fmt::Debug for OpenAiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiConfig")
            .field("base_url", &self.base_url)
            .field("has_api_key", &self.api_key.is_some())
            .field("model", &self.model)
            .field("max_tokens", &self.max_tokens)
            .field("thinking", &self.thinking)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct OpenAiCompatible {
    http: reqwest::Client,
    config: OpenAiConfig,
}

impl OpenAiCompatible {
    pub fn new(http: reqwest::Client, config: OpenAiConfig) -> Self {
        Self { http, config }
    }
}

impl Provider for OpenAiCompatible {
    fn stream(&self, request: &ChatRequest) -> EventStream<'_> {
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let mut builder = self
            .http
            .post(url)
            .json(&request_body(&self.config, request));
        if let Some(key) = &self.config.api_key {
            builder = builder.bearer_auth(key);
        }
        sse_stream(builder, OpenAiDecoder::default())
    }
}

pub(crate) fn request_body(config: &OpenAiConfig, request: &ChatRequest) -> Value {
    let mut messages = vec![json!({ "role": "system", "content": request.system })];
    for m in &request.messages {
        messages.extend(message_json(m));
    }
    let mut body = json!({
        "model": config.model,
        "max_tokens": config.max_tokens,
        "stream": true,
        "messages": messages,
    });
    if let Some(thinking) = config.thinking {
        body["chat_template_kwargs"] = json!({ "enable_thinking": thinking });
    }
    if !request.tools.is_empty() {
        body["tools"] = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    },
                })
            })
            .collect();
    }
    body
}

/// One neutral message can become several OpenAI messages: tool results are
/// separate `tool` role messages.
fn message_json(m: &Message) -> Vec<Value> {
    let text: String = m
        .content
        .iter()
        .filter_map(|b| match b {
            Block::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    match m.role {
        Role::Assistant => {
            let calls: Vec<Value> = m
                .content
                .iter()
                .filter_map(|b| match b {
                    Block::ToolUse(c) => Some(json!({
                        "id": c.id,
                        "type": "function",
                        "function": { "name": c.name, "arguments": c.input.to_string() },
                    })),
                    _ => None,
                })
                .collect();
            let mut msg = json!({
                "role": "assistant",
                "content": if text.is_empty() { Value::Null } else { Value::String(text) },
            });
            if !calls.is_empty() {
                msg["tool_calls"] = Value::Array(calls);
            }
            vec![msg]
        }
        Role::User => {
            let mut out: Vec<Value> = m
                .content
                .iter()
                .filter_map(|b| match b {
                    Block::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => Some(json!({
                        "role": "tool",
                        "tool_call_id": tool_use_id,
                        "content": if *is_error { format!("Error: {content}") } else { content.clone() },
                    })),
                    _ => None,
                })
                .collect();
            let images: Vec<Value> = m
                .content
                .iter()
                .filter_map(|b| match b {
                    Block::Image(img) => Some(json!({
                        "type": "image_url",
                        "image_url": { "url": format!("data:{};base64,{}", img.media_type, img.base64) },
                    })),
                    _ => None,
                })
                .collect();
            if images.is_empty() {
                if !text.is_empty() {
                    out.push(json!({ "role": "user", "content": text }));
                }
            } else {
                // Images require the content-parts form of a user message.
                let mut parts = Vec::new();
                if !text.is_empty() {
                    parts.push(json!({ "type": "text", "text": text }));
                }
                parts.extend(images);
                out.push(json!({ "role": "user", "content": parts }));
            }
            out
        }
    }
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    args: String,
}

#[derive(Default)]
pub(crate) struct OpenAiDecoder {
    calls: BTreeMap<u64, PartialCall>,
    finish: Option<String>,
    usage: Option<Usage>,
    done: bool,
}

impl OpenAiDecoder {
    fn complete(&mut self) -> Result<Vec<StreamEvent>, ProviderError> {
        if self.done {
            return Ok(Vec::new());
        }
        self.done = true;
        let mut out = Vec::new();
        for (index, c) in std::mem::take(&mut self.calls) {
            out.push(StreamEvent::ToolUse(ToolCall {
                id: if c.id.is_empty() {
                    format!("call_{index}")
                } else {
                    c.id
                },
                name: c.name,
                input: parse_tool_input(&c.args)?,
            }));
        }
        let has_calls = !out.is_empty();
        let stop = match self.finish.as_deref() {
            Some("tool_calls") => StopReason::ToolUse,
            Some("length") => StopReason::MaxTokens,
            Some("stop") | None if has_calls => StopReason::ToolUse,
            Some("stop") | None => StopReason::EndTurn,
            Some(other) => StopReason::Other(other.to_owned()),
        };
        out.push(StreamEvent::Done {
            stop,
            usage: self.usage,
        });
        Ok(out)
    }
}

impl Decoder for OpenAiDecoder {
    fn decode(&mut self, event: SseEvent) -> Result<Vec<StreamEvent>, ProviderError> {
        if event.data.trim() == "[DONE]" {
            return self.complete();
        }
        let v: Value = serde_json::from_str(&event.data)
            .map_err(|e| ProviderError::Protocol(format!("invalid chunk JSON: {e}")))?;
        if let Some(err) = v.get("error") {
            return Err(ProviderError::Api {
                kind: err["type"].as_str().unwrap_or("error").to_owned(),
                message: err["message"].as_str().unwrap_or_default().to_owned(),
            });
        }
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            self.usage = Some(Usage {
                input_tokens: u["prompt_tokens"].as_u64().unwrap_or_default(),
                output_tokens: u["completion_tokens"].as_u64().unwrap_or_default(),
            });
        }
        let mut out = Vec::new();
        let choice = &v["choices"][0];
        let delta = &choice["delta"];
        if let Some(t) = delta["content"].as_str().filter(|t| !t.is_empty()) {
            out.push(StreamEvent::TextDelta(t.to_owned()));
        }
        if let Some(calls) = delta["tool_calls"].as_array() {
            for (pos, c) in calls.iter().enumerate() {
                let index = c["index"].as_u64().unwrap_or(pos as u64);
                let entry = self.calls.entry(index).or_default();
                if let Some(id) = c["id"].as_str() {
                    entry.id = id.to_owned();
                }
                if let Some(name) = c["function"]["name"].as_str() {
                    entry.name.push_str(name);
                }
                if let Some(args) = c["function"]["arguments"].as_str() {
                    entry.args.push_str(args);
                }
            }
        }
        if let Some(f) = choice["finish_reason"].as_str() {
            self.finish = Some(f.to_owned());
        }
        Ok(out)
    }

    /// Some servers close the stream without `[DONE]` after a finish reason.
    fn finish(&mut self) -> Result<Vec<StreamEvent>, ProviderError> {
        if self.done {
            return Ok(Vec::new());
        }
        if self.finish.is_some() {
            return self.complete();
        }
        Err(ProviderError::Protocol(
            "stream ended without a finish reason".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sse::SseParser;
    use deskhand_tools::ToolDefinition;

    fn decode_all(raw: &str) -> Result<Vec<StreamEvent>, ProviderError> {
        let mut parser = SseParser::new();
        let mut d = OpenAiDecoder::default();
        let mut out = Vec::new();
        for e in parser.push(raw.as_bytes()) {
            out.extend(d.decode(e)?);
        }
        out.extend(d.finish()?);
        Ok(out)
    }

    #[test]
    fn decodes_streamed_tool_call_arguments() {
        let raw = "\
data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"On it.\"}}]}

data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"type\":\"function\",\"function\":{\"name\":\"snap_window\",\"arguments\":\"\"}}]}}]}

data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"window_id\\\":7,\"}}]}}]}

data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"position\\\":\\\"maximize\\\"}\"}}]}}]}

data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}

data: [DONE]

";
        let events = decode_all(raw).unwrap();
        assert_eq!(
            events,
            [
                StreamEvent::TextDelta("On it.".into()),
                StreamEvent::ToolUse(ToolCall {
                    id: "call_a".into(),
                    name: "snap_window".into(),
                    input: json!({ "window_id": 7, "position": "maximize" }),
                }),
                StreamEvent::Done {
                    stop: StopReason::ToolUse,
                    usage: None
                },
            ]
        );
    }

    #[test]
    fn whole_tool_call_in_one_chunk_without_id_and_without_done() {
        // Ollama-style: complete call in one chunk, finish_reason "stop".
        let raw = "\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"function\":{\"name\":\"list_windows\",\"arguments\":\"{}\"}}]},\"finish_reason\":\"stop\"}]}

";
        let events = decode_all(raw).unwrap();
        assert!(
            matches!(&events[0], StreamEvent::ToolUse(c) if c.id == "call_0" && c.name == "list_windows")
        );
        assert!(matches!(
            events[1],
            StreamEvent::Done {
                stop: StopReason::ToolUse,
                ..
            }
        ));
    }

    #[test]
    fn plain_text_ends_turn() {
        let raw = "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let events = decode_all(raw).unwrap();
        assert_eq!(
            events.last(),
            Some(&StreamEvent::Done {
                stop: StopReason::EndTurn,
                usage: None
            })
        );
    }

    #[test]
    fn surfaces_errors_and_truncation() {
        let raw = "data: {\"error\":{\"type\":\"invalid_request_error\",\"message\":\"bad\"}}\n\n";
        assert!(matches!(decode_all(raw), Err(ProviderError::Api { .. })));
        let raw = "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n";
        assert!(matches!(decode_all(raw), Err(ProviderError::Protocol(_))));
    }

    #[test]
    fn thinking_is_sent_only_when_set() {
        let request = ChatRequest {
            system: String::new(),
            messages: vec![],
            tools: vec![],
        };
        let mut config = OpenAiConfig::new(OLLAMA_BASE_URL, "qwen3");
        assert!(
            request_body(&config, &request)
                .get("chat_template_kwargs")
                .is_none()
        );
        config.thinking = Some(false);
        assert_eq!(
            request_body(&config, &request)["chat_template_kwargs"]["enable_thinking"],
            false
        );
    }

    #[test]
    fn images_use_content_parts_with_data_urls() {
        let config = OpenAiConfig::new(OLLAMA_BASE_URL, "llava");
        let request = ChatRequest {
            system: "sys".into(),
            messages: vec![Message {
                role: Role::User,
                content: vec![
                    Block::Text("describe".into()),
                    Block::Image(crate::message::ImageData {
                        media_type: "image/png".into(),
                        base64: "iVBOR".into(),
                    }),
                ],
            }],
            tools: vec![],
        };
        let body = request_body(&config, &request);
        let parts = &body["messages"][1]["content"];
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[1]["type"], "image_url");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,iVBOR");
    }

    #[test]
    fn request_body_maps_tool_results_and_tools() {
        let config = OpenAiConfig::new(OLLAMA_BASE_URL, "qwen3");
        let request = ChatRequest {
            system: "sys".into(),
            messages: vec![
                Message {
                    role: Role::Assistant,
                    content: vec![Block::ToolUse(ToolCall {
                        id: "c1".into(),
                        name: "list_windows".into(),
                        input: json!({}),
                    })],
                },
                Message {
                    role: Role::User,
                    content: vec![Block::ToolResult {
                        tool_use_id: "c1".into(),
                        content: "nope".into(),
                        is_error: true,
                    }],
                },
            ],
            tools: vec![ToolDefinition {
                name: "list_windows".into(),
                description: "d".into(),
                input_schema: json!({ "type": "object" }),
            }],
        };
        let body = request_body(&config, &request);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[1]["content"], Value::Null);
        assert_eq!(msgs[1]["tool_calls"][0]["function"]["arguments"], "{}");
        assert_eq!(msgs[2]["role"], "tool");
        assert_eq!(msgs[2]["content"], "Error: nope");
        assert_eq!(body["tools"][0]["function"]["parameters"]["type"], "object");
    }
}
