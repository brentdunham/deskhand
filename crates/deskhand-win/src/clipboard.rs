//! Clipboard text access.
//!
//! Content that password managers mark as sensitive (the standard
//! `ExcludeClipboardContentFromMonitorProcessing` format) is never read.

use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::core::w;

use crate::error::{Context, Result, WinError};

/// What the clipboard holds, from DeskHand's point of view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardContent {
    Text(String),
    /// Empty, or holding only non-text data (an image, files).
    NoText,
    /// Marked sensitive by the application that set it; not read.
    Sensitive,
}

fn text_format() -> u32 {
    u32::from(CF_UNICODETEXT.0)
}

/// Another application can hold the clipboard open briefly; retry for up to
/// about half a second.
struct Open;

impl Open {
    fn new() -> Result<Self> {
        let mut last = None;
        for _ in 0..25 {
            // SAFETY: no memory preconditions; `None` associates the
            // clipboard with the current task.
            match unsafe { OpenClipboard(None) } {
                Ok(()) => return Ok(Self),
                Err(e) => last = Some(e),
            }
            thread::sleep(Duration::from_millis(20));
        }
        Err(WinError::Api {
            op: "OpenClipboard (another application is using the clipboard)",
            source: last.expect("loop ran"),
        })
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: pairs with the successful OpenClipboard in `new`.
        let _ = unsafe { CloseClipboard() };
    }
}

fn is_sensitive() -> bool {
    // SAFETY: registering a named format has no memory preconditions; the
    // name is a static wide string. Requires the clipboard to be open.
    unsafe {
        let format = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
        format != 0 && IsClipboardFormatAvailable(format).is_ok()
    }
}

/// Reads up to `max_chars` UTF-16 code units of clipboard text. Returns the
/// text and whether it was cut short.
pub fn read(max_chars: usize) -> Result<(ClipboardContent, bool)> {
    let _open = Open::new()?;
    if is_sensitive() {
        return Ok((ClipboardContent::Sensitive, false));
    }
    // SAFETY: the clipboard is open for the guard's lifetime. The data handle
    // is owned by the clipboard and only read between GlobalLock and
    // GlobalUnlock; the slice is bounded by GlobalSize, so a missing NUL
    // cannot read past the allocation.
    unsafe {
        if IsClipboardFormatAvailable(text_format()).is_err() {
            return Ok((ClipboardContent::NoText, false));
        }
        let handle = GetClipboardData(text_format()).ctx("GetClipboardData")?;
        let global = HGLOBAL(handle.0);
        let units = GlobalSize(global) / 2;
        let ptr = GlobalLock(global) as *const u16;
        if ptr.is_null() {
            return Err(WinError::Api {
                op: "GlobalLock",
                source: windows::core::Error::from_thread(),
            });
        }
        let all = std::slice::from_raw_parts(ptr, units);
        let len = all.iter().position(|&c| c == 0).unwrap_or(all.len());
        let truncated = len > max_chars;
        let text = String::from_utf16_lossy(&all[..len.min(max_chars)]);
        let _ = GlobalUnlock(global);
        Ok((ClipboardContent::Text(text), truncated))
    }
}

/// Replaces the clipboard contents with `text`.
pub fn write(text: &str) -> Result<()> {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let _open = Open::new()?;
    // SAFETY: the clipboard is open for the guard's lifetime. The allocation
    // is sized for `wide`, written only between GlobalLock and GlobalUnlock,
    // and freed here unless SetClipboardData succeeds, after which the
    // clipboard owns it.
    unsafe {
        EmptyClipboard().ctx("EmptyClipboard")?;
        let global = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).ctx("GlobalAlloc")?;
        let ptr = GlobalLock(global) as *mut u16;
        if ptr.is_null() {
            let _ = GlobalFree(Some(global));
            return Err(WinError::Api {
                op: "GlobalLock",
                source: windows::core::Error::from_thread(),
            });
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
        let _ = GlobalUnlock(global);
        if let Err(e) = SetClipboardData(text_format(), Some(HANDLE(global.0))) {
            let _ = GlobalFree(Some(global));
            return Err(WinError::Api {
                op: "SetClipboardData",
                source: e,
            });
        }
    }
    Ok(())
}
