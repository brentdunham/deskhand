//! Commands the webview can invoke.

use std::sync::PoisonError;
use std::sync::atomic::Ordering;

use deskhand_agent::anthropic::{Anthropic, AnthropicConfig};
use deskhand_agent::openai::{OpenAiCompatible, OpenAiConfig};
use deskhand_agent::{Agent, AgentConfig, ChatRequest, EventStream, Provider, Session};
use deskhand_config::ProviderConfig;
use deskhand_policy::ActionEntry;
use serde::Serialize;
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, State};

use crate::host::{AGENT_EVENT, COMPARE_EVENT, CompareHost, CompareMessage, TauriHost, UiEvent};
use crate::secrets::{self, KeySlot};
use crate::{AppState, context, lock};

/// Either provider, so one code path can run whichever the config names.
enum AnyProvider {
    Anthropic(Anthropic),
    OpenAi(OpenAiCompatible),
}

impl Provider for AnyProvider {
    fn stream(&self, request: &ChatRequest) -> EventStream<'_> {
        match self {
            Self::Anthropic(p) => p.stream(request),
            Self::OpenAi(p) => p.stream(request),
        }
    }
}

fn build_provider(state: &AppState, config: &ProviderConfig) -> Result<AnyProvider, String> {
    Ok(match config.clone() {
        ProviderConfig::Anthropic {
            model,
            base_url,
            max_tokens,
        } => {
            let key = secrets::get(KeySlot::Anthropic)?
                .ok_or("Add an Anthropic API key in Settings first.")?;
            let mut c = AnthropicConfig::new(key);
            c.model = model;
            c.max_tokens = max_tokens;
            if let Some(url) = base_url {
                c.base_url = url;
            }
            AnyProvider::Anthropic(Anthropic::new(state.http.clone(), c))
        }
        ProviderConfig::OpenaiCompatible {
            base_url,
            model,
            max_tokens,
            thinking,
        } => {
            let mut c = OpenAiConfig::new(base_url, model);
            c.max_tokens = max_tokens;
            c.thinking = thinking;
            c.api_key = secrets::get(KeySlot::OpenaiCompatible)?;
            AnyProvider::OpenAi(OpenAiCompatible::new(state.http.clone(), c))
        }
    })
}

/// How a provider is named in compare mode: the model, plus the host for
/// OpenAI-compatible endpoints (the same model name can run in many places).
fn provider_label(config: &ProviderConfig) -> String {
    match config {
        ProviderConfig::Anthropic { model, .. } => model.clone(),
        ProviderConfig::OpenaiCompatible {
            model, base_url, ..
        } => {
            let host = base_url
                .split("://")
                .nth(1)
                .and_then(|rest| rest.split(['/', ':']).next())
                .unwrap_or(base_url);
            format!("{model} ({host})")
        }
    }
}

fn current_config(state: &AppState) -> deskhand_config::Config {
    state
        .config
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

#[derive(Debug, Serialize)]
pub struct CompareStatus {
    /// Labels for the two answer columns: primary, then compare.
    labels: [String; 2],
    /// The compare provider's endpoint, when it is OpenAI-compatible; its
    /// key (if any) is sent there.
    base_url: Option<String>,
    kind: &'static str,
}

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
    compare: Option<CompareStatus>,
    mcp: Vec<crate::mcp::ServerView>,
}

fn kind_name(p: &ProviderConfig) -> &'static str {
    match p {
        ProviderConfig::Anthropic { .. } => "anthropic",
        ProviderConfig::OpenaiCompatible { .. } => "openai_compatible",
    }
}

#[tauri::command]
pub fn status(state: State<'_, AppState>) -> Result<Status, String> {
    let config = current_config(&state);
    let has_anthropic_key = secrets::get(KeySlot::Anthropic)?.is_some();
    let has_openai_key = secrets::get(KeySlot::OpenaiCompatible)?.is_some();
    let (base_url, needs_key) = match &config.provider {
        ProviderConfig::Anthropic { base_url, .. } => (base_url.clone(), !has_anthropic_key),
        ProviderConfig::OpenaiCompatible { base_url, .. } => (Some(base_url.clone()), false),
    };
    let compare = config.compare.as_ref().map(|c| CompareStatus {
        labels: [provider_label(&config.provider), provider_label(c)],
        base_url: match c {
            ProviderConfig::OpenaiCompatible { base_url, .. } => Some(base_url.clone()),
            ProviderConfig::Anthropic { base_url, .. } => base_url.clone(),
        },
        kind: kind_name(c),
    });
    Ok(Status {
        provider: kind_name(&config.provider),
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
        compare,
        mcp: crate::mcp::statuses(&state),
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

/// Marks the app busy for the duration of a request. Returns an error if
/// something is already running.
fn begin(state: &AppState) -> Result<(), String> {
    if state.busy.swap(true, Ordering::SeqCst) {
        Err("DeskHand is still working on the previous request.".into())
    } else {
        Ok(())
    }
}

fn clean(text: String) -> Result<String, String> {
    let text = text.trim().to_owned();
    if text.is_empty() {
        Err("The message is empty.".into())
    } else {
        Ok(text)
    }
}

/// Starts a turn in the background with the provider from the current
/// config. Progress arrives as `agent-event`s, ending with `turn_ended`.
#[tauri::command]
pub fn send_message(
    app: AppHandle,
    state: State<'_, AppState>,
    text: String,
    include: Option<context::Include>,
) -> Result<(), String> {
    let text = clean(text)?;
    begin(&state)?;
    let provider = match build_provider(&state, &current_config(&state).provider) {
        Ok(p) => p,
        Err(e) => {
            state.busy.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };
    let context = context::take(&app, &state, include);

    let agent = Agent::new(provider, state.toolbox.clone(), AgentConfig::default());
    let host = TauriHost {
        app: app.clone(),
        confirmations: state.confirmations.clone(),
    };
    let session = state.session.clone();
    let busy = state.busy.clone();
    let handle: JoinHandle<()> = tauri::async_runtime::spawn(async move {
        let mut session = session.lock().await;
        if let Err(e) = agent
            .run_turn_with_context(&mut session, &text, context, &host)
            .await
        {
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
    *lock(&state.turn) = Some(handle);
    Ok(())
}

/// Sends the same question and context to the primary and compare
/// providers at once, without tools, in throwaway sessions that do not touch
/// the main conversation. Progress arrives as `compare-event`s tagged with
/// the lane (0 = primary, 1 = compare), ending with an untagged `turn_ended`.
#[tauri::command]
pub fn compare_message(
    app: AppHandle,
    state: State<'_, AppState>,
    text: String,
    include: Option<context::Include>,
) -> Result<(), String> {
    let text = clean(text)?;
    let config = current_config(&state);
    let compare = config
        .compare
        .clone()
        .ok_or("No comparison model is configured. Add a [compare] section to the config file.")?;
    begin(&state)?;
    let providers = build_provider(&state, &config.provider)
        .and_then(|a| Ok((a, build_provider(&state, &compare)?)));
    let (primary, second) = match providers {
        Ok(p) => p,
        Err(e) => {
            state.busy.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };
    let context = context::take(&app, &state, include);

    let text_only = || AgentConfig {
        tools: false,
        ..AgentConfig::default()
    };
    let agents = (
        Agent::new(primary, state.toolbox.clone(), text_only()),
        Agent::new(second, state.toolbox.clone(), text_only()),
    );
    let hosts = (
        CompareHost {
            app: app.clone(),
            lane: 0,
        },
        CompareHost {
            app: app.clone(),
            lane: 1,
        },
    );
    let busy = state.busy.clone();
    let handle: JoinHandle<()> = tauri::async_runtime::spawn(async move {
        let (mut s0, mut s1) = (Session::new(), Session::new());
        let (r0, r1) = tokio::join!(
            agents
                .0
                .run_turn_with_context(&mut s0, &text, context.clone(), &hosts.0),
            agents
                .1
                .run_turn_with_context(&mut s1, &text, context, &hosts.1),
        );
        for (lane, result) in [(0u8, r0), (1u8, r1)] {
            if let Err(e) = result {
                let _ = app.emit(
                    COMPARE_EVENT,
                    CompareMessage {
                        lane: Some(lane),
                        event: UiEvent::TurnFailed {
                            error: e.to_string(),
                        },
                    },
                );
            }
        }
        busy.store(false, Ordering::SeqCst);
        let _ = app.emit(
            COMPARE_EVENT,
            CompareMessage {
                lane: None,
                event: UiEvent::TurnEnded,
            },
        );
    });
    *lock(&state.turn) = Some(handle);
    Ok(())
}

/// Stops the running turn or comparison. Pending confirmations resolve as
/// declined. Tools that already ran stay in the action log.
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
