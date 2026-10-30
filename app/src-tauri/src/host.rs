//! Bridges the agent loop to the webview: agent events become Tauri events,
//! and confirmation requests wait on an answer from the UI.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use deskhand_agent::{AgentEvent, AgentHost, ConfirmationRequest, StopReason};
use deskhand_policy::{ConfirmReason, Tier};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;

pub const AGENT_EVENT: &str = "agent-event";
pub const CONFIRM_EVENT: &str = "confirm-request";

/// Confirmation requests waiting on the user, keyed by request id.
#[derive(Default)]
pub struct Confirmations {
    next: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<bool>>>,
}

impl Confirmations {
    fn pending(&self) -> std::sync::MutexGuard<'_, HashMap<u64, oneshot::Sender<bool>>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn open(&self) -> (u64, oneshot::Receiver<bool>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending().insert(id, tx);
        (id, rx)
    }

    /// Delivers the user's answer. Returns false if the request is gone
    /// (already answered, or the turn was cancelled).
    pub fn resolve(&self, id: u64, approved: bool) -> bool {
        self.pending()
            .remove(&id)
            .is_some_and(|tx| tx.send(approved).is_ok())
    }

    /// Drops every pending request. Waiting calls resolve as declined.
    pub fn cancel_all(&self) {
        self.pending().clear();
    }

    /// No confirmation is waiting on the user.
    pub fn is_empty(&self) -> bool {
        self.pending().is_empty()
    }
}

/// Agent events as the frontend receives them.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiEvent {
    TextDelta {
        text: String,
    },
    ToolStarted {
        call_id: String,
        tool: String,
        input: Value,
    },
    ToolFinished {
        call_id: String,
        tool: String,
        action_id: Option<u64>,
        ok: bool,
        /// The result on success, the error message on failure.
        detail: Value,
    },
    TurnComplete {
        stop: String,
    },
    TurnFailed {
        error: String,
    },
    /// Always the last event of a turn, sent after the backend is ready for
    /// the next message.
    TurnEnded,
    TurnCancelled,
}

impl From<AgentEvent> for UiEvent {
    fn from(event: AgentEvent) -> Self {
        match event {
            AgentEvent::TextDelta(text) => Self::TextDelta { text },
            AgentEvent::ToolStarted {
                call_id,
                tool,
                input,
            } => Self::ToolStarted {
                call_id,
                tool,
                input,
            },
            AgentEvent::ToolFinished {
                call_id,
                tool,
                action_id,
                result,
            } => {
                let (ok, detail) = match result {
                    Ok(v) => (true, v),
                    Err(e) => (false, Value::String(e)),
                };
                Self::ToolFinished {
                    call_id,
                    tool,
                    action_id,
                    ok,
                    detail,
                }
            }
            AgentEvent::TurnComplete(stop) => Self::TurnComplete {
                stop: match stop {
                    StopReason::EndTurn => "end_turn".into(),
                    StopReason::ToolUse => "tool_use".into(),
                    StopReason::MaxTokens => "max_tokens".into(),
                    StopReason::Other(s) => s,
                },
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct ConfirmPayload {
    id: u64,
    tool: String,
    input: Value,
    tier: Tier,
    reason: ConfirmReason,
}

pub struct TauriHost {
    pub app: AppHandle,
    pub confirmations: Arc<Confirmations>,
}

impl AgentHost for TauriHost {
    fn on_event(&self, event: AgentEvent) {
        let _ = self.app.emit(AGENT_EVENT, UiEvent::from(event));
    }

    async fn confirm(&self, request: ConfirmationRequest) -> bool {
        let (id, answer) = self.confirmations.open();
        let payload = ConfirmPayload {
            id,
            tool: request.tool,
            input: request.input,
            tier: request.tier,
            reason: request.reason,
        };
        if self.app.emit(CONFIRM_EVENT, payload).is_err() {
            self.confirmations.resolve(id, false);
            return false;
        }
        // Make sure the user can see the request.
        crate::show_panel(&self.app);
        // A dropped sender (turn cancelled) counts as declined.
        answer.await.unwrap_or(false)
    }
}

pub const COMPARE_EVENT: &str = "compare-event";

/// A compare-mode event. `lane` is 0 for the primary model and 1 for the
/// compare model; `None` marks the end of the whole comparison.
#[derive(Debug, Clone, Serialize)]
pub struct CompareMessage {
    pub lane: Option<u8>,
    pub event: UiEvent,
}

/// Host for one side of a comparison. Compare runs offer no tools, so
/// confirmation is never requested; if it were, it would be declined.
pub struct CompareHost {
    pub app: AppHandle,
    pub lane: u8,
}

impl AgentHost for CompareHost {
    fn on_event(&self, event: AgentEvent) {
        let _ = self.app.emit(
            COMPARE_EVENT,
            CompareMessage {
                lane: Some(self.lane),
                event: UiEvent::from(event),
            },
        );
    }

    async fn confirm(&self, _request: ConfirmationRequest) -> bool {
        false
    }
}
