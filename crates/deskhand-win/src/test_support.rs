//! Test helpers for other crates. Enabled by the `test-support` feature.

use std::marker::PhantomData;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, WINDOW_EX_STYLE, WNDCLASSW,
    WS_OVERLAPPEDWINDOW,
};
use windows::core::w;

use crate::error::{Context, Result};
use crate::window::WindowId;

/// A hidden top-level window owned by the current process, destroyed on drop.
///
/// Not `Send`: a window must be destroyed on the thread that created it.
pub struct TestWindow {
    hwnd: HWND,
    _not_send: PhantomData<*const ()>,
}

impl TestWindow {
    pub fn create_hidden() -> Result<Self> {
        let class = w!("DeskHandTestSupportWindow");
        // SAFETY: plain Win32 setup with valid, 'static arguments; `w!` yields
        // static NUL-terminated wide strings. RegisterClassW failing because
        // the class already exists is harmless and ignored.
        let hwnd = unsafe {
            let instance = GetModuleHandleW(None).ctx("GetModuleHandleW")?;
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassW(&wc);
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                w!("DeskHand test window"),
                WS_OVERLAPPEDWINDOW,
                100,
                100,
                400,
                300,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .ctx("CreateWindowExW")?
        };
        Ok(Self {
            hwnd,
            _not_send: PhantomData,
        })
    }

    pub fn id(&self) -> WindowId {
        WindowId::from_raw(self.hwnd.0 as isize)
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        // SAFETY: `hwnd` was created on this thread (the type is not Send) and
        // is not used after this.
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}

unsafe extern "system" fn wndproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: forwards the arguments Windows passed to this window procedure.
    unsafe { DefWindowProcW(h, msg, wp, lp) }
}
