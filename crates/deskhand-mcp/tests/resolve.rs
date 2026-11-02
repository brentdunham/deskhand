//! Command resolution: only runnable files are picked.

use deskhand_mcp::resolve_in;

#[cfg(windows)]
#[test]
fn picks_the_cmd_shim_over_npms_extensionless_bash_script() {
    let dir = tempfile::tempdir().unwrap();
    // What npm installs side by side.
    for (name, body) in [
        ("npx", "#!/bin/sh"),
        ("npx.cmd", "@echo off"),
        ("npx.ps1", ""),
    ] {
        std::fs::write(dir.path().join(name), body).unwrap();
    }
    std::fs::write(dir.path().join("uvx.exe"), "").unwrap();
    let dirs = vec![dir.path().to_owned()];

    assert_eq!(
        resolve_in("npx", &dirs).unwrap(),
        dir.path().join("npx.cmd")
    );
    assert_eq!(
        resolve_in("uvx", &dirs).unwrap(),
        dir.path().join("uvx.exe")
    );
    assert_eq!(
        resolve_in("uvx.exe", &dirs).unwrap(),
        dir.path().join("uvx.exe")
    );
    assert!(resolve_in("missing", &dirs).is_none());
}

#[cfg(not(windows))]
#[test]
fn finds_plain_executables() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("tool"), "").unwrap();
    let dirs = vec![dir.path().to_owned()];
    assert_eq!(resolve_in("tool", &dirs).unwrap(), dir.path().join("tool"));
}
