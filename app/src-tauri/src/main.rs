// Release builds run without a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod host;
mod secrets;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use deskhand_agent::{HttpClient, Session};
use deskhand_tools::{Toolbox, window_tools};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::host::Confirmations;

pub const HOTKEY_LABEL: &str = "Ctrl+Alt+Space";

fn hotkey() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Space)
}

pub struct AppState {
    /// Shared with each turn's agent; the UI reads the log and runs undo.
    pub toolbox: Arc<Mutex<Toolbox>>,
    /// Held by the running turn for its whole duration.
    pub session: Arc<tokio::sync::Mutex<Session>>,
    pub http: HttpClient,
    pub busy: Arc<AtomicBool>,
    pub turn: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    pub confirmations: Arc<Confirmations>,
    pub hotkey_registered: AtomicBool,
}

pub(crate) fn show_panel(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn toggle_panel(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let visible = w.is_visible().unwrap_or(false);
        let focused = w.is_focused().unwrap_or(false);
        if visible && focused {
            let _ = w.hide();
        } else {
            show_panel(app);
        }
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show DeskHand", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit DeskHand", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut tray = TrayIconBuilder::with_id("deskhand")
        .tooltip("DeskHand")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_panel(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_panel(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn main() {
    let mut toolbox = Toolbox::new();
    window_tools::register(&mut toolbox).expect("built-in tool names are unique");

    let state = AppState {
        toolbox: Arc::new(Mutex::new(toolbox)),
        session: Arc::default(),
        http: deskhand_agent::http_client().expect("failed to build HTTP client"),
        busy: Arc::default(),
        turn: Mutex::new(None),
        confirmations: Arc::default(),
        hotkey_registered: AtomicBool::new(false),
    };

    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed && *shortcut == hotkey() {
                        toggle_panel(app);
                    }
                })
                .build(),
        )
        .manage(state)
        .setup(|app| {
            // Another app may own the hotkey. DeskHand still works from the
            // tray; the UI shows a warning.
            match app.global_shortcut().register(hotkey()) {
                Ok(()) => app
                    .state::<AppState>()
                    .hotkey_registered
                    .store(true, Ordering::SeqCst),
                Err(e) => eprintln!("could not register {HOTKEY_LABEL}: {e}"),
            }
            build_tray(app.handle())?;
            Ok(())
        })
        // Closing the panel hides it; quit from the tray.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::status,
            commands::set_api_key,
            commands::clear_api_key,
            commands::send_message,
            commands::cancel_turn,
            commands::answer_confirmation,
            commands::action_log,
            commands::undo_action,
            commands::new_conversation,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start DeskHand");
}
