//! Commands the webview can invoke.

use std::sync::PoisonError;
use std::sync::atomic::Ordering;

use deskhand_agent::anthropic::{Anthropic, AnthropicConfig, DEFAULT_MODEL};
use deskhand_agent::{Agent, AgentConfig};
use deskhand_policy::ActionEntry;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::host::{AGENT_EVENT, TauriHost, UiEvent};
use crate::{AppState, HOTKEY_LABEL, secrets};

#[derive(Debug, Serialize)]
pub struct Status {
    has_api_key: bool,
    model: &'static str,
    busy: bool,
    hotkey: &'static str,
    hotkey_registered: bool,
}

#[tauri::command]
pub fn status(state: State<'_, AppState>) -> Result<Status, String> {
    Ok(Status {
        has_api_key: secrets::get()?.is_some(),
        model: DEFAULT_MODEL,
        busy: state.busy.load(Ordering::SeqCst),
        hotkey: HOTKEY_LABEL,
        hotkey_registered: state.hotkey_registered.load(Ordering::SeqCst),
    })
}

#[tauri::command]
pub fn set_api_key(key: String) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("The API key is empty.".into());
    }
    secrets::set(key)
}

#[tauri::command]
pub fn clear_api_key() -> Result<(), String> {
    secrets::clear()
}

/// Starts a turn in the background. Progress arrives as `agent-event`s,
/// ending with `turn_ended`.
#[tauri::command]
pub fn send_message(
    app: AppHandle,
    state: State<'_, AppState>,
    text: String,
) -> Result<(), String> {
    let text = text.trim().to_owned();
    if text.is_empty() {
        return Err("The message is empty.".into());
    }
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("DeskHand is still working on the previous request.".into());
    }
    let key = match secrets::get() {
        Ok(Some(key)) => key,
        Ok(None) => {
            state.busy.store(false, Ordering::SeqCst);
            return Err("Add an Anthropic API key in Settings first.".into());
        }
        Err(e) => {
            state.busy.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };

    let agent = Agent::new(
        Anthropic::new(state.http.clone(), AnthropicConfig::new(key)),
        state.toolbox.clone(),
        AgentConfig::default(),
    );
    let host = TauriHost {
        app: app.clone(),
        confirmations: state.confirmations.clone(),
    };
    let session = state.session.clone();
    let busy = state.busy.clone();

    let handle = tauri::async_runtime::spawn(async move {
        let mut session = session.lock().await;
        if let Err(e) = agent.run_turn(&mut session, &text, &host).await {
            let _ = app.emit(
                AGENT_EVENT,
                UiEvent::TurnFailed {
                    error: e.to_string(),
                },
            );
        }
        drop(session);
        busy.store(false, Ordering::SeqCst);
        let _ = app.emit(AGENT_EVENT, UiEvent::TurnEnded);
    });
    *state.turn.lock().unwrap_or_else(PoisonError::into_inner) = Some(handle);
    Ok(())
}

/// Stops the running turn. Pending confirmations resolve as declined. Tools
/// that already ran stay in the action log and can be undone there.
#[tauri::command]
pub fn cancel_turn(app: AppHandle, state: State<'_, AppState>) {
    if let Some(handle) = state
        .turn
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
    {
        handle.abort();
    }
    state.confirmations.cancel_all();
    state.busy.store(false, Ordering::SeqCst);
    let _ = app.emit(AGENT_EVENT, UiEvent::TurnCancelled);
}

#[tauri::command]
pub fn answer_confirmation(state: State<'_, AppState>, id: u64, approved: bool) -> bool {
    state.confirmations.resolve(id, approved)
}

#[tauri::command]
pub fn action_log(state: State<'_, AppState>) -> Vec<ActionEntry> {
    state
        .toolbox
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .log()
        .entries()
        .to_vec()
}

#[tauri::command]
pub fn undo_action(state: State<'_, AppState>, id: u64) -> Result<(), String> {
    state
        .toolbox
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .undo(id)
        .map_err(|e| e.to_string())
}

/// Clears the conversation, including its untrusted-content taint. The
/// action log is kept.
#[tauri::command]
pub async fn new_conversation(state: State<'_, AppState>) -> Result<(), String> {
    if state.busy.load(Ordering::SeqCst) {
        return Err("Stop the current request first.".into());
    }
    state.session.lock().await.clear();
    Ok(())
}
