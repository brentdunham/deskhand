//! Window tools through the full toolbox path: policy, execution, logging,
//! and taint. None of these tests move or focus a user window.
#![cfg(windows)]

use deskhand_policy::{Authorization, ContextTaint, Outcome, Permit, Source};
use deskhand_tools::{ToolError, Toolbox, window_tools};
use deskhand_win::test_support::TestWindow;
use serde_json::{Value, json};

fn toolbox() -> Toolbox {
    let mut tb = Toolbox::new();
    window_tools::register(&mut tb).unwrap();
    tb
}

fn permit(tb: &mut Toolbox, tool: &str, args: &Value, taint: &ContextTaint) -> Permit {
    match tb.authorize(tool, args, taint) {
        Authorization::Permitted(p) => p,
        other => panic!("expected permit for {tool}, got {other:?}"),
    }
}

#[test]
fn registers_all_window_tools() {
    let names: Vec<String> = toolbox()
        .definitions()
        .into_iter()
        .map(|d| d.name)
        .collect();
    assert_eq!(
        names,
        [
            "focus_window",
            "list_monitors",
            "list_windows",
            "minimize_window",
            "move_window",
            "read_window_text",
            "snap_window",
        ]
    );
}

#[test]
fn list_windows_taints_context_and_excludes_own_windows() {
    let own = TestWindow::create_hidden().unwrap();
    let mut tb = toolbox();
    let mut taint = ContextTaint::new();
    let args = json!({});
    let p = permit(&mut tb, "list_windows", &args, &taint);
    let outcome = tb.execute(p, args, &mut taint);

    let content = outcome.result.unwrap();
    let windows = content["windows"].as_array().unwrap();
    assert!(windows.iter().all(|w| w["window_id"] != json!(own.id())));
    assert_eq!(outcome.untrusted, Some(Source::Screen));
    assert!(taint.is_tainted());
}

#[test]
fn list_monitors_does_not_taint() {
    let mut tb = toolbox();
    let mut taint = ContextTaint::new();
    let args = json!({});
    let p = permit(&mut tb, "list_monitors", &args, &taint);
    let outcome = tb.execute(p, args, &mut taint);
    assert!(
        !outcome.result.unwrap()["monitors"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(!taint.is_tainted());
}

#[test]
fn refuses_to_act_on_own_windows() {
    let own = TestWindow::create_hidden().unwrap();
    let mut tb = toolbox();
    let mut taint = ContextTaint::new();
    for (tool, args) in [
        ("focus_window", json!({ "window_id": own.id() })),
        ("minimize_window", json!({ "window_id": own.id() })),
        ("read_window_text", json!({ "window_id": own.id() })),
        (
            "snap_window",
            json!({ "window_id": own.id(), "position": "left_half" }),
        ),
        (
            "move_window",
            json!({ "window_id": own.id(), "x": 0, "y": 0, "width": 100, "height": 100 }),
        ),
    ] {
        let p = permit(&mut tb, tool, &args, &taint);
        let outcome = tb.execute(p, args, &mut taint);
        assert_eq!(
            outcome.result,
            Err(ToolError::Failed(
                "DeskHand does not act on its own windows".into()
            )),
            "{tool}"
        );
    }
}

#[test]
fn rejects_invalid_arguments_and_logs_failure() {
    let mut tb = toolbox();
    let mut taint = ContextTaint::new();
    for args in [
        json!({ "window_id": 1, "position": "sideways" }),
        json!({ "window_id": 1, "position": "left_half", "extra": true }),
        json!({ "position": "left_half" }),
    ] {
        let p = permit(&mut tb, "snap_window", &args, &taint);
        let outcome = tb.execute(p, args, &mut taint);
        assert!(
            matches!(outcome.result, Err(ToolError::InvalidArgs(_))),
            "{:?}",
            outcome.result
        );
        assert!(matches!(
            tb.log().get(outcome.action_id).unwrap().outcome,
            Outcome::Failed { .. }
        ));
    }

    let args = json!({ "window_id": 1, "x": 0, "y": 0, "width": 0, "height": 10 });
    let p = permit(&mut tb, "move_window", &args, &taint);
    assert!(matches!(
        tb.execute(p, args, &mut taint).result,
        Err(ToolError::InvalidArgs(_))
    ));
}

#[test]
fn closed_window_fails_cleanly() {
    let mut tb = toolbox();
    let mut taint = ContextTaint::new();
    let args = json!({ "window_id": 0 });
    let p = permit(&mut tb, "minimize_window", &args, &taint);
    let outcome = tb.execute(p, args, &mut taint);
    match outcome.result {
        Err(ToolError::Failed(m)) => assert!(m.contains("no longer exists"), "{m}"),
        other => panic!("expected failure, got {other:?}"),
    }
}
