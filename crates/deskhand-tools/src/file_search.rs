//! File name search over a set of root folders. Platform-independent.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::Serialize;

#[derive(Debug, Clone, Copy)]
pub struct SearchLimits {
    pub max_results: usize,
    /// Directory entries examined before giving up.
    pub max_visited: usize,
    pub time_budget: Duration,
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            max_results: 50,
            max_visited: 200_000,
            time_budget: Duration::from_secs(3),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileMatch {
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    /// Seconds since the Unix epoch, when available.
    pub modified: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchResult {
    pub matches: Vec<FileMatch>,
    /// A limit stopped the walk before every root was fully searched.
    pub truncated: bool,
    pub visited: usize,
}

/// Directory names never descended into: version control, dependency and
/// build caches, and the recycle bin. They are large and rarely what a person
/// is looking for.
const SKIPPED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "__pycache__",
    ".venv",
    "$recycle.bin",
];

fn skip_dir(name: &str) -> bool {
    name.starts_with('.') || SKIPPED_DIRS.iter().any(|d| d.eq_ignore_ascii_case(name))
}

/// Lowercased, whitespace-separated terms. Every term must appear in a file
/// name for it to match.
pub fn terms(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

/// Breadth-first, so shallow (usually more relevant) matches come first;
/// entries within a folder are visited in name order.
/// Symbolic links and junctions are listed but never followed, so a link
/// cannot loop the walk or lead outside the roots.
pub fn search(roots: &[PathBuf], query: &str, limits: SearchLimits) -> SearchResult {
    let terms = terms(query);
    let deadline = Instant::now() + limits.time_budget;
    let mut result = SearchResult {
        matches: Vec::new(),
        truncated: false,
        visited: 0,
    };
    if terms.is_empty() {
        return result;
    }

    let mut queue: VecDeque<PathBuf> = roots.iter().filter(|r| r.is_dir()).cloned().collect();
    'walk: while let Some(dir) = queue.pop_front() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue; // unreadable (permissions); skip quietly
        };
        // Directory order is filesystem-defined; sort for stable results.
        let mut entries: Vec<fs::DirEntry> = entries.flatten().collect();
        entries.sort_by_cached_key(|e| e.file_name().to_string_lossy().to_lowercase());
        for entry in entries {
            result.visited += 1;
            if result.visited > limits.max_visited || Instant::now() >= deadline {
                result.truncated = true;
                break 'walk;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let lower = name.to_lowercase();

            if terms.iter().all(|t| lower.contains(t.as_str())) {
                let meta = entry.metadata().ok();
                result.matches.push(FileMatch {
                    path: entry.path().to_string_lossy().into_owned(),
                    is_dir: file_type.is_dir(),
                    size: meta
                        .as_ref()
                        .map_or(0, |m| if m.is_file() { m.len() } else { 0 }),
                    modified: meta
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs()),
                });
                if result.matches.len() >= limits.max_results {
                    result.truncated = true;
                    break 'walk;
                }
            }
            if file_type.is_dir() && !file_type.is_symlink() && !skip_dir(&name) {
                queue.push_back(entry.path());
            }
        }
    }
    result
}

/// Expands a leading `~` to `home` and rejects relative paths.
pub fn resolve_root(raw: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    let path = match raw.strip_prefix('~') {
        Some(rest) => {
            let home = home.ok_or("`~` used but the home folder is unknown")?;
            home.join(rest.trim_start_matches(['/', '\\']))
        }
        None => PathBuf::from(raw),
    };
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!(
            "search root `{raw}` must be an absolute path or start with ~"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        for d in [
            "docs/taxes",
            "docs/node_modules/pkg",
            ".hidden",
            "deep/a/b/c",
        ] {
            fs::create_dir_all(p.join(d)).unwrap();
        }
        for f in [
            "docs/Tax Return 2025.pdf",
            "docs/taxes/receipt.pdf",
            "docs/node_modules/pkg/tax-lib.js",
            ".hidden/tax secrets.txt",
            "deep/a/b/c/Tax Return 2024.pdf",
            "notes.txt",
        ] {
            fs::write(p.join(f), b"x").unwrap();
        }
        dir
    }

    fn names(r: &SearchResult) -> Vec<String> {
        r.matches
            .iter()
            .map(|m| {
                Path::new(&m.path)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    #[test]
    fn all_terms_must_match_case_insensitively() {
        let dir = tree();
        let r = search(
            &[dir.path().to_owned()],
            "TAX return",
            SearchLimits::default(),
        );
        assert_eq!(names(&r), ["Tax Return 2025.pdf", "Tax Return 2024.pdf"]);
        assert!(!r.truncated);
    }

    #[test]
    fn breadth_first_and_matches_directories() {
        let dir = tree();
        let r = search(&[dir.path().to_owned()], "tax", SearchLimits::default());
        let n = names(&r);
        assert_eq!(n[0], "Tax Return 2025.pdf");
        assert!(n.contains(&"taxes".to_owned()));
        assert!(r.matches.iter().any(|m| m.is_dir));
    }

    #[test]
    fn skips_hidden_and_dependency_folders() {
        let dir = tree();
        let r = search(&[dir.path().to_owned()], "tax", SearchLimits::default());
        let n = names(&r);
        assert!(
            !n.iter()
                .any(|f| f == "tax-lib.js" || f == "tax secrets.txt"),
            "{n:?}"
        );
    }

    #[test]
    fn stops_at_max_results() {
        let dir = tree();
        let limits = SearchLimits {
            max_results: 1,
            ..SearchLimits::default()
        };
        let r = search(&[dir.path().to_owned()], "pdf", limits);
        assert_eq!(r.matches.len(), 1);
        assert!(r.truncated);
    }

    #[test]
    fn empty_query_and_missing_roots_return_nothing() {
        let dir = tree();
        assert!(
            search(&[dir.path().to_owned()], "   ", SearchLimits::default())
                .matches
                .is_empty()
        );
        let missing = dir.path().join("nope");
        assert!(
            search(&[missing], "tax", SearchLimits::default())
                .matches
                .is_empty()
        );
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_symlinks() {
        let dir = tree();
        std::os::unix::fs::symlink(dir.path(), dir.path().join("docs/loop")).unwrap();
        let r = search(&[dir.path().to_owned()], "receipt", SearchLimits::default());
        assert_eq!(names(&r), ["receipt.pdf"]);
    }

    #[test]
    fn resolves_roots() {
        let home = PathBuf::from(if cfg!(windows) {
            "C:\\Users\\me"
        } else {
            "/home/me"
        });
        let r = resolve_root("~/Projects", Some(&home)).unwrap();
        assert_eq!(r, home.join("Projects"));
        assert!(resolve_root("relative/dir", Some(&home)).is_err());
        assert!(resolve_root("~", None).is_err());
    }
}
