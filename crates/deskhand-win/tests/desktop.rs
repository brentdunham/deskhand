//! Read-only checks against the live desktop. These never move or focus
//! windows, so they are safe to run on a developer machine and in CI.
#![cfg(windows)]

use deskhand_win::WinError;
use deskhand_win::monitor::{list_monitors, monitor};
use deskhand_win::window::{WindowId, list_windows, placement, window};

#[test]
fn monitors_have_exactly_one_primary_and_sane_work_areas() {
    let monitors = list_monitors().expect("enumerate monitors");
    assert!(!monitors.is_empty());
    assert_eq!(monitors.iter().filter(|m| m.primary).count(), 1);
    for m in &monitors {
        assert!(m.bounds.width() > 0 && m.bounds.height() > 0);
        assert!(m.bounds.contains(&m.work_area), "{m:?}");
        assert_eq!(monitor(m.id).expect("lookup by id"), *m);
    }
}

#[test]
fn listed_windows_are_titled_and_queryable() {
    let windows = list_windows().expect("enumerate windows");
    for w in &windows {
        assert!(!w.title.is_empty());
        // A window may close between the two calls; that is the only
        // acceptable error.
        match window(w.id) {
            Ok(again) => assert_eq!(again.id, w.id),
            Err(WinError::WindowGone(_)) => {}
            Err(e) => panic!("unexpected error for {:?}: {e}", w.title),
        }
        match placement(w.id) {
            Ok(_) | Err(WinError::WindowGone(_)) => {}
            Err(e) => panic!("unexpected error for {:?}: {e}", w.title),
        }
    }
}

#[test]
fn closed_window_reports_gone() {
    // Handle values are never zero for live windows.
    let bogus = WindowId::from_raw(0);
    assert!(matches!(window(bogus), Err(WinError::WindowGone(_))));
    assert!(matches!(placement(bogus), Err(WinError::WindowGone(_))));
}
