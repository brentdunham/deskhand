//! The panel window: command-bar and expanded modes, sizing, positioning,
//! hiding, and paste-back.

use std::sync::atomic::Ordering;

use deskhand_win::{Rect, WinError};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, State, WebviewWindow};

use crate::{AppState, lock};

/// Emitted with `true` (expanded) or `false` (command bar) on mode changes.
pub const MODE_EVENT: &str = "panel-mode";

const BAR_WIDTH: f64 = 680.0;
const BAR_MIN_HEIGHT: f64 = 64.0;
const BAR_MAX_HEIGHT: f64 = 640.0;
const EXPANDED_SIZE: (f64, f64) = (560.0, 760.0);

fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}

pub fn set_mode(app: &AppHandle, expanded: bool) {
    app.state::<AppState>()
        .expanded
        .store(expanded, Ordering::SeqCst);
    if let Some(w) = main_window(app) {
        let size = if expanded {
            LogicalSize::new(EXPANDED_SIZE.0, EXPANDED_SIZE.1)
        } else {
            LogicalSize::new(BAR_WIDTH, BAR_MIN_HEIGHT)
        };
        let _ = w.set_size(size);
    }
    let _ = app.emit(MODE_EVENT, expanded);
}

/// Places the bar centered horizontally, a sixth of the way down the given
/// monitor work area (physical pixels).
pub fn position_on(app: &AppHandle, work: Rect) {
    if let Some(w) = main_window(app)
        && let Ok(size) = w.outer_size()
    {
        let width = i32::try_from(size.width).unwrap_or(0);
        let x = work.left + (work.width() - width) / 2;
        let y = work.top + work.height() / 6;
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
}

/// The bar hides when it loses focus, unless something is still in
/// progress: a running request or a confirmation waiting for the user.
pub fn should_hide_on_blur(state: &AppState) -> bool {
    !state.expanded.load(Ordering::SeqCst)
        && !state.busy.load(Ordering::SeqCst)
        && state.confirmations.is_empty()
}

#[tauri::command]
pub fn set_panel_mode(app: AppHandle, expanded: bool) {
    set_mode(&app, expanded);
}

/// The frontend reports its content height; the bar grows to fit.
#[tauri::command]
pub fn resize_panel(app: AppHandle, state: State<'_, AppState>, height: f64) {
    if state.expanded.load(Ordering::SeqCst) {
        return;
    }
    if let Some(w) = main_window(&app) {
        let h = height.clamp(BAR_MIN_HEIGHT, BAR_MAX_HEIGHT);
        let _ = w.set_size(LogicalSize::new(BAR_WIDTH, h));
    }
}

#[tauri::command]
pub fn hide_panel(app: AppHandle) {
    if let Some(w) = main_window(&app) {
        let _ = w.hide();
    }
}

/// Pastes `text` into the window the hotkey was pressed in. The panel hides
/// first so the target can take the foreground; on failure it comes back to
/// show the error. Triggered only by the user in the UI; the model has no
/// way to call it.
#[tauri::command]
pub async fn insert_text(
    app: AppHandle,
    state: State<'_, AppState>,
    text: String,
) -> Result<(), String> {
    let target = (*lock(&state.last_target)).ok_or(
        "There is no window to insert into. Open DeskHand with the hotkey from the app you want to paste into.",
    )?;
    if let Some(w) = main_window(&app) {
        let _ = w.hide();
    }
    let result = tauri::async_runtime::spawn_blocking(move || {
        deskhand_win::input::paste_into(target, &text)
    })
    .await
    .map_err(|e| e.to_string())?;
    result.map_err(|e| {
        crate::show_panel(&app);
        match e {
            WinError::WindowGone(_) => {
                "That window has closed. The text is on your clipboard.".to_owned()
            }
            WinError::ForegroundDenied(_) => {
                "Windows would not switch to that window. The text is on your clipboard; paste it with Ctrl+V.".to_owned()
            }
            other => format!("Could not paste: {other}"),
        }
    })
}
