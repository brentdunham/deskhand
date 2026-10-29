//! Which paths file tools may touch.
//!
//! Every path is canonicalized (resolving `..`, symbolic links, and
//! junctions) and must then lie inside one of the configured roots and
//! outside every protected directory (DeskHand's own config and data). A
//! link inside a root that points elsewhere therefore resolves outside it and
//! is refused.

use std::fs;
use std::path::{Component, Path, PathBuf};

/// File types that run code when opened with their default handler.
/// `open_path` refuses them. Based on the attachment types mail clients
/// block, plus Windows shortcut and installer formats.
const EXECUTABLE_EXTENSIONS: &[&str] = &[
    "ade",
    "adp",
    "app",
    "application",
    "appinstaller",
    "appref-ms",
    "appx",
    "appxbundle",
    "bas",
    "bat",
    "chm",
    "cmd",
    "com",
    "cpl",
    "crt",
    "csh",
    "der",
    "diagcab",
    "dll",
    "exe",
    "fxp",
    "gadget",
    "grp",
    "hlp",
    "hpj",
    "hta",
    "htc",
    "inf",
    "ins",
    "isp",
    "its",
    "jar",
    "jnlp",
    "js",
    "jse",
    "ksh",
    "library-ms",
    "lnk",
    "mad",
    "maf",
    "mag",
    "mam",
    "maq",
    "mar",
    "mas",
    "mat",
    "mau",
    "mav",
    "maw",
    "mcf",
    "mda",
    "mdb",
    "mde",
    "mdt",
    "mdw",
    "mdz",
    "msc",
    "msh",
    "msh1",
    "msh1xml",
    "msh2",
    "msh2xml",
    "mshxml",
    "msi",
    "msix",
    "msixbundle",
    "msp",
    "mst",
    "ocx",
    "ops",
    "pcd",
    "pif",
    "plg",
    "prf",
    "prg",
    "ps1",
    "ps1xml",
    "ps2",
    "ps2xml",
    "psc1",
    "psc2",
    "psd1",
    "psm1",
    "pst",
    "py",
    "pyc",
    "pyo",
    "pyw",
    "pyz",
    "pyzw",
    "reg",
    "scf",
    "scr",
    "sct",
    "search-ms",
    "searchconnector-ms",
    "settingcontent-ms",
    "shb",
    "shs",
    "slk",
    "sys",
    "theme",
    "themepack",
    "tmp",
    "url",
    "vb",
    "vbe",
    "vbp",
    "vbs",
    "vhd",
    "vhdx",
    "vsmacros",
    "vsw",
    "webpnp",
    "website",
    "ws",
    "wsb",
    "wsc",
    "wsf",
    "wsh",
    "xbap",
    "xll",
    "xnk",
];

pub fn is_executable_type(path: &Path) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|e| EXECUTABLE_EXTENSIONS.contains(&e.as_str()))
}

/// Removes the `\\?\` prefix Windows canonicalization adds, so paths work
/// with the shell and read naturally. `\\?\UNC\server\share` becomes
/// `\\server\share`.
pub fn simplify(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_owned()
    }
}

/// Rejects alternate data streams (`file.txt:stream`). A colon is only valid
/// as the drive separator.
fn reject_streams(path: &Path) -> Result<(), String> {
    let s = simplify(path).to_string_lossy().into_owned();
    let after_drive = if s.as_bytes().get(1) == Some(&b':') {
        &s[2..]
    } else {
        &s[..]
    };
    if after_drive.contains(':') {
        return Err(format!("`{s}` names an alternate data stream"));
    }
    Ok(())
}

/// Names Windows reserves for devices, in any case and with any extension.
fn is_reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit())
}

/// Validates a single new file or folder name.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." {
        return Err(format!("`{name}` is not a valid name"));
    }
    if name.ends_with(['.', ' ']) {
        return Err(format!(
            "`{name}` ends with a dot or space, which Windows strips"
        ));
    }
    if let Some(c) = name
        .chars()
        .find(|c| r#"<>:"/\|?*"#.contains(*c) || c.is_control())
    {
        return Err(format!("`{name}` contains the invalid character {c:?}"));
    }
    if is_reserved_name(name) {
        return Err(format!("`{name}` is a reserved device name"));
    }
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct FileScope {
    pub roots: Vec<PathBuf>,
    /// Never touched, even inside a root.
    pub protected: Vec<PathBuf>,
}

impl FileScope {
    fn canonical(list: &[PathBuf]) -> Vec<PathBuf> {
        list.iter()
            .filter_map(|p| fs::canonicalize(p).ok())
            .collect()
    }

    fn check(&self, canonical: &Path) -> Result<(), String> {
        reject_streams(canonical)?;
        let shown = simplify(canonical);
        if Self::canonical(&self.protected)
            .iter()
            .any(|p| canonical.starts_with(p))
        {
            return Err(format!(
                "`{}` is inside DeskHand's own data, which file tools never touch",
                shown.display()
            ));
        }
        if !Self::canonical(&self.roots)
            .iter()
            .any(|r| canonical.starts_with(r))
        {
            return Err(format!(
                "`{}` is outside the folders DeskHand may use (see [search] roots in the config file)",
                shown.display()
            ));
        }
        Ok(())
    }

    /// An existing path inside the scope, canonicalized.
    pub fn existing(&self, raw: &str) -> Result<PathBuf, String> {
        let canonical = fs::canonicalize(raw).map_err(|e| format!("`{raw}`: {e}"))?;
        self.check(&canonical)?;
        Ok(canonical)
    }

    /// A path that does not exist yet, whose parent exists inside the scope.
    pub fn new_path(&self, raw: &str) -> Result<PathBuf, String> {
        let path = Path::new(raw);
        if path.exists() {
            return Err(format!("`{raw}` already exists"));
        }
        let name = path
            .file_name()
            .ok_or_else(|| format!("`{raw}` has no file name"))?
            .to_string_lossy()
            .into_owned();
        check_name(&name)?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| format!("`{raw}` must be an absolute path"))?;
        let parent =
            fs::canonicalize(parent).map_err(|e| format!("parent folder of `{raw}`: {e}"))?;
        let full = parent.join(&name);
        self.check(&full)?;
        Ok(full)
    }

    /// Whether `path` is one of the roots themselves (never moved or deleted).
    pub fn is_root(&self, canonical: &Path) -> bool {
        Self::canonical(&self.roots).iter().any(|r| r == canonical)
    }

    /// Folders from `raw` down that do not exist yet, outermost first. Every
    /// one must be creatable inside the scope.
    pub fn missing_folders(&self, raw: &str) -> Result<Vec<PathBuf>, String> {
        let path = Path::new(raw);
        if !path.is_absolute() {
            return Err(format!("`{raw}` must be an absolute path"));
        }
        if path.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(format!("`{raw}` must not contain `..`"));
        }
        let mut missing = Vec::new();
        let mut cur = path;
        while !cur.exists() {
            missing.push(cur.to_owned());
            cur = cur
                .parent()
                .ok_or_else(|| format!("no existing parent folder for `{raw}`"))?;
        }
        if missing.is_empty() {
            return Err(format!("`{raw}` already exists"));
        }
        // The deepest existing ancestor anchors the check; every new folder
        // is checked by name and by position under it.
        let anchor = fs::canonicalize(cur).map_err(|e| format!("`{}`: {e}", cur.display()))?;
        missing.reverse();
        let mut out = Vec::with_capacity(missing.len());
        let mut base = anchor;
        for m in missing {
            let name = m
                .file_name()
                .ok_or_else(|| format!("`{}` has no folder name", m.display()))?
                .to_string_lossy()
                .into_owned();
            check_name(&name)?;
            base = base.join(&name);
            self.check(&base)?;
            out.push(base.clone());
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(root: &Path, protected: &Path) -> FileScope {
        FileScope {
            roots: vec![root.to_owned()],
            protected: vec![protected.to_owned()],
        }
    }

    fn setup() -> (tempfile::TempDir, FileScope) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let protected = root.join("app-data");
        fs::create_dir_all(&protected).unwrap();
        fs::create_dir_all(dir.path().join("outside")).unwrap();
        fs::write(root.join("a.txt"), b"x").unwrap();
        fs::write(protected.join("config.toml"), b"x").unwrap();
        fs::write(dir.path().join("outside").join("b.txt"), b"x").unwrap();
        let s = scope(&root, &protected);
        (dir, s)
    }

    fn p(dir: &tempfile::TempDir, rel: &str) -> String {
        dir.path().join(rel).to_string_lossy().into_owned()
    }

    #[test]
    fn existing_paths_must_be_inside_roots_and_outside_protected() {
        let (dir, s) = setup();
        assert!(s.existing(&p(&dir, "root/a.txt")).is_ok());
        assert!(
            s.existing(&p(&dir, "outside/b.txt"))
                .unwrap_err()
                .contains("outside")
        );
        assert!(
            s.existing(&p(&dir, "root/app-data/config.toml"))
                .unwrap_err()
                .contains("DeskHand's own data")
        );
        assert!(s.existing(&p(&dir, "root/missing.txt")).is_err());
    }

    #[test]
    fn dot_dot_cannot_escape() {
        let (dir, s) = setup();
        let sneaky = p(&dir, "root/../outside/b.txt");
        assert!(s.existing(&sneaky).unwrap_err().contains("outside"));
        assert!(s.new_path(&p(&dir, "root/../outside/new.txt")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn links_cannot_escape() {
        let (dir, s) = setup();
        std::os::unix::fs::symlink(dir.path().join("outside"), dir.path().join("root/link"))
            .unwrap();
        assert!(
            s.existing(&p(&dir, "root/link/b.txt"))
                .unwrap_err()
                .contains("outside")
        );
    }

    #[test]
    fn new_paths_validate_names_and_never_exist() {
        let (dir, s) = setup();
        assert!(s.new_path(&p(&dir, "root/new.txt")).is_ok());
        assert!(
            s.new_path(&p(&dir, "root/a.txt"))
                .unwrap_err()
                .contains("already exists")
        );
        for bad in [
            "root/CON",
            "root/nul.txt",
            "root/trailing.",
            "root/q?.txt",
            "root/COM1.log",
        ] {
            assert!(s.new_path(&p(&dir, bad)).is_err(), "{bad}");
        }
        assert!(s.new_path("relative.txt").is_err());
    }

    #[test]
    fn missing_folders_lists_new_levels_in_order() {
        let (dir, s) = setup();
        let made = s.missing_folders(&p(&dir, "root/x/y/z")).unwrap();
        assert_eq!(made.len(), 3);
        assert!(made[0].ends_with("x") && made[2].ends_with("z"));
        assert!(
            s.missing_folders(&p(&dir, "root"))
                .unwrap_err()
                .contains("already exists")
        );
        assert!(s.missing_folders(&p(&dir, "outside/new")).is_err());
        assert!(s.missing_folders(&p(&dir, "root/app-data/new")).is_err());
        assert!(s.missing_folders(&p(&dir, "root/x/../y")).is_err());
    }

    #[test]
    fn roots_are_recognized() {
        let (dir, s) = setup();
        let root = fs::canonicalize(dir.path().join("root")).unwrap();
        assert!(s.is_root(&root));
        assert!(!s.is_root(&root.join("a.txt")));
    }

    #[test]
    fn executable_types_are_detected_case_insensitively() {
        for f in [
            "setup.EXE",
            "run.bat",
            "x.ps1",
            "link.lnk",
            "page.HTA",
            "pkg.msi",
            "a.url",
        ] {
            assert!(is_executable_type(Path::new(f)), "{f}");
        }
        for f in [
            "report.pdf",
            "notes.txt",
            "photo.jpg",
            "sheet.xlsx",
            "noext",
        ] {
            assert!(!is_executable_type(Path::new(f)), "{f}");
        }
    }

    #[test]
    fn simplify_strips_verbatim_prefixes() {
        assert_eq!(
            simplify(Path::new(r"\\?\C:\Users\me")),
            PathBuf::from(r"C:\Users\me")
        );
        assert_eq!(
            simplify(Path::new(r"\\?\UNC\srv\share\f")),
            PathBuf::from(r"\\srv\share\f")
        );
        assert_eq!(simplify(Path::new(r"C:\x")), PathBuf::from(r"C:\x"));
    }

    #[test]
    fn streams_are_rejected() {
        assert!(reject_streams(Path::new(r"C:\docs\a.txt:hidden")).is_err());
        assert!(reject_streams(Path::new(r"\\?\C:\docs\a.txt:hidden")).is_err());
        assert!(reject_streams(Path::new(r"C:\docs\a.txt")).is_ok());
    }
}
