//! Monitor enumeration.

use std::ffi::c_void;

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;
use windows::core::BOOL;

use crate::error::{Result, WinError};
use crate::geometry::Rect;

/// Opaque monitor identifier (an `HMONITOR` value). Valid until the display
/// configuration changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MonitorId(isize);

impl MonitorId {
    pub fn raw(self) -> isize {
        self.0
    }

    pub fn from_raw(raw: isize) -> Self {
        Self(raw)
    }

    pub(crate) fn from_handle(h: HMONITOR) -> Self {
        Self(h.0 as isize)
    }

    pub(crate) fn handle(self) -> HMONITOR {
        HMONITOR(self.0 as *mut c_void)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorInfo {
    pub id: MonitorId,
    /// Device name, e.g. `\\.\DISPLAY1`.
    pub device: String,
    pub bounds: Rect,
    /// Bounds minus the taskbar and docked app bars.
    pub work_area: Rect,
    pub primary: bool,
}

pub fn list_monitors() -> Result<Vec<MonitorInfo>> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    // SAFETY: `collect_monitor` runs only during this call, and `lparam` points
    // to `handles`, which outlives the enumeration.
    let ok = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM(&raw mut handles as isize),
        )
    };
    if !ok.as_bool() {
        return Err(WinError::Api {
            op: "EnumDisplayMonitors",
            source: windows::core::Error::from_thread(),
        });
    }
    handles.into_iter().map(info_for).collect()
}

pub fn monitor(id: MonitorId) -> Result<MonitorInfo> {
    info_for(id.handle())
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _hdc: HDC,
    _clip: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    // SAFETY: `lparam` is the `&mut Vec<HMONITOR>` passed by `list_monitors`,
    // valid and exclusively borrowed for the duration of the enumeration.
    let handles = unsafe { &mut *(lparam.0 as *mut Vec<HMONITOR>) };
    handles.push(monitor);
    true.into()
}

fn info_for(h: HMONITOR) -> Result<MonitorInfo> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: MONITORINFOEXW starts with MONITORINFO, and `cbSize` tells the API
    // the extended struct is available to write into.
    let ok = unsafe { GetMonitorInfoW(h, (&raw mut info).cast::<MONITORINFO>()) };
    if !ok.as_bool() {
        return Err(WinError::Api {
            op: "GetMonitorInfoW",
            source: windows::core::Error::from_thread(),
        });
    }
    let name_len = info
        .szDevice
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(info.szDevice.len());
    Ok(MonitorInfo {
        id: MonitorId::from_handle(h),
        device: String::from_utf16_lossy(&info.szDevice[..name_len]),
        bounds: info.monitorInfo.rcMonitor.into(),
        work_area: info.monitorInfo.rcWork.into(),
        primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
    })
}

impl From<RECT> for Rect {
    fn from(r: RECT) -> Self {
        Rect::new(r.left, r.top, r.right, r.bottom)
    }
}

impl From<Rect> for RECT {
    fn from(r: Rect) -> Self {
        RECT {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        }
    }
}
