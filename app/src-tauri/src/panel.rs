//! The panel window: command-bar and expanded modes, sizing, positioning,
//! hiding, and paste-back.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

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

/// Places the bar on the monitor with the given work area (physical
/// pixels): where the user last dragged it, if that spot is on this monitor,
/// otherwise centered horizontally a sixth of the way down.
pub fn position_on(app: &AppHandle, work: Rect) {
    let state = app.state::<AppState>();
    // Moves from here on are programmatic, not the user's.
    state.dragging.store(false, Ordering::SeqCst);
    let Some(w) = main_window(app) else {
        return;
    };
    if let Some((x, y)) = *lock(&state.user_position)
        && x >= work.left
        && x < work.right
        && y >= work.top
        && y < work.bottom
    {
        let _ = w.set_position(PhysicalPosition::new(x, y));
        return;
    }
    if let Ok(size) = w.outer_size() {
        let width = i32::try_from(size.width).unwrap_or(0);
        let x = work.left + (work.width() - width) / 2;
        let y = work.top + work.height() / 6;
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
}

/// How long hiding on blur stays suppressed after a drag starts or the
/// window moves. Starting a native drag can briefly take focus from the
/// webview; without this the bar would hide itself the moment it is grabbed.
const DRAG_GRACE: Duration = Duration::from_millis(700);

fn suppress_blur(state: &AppState) {
    *lock(&state.suppress_blur_until) = Some(Instant::now() + DRAG_GRACE);
}

/// Called for every window move. Moves during a user drag are remembered so
/// the bar reopens where it was put.
pub fn on_moved(state: &AppState, x: i32, y: i32) {
    if state.dragging.load(Ordering::SeqCst) {
        *lock(&state.user_position) = Some((x, y));
        suppress_blur(state);
    }
}

/// Starts a native window drag from the bar's grip.
#[tauri::command]
pub fn start_panel_drag(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.dragging.store(true, Ordering::SeqCst);
    suppress_blur(&state);
    main_window(&app)
        .ok_or("the panel window is missing")?
        .start_dragging()
        .map_err(|e| e.to_string())
}

/// The bar hides when it loses focus, unless something is still in
/// progress: a running request or a confirmation waiting for the user.
pub fn should_hide_on_blur(state: &AppState) -> bool {
    let dragging = (*lock(&state.suppress_blur_until)).is_some_and(|until| Instant::now() < until);
    !dragging
        && !state.expanded.load(Ordering::SeqCst)
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
