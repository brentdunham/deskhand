//! Shell integration: installed apps (the Start menu's Apps folder), app
//! launch, and the user's known folders.
//!
//! Shell COM objects expect a single-threaded apartment, so each operation
//! runs on its own STA thread under a deadline.

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads,
    FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Profile, FOLDERID_Videos, IEnumShellItems,
    IShellItem, KF_FLAG_DEFAULT, SHGetKnownFolderItem, SHGetKnownFolderPath, SIGDN,
    SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{GUID, HSTRING, PWSTR, w};

use crate::error::{Context, Result, WinError};

/// An entry in the Apps folder: a classic program's shortcut or a Store app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct App {
    pub name: String,
    /// The shell parsing name: an AppUserModelID for Store apps, or a path
    /// or known-folder reference for classic programs.
    pub id: String,
}

const DEADLINE: Duration = Duration::from_secs(10);

/// Runs `f` on a fresh STA thread with a deadline.
fn on_sta_thread<T: Send + 'static>(
    what: &'static str,
    f: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("deskhand-shell".into())
        .spawn(move || {
            // SAFETY: first call on a thread this function owns; balanced by
            // CoUninitialize below after `f` and everything it created are
            // dropped.
            let init =
                unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
            let result = init.ok().ctx("CoInitializeEx").and_then(|()| f());
            if init.is_ok() {
                // SAFETY: pairs with the successful CoInitializeEx above.
                unsafe { CoUninitialize() };
            }
            let _ = tx.send(result);
        })
        .map_err(|e| WinError::Api {
            op: what,
            source: windows::core::Error::new(windows::core::HRESULT(-1), e.to_string()),
        })?;
    rx.recv_timeout(DEADLINE).unwrap_or_else(|_| {
        Err(WinError::Api {
            op: what,
            source: windows::core::Error::new(
                windows::core::HRESULT(-1),
                "the shell did not respond in time",
            ),
        })
    })
}

/// Reads and frees a shell-allocated string.
///
/// # Safety
/// `p` must be a CoTaskMemAlloc'd, NUL-terminated wide string or null.
unsafe fn take_string(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: guaranteed by the caller.
    let s = unsafe { p.to_string() }.unwrap_or_default();
    // SAFETY: the shell allocated `p` with CoTaskMemAlloc; it is not used again.
    unsafe { CoTaskMemFree(Some(p.0 as *const c_void)) };
    s
}

fn display_name(item: &IShellItem, kind: SIGDN) -> Result<String> {
    // SAFETY: valid interface pointer; the returned string is freed by
    // `take_string`.
    unsafe {
        Ok(take_string(
            item.GetDisplayName(kind).ctx("GetDisplayName")?,
        ))
    }
}

/// Every app in the Apps folder, sorted by name.
pub fn list_apps() -> Result<Vec<App>> {
    on_sta_thread("enumerate apps", || {
        // SAFETY: COM is initialized on this thread; the folder id is a
        // static GUID; interface pointers are used only on this thread.
        let items: IEnumShellItems = unsafe {
            let folder: IShellItem =
                SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)
                    .ctx("SHGetKnownFolderItem(AppsFolder)")?;
            folder
                .BindToHandler(None, &BHID_EnumItems)
                .ctx("BindToHandler(EnumItems)")?
        };
        let mut apps = Vec::new();
        loop {
            let mut batch = [None];
            let mut fetched = 0u32;
            // SAFETY: `batch` and `fetched` are valid for the call.
            let hr = unsafe { items.Next(&mut batch, Some(&mut fetched)) };
            if hr.is_err() || fetched == 0 {
                break;
            }
            if let Some(item) = batch[0].take() {
                let name = display_name(&item, SIGDN_NORMALDISPLAY)?;
                let id = display_name(&item, SIGDN_PARENTRELATIVEPARSING)?;
                if !name.is_empty() && !id.is_empty() {
                    apps.push(App { name, id });
                }
            }
        }
        apps.sort_by_cached_key(|a| a.name.to_lowercase());
        Ok(apps)
    })
}

/// Launches an app by its Apps-folder id. Only ids from [`list_apps`] should
/// be passed; this is not a general command runner.
pub fn launch(app: &App) -> Result<()> {
    shell_open(
        "launch app",
        HSTRING::from(format!("shell:AppsFolder\\{}", app.id)),
    )
}

/// Opens a document or folder with its default handler. The caller is
/// responsible for refusing executable types; this runs whatever handler
/// Windows has registered.
pub fn open_path(path: &std::path::Path) -> Result<()> {
    shell_open("open path", HSTRING::from(path.as_os_str()))
}

/// Opens a URL in the default browser. The caller validates the scheme.
pub fn open_url(url: &str) -> Result<()> {
    shell_open("open URL", HSTRING::from(url))
}

fn shell_open(what: &'static str, target: HSTRING) -> Result<()> {
    on_sta_thread(what, move || {
        // SAFETY: COM is initialized on this thread; all strings are valid,
        // NUL-terminated, and outlive the call.
        let h = unsafe { ShellExecuteW(None, w!("open"), &target, None, None, SW_SHOWNORMAL) };
        // ShellExecute reports success as a value greater than 32.
        if h.0 as usize > 32 {
            Ok(())
        } else {
            Err(WinError::Api {
                op: "ShellExecuteW",
                source: windows::core::Error::new(
                    windows::core::HRESULT(-1),
                    format!("{what} failed with code {}", h.0 as usize),
                ),
            })
        }
    })
}

/// A known folder's path, following redirection (e.g. to OneDrive).
fn known_folder(id: &GUID) -> Option<PathBuf> {
    // SAFETY: static GUID; the returned string is freed by `take_string`.
    unsafe {
        SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None)
            .ok()
            .map(|p| PathBuf::from(take_string(p)))
            .filter(|p| !p.as_os_str().is_empty())
    }
}

/// The user's profile folder (`~`).
pub fn home_folder() -> Option<PathBuf> {
    known_folder(&FOLDERID_Profile)
}

/// Desktop, Documents, Downloads, Pictures, Music, and Videos, deduplicated
/// (redirection can point two of them at the same place).
pub fn default_search_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for id in [
        &FOLDERID_Desktop,
        &FOLDERID_Documents,
        &FOLDERID_Downloads,
        &FOLDERID_Pictures,
        &FOLDERID_Music,
        &FOLDERID_Videos,
    ] {
        if let Some(p) = known_folder(id)
            && !roots.contains(&p)
        {
            roots.push(p);
        }
    }
    roots
}

/// Moves a file or folder to the Recycle Bin.
///
/// If Windows cannot recycle it (some network drives, items larger than the
/// bin), it shows its own "permanently delete?" warning instead of silently
/// destroying the item: `FOF_WANTNUKEWARNING` overrides `FOF_NOCONFIRMATION`
/// for exactly that case.
pub fn recycle(path: &std::path::Path) -> Result<()> {
    use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
    use windows::Win32::UI::Shell::{
        FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING,
        FOFX_RECYCLEONDELETE, FileOperation, IFileOperation, SHCreateItemFromParsingName,
    };

    let target = HSTRING::from(path.as_os_str());
    on_sta_thread("recycle", move || {
        // SAFETY: COM is initialized on this thread; interface pointers and
        // strings are valid for the duration of each call and used only on
        // this thread.
        unsafe {
            let op: IFileOperation =
                CoCreateInstance(&FileOperation, None, CLSCTX_ALL).ctx("create IFileOperation")?;
            op.SetOperationFlags(
                FOF_ALLOWUNDO
                    | FOF_NOCONFIRMATION
                    | FOF_SILENT
                    | FOF_NOERRORUI
                    | FOF_WANTNUKEWARNING
                    | FOFX_RECYCLEONDELETE,
            )
            .ctx("SetOperationFlags")?;
            let item: IShellItem =
                SHCreateItemFromParsingName(&target, None).ctx("SHCreateItemFromParsingName")?;
            op.DeleteItem(&item, None).ctx("DeleteItem")?;
            op.PerformOperations().ctx("PerformOperations")?;
            if op
                .GetAnyOperationsAborted()
                .ctx("GetAnyOperationsAborted")?
                .as_bool()
            {
                return Err(WinError::Api {
                    op: "recycle",
                    source: windows::core::Error::new(
                        windows::core::HRESULT(-1),
                        "the operation was cancelled",
                    ),
                });
            }
            Ok(())
        }
    })
}
