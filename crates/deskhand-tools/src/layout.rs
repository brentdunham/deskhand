//! Named window layouts: the data model, window matching, and the on-disk
//! store. Platform-independent; the tools that capture and apply layouts are
//! in `layout_tools`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use deskhand_win::Placement;
use serde::{Deserialize, Serialize};

pub const MAX_NAME_LEN: usize = 64;
const FORMAT_VERSION: u32 = 1;

/// One window's identity and placement at the time the layout was saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedWindow {
    /// Full executable path, when the process could be queried.
    pub app: Option<String>,
    pub class: String,
    /// Untrusted: set by the owning application.
    pub title: String,
    pub placement: Placement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    /// As the user typed it. Lookup is case-insensitive.
    pub name: String,
    pub windows: Vec<SavedWindow>,
}

/// How a current window identifies itself for matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowKey<'a> {
    pub app: Option<&'a str>,
    pub class: &'a str,
    pub title: &'a str,
}

fn same_app(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        (None, None) => true,
        _ => false,
    }
}

/// Pairs each saved window with a current one. Window handles do not survive
/// restarts, so identity is the executable plus window class:
///
/// 1. Exact title matches are paired first, so two windows of the same app
///    keep their own positions when titles are stable.
/// 2. Remaining saved windows take the next unused window of the same app
///    and class, in Z order (browser titles change with the active tab).
///
/// Each current window is used at most once. `None` means no match.
pub fn match_windows<T: Copy>(
    saved: &[SavedWindow],
    current: &[(T, WindowKey<'_>)],
) -> Vec<Option<T>> {
    let mut used = vec![false; current.len()];
    let mut result: Vec<Option<T>> = vec![None; saved.len()];

    let same_kind = |s: &SavedWindow, k: &WindowKey<'_>| {
        same_app(s.app.as_deref(), k.app) && s.class == k.class
    };

    for (i, s) in saved.iter().enumerate() {
        if let Some(j) = (0..current.len())
            .find(|&j| !used[j] && same_kind(s, &current[j].1) && s.title == current[j].1.title)
        {
            used[j] = true;
            result[i] = Some(current[j].0);
        }
    }
    for (i, s) in saved.iter().enumerate() {
        if result[i].is_some() {
            continue;
        }
        if let Some(j) = (0..current.len()).find(|&j| !used[j] && same_kind(s, &current[j].1)) {
            used[j] = true;
            result[i] = Some(current[j].0);
        }
    }
    result
}

/// Validates a layout name and returns its lookup key.
pub fn name_key(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("layout name is empty".into());
    }
    if trimmed.chars().count() > MAX_NAME_LEN {
        return Err(format!(
            "layout name is longer than {MAX_NAME_LEN} characters"
        ));
    }
    if trimmed.chars().any(char::is_control) {
        return Err("layout name contains control characters".into());
    }
    Ok(trimmed.to_lowercase())
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    layouts: Vec<Layout>,
}

/// Layouts persisted as JSON. Every change is written through with a
/// write-to-temp-then-rename, so a crash never leaves a half-written file.
#[derive(Debug)]
pub struct LayoutStore {
    path: PathBuf,
    layouts: BTreeMap<String, Layout>,
}

impl LayoutStore {
    /// Opens the store. A missing file is an empty store. An unreadable file
    /// is moved aside (never overwritten) and reported in the returned
    /// warning, so a later save cannot destroy recoverable data.
    pub fn open(path: PathBuf) -> (Self, Option<String>) {
        let mut store = Self {
            path,
            layouts: BTreeMap::new(),
        };
        let text = match fs::read_to_string(&store.path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (store, None),
            Err(e) => return store.quarantine(&e.to_string()),
        };
        match serde_json::from_str::<File>(&text) {
            Ok(f) if f.version == FORMAT_VERSION => {
                for l in f.layouts {
                    if let Ok(key) = name_key(&l.name) {
                        store.layouts.insert(key, l);
                    }
                }
                (store, None)
            }
            Ok(f) => store.quarantine(&format!("unsupported format version {}", f.version)),
            Err(e) => store.quarantine(&e.to_string()),
        }
    }

    fn quarantine(self, reason: &str) -> (Self, Option<String>) {
        let aside = self.path.with_extension("json.unreadable");
        let moved = fs::rename(&self.path, &aside).is_ok();
        let warning = if moved {
            format!(
                "The layouts file could not be read ({reason}). It was moved to {} and DeskHand started with no layouts.",
                aside.display()
            )
        } else {
            format!("The layouts file could not be read ({reason}).")
        };
        (self, Some(warning))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self, name: &str) -> Option<&Layout> {
        self.layouts.get(&name_key(name).ok()?)
    }

    /// Sorted by lookup key.
    pub fn list(&self) -> impl Iterator<Item = &Layout> {
        self.layouts.values()
    }

    /// Adds or replaces a layout. Returns the replaced one.
    pub fn put(&mut self, layout: Layout) -> Result<Option<Layout>, String> {
        let key = name_key(&layout.name)?;
        let previous = self.layouts.insert(key.clone(), layout);
        if let Err(e) = self.persist() {
            // Keep memory consistent with disk.
            match previous.clone() {
                Some(p) => self.layouts.insert(key, p),
                None => self.layouts.remove(&key),
            };
            return Err(e);
        }
        Ok(previous)
    }

    pub fn remove(&mut self, name: &str) -> Result<Option<Layout>, String> {
        let key = name_key(name)?;
        let Some(removed) = self.layouts.remove(&key) else {
            return Ok(None);
        };
        if let Err(e) = self.persist() {
            self.layouts.insert(key, removed);
            return Err(e);
        }
        Ok(Some(removed))
    }

    fn persist(&self) -> Result<(), String> {
        let file = File {
            version: FORMAT_VERSION,
            layouts: self.layouts.values().cloned().collect(),
        };
        let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)
                .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        }
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
        fs::rename(&tmp, &self.path)
            .map_err(|e| format!("could not replace {}: {e}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use deskhand_win::{Rect, ShowState};

    use super::*;

    fn saved(app: &str, class: &str, title: &str, x: i32) -> SavedWindow {
        SavedWindow {
            app: Some(app.into()),
            class: class.into(),
            title: title.into(),
            placement: Placement {
                state: ShowState::Normal,
                normal: Rect::new(x, 0, x + 100, 100),
            },
        }
    }

    fn key<'a>(app: &'a str, class: &'a str, title: &'a str) -> WindowKey<'a> {
        WindowKey {
            app: Some(app),
            class,
            title,
        }
    }

    #[test]
    fn exact_titles_win_over_order() {
        let saved = [
            saved("C:\\code.exe", "Chrome_WidgetWin_1", "project-a", 0),
            saved("C:\\code.exe", "Chrome_WidgetWin_1", "project-b", 100),
        ];
        // Current Z order is reversed relative to the saved order.
        let current = [
            (1, key("C:\\code.exe", "Chrome_WidgetWin_1", "project-b")),
            (2, key("C:\\code.exe", "Chrome_WidgetWin_1", "project-a")),
        ];
        assert_eq!(match_windows(&saved, &current), [Some(2), Some(1)]);
    }

    #[test]
    fn falls_back_to_app_and_class_when_title_changed() {
        let saved = [saved("C:\\chrome.exe", "Chrome_WidgetWin_1", "Inbox", 0)];
        let current = [(7, key("c:\\CHROME.EXE", "Chrome_WidgetWin_1", "Calendar"))];
        assert_eq!(match_windows(&saved, &current), [Some(7)]);
    }

    #[test]
    fn never_reuses_a_window_and_reports_missing() {
        let saved = [
            saved("C:\\a.exe", "A", "one", 0),
            saved("C:\\a.exe", "A", "two", 0),
            saved("C:\\b.exe", "B", "b", 0),
        ];
        let current = [(1, key("C:\\a.exe", "A", "other"))];
        assert_eq!(match_windows(&saved, &current), [Some(1), None, None]);
    }

    #[test]
    fn class_must_match() {
        let saved = [saved("C:\\a.exe", "Main", "x", 0)];
        let current = [(1, key("C:\\a.exe", "Dialog", "x"))];
        assert_eq!(match_windows(&saved, &current), [None]);
    }

    #[test]
    fn validates_names() {
        assert_eq!(name_key("  Coding ").unwrap(), "coding");
        assert!(name_key("   ").is_err());
        assert!(name_key(&"x".repeat(MAX_NAME_LEN + 1)).is_err());
        assert!(name_key("a\nb").is_err());
    }

    #[test]
    fn store_round_trips_and_is_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("layouts.json");
        let (mut store, warning) = LayoutStore::open(path.clone());
        assert!(warning.is_none());

        let layout = Layout {
            name: "Coding".into(),
            windows: vec![saved("C:\\a.exe", "A", "t", 0)],
        };
        assert_eq!(store.put(layout.clone()).unwrap(), None);
        assert_eq!(store.get("CODING"), Some(&layout));

        let (reopened, _) = LayoutStore::open(path.clone());
        assert_eq!(reopened.get("coding"), Some(&layout));

        let replacement = Layout {
            name: "coding".into(),
            windows: vec![],
        };
        let mut store = reopened;
        assert_eq!(store.put(replacement).unwrap(), Some(layout.clone()));
        assert_eq!(
            store.remove("Coding").unwrap().map(|l| l.windows.len()),
            Some(0)
        );
        assert_eq!(store.remove("coding").unwrap(), None);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn unreadable_file_is_moved_aside_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layouts.json");
        fs::write(&path, "{ not json").unwrap();

        let (mut store, warning) = LayoutStore::open(path.clone());
        assert!(warning.unwrap().contains("could not be read"));
        let aside = path.with_extension("json.unreadable");
        assert_eq!(fs::read_to_string(&aside).unwrap(), "{ not json");

        store
            .put(Layout {
                name: "x".into(),
                windows: vec![],
            })
            .unwrap();
        assert_eq!(fs::read_to_string(&aside).unwrap(), "{ not json");
    }
}
