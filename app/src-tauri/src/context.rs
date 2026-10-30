//! Screen context: what the user was looking at when they pressed the hotkey.
//!
//! Capture runs only on the hotkey, never in the background. The UI shows
//! every captured part as a chip the user can remove before sending.

use std::sync::atomic::Ordering;
use std::time::Duration;

use deskhand_agent::{ImageData, ScreenContext};
use deskhand_win::capture::{self, NoScreenshot, Screenshot};
use deskhand_win::uia::{self, TextLimits};
use deskhand_win::window::{self, WindowInfo};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{AppState, lock};

/// Emitted whenever the pending context changes; the UI re-reads it.
pub const CONTEXT_EVENT: &str = "context-changed";

const MAX_SELECTION_CHARS: usize = 20_000;
const MAX_WINDOW_TEXT_CHARS: usize = 12_000;
/// Anthropic's recommended maximum image edge; larger images are resized
/// server-side anyway, so sending more only costs upload time.
const MAX_SCREENSHOT_EDGE: u32 = 1568;
/// Bounds how long the hotkey can delay the panel.
const SELECTION_TIMEOUT: Duration = Duration::from_millis(400);

pub struct PendingContext {
    seq: u64,
    window: WindowInfo,
    selection: Option<String>,
    window_text: Option<(String, bool)>,
    screenshot: Option<Screenshot>,
    screenshot_note: Option<&'static str>,
    capturing: bool,
}

fn app_name(w: &WindowInfo) -> Option<&str> {
    w.process_path
        .as_deref()
        .and_then(|p| p.rsplit(['\\', '/']).next())
}

/// Captures the foreground window. Call on the main thread before showing
/// the panel: the selection is read from whatever has keyboard focus.
/// Returns false, capturing nothing, when the foreground window is DeskHand.
pub fn capture_foreground(app: &AppHandle) -> bool {
    let Some(info) = window::foreground().and_then(|id| window::window(id).ok()) else {
        return false;
    };
    if info.pid == std::process::id() {
        return false;
    }
    let selection = uia::read_focused_selection(MAX_SELECTION_CHARS, SELECTION_TIMEOUT);

    let state = app.state::<AppState>();
    let seq = state.capture_seq.fetch_add(1, Ordering::SeqCst) + 1;
    *lock(&state.pending_context) = Some(PendingContext {
        seq,
        window: info.clone(),
        selection,
        window_text: None,
        screenshot: None,
        screenshot_note: None,
        capturing: true,
    });
    let _ = app.emit(CONTEXT_EVENT, ());

    // Window text and the screenshot take longer; finish them after the
    // panel is up. Neither depends on focus.
    let app = app.clone();
    std::thread::spawn(move || {
        let limits = TextLimits {
            max_chars: MAX_WINDOW_TEXT_CHARS,
            ..TextLimits::default()
        };
        let text = uia::read_text(info.id, limits)
            .ok()
            .filter(|t| !t.text.trim().is_empty())
            .map(|t| (t.text, t.truncated));
        let (screenshot, note) = match capture::capture_window(info.id, MAX_SCREENSHOT_EDGE) {
            Ok(Ok(s)) => (Some(s), None),
            Ok(Err(NoScreenshot::Blank)) => (None, Some("protected or blank")),
            Ok(Err(NoScreenshot::Minimized)) => (None, Some("window is minimized")),
            Err(_) => (None, Some("capture failed")),
        };

        let state = app.state::<AppState>();
        // A newer hotkey press replaces this capture; drop stale results.
        if let Some(p) = lock(&state.pending_context)
            .as_mut()
            .filter(|p| p.seq == seq)
        {
            p.window_text = text;
            p.screenshot = screenshot;
            p.screenshot_note = note;
            p.capturing = false;
        }
        let _ = app.emit(CONTEXT_EVENT, ());
    });
    true
}

#[derive(Serialize)]
struct Preview {
    chars: usize,
    preview: String,
    truncated: bool,
}

fn preview(s: &str, truncated: bool) -> Preview {
    Preview {
        chars: s.chars().count(),
        preview: s.chars().take(200).collect(),
        truncated,
    }
}

#[derive(Serialize)]
struct ShotView {
    data_url: String,
    width: u32,
    height: u32,
}

#[derive(Serialize)]
pub struct ContextView {
    window: String,
    app: Option<String>,
    selection: Option<Preview>,
    window_text: Option<Preview>,
    screenshot: Option<ShotView>,
    screenshot_note: Option<&'static str>,
    capturing: bool,
}

#[tauri::command]
pub fn context(state: State<'_, AppState>) -> Option<ContextView> {
    let pending = lock(&state.pending_context);
    let p = pending.as_ref()?;
    Some(ContextView {
        window: p.window.title.clone(),
        app: app_name(&p.window).map(str::to_owned),
        selection: p.selection.as_deref().map(|s| preview(s, false)),
        window_text: p.window_text.as_ref().map(|(t, cut)| preview(t, *cut)),
        screenshot: p.screenshot.as_ref().map(|s| ShotView {
            data_url: format!("data:image/png;base64,{}", s.base64()),
            width: s.width,
            height: s.height,
        }),
        screenshot_note: p.screenshot_note,
        capturing: p.capturing,
    })
}

#[tauri::command]
pub fn dismiss_context(app: AppHandle, state: State<'_, AppState>) {
    *lock(&state.pending_context) = None;
    let _ = app.emit(CONTEXT_EVENT, ());
}

/// Which captured parts the user left attached.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Include {
    selection: bool,
    window_text: bool,
    screenshot: bool,
}

/// Takes the pending context for a message, keeping only the parts the user
/// left attached. With no `include`, the pending context stays untouched.
pub fn take(app: &AppHandle, state: &AppState, include: Option<Include>) -> Option<ScreenContext> {
    let include = include?;
    let p = lock(&state.pending_context).take()?;
    let _ = app.emit(CONTEXT_EVENT, ());
    let window = match app_name(&p.window) {
        Some(a) => format!("{} ({a})", p.window.title),
        None => p.window.title.clone(),
    };
    Some(ScreenContext {
        window,
        selection: p.selection.filter(|_| include.selection),
        window_text: p
            .window_text
            .filter(|_| include.window_text)
            .map(|(t, _)| t),
        screenshot: p
            .screenshot
            .filter(|_| include.screenshot)
            .map(|s| ImageData {
                media_type: "image/png".into(),
                base64: s.base64(),
            }),
    })
}
