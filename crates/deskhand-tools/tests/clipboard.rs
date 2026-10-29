//! Clipboard round trip through the tools, including undo. Opt-in because it
//! replaces the clipboard; the original text, if any, is restored afterward.
//!
//! ```text
//! cargo test -p deskhand-tools --test clipboard -- --ignored --test-threads=1
//! ```
#![cfg(windows)]

use deskhand_policy::{Authorization, ContextTaint, Source};
use deskhand_tools::{Toolbox, system_tools};
use deskhand_win::clipboard::{self, ClipboardContent};
use serde_json::{Value, json};

struct Restore(Option<String>);

impl Drop for Restore {
    fn drop(&mut self) {
        if let Some(text) = &self.0 {
            let _ = clipboard::write(text);
        }
    }
}

fn run(
    tb: &mut Toolbox,
    tool: &str,
    args: Value,
    taint: &mut ContextTaint,
) -> deskhand_tools::CallOutcome {
    match tb.authorize(tool, &args, taint) {
        Authorization::Permitted(p) => tb.execute(p, args, taint),
        Authorization::NeedsConfirmation(p) => tb.execute(p.approve(), args, taint),
        Authorization::Denied(r) => panic!("{tool} denied: {r:?}"),
    }
}

#[test]
#[ignore = "replaces the clipboard (restored afterward)"]
fn write_read_undo_round_trip() {
    let _restore = Restore(match clipboard::read(usize::MAX).unwrap() {
        (ClipboardContent::Text(t), _) => Some(t),
        _ => None,
    });
    let mut tb = Toolbox::new();
    system_tools::register(&mut tb).unwrap();
    let mut taint = ContextTaint::new();

    clipboard::write("before").unwrap();
    let wrote = run(
        &mut tb,
        "write_clipboard",
        json!({ "text": "héllo ✓ from DeskHand" }),
        &mut taint,
    );
    assert!(wrote.result.is_ok());

    let read = run(&mut tb, "read_clipboard", json!({}), &mut taint);
    let v = read.result.unwrap();
    assert_eq!(v["status"], "text");
    assert_eq!(v["text"], "héllo ✓ from DeskHand");
    assert_eq!(read.untrusted, Some(Source::Clipboard));
    assert!(taint.is_tainted());

    tb.undo(wrote.action_id).unwrap();
    assert_eq!(
        clipboard::read(100).unwrap().0,
        ClipboardContent::Text("before".into())
    );
}

#[test]
fn write_and_launch_are_gated_by_taint() {
    let mut tb = Toolbox::new();
    system_tools::register(&mut tb).unwrap();
    let mut taint = ContextTaint::new();
    taint.ingest(Source::Screen);
    assert!(matches!(
        tb.authorize("write_clipboard", &json!({ "text": "x" }), &taint),
        Authorization::NeedsConfirmation(_)
    ));
    assert!(matches!(
        tb.authorize("launch_app", &json!({ "name": "x" }), &taint),
        Authorization::NeedsConfirmation(_)
    ));
}
