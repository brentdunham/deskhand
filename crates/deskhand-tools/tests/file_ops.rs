//! Windows-only file tools through the toolbox. The refusal tests never reach
//! the shell. The recycle test is opt-in because it puts a small file in the
//! user's Recycle Bin:
//!
//! ```text
//! cargo test -p deskhand-tools --test file_ops -- --ignored
//! ```
#![cfg(windows)]

use std::fs;
use std::sync::{Arc, RwLock};

use deskhand_policy::{Authorization, ContextTaint};
use deskhand_tools::file_tools::{self, SearchRoots};
use deskhand_tools::{CallOutcome, Toolbox};
use serde_json::{Value, json};

fn setup() -> (tempfile::TempDir, Toolbox) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    fs::create_dir_all(root.join("app")).unwrap();
    fs::create_dir_all(dir.path().join("outside")).unwrap();
    for f in [
        "root/setup.exe",
        "root/run.BAT",
        "root/link.lnk",
        "root/notes.txt",
        "outside/x.txt",
    ] {
        fs::write(dir.path().join(f), b"x").unwrap();
    }
    let roots: SearchRoots = Arc::new(RwLock::new(vec![root.clone()]));
    let mut tb = Toolbox::new();
    file_tools::register(&mut tb, roots, vec![root.join("app")]).unwrap();
    (dir, tb)
}

fn run(tb: &mut Toolbox, tool: &str, args: Value) -> CallOutcome {
    let mut taint = ContextTaint::new();
    match tb.authorize(tool, &args, &taint) {
        Authorization::Permitted(p) => tb.execute(p, args, &mut taint),
        Authorization::NeedsConfirmation(p) => tb.execute(p.approve(), args, &mut taint),
        Authorization::Denied(r) => panic!("{tool} denied: {r:?}"),
    }
}

fn err(outcome: CallOutcome) -> String {
    outcome.result.unwrap_err().to_string()
}

fn p(dir: &tempfile::TempDir, rel: &str) -> String {
    dir.path().join(rel).to_string_lossy().into_owned()
}

#[test]
fn open_path_refuses_programs_and_outside_paths() {
    let (dir, mut tb) = setup();
    for f in ["root/setup.exe", "root/run.BAT", "root/link.lnk"] {
        let e = err(run(&mut tb, "open_path", json!({ "path": p(&dir, f) })));
        assert!(e.contains("runs code"), "{f}: {e}");
    }
    let e = err(run(
        &mut tb,
        "open_path",
        json!({ "path": p(&dir, "outside/x.txt") }),
    ));
    assert!(e.contains("outside"), "{e}");
    let stream = format!("{}:hidden", p(&dir, "root/notes.txt"));
    assert!(
        run(&mut tb, "open_path", json!({ "path": stream }))
            .result
            .is_err()
    );
}

#[test]
fn open_url_refuses_non_web_schemes() {
    let (_dir, mut tb) = setup();
    for url in [
        "file:///C:/Windows/System32/calc.exe",
        "ms-settings:privacy",
        "javascript:x",
    ] {
        assert!(
            run(&mut tb, "open_url", json!({ "url": url }))
                .result
                .is_err(),
            "{url}"
        );
    }
}

#[test]
fn recycle_requires_confirmation_and_refuses_roots_and_outside() {
    let (dir, mut tb) = setup();
    assert!(matches!(
        tb.authorize(
            "recycle_path",
            &json!({ "path": "x" }),
            &ContextTaint::new()
        ),
        Authorization::NeedsConfirmation(_)
    ));
    let e = err(run(
        &mut tb,
        "recycle_path",
        json!({ "path": p(&dir, "root") }),
    ));
    assert!(e.contains("configured folders"), "{e}");
    let e = err(run(
        &mut tb,
        "recycle_path",
        json!({ "path": p(&dir, "outside/x.txt") }),
    ));
    assert!(e.contains("outside"), "{e}");
    assert!(dir.path().join("outside/x.txt").exists());
}

#[test]
#[ignore = "puts a small temp file in the Recycle Bin"]
fn recycle_moves_to_recycle_bin() {
    let (dir, mut tb) = setup();
    let target = dir.path().join("root/notes.txt");
    let out = run(
        &mut tb,
        "recycle_path",
        json!({ "path": target.to_string_lossy() }),
    );
    assert!(out.result.is_ok(), "{:?}", out.result);
    assert!(!target.exists());
}
