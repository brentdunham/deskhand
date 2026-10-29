//! UI Automation text reading against a real window this test owns.
#![cfg(windows)]

use std::time::Duration;

use deskhand_win::WinError;
use deskhand_win::test_support::TestTextWindow;
use deskhand_win::uia::{TextLimits, read_text};
use deskhand_win::window::WindowId;

#[test]
fn reads_plain_fields_and_never_password_fields() {
    let window = TestTextWindow::spawn("hello from uia", "hunter2-secret").unwrap();
    let text = read_text(window.id(), TextLimits::default()).unwrap();

    assert!(
        text.text.contains("hello from uia"),
        "plain field missing from:\n{}",
        text.text
    );
    assert!(
        !text.text.contains("hunter2"),
        "password leaked into:\n{}",
        text.text
    );
    assert!(text.password_fields_skipped >= 1);
    assert!(!text.truncated);
}

#[test]
fn respects_the_character_limit() {
    let long = "x".repeat(5_000);
    let window = TestTextWindow::spawn(&long, "pw").unwrap();
    let limits = TextLimits {
        max_chars: 1_000,
        ..TextLimits::default()
    };
    let text = read_text(window.id(), limits).unwrap();
    assert!(text.text.len() <= 1_000, "{} chars", text.text.len());
    assert!(text.truncated);
}

#[test]
fn closed_window_is_reported_without_waiting() {
    let start = std::time::Instant::now();
    let result = read_text(WindowId::from_raw(0), TextLimits::default());
    assert!(matches!(result, Err(WinError::WindowGone(_))));
    assert!(start.elapsed() < Duration::from_secs(1));
}
