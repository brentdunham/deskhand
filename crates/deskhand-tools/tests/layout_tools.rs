//! Layout tools through the toolbox. Read-only with respect to windows:
//! saving captures placements, and restore is only exercised on a layout
//! name that does not exist, so no user window is moved.
#![cfg(windows)]

use deskhand_policy::{Authorization, ContextTaint, Outcome};
use deskhand_tools::{CallOutcome, ToolError, Toolbox, layout_tools};
use serde_json::{Value, json};

fn toolbox(dir: &tempfile::TempDir) -> Toolbox {
    let mut tb = Toolbox::new();
    let warning = layout_tools::register(&mut tb, dir.path().join("layouts.json")).unwrap();
    assert!(warning.is_none());
    tb
}

fn call(tb: &mut Toolbox, tool: &str, args: Value) -> CallOutcome {
    let mut taint = ContextTaint::new();
    match tb.authorize(tool, &args, &taint) {
        Authorization::Permitted(p) => tb.execute(p, args, &mut taint),
        other => panic!("expected permit for {tool}, got {other:?}"),
    }
}

fn layout_names(tb: &mut Toolbox) -> Vec<String> {
    call(tb, "list_layouts", json!({})).result.unwrap()["layouts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn save_list_delete_with_undo() {
    let dir = tempfile::tempdir().unwrap();
    let mut tb = toolbox(&dir);

    let saved = call(&mut tb, "save_layout", json!({ "name": "  Test Layout " }));
    match &saved.result {
        Ok(v) => assert_eq!(v["saved"], "Test Layout"),
        // CI runners may have no switchable windows.
        Err(ToolError::Failed(m)) if m.contains("no open windows") => return,
        Err(e) => panic!("save failed: {e}"),
    }
    assert_eq!(layout_names(&mut tb), ["Test Layout"]);

    // Lookup is case-insensitive.
    let deleted = call(&mut tb, "delete_layout", json!({ "name": "test layout" }));
    assert!(deleted.result.is_ok());
    assert!(layout_names(&mut tb).is_empty());

    tb.undo(deleted.action_id).unwrap();
    assert_eq!(layout_names(&mut tb), ["Test Layout"]);

    // Undoing the original save of a new name removes it.
    tb.undo(saved.action_id).unwrap();
    assert!(layout_names(&mut tb).is_empty());
    assert_eq!(
        tb.log().get(saved.action_id).unwrap().outcome,
        Outcome::Undone
    );
}

#[test]
fn layouts_persist_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let mut tb = toolbox(&dir);
    if call(&mut tb, "save_layout", json!({ "name": "persist" }))
        .result
        .is_err()
    {
        return; // no windows on this machine
    }
    let mut reopened = toolbox(&dir);
    assert_eq!(layout_names(&mut reopened), ["persist"]);
}

#[test]
fn missing_layouts_and_bad_names_fail_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let mut tb = toolbox(&dir);
    for (tool, args, needle) in [
        (
            "restore_layout",
            json!({ "name": "nope" }),
            "no layout named",
        ),
        (
            "delete_layout",
            json!({ "name": "nope" }),
            "no layout named",
        ),
        ("save_layout", json!({ "name": "   " }), "empty"),
        (
            "save_layout",
            json!({ "name": "x".repeat(65) }),
            "longer than",
        ),
        ("save_layout", json!({ "layout": "x" }), "invalid arguments"),
    ] {
        let outcome = call(&mut tb, tool, args);
        let err = outcome.result.unwrap_err().to_string();
        assert!(
            err.contains(needle),
            "{tool}: `{err}` should contain `{needle}`"
        );
    }
}
