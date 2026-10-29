//! The provider abstraction and the shared HTTP streaming path.

use std::collections::VecDeque;
use std::fmt;
use std::pin::Pin;

use futures_util::{Stream, StreamExt, stream};

use crate::message::{ChatRequest, StreamEvent};
use crate::sse::{SseEvent, SseParser};

pub type EventStream<'a> =
    Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send + 'a>>;

type ByteStream = Pin<Box<dyn Stream<Item = reqwest::Result<Vec<u8>>> + Send>>;

/// A model backend. Implementations must emit exactly one
/// [`StreamEvent::Done`] as the final item of a successful stream.
pub trait Provider: Send + Sync {
    fn stream(&self, request: &ChatRequest) -> EventStream<'_>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// Could not reach the endpoint.
    Transport(String),
    /// The endpoint returned a non-success status.
    Http { status: u16, body: String },
    /// The endpoint reported an error inside the stream.
    Api { kind: String, message: String },
    /// The stream did not follow the expected protocol.
    Protocol(String),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(m) => write!(f, "could not reach the model provider: {m}"),
            Self::Http { status, body } => {
                write!(f, "model provider returned HTTP {status}: {body}")
            }
            Self::Api { kind, message } => write!(f, "model provider error ({kind}): {message}"),
            Self::Protocol(m) => write!(f, "unexpected response from model provider: {m}"),
        }
    }
}

impl std::error::Error for ProviderError {}

/// Turns one provider's SSE events into normalized stream events.
pub(crate) trait Decoder: Send {
    fn decode(&mut self, event: SseEvent) -> Result<Vec<StreamEvent>, ProviderError>;
    /// Called when the byte stream ends.
    fn finish(&mut self) -> Result<Vec<StreamEvent>, ProviderError>;
}

/// Longest error body kept in [`ProviderError::Http`].
const MAX_ERROR_BODY: usize = 2048;

/// Sends `request` and streams decoded events from the SSE response body.
pub(crate) fn sse_stream<D: Decoder + 'static>(
    request: reqwest::RequestBuilder,
    decoder: D,
) -> EventStream<'static> {
    struct State<D> {
        request: Option<reqwest::RequestBuilder>,
        body: Option<ByteStream>,
        parser: SseParser,
        decoder: D,
        queue: VecDeque<StreamEvent>,
        done: bool,
    }

    let state = State {
        request: Some(request),
        body: None,
        parser: SseParser::new(),
        decoder,
        queue: VecDeque::new(),
        done: false,
    };

    Box::pin(stream::try_unfold(state, |mut st| async move {
        loop {
            if let Some(event) = st.queue.pop_front() {
                return Ok(Some((event, st)));
            }
            if st.done {
                return Ok(None);
            }
            if st.body.is_none() {
                let request = st.request.take().expect("request is sent once");
                let response = request
                    .send()
                    .await
                    .map_err(|e| ProviderError::Transport(e.to_string()))?;
                let status = response.status();
                if !status.is_success() {
                    let mut body = response.text().await.unwrap_or_default();
                    body.truncate(MAX_ERROR_BODY);
                    return Err(ProviderError::Http {
                        status: status.as_u16(),
                        body,
                    });
                }
                st.body = Some(Box::pin(
                    response.bytes_stream().map(|r| r.map(|b| b.to_vec())),
                ));
                continue;
            }
            let body = st.body.as_mut().expect("body is set above");
            match body.next().await {
                Some(chunk) => {
                    let chunk = chunk.map_err(|e| ProviderError::Transport(e.to_string()))?;
                    for event in st.parser.push(&chunk) {
                        st.queue.extend(st.decoder.decode(event)?);
                    }
                }
                None => {
                    st.queue.extend(st.decoder.finish()?);
                    st.done = true;
                }
            }
        }
    }))
}

/// Parses accumulated tool arguments. Providers send `""` for no arguments.
pub(crate) fn parse_tool_input(raw: &str) -> Result<serde_json::Value, ProviderError> {
    if raw.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    serde_json::from_str(raw)
        .map_err(|e| ProviderError::Protocol(format!("tool arguments are not valid JSON: {e}")))
}
