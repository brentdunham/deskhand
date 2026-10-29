//! Commands the webview can invoke.

use std::sync::PoisonError;
use std::sync::atomic::Ordering;

use deskhand_agent::anthropic::{Anthropic, AnthropicConfig};
use deskhand_agent::openai::{OpenAiCompatible, OpenAiConfig};
use deskhand_agent::{Agent, AgentConfig, Provider};
use deskhand_config::ProviderConfig;
use deskhand_policy::ActionEntry;
use serde::Serialize;
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, State};

use crate::host::{AGENT_EVENT, TauriHost, UiEvent};
use crate::secrets::{self, KeySlot};
use crate::{AppState, lock};

#[derive(Debug, Serialize)]
pub struct Status {
    provider: &'static str,
    model: String,
    /// Shown whenever set, because API keys are sent to this URL.
    base_url: Option<String>,
    has_anthropic_key: bool,
    has_openai_key: bool,
    /// The active provider cannot run without a key that is missing.
    needs_key: bool,
    busy: bool,
    hotkey: String,
    hotkey_registered: bool,
    config_path: String,
    config_error: Option<String>,
    config_warnings: Vec<String>,
}

#[tauri::command]
pub fn status(state: State<'_, AppState>) -> Result<Status, String> {
    let config = state
        .config
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let has_anthropic_key = secrets::get(KeySlot::Anthropic)?.is_some();
    let has_openai_key = secrets::get(KeySlot::OpenaiCompatible)?.is_some();
    let (provider, base_url, needs_key) = match &config.provider {
        ProviderConfig::Anthropic { base_url, .. } => {
            ("anthropic", base_url.clone(), !has_anthropic_key)
        }
        ProviderConfig::OpenaiCompatible { base_url, .. } => {
            ("openai_compatible", Some(base_url.clone()), false)
        }
    };
    Ok(Status {
        provider,
        model: config.provider.model().to_owned(),
        base_url,
        has_anthropic_key,
        has_openai_key,
        needs_key,
        busy: state.busy.load(Ordering::SeqCst),
        hotkey: config.hotkey.to_string(),
        hotkey_registered: state.hotkey_registered.load(Ordering::SeqCst),
        config_path: state
            .config_path
            .get()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        config_error: lock(&state.config_error).clone(),
        config_warnings: lock(&state.config_warnings)
            .iter()
            .chain(lock(&state.startup_warnings).iter())
            .cloned()
            .collect(),
    })
}

#[tauri::command]
pub fn set_api_key(provider: KeySlot, key: String) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("The API key is empty.".into());
    }
    secrets::set(provider, key)
}

#[tauri::command]
pub fn clear_api_key(provider: KeySlot) -> Result<(), String> {
    secrets::clear(provider)
}

/// Opens the config file in Notepad, which every Windows install has.
#[tauri::command]
pub fn open_config(state: State<'_, AppState>) -> Result<(), String> {
    let path = state
        .config_path
        .get()
        .ok_or("The config file location is not known yet.")?;
    std::process::Command::new("notepad.exe")
        .arg(path)
        .spawn()
        .map(drop)
        .map_err(|e| format!("could not open Notepad: {e}"))
}

fn spawn_turn<P: Provider + 'static>(
    app: AppHandle,
    state: &AppState,
    provider: P,
    text: String,
) -> JoinHandle<()> {
    let agent = Agent::new(provider, state.toolbox.clone(), AgentConfig::default());
    let host = TauriHost {
        app: app.clone(),
        confirmations: state.confirmations.clone(),
    };
    let session = state.session.clone();
    let busy = state.busy.clone();
    tauri::async_runtime::spawn(async move {
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
    })
}

/// Builds the configured provider and starts the turn.
fn start_turn(app: AppHandle, state: &AppState, text: String) -> Result<JoinHandle<()>, String> {
    let provider = state
        .config
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .provider
        .clone();
    Ok(match provider {
        ProviderConfig::Anthropic {
            model,
            base_url,
            max_tokens,
        } => {
            let key = secrets::get(KeySlot::Anthropic)?
                .ok_or("Add an Anthropic API key in Settings first.")?;
            let mut config = AnthropicConfig::new(key);
            config.model = model;
            config.max_tokens = max_tokens;
            if let Some(url) = base_url {
                config.base_url = url;
            }
            spawn_turn(app, state, Anthropic::new(state.http.clone(), config), text)
        }
        ProviderConfig::OpenaiCompatible {
            base_url,
            model,
            max_tokens,
        } => {
            let mut config = OpenAiConfig::new(base_url, model);
            config.max_tokens = max_tokens;
            config.api_key = secrets::get(KeySlot::OpenaiCompatible)?;
            spawn_turn(
                app,
                state,
                OpenAiCompatible::new(state.http.clone(), config),
                text,
            )
        }
    })
}

/// Starts a turn in the background with the provider from the current
/// config. Progress arrives as `agent-event`s, ending with `turn_ended`.
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
    match start_turn(app, &state, text) {
        Ok(handle) => {
            *lock(&state.turn) = Some(handle);
            Ok(())
        }
        Err(e) => {
            state.busy.store(false, Ordering::SeqCst);
            Err(e)
        }
    }
}

/// Stops the running turn. Pending confirmations resolve as declined. Tools
/// that already ran stay in the action log and can be undone there.
#[tauri::command]
pub fn cancel_turn(app: AppHandle, state: State<'_, AppState>) {
    if let Some(handle) = lock(&state.turn).take() {
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
    lock(&state.toolbox).log().entries().to_vec()
}

#[tauri::command]
pub fn undo_action(state: State<'_, AppState>, id: u64) -> Result<(), String> {
    lock(&state.toolbox).undo(id).map_err(|e| e.to_string())
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
