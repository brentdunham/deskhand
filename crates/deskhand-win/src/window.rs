//! Top-level window enumeration and placement.
//!
//! Every mutating function returns the window's prior [`Placement`], which is
//! the undo data for the action log: pass it to [`restore_placement`] to undo.
//!
//! Window titles come from other applications and are untrusted content.

use std::ffi::c_void;

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GetClassNameW, GetWindow, GetWindowLongPtrW,
    GetWindowPlacement, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, IsZoomed, SET_WINDOW_POS_FLAGS,
    SHOW_WINDOW_CMD, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SW_SHOWMAXIMIZED, SW_SHOWMINNOACTIVE,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOZORDER, SetForegroundWindow, SetWindowPlacement,
    SetWindowPos, ShowWindow, WINDOWPLACEMENT, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, PWSTR};

use crate::error::{Context, Result, WinError};
pub use crate::geometry::{Placement, ShowState};
use crate::geometry::{Rect, SnapTarget, outer_rect_for_frame, snap_rect};
use crate::monitor::{self, MonitorId};

/// Opaque window identifier (an `HWND` value). Becomes invalid when the
/// window closes; functions then return [`WinError::WindowGone`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WindowId(isize);

impl WindowId {
    pub fn raw(self) -> isize {
        self.0
    }

    pub fn from_raw(raw: isize) -> Self {
        Self(raw)
    }

    fn from_hwnd(h: HWND) -> Self {
        Self(h.0 as isize)
    }

    pub(crate) fn hwnd(self) -> HWND {
        HWND(self.0 as *mut c_void)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: WindowId,
    /// Untrusted: set by the owning application.
    pub title: String,
    pub class_name: String,
    pub pid: u32,
    /// Full executable path, when the process can be queried.
    pub process_path: Option<String>,
    /// Visible frame, excluding invisible resize borders.
    pub frame: Rect,
    pub monitor: MonitorId,
    pub state: ShowState,
}

/// Windows that appear in Alt+Tab, in Z order (topmost first).
pub fn list_windows() -> Result<Vec<WindowInfo>> {
    let mut hwnds: Vec<HWND> = Vec::new();
    // SAFETY: `collect_hwnd` runs only during this call, and `lparam` points to
    // `hwnds`, which outlives the enumeration.
    unsafe { EnumWindows(Some(collect_hwnd), LPARAM(&raw mut hwnds as isize)) }
        .ctx("EnumWindows")?;
    Ok(hwnds
        .into_iter()
        .filter(|&h| is_alt_tab_window(h))
        // A window can close between enumeration and inspection; skip it.
        .filter_map(|h| info_for(h).ok())
        .collect())
}

pub fn window(id: WindowId) -> Result<WindowInfo> {
    info_for(live(id)?)
}

pub fn placement(id: WindowId) -> Result<Placement> {
    let h = live(id)?;
    let mut wp = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    // SAFETY: `wp` is a valid WINDOWPLACEMENT with `length` set.
    unsafe { GetWindowPlacement(h, &mut wp) }.ctx("GetWindowPlacement")?;
    Ok(Placement {
        state: show_state(h),
        normal: wp.rcNormalPosition.into(),
    })
}

/// Restores a placement captured earlier. Does not activate the window unless
/// it is restored to maximized.
pub fn restore_placement(id: WindowId, p: &Placement) -> Result<()> {
    let h = live(id)?;
    let show = match p.state {
        ShowState::Normal => SW_SHOWNOACTIVATE,
        ShowState::Minimized => SW_SHOWMINNOACTIVE,
        ShowState::Maximized => SW_SHOWMAXIMIZED,
    };
    let wp = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        showCmd: show.0 as u32,
        ptMinPosition: POINT { x: -1, y: -1 },
        ptMaxPosition: POINT { x: -1, y: -1 },
        rcNormalPosition: p.normal.into(),
        ..Default::default()
    };
    // SAFETY: `wp` is a fully initialized WINDOWPLACEMENT with `length` set.
    unsafe { SetWindowPlacement(h, &wp) }.ctx("SetWindowPlacement")
}

/// Brings the window to the foreground, restoring it first if minimized.
///
/// Windows only allows this when the calling process is in the foreground,
/// which holds when the user has just invoked DeskHand.
pub fn focus(id: WindowId) -> Result<()> {
    let h = live(id)?;
    if show_state(h) == ShowState::Minimized {
        show(h, SW_RESTORE);
    }
    // SAFETY: `h` was validated by `live`; the call has no memory preconditions.
    if unsafe { SetForegroundWindow(h) }.as_bool() {
        Ok(())
    } else {
        Err(WinError::ForegroundDenied(id))
    }
}

pub fn minimize(id: WindowId) -> Result<Placement> {
    let prior = placement(id)?;
    show(id.hwnd(), SW_MINIMIZE);
    Ok(prior)
}

/// Moves and resizes the window so its visible frame matches `frame`,
/// restoring it first if minimized or maximized.
pub fn set_frame(id: WindowId, frame: Rect) -> Result<Placement> {
    let prior = placement(id)?;
    let h = id.hwnd();
    if prior.state != ShowState::Normal {
        show(h, SW_RESTORE);
    }
    apply_frame(h, frame)?;
    // Moving to a monitor with a different DPI rescales the window and its
    // invisible borders after the first move. Measure again and correct.
    if let Ok(actual) = frame_rect(h)
        && actual != frame
    {
        apply_frame(h, frame)?;
    }
    Ok(prior)
}

/// Snaps the window within a monitor's work area. Uses the window's current
/// monitor when `monitor` is `None`.
pub fn snap(id: WindowId, target: SnapTarget, monitor: Option<MonitorId>) -> Result<Placement> {
    let h = live(id)?;
    let monitor_id = match monitor {
        Some(m) => m,
        // SAFETY: `h` was validated by `live`; the call has no memory preconditions.
        None => MonitorId::from_handle(unsafe { MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST) }),
    };
    let work = monitor::monitor(monitor_id)?.work_area;

    if target == SnapTarget::Maximize {
        // Move onto the target monitor first so the native maximize lands there.
        let prior = set_frame(id, snap_rect(work, SnapTarget::Center))?;
        show(h, SW_MAXIMIZE);
        return Ok(prior);
    }
    set_frame(id, snap_rect(work, target))
}

unsafe extern "system" fn collect_hwnd(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: `lparam` is the `&mut Vec<HWND>` passed by `list_windows`, valid
    // and exclusively borrowed for the duration of the enumeration.
    let hwnds = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    hwnds.push(hwnd);
    true.into()
}

/// Approximates the Alt+Tab filter: visible, titled, not a tool window, not
/// owned (unless it opts in with WS_EX_APPWINDOW), and not cloaked by DWM
/// (suspended UWP apps and windows on other virtual desktops are cloaked).
fn is_alt_tab_window(h: HWND) -> bool {
    // SAFETY: these calls read state for a window handle and have no memory
    // preconditions; an invalid handle yields false, 0, or an error.
    let (visible, title_len, ex_style, owned) = unsafe {
        (
            IsWindowVisible(h).as_bool(),
            GetWindowTextLengthW(h),
            GetWindowLongPtrW(h, GWL_EXSTYLE) as u32,
            GetWindow(h, GW_OWNER).is_ok_and(|o| !o.is_invalid()),
        )
    };
    if !visible || title_len == 0 || ex_style & WS_EX_TOOLWINDOW.0 != 0 {
        return false;
    }
    if owned && ex_style & WS_EX_APPWINDOW.0 == 0 {
        return false;
    }
    !is_cloaked(h)
}

fn is_cloaked(h: HWND) -> bool {
    let mut cloaked: u32 = 0;
    // SAFETY: the pointer and size describe `cloaked`, a u32, which is the
    // type DWMWA_CLOAKED writes.
    let r = unsafe {
        DwmGetWindowAttribute(
            h,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast::<c_void>(),
            size_of::<u32>() as u32,
        )
    };
    r.is_ok() && cloaked != 0
}

/// Returns the handle if the window still exists.
pub(crate) fn live(id: WindowId) -> Result<HWND> {
    let h = id.hwnd();
    // SAFETY: IsWindow accepts any value and has no memory preconditions.
    if unsafe { IsWindow(Some(h)) }.as_bool() {
        Ok(h)
    } else {
        Err(WinError::WindowGone(id))
    }
}

fn info_for(h: HWND) -> Result<WindowInfo> {
    let mut pid = 0u32;
    // SAFETY: `pid` is a valid out pointer for the duration of the call.
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    Ok(WindowInfo {
        id: WindowId::from_hwnd(h),
        title: title(h),
        class_name: class_name(h),
        pid,
        process_path: process_path(pid),
        frame: frame_rect(h)?,
        // SAFETY: no memory preconditions.
        monitor: MonitorId::from_handle(unsafe { MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST) }),
        state: show_state(h),
    })
}

fn show_state(h: HWND) -> ShowState {
    // SAFETY: no memory preconditions.
    let (iconic, zoomed) = unsafe { (IsIconic(h).as_bool(), IsZoomed(h).as_bool()) };
    if iconic {
        ShowState::Minimized
    } else if zoomed {
        ShowState::Maximized
    } else {
        ShowState::Normal
    }
}

fn show(h: HWND, cmd: SHOW_WINDOW_CMD) {
    // SAFETY: no memory preconditions. The return value is the previous
    // visibility, not an error indicator, so it is ignored.
    let _ = unsafe { ShowWindow(h, cmd) };
}

fn apply_frame(h: HWND, frame: Rect) -> Result<()> {
    let outer = outer_rect_for_frame(frame, window_rect(h)?, frame_rect(h)?);
    let flags: SET_WINDOW_POS_FLAGS = SWP_NOZORDER | SWP_NOACTIVATE;
    // SAFETY: no memory preconditions.
    unsafe {
        SetWindowPos(
            h,
            None,
            outer.left,
            outer.top,
            outer.width(),
            outer.height(),
            flags,
        )
    }
    .ctx("SetWindowPos")
}

fn window_rect(h: HWND) -> Result<Rect> {
    let mut r = RECT::default();
    // SAFETY: `r` is a valid out pointer.
    unsafe { GetWindowRect(h, &mut r) }.ctx("GetWindowRect")?;
    Ok(r.into())
}

/// The visible frame, falling back to the window rect if DWM has no answer.
fn frame_rect(h: HWND) -> Result<Rect> {
    let mut r = RECT::default();
    // SAFETY: the pointer and size describe `r`, a RECT, which is the type
    // DWMWA_EXTENDED_FRAME_BOUNDS writes.
    let dwm = unsafe {
        DwmGetWindowAttribute(
            h,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&raw mut r).cast::<c_void>(),
            size_of::<RECT>() as u32,
        )
    };
    match dwm {
        Ok(()) => Ok(r.into()),
        Err(_) => window_rect(h),
    }
}

fn title(h: HWND) -> String {
    // SAFETY: no memory preconditions.
    let len = unsafe { GetWindowTextLengthW(h) };
    let Ok(len) = usize::try_from(len) else {
        return String::new();
    };
    let mut buf = vec![0u16; len + 1];
    // SAFETY: `buf` is a writable buffer; the API writes at most `buf.len()` units.
    let n = unsafe { GetWindowTextW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..usize::try_from(n).unwrap_or(0)])
}

fn class_name(h: HWND) -> String {
    // Window class names are limited to 256 characters.
    let mut buf = [0u16; 257];
    // SAFETY: `buf` is a writable buffer; the API writes at most `buf.len()` units.
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..usize::try_from(n).unwrap_or(0)])
}

fn process_path(pid: u32) -> Option<String> {
    // SAFETY: no memory preconditions. The returned handle is closed below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: `buf` and `len` describe a writable buffer; `handle` is open.
    let r = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    };
    // SAFETY: `handle` was opened above and is not used after this.
    let _ = unsafe { CloseHandle(handle) };
    r.ok()?;
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}
