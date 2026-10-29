//! Shell integration against the real system. Read-only: lists apps and
//! resolves folders, never launches anything.
#![cfg(windows)]

use deskhand_win::shell;

#[test]
fn lists_apps_with_names_and_ids() {
    let apps = shell::list_apps().unwrap();
    // Every Windows install has apps; CI images included.
    assert!(!apps.is_empty());
    assert!(apps.iter().all(|a| !a.name.is_empty() && !a.id.is_empty()));
    let sorted = apps
        .windows(2)
        .all(|w| w[0].name.to_lowercase() <= w[1].name.to_lowercase());
    assert!(sorted);
    println!("{} apps; first: {:?}", apps.len(), apps.first());
}

#[test]
fn resolves_known_folders() {
    let home = shell::home_folder().expect("profile folder");
    assert!(home.is_dir(), "{home:?}");
    let roots = shell::default_search_roots();
    assert!(!roots.is_empty());
    assert!(roots.iter().all(|r| r.is_absolute()));
    println!("roots: {roots:?}");
}
