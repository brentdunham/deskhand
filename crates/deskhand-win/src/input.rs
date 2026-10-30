//! Pasting text into another application.

use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    VIRTUAL_KEY, VK_CONTROL, VK_MENU, VK_SHIFT, VK_V,
};

use crate::clipboard::{self, ClipboardContent};
use crate::error::{Result, WinError};
use crate::window::{self, WindowId};

/// How long to wait for the target window to come to the front.
const FOCUS_TIMEOUT: Duration = Duration::from_millis(800);
/// How long the target gets to read the clipboard before it is restored.
const RESTORE_AFTER: Duration = Duration::from_millis(700);

/// Puts `text` on the clipboard, brings `target` to the front, and sends
/// Ctrl+V. Keys are sent only once `target` is confirmed to be the
/// foreground window, so text is never typed into the wrong place. The
/// previous clipboard text, if any, is restored afterward.
///
/// Fails with [`WinError::ForegroundDenied`] if the window cannot be brought
/// forward; the text is then still on the clipboard for a manual paste.
/// Windows of elevated processes ignore synthetic input from a normal
/// process; the paste then silently does nothing.
pub fn paste_into(target: WindowId, text: &str) -> Result<()> {
    window::live(target)?;
    let previous = match clipboard::read(1_000_000) {
        Ok((ClipboardContent::Text(t), false)) => Some(t),
        _ => None,
    };
    clipboard::write(text)?;

    window::focus(target)?;
    let deadline = Instant::now() + FOCUS_TIMEOUT;
    while window::foreground() != Some(target) {
        if Instant::now() >= deadline {
            return Err(WinError::ForegroundDenied(target));
        }
        thread::sleep(Duration::from_millis(10));
    }
    // Give the target a moment to finish activating before input arrives.
    thread::sleep(Duration::from_millis(60));
    send_ctrl_v()?;

    if let Some(previous) = previous {
        thread::spawn(move || {
            thread::sleep(RESTORE_AFTER);
            let _ = clipboard::write(&previous);
        });
    }
    Ok(())
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send_ctrl_v() -> Result<()> {
    // Release Shift and Alt first, in case the user is still holding them
    // from the shortcut that triggered the paste: Ctrl+Shift+V and
    // Ctrl+Alt+V mean something else in many apps.
    let inputs = [
        key(VK_SHIFT, true),
        key(VK_MENU, true),
        key(VK_CONTROL, false),
        key(VK_V, false),
        key(VK_V, true),
        key(VK_CONTROL, true),
    ];
    // SAFETY: `inputs` is a valid slice of fully initialized INPUT structs
    // and the size argument matches the element type.
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err(WinError::Api {
            op: "SendInput",
            source: windows::core::Error::from_thread(),
        })
    }
}
