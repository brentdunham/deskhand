//! Exercises every mutating window function against a window this test
//! creates, so no user windows are touched. The window appears on screen
//! briefly, so the test is opt-in:
//!
//! ```text
//! cargo test -p deskhand-win --test placement -- --ignored
//! ```
#![cfg(windows)]

use deskhand_win::geometry::snap_rect;
use deskhand_win::monitor::monitor;
use deskhand_win::window::{
    ShowState, WindowId, minimize, placement, restore_placement, set_frame, snap, window,
};
use deskhand_win::{Rect, SnapTarget, WinError};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, WINDOW_EX_STYLE, WNDCLASSW,
    WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};
use windows::core::w;

unsafe extern "system" fn wndproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: forwards the arguments Windows passed to this window procedure.
    unsafe { DefWindowProcW(h, msg, wp, lp) }
}

fn create_test_window() -> HWND {
    // SAFETY: plain Win32 setup calls with valid, 'static arguments. The class
    // name and title are static wide strings from `w!`.
    unsafe {
        // Match the DPI mode of the real host (Tauri) so DWM frame bounds and
        // window rects use the same physical coordinates.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(None).expect("module handle");
        let class = w!("DeskHandPlacementTest");
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
            w!("DeskHand placement test"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            200,
            200,
            640,
            480,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .expect("create test window")
    }
}

fn work_area(id: WindowId) -> Rect {
    monitor(window(id).unwrap().monitor).unwrap().work_area
}

#[test]
#[ignore = "creates and moves a real window"]
fn placement_round_trips() {
    let hwnd = create_test_window();
    let id = WindowId::from_raw(hwnd.0 as isize);
    let original = placement(id).unwrap();
    let original_frame = window(id).unwrap().frame;
    assert_eq!(original.state, ShowState::Normal);

    // Snap: visible frame lands exactly on the computed half, and the returned
    // undo data is the prior placement.
    let work = work_area(id);
    let undo = snap(id, SnapTarget::LeftHalf, None).unwrap();
    assert_eq!(undo, original);
    assert_eq!(
        window(id).unwrap().frame,
        snap_rect(work, SnapTarget::LeftHalf)
    );

    // Explicit frame.
    let target = Rect::new(
        work.left + 50,
        work.top + 50,
        work.left + 850,
        work.top + 650,
    );
    set_frame(id, target).unwrap();
    assert_eq!(window(id).unwrap().frame, target);

    // Undo restores the original frame.
    restore_placement(id, &original).unwrap();
    assert_eq!(window(id).unwrap().frame, original_frame);

    // Minimize and restore.
    let undo = minimize(id).unwrap();
    assert_eq!(window(id).unwrap().state, ShowState::Minimized);
    restore_placement(id, &undo).unwrap();
    assert_eq!(window(id).unwrap().state, ShowState::Normal);
    assert_eq!(window(id).unwrap().frame, original_frame);

    // Maximize and restore.
    let undo = snap(id, SnapTarget::Maximize, None).unwrap();
    assert_eq!(window(id).unwrap().state, ShowState::Maximized);
    restore_placement(id, &undo).unwrap();
    assert_eq!(window(id).unwrap().state, ShowState::Normal);
    assert_eq!(window(id).unwrap().frame, original_frame);

    // SAFETY: `hwnd` was created on this thread and is not used afterward.
    unsafe { DestroyWindow(hwnd) }.unwrap();
    assert!(matches!(window(id), Err(WinError::WindowGone(_))));
}
