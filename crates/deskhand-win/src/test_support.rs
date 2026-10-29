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

/// A hidden top-level window holding a plain edit field and a password edit
/// field, owned by a dedicated thread that runs a message loop. UI Automation
/// reads Win32 edit fields by sending them window messages, so the owning
/// thread must be pumping while another thread reads it.
pub struct TestTextWindow {
    hwnd: isize,
    thread_id: u32,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

impl TestTextWindow {
    pub fn spawn(plain: &str, secret: &str) -> Result<Self> {
        use windows::Win32::System::Threading::GetCurrentThreadId;
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, ES_PASSWORD, GetMessageW, MSG, TranslateMessage, WINDOW_STYLE,
            WS_CHILD, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_VISIBLE,
        };
        use windows::core::PCWSTR;

        let (plain, secret) = (wide(plain), wide(secret));
        let (tx, rx) = std::sync::mpsc::channel::<Result<(isize, u32)>>();
        let thread = std::thread::spawn(move || {
            // SAFETY: standard Win32 window creation and message loop on this
            // thread. The wide strings outlive the calls that read them, and
            // every handle is used only on this thread.
            unsafe {
                let created = (|| -> Result<HWND> {
                    let instance = GetModuleHandleW(None).ctx("GetModuleHandleW")?;
                    let class = w!("DeskHandTestTextWindow");
                    RegisterClassW(&WNDCLASSW {
                        lpfnWndProc: Some(wndproc),
                        hInstance: instance.into(),
                        lpszClassName: class,
                        ..Default::default()
                    });
                    // Visible (UI Automation does not expose the children of
                    // hidden windows) but far off-screen, and a tool window so
                    // it never appears in the taskbar or Alt+Tab.
                    let parent = CreateWindowExW(
                        WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                        class,
                        w!("DeskHand text test window"),
                        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                        -32000,
                        -32000,
                        400,
                        200,
                        None,
                        None,
                        Some(instance.into()),
                        None,
                    )
                    .ctx("CreateWindowExW(parent)")?;
                    for (text, extra, y) in [
                        (&plain, WINDOW_STYLE(0), 10),
                        (&secret, WINDOW_STYLE(ES_PASSWORD as u32), 50),
                    ] {
                        CreateWindowExW(
                            WINDOW_EX_STYLE(0),
                            w!("EDIT"),
                            PCWSTR(text.as_ptr()),
                            WS_CHILD | WS_VISIBLE | extra,
                            10,
                            y,
                            300,
                            24,
                            Some(parent),
                            None,
                            Some(instance.into()),
                            None,
                        )
                        .ctx("CreateWindowExW(edit)")?;
                    }
                    Ok(parent)
                })();
                let parent = match created {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        return;
                    }
                };
                let _ = tx.send(Ok((parent.0 as isize, GetCurrentThreadId())));
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                let _ = DestroyWindow(parent);
            }
        });
        let (hwnd, thread_id) = rx.recv().expect("test window thread exited early")?;
        Ok(Self {
            hwnd,
            thread_id,
            thread: Some(thread),
        })
    }

    pub fn id(&self) -> WindowId {
        WindowId::from_raw(self.hwnd)
    }
}

impl Drop for TestTextWindow {
    fn drop(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
        // SAFETY: posts to a thread id obtained from that thread; no memory
        // is shared through the message.
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
