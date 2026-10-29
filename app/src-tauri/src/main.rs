// Release builds run without a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod host;
mod secrets;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};
use std::time::Duration;

use deskhand_agent::{HttpClient, Session};
use deskhand_config::{Config, ConfigError, Watcher};
use deskhand_tools::file_tools::{self, SearchRoots};
use deskhand_tools::{Toolbox, layout_tools, system_tools, window_tools};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::host::Confirmations;

/// Emitted whenever the config reloads or fails to; the UI re-reads status.
pub const CONFIG_EVENT: &str = "config-changed";

pub struct AppState {
    /// Shared with each turn's agent; the UI reads the log and runs undo.
    pub toolbox: Arc<Mutex<Toolbox>>,
    /// Held by the running turn for its whole duration.
    pub session: Arc<tokio::sync::Mutex<Session>>,
    pub http: HttpClient,
    pub busy: Arc<AtomicBool>,
    pub turn: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    pub confirmations: Arc<Confirmations>,

    /// The last config that loaded successfully (defaults until then).
    pub config: RwLock<Config>,
    pub config_path: OnceLock<PathBuf>,
    /// Set while the file on disk fails to load; cleared on the next good load.
    pub config_error: Mutex<Option<String>>,
    pub config_warnings: Mutex<Vec<String>>,
    /// Problems found at startup outside the config file (e.g. an unreadable
    /// layouts store). Shown alongside config warnings.
    pub startup_warnings: Mutex<Vec<String>>,
    /// Shared with the `search_files` tool; replaced on each config load.
    pub search_roots: SearchRoots,
    active_hotkey: Mutex<Option<Shortcut>>,
    pub hotkey_registered: AtomicBool,
    watcher: Mutex<Option<Watcher>>,
}

/// Poison only means a panic elsewhere; the data is still usable.
pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
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

/// Applies a config: tool overrides, then the hotkey (re-registered only if
/// it changed or is not currently registered, so saving the file after
/// closing a conflicting app retries registration).
///
/// Must run on the main thread: the hotkey backend is tied to it.
fn apply_config(app: &AppHandle, config: Config) {
    let state = app.state::<AppState>();

    let mut warnings: Vec<String> = {
        let mut toolbox = lock(&state.toolbox);
        toolbox.policy_mut().replace_overrides(config.tools.clone());
        let known: Vec<String> = toolbox.definitions().into_iter().map(|d| d.name).collect();
        config
            .unknown_tools(&known)
            .into_iter()
            .map(|t| format!("[tools] has an override for `{t}`, which is not a DeskHand tool."))
            .collect()
    };

    match config.hotkey.accelerator().parse::<Shortcut>() {
        Ok(wanted) => {
            let mut active = lock(&state.active_hotkey);
            let registered = state.hotkey_registered.load(Ordering::SeqCst);
            if *active != Some(wanted) || !registered {
                let shortcuts = app.global_shortcut();
                if let Some(old) = active.take() {
                    let _ = shortcuts.unregister(old);
                }
                match shortcuts.register(wanted) {
                    Ok(()) => {
                        *active = Some(wanted);
                        state.hotkey_registered.store(true, Ordering::SeqCst);
                    }
                    Err(e) => {
                        eprintln!("could not register {}: {e}", config.hotkey);
                        state.hotkey_registered.store(false, Ordering::SeqCst);
                    }
                }
            }
        }
        // The config crate only produces accelerators the plugin accepts
        // (covered by a test), so this indicates a bug, not a user error.
        Err(e) => warnings.push(format!(
            "internal error: hotkey {} did not parse: {e}",
            config.hotkey
        )),
    }

    // Search roots: empty means the standard user folders. Invalid entries
    // are skipped with a warning rather than failing the whole config.
    let roots = if config.search.roots.is_empty() {
        deskhand_win::shell::default_search_roots()
    } else {
        let home = deskhand_win::shell::home_folder();
        config
            .search
            .roots
            .iter()
            .filter_map(|raw| {
                deskhand_tools::file_search::resolve_root(raw, home.as_deref())
                    .map_err(|e| warnings.push(format!("[search] {e}")))
                    .ok()
            })
            .collect()
    };
    *state
        .search_roots
        .write()
        .unwrap_or_else(PoisonError::into_inner) = roots;

    *lock(&state.config_warnings) = warnings;
    *state.config.write().unwrap_or_else(PoisonError::into_inner) = config;
}

/// Handles a load or reload. On error the previous config stays in effect.
fn config_loaded(app: &AppHandle, result: Result<Config, ConfigError>) {
    let state = app.state::<AppState>();
    match result {
        Ok(config) => {
            apply_config(app, config);
            *lock(&state.config_error) = None;
        }
        Err(e) => *lock(&state.config_error) = Some(e.to_string()),
    }
    let _ = app.emit(CONFIG_EVENT, ());
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
    let search_roots = SearchRoots::default();
    window_tools::register(&mut toolbox).expect("built-in tool names are unique");
    system_tools::register(&mut toolbox).expect("built-in tool names are unique");

    let state = AppState {
        toolbox: Arc::new(Mutex::new(toolbox)),
        session: Arc::default(),
        http: deskhand_agent::http_client().expect("failed to build HTTP client"),
        busy: Arc::default(),
        turn: Mutex::new(None),
        confirmations: Arc::default(),
        config: RwLock::new(Config::default()),
        config_path: OnceLock::new(),
        config_error: Mutex::new(None),
        config_warnings: Mutex::new(Vec::new()),
        startup_warnings: Mutex::new(Vec::new()),
        search_roots,
        active_hotkey: Mutex::new(None),
        hotkey_registered: AtomicBool::new(false),
        watcher: Mutex::new(None),
    };

    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                // Only DeskHand's one hotkey is ever registered.
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_panel(app);
                    }
                })
                .build(),
        )
        .manage(state)
        .setup(|app| {
            let handle = app.handle().clone();
            let state = app.state::<AppState>();

            // Layouts are app-written data, kept out of the config directory
            // (%APPDATA%) in %LOCALAPPDATA%. Registered before the config is
            // applied so overrides for layout tools are recognized.
            let layouts_path = app.path().app_local_data_dir()?.join("layouts.json");
            if let Some(w) = layout_tools::register(&mut lock(&state.toolbox), layouts_path)? {
                lock(&state.startup_warnings).push(w);
            }

            // File tools never touch DeskHand's own directories, even when a
            // configured root contains them: the config decides where API
            // keys are sent.
            let protected = vec![
                app.path().app_config_dir()?,
                app.path().app_data_dir()?,
                app.path().app_local_data_dir()?,
            ];
            file_tools::register(
                &mut lock(&state.toolbox),
                state.search_roots.clone(),
                protected,
            )?;

            let path = app
                .path()
                .app_config_dir()?
                .join(deskhand_config::FILE_NAME);
            let _ = state.config_path.set(path.clone());

            // Start watching before the first load so an edit made during
            // startup is not missed. Reloads hop to the main thread.
            let watch_app = handle.clone();
            let watcher = Watcher::spawn(path.clone(), Duration::from_secs(1), move |result| {
                let app = watch_app.clone();
                let _ = watch_app.run_on_main_thread(move || config_loaded(&app, result));
            });
            *lock(&state.watcher) = Some(watcher);

            // Defaults first, so the hotkey works even if the file is broken.
            apply_config(&handle, Config::default());
            config_loaded(&handle, Config::load_or_create(&path));

            build_tray(&handle)?;
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
            commands::open_config,
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

#[cfg(test)]
mod tests {
    use deskhand_config::Hotkey;
    use tauri_plugin_global_shortcut::Shortcut;

    /// Every key the config accepts must produce an accelerator the plugin
    /// parses, or a valid config could fail to register.
    #[test]
    fn every_configurable_key_parses_as_a_shortcut() {
        let keys = ('A'..='Z')
            .map(String::from)
            .chain((0..=9).map(|d| d.to_string()))
            .chain((1..=24).map(|n| format!("F{n}")))
            .chain(["Space".to_owned()]);
        for key in keys {
            for mods in ["Ctrl", "Alt", "Win", "Ctrl+Alt+Shift+Win"] {
                let text = format!("{mods}+{key}");
                let hotkey: Hotkey = text.parse().unwrap();
                assert!(
                    hotkey.accelerator().parse::<Shortcut>().is_ok(),
                    "{text} -> {} did not parse",
                    hotkey.accelerator()
                );
            }
        }
    }
}
