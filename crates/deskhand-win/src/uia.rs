//! Reading text from other applications' windows through UI Automation.
//!
//! Every read runs on its own thread with its own COM apartment, under a
//! deadline: UI Automation calls execute against the target application, and
//! a hung application can otherwise block the caller indefinitely.
//!
//! Password fields are never read. The returned text comes from another
//! application and is untrusted.

use std::ffi::c_void;
use std::sync::{Mutex, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomation2, IUIAutomationCacheRequest, IUIAutomationElement,
    IUIAutomationTextPattern, IUIAutomationTreeWalker, IUIAutomationValuePattern,
    UIA_ButtonControlTypeId, UIA_CONTROLTYPE_ID, UIA_CheckBoxControlTypeId,
    UIA_ComboBoxControlTypeId, UIA_ControlTypePropertyId, UIA_DocumentControlTypeId,
    UIA_EditControlTypeId, UIA_HeaderItemControlTypeId, UIA_HyperlinkControlTypeId,
    UIA_IsPasswordPropertyId, UIA_ListItemControlTypeId, UIA_MenuItemControlTypeId,
    UIA_NamePropertyId, UIA_RadioButtonControlTypeId, UIA_ScrollBarControlTypeId,
    UIA_TabItemControlTypeId, UIA_TextPatternId, UIA_TitleBarControlTypeId,
    UIA_TreeItemControlTypeId, UIA_ValuePatternId,
};
use windows::core::Interface;

use crate::error::{Context, Result, WinError};
use crate::window::{WindowId, live};

#[derive(Debug, Clone, Copy)]
pub struct TextLimits {
    pub max_chars: usize,
    pub max_elements: usize,
    /// Wall-clock budget for walking the tree. The thread gets a few extra
    /// seconds beyond this to finish its current call before it is abandoned.
    pub time_budget: Duration,
}

impl Default for TextLimits {
    fn default() -> Self {
        Self {
            max_chars: 20_000,
            max_elements: 4_000,
            time_budget: Duration::from_secs(4),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowText {
    /// One line per element; non-text elements carry a role prefix such as
    /// `[Button]`. Untrusted.
    pub text: String,
    /// A limit was reached before the whole tree was read.
    pub truncated: bool,
    pub elements: usize,
    pub password_fields_skipped: usize,
}

/// Grace period for an in-flight UIA call after the budget runs out.
const ABANDON_AFTER: Duration = Duration::from_secs(4);

/// Serializes reads. Concurrent UI Automation clients in one process have
/// failed intermittently with E_FAIL during setup; reads are short and rare,
/// so one at a time costs nothing.
static READ_LOCK: Mutex<()> = Mutex::new(());

/// Reads the text of a window and its descendants.
pub fn read_text(id: WindowId, limits: TextLimits) -> Result<WindowText> {
    live(id)?;
    let raw = id.raw();
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("deskhand-uia".into())
        .spawn(move || {
            let _serial = READ_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = tx.send(read_on_this_thread(raw, limits));
        })
        .map_err(|e| WinError::Api {
            op: "spawn UI Automation thread",
            source: windows::core::Error::new(windows::core::HRESULT(-1), e.to_string()),
        })?;
    // A hung target leaves the thread blocked until UIA's own timeouts
    // release it; the caller does not wait for that.
    rx.recv_timeout(limits.time_budget + ABANDON_AFTER)
        .unwrap_or(Err(WinError::NotResponding(id)))
}

/// Balances COM initialization on the current thread.
struct ComApartment;

impl ComApartment {
    fn init() -> Result<Self> {
        // SAFETY: called once at the start of a thread this module owns;
        // balanced by `CoUninitialize` in `Drop`.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .ctx("CoInitializeEx")?;
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        // SAFETY: pairs with the successful `CoInitializeEx` in `init`. All COM
        // objects created on this thread are dropped before this guard,
        // because it is declared first in `read_on_this_thread`.
        unsafe { CoUninitialize() };
    }
}

fn read_on_this_thread(raw: isize, limits: TextLimits) -> Result<WindowText> {
    let _apartment = ComApartment::init()?;

    // SAFETY: COM is initialized on this thread for the apartment guard's
    // lifetime, and CUIAutomation is the documented class for IUIAutomation.
    let automation: IUIAutomation =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
            .ctx("create UI Automation client")?;

    // Bound how long a hung target can block each call (Windows 8+).
    if let Ok(a2) = automation.cast::<IUIAutomation2>() {
        // SAFETY: plain setters on a valid interface pointer.
        unsafe {
            let _ = a2.SetConnectionTimeout(2_000);
            let _ = a2.SetTransactionTimeout(3_000);
        }
    }

    let hwnd = HWND(raw as *mut c_void);
    // SAFETY: valid interface pointers; `hwnd` was checked live by the caller
    // and an invalid handle produces an error, not undefined behavior.
    let (cache, walker, root) = unsafe {
        let cache: IUIAutomationCacheRequest =
            automation.CreateCacheRequest().ctx("CreateCacheRequest")?;
        for property in [
            UIA_NamePropertyId,
            UIA_ControlTypePropertyId,
            UIA_IsPasswordPropertyId,
        ] {
            cache.AddProperty(property).ctx("AddProperty")?;
        }
        let walker: IUIAutomationTreeWalker =
            automation.ControlViewWalker().ctx("ControlViewWalker")?;
        let root = automation
            .ElementFromHandle(hwnd)
            .ctx("ElementFromHandle")?
            .BuildUpdatedCache(&cache)
            .ctx("BuildUpdatedCache")?;
        (cache, walker, root)
    };

    let mut out = Collector::new(limits);
    // Depth-first, document order.
    let mut stack = vec![root];
    while let Some(element) = stack.pop() {
        if out.limit_reached() {
            out.truncated = true;
            break;
        }
        if !out.visit(&element) {
            continue;
        }
        let mut children = Vec::new();
        // SAFETY: valid interface pointers. A missing child or sibling comes
        // back as an error, which ends the loop.
        let mut next = unsafe { walker.GetFirstChildElementBuildCache(&element, &cache) }.ok();
        while let Some(child) = next {
            // SAFETY: as above.
            next = unsafe { walker.GetNextSiblingElementBuildCache(&child, &cache) }.ok();
            children.push(child);
            if children.len() >= limits.max_elements {
                break;
            }
        }
        stack.extend(children.into_iter().rev());
    }
    Ok(out.finish())
}

struct Collector {
    limits: TextLimits,
    deadline: Instant,
    text: String,
    last_line: String,
    elements: usize,
    passwords: usize,
    truncated: bool,
}

impl Collector {
    fn new(limits: TextLimits) -> Self {
        Self {
            limits,
            deadline: Instant::now() + limits.time_budget,
            text: String::new(),
            last_line: String::new(),
            elements: 0,
            passwords: 0,
            truncated: false,
        }
    }

    fn limit_reached(&self) -> bool {
        self.text.len() >= self.limits.max_chars
            || self.elements >= self.limits.max_elements
            || Instant::now() >= self.deadline
    }

    fn remaining_chars(&self) -> usize {
        self.limits.max_chars.saturating_sub(self.text.len())
    }

    /// Records one element. Returns whether to walk its children.
    fn visit(&mut self, el: &IUIAutomationElement) -> bool {
        self.elements += 1;
        // SAFETY: reads cached properties from a valid element.
        let (control_type, is_password, name) = unsafe {
            (
                el.CachedControlType().unwrap_or_default(),
                el.CachedIsPassword().map(|b| b.as_bool()),
                el.CachedName().map(|b| b.to_string()).unwrap_or_default(),
            )
        };

        // Window chrome adds noise, not content.
        if control_type == UIA_TitleBarControlTypeId || control_type == UIA_ScrollBarControlTypeId {
            return false;
        }

        let is_text_field =
            control_type == UIA_EditControlTypeId || control_type == UIA_DocumentControlTypeId;
        // Never read a password field. For text fields, an unknown answer
        // counts as a password field.
        if is_password.unwrap_or(is_text_field) {
            self.passwords += 1;
            return false;
        }

        if is_text_field && let Some(content) = field_text(el, self.remaining_chars()) {
            let role = (control_type == UIA_EditControlTypeId).then_some("Edit");
            if !name.trim().is_empty() && name.trim() != content.trim() {
                self.push(role, &format!("{}: {content}", name.trim()));
            } else {
                self.push(role, &content);
            }
            // The field's text already covers its children.
            return false;
        }

        self.push(role_label(control_type), &name);
        true
    }

    fn push(&mut self, role: Option<&str>, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let line = match role {
            Some(r) => format!("[{r}] {text}"),
            None => text.to_owned(),
        };
        // Containers often repeat their only child's name.
        if line == self.last_line {
            return;
        }
        let remaining = self.remaining_chars();
        if line.len() + 1 > remaining {
            let mut cut = remaining.min(line.len());
            while !line.is_char_boundary(cut) {
                cut -= 1;
            }
            self.text.push_str(&line[..cut]);
            self.truncated = true;
        } else {
            self.text.push_str(&line);
            self.text.push('\n');
        }
        self.last_line = line;
    }

    fn finish(self) -> WindowText {
        WindowText {
            text: self.text.trim_end().to_owned(),
            truncated: self.truncated,
            elements: self.elements,
            password_fields_skipped: self.passwords,
        }
    }
}

/// Full text of a document or edit field: the Text pattern when available
/// (whole documents in one call), otherwise the Value pattern.
fn field_text(el: &IUIAutomationElement, max_chars: usize) -> Option<String> {
    let max = i32::try_from(max_chars).unwrap_or(i32::MAX);
    // SAFETY: pattern queries and reads on a valid element; failures are
    // errors, mapped to `None`.
    unsafe {
        if let Ok(pattern) = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
            && let Ok(text) = pattern.DocumentRange().and_then(|r| r.GetText(max))
        {
            return Some(text.to_string());
        }
        el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .and_then(|p| p.CurrentValue())
            .ok()
            .map(|v| v.to_string())
    }
}

const ROLE_LABELS: [(UIA_CONTROLTYPE_ID, &str); 10] = [
    (UIA_ButtonControlTypeId, "Button"),
    (UIA_HyperlinkControlTypeId, "Link"),
    (UIA_MenuItemControlTypeId, "MenuItem"),
    (UIA_TabItemControlTypeId, "Tab"),
    (UIA_ListItemControlTypeId, "ListItem"),
    (UIA_TreeItemControlTypeId, "TreeItem"),
    (UIA_CheckBoxControlTypeId, "CheckBox"),
    (UIA_RadioButtonControlTypeId, "Radio"),
    (UIA_ComboBoxControlTypeId, "ComboBox"),
    (UIA_HeaderItemControlTypeId, "Header"),
];

fn role_label(t: UIA_CONTROLTYPE_ID) -> Option<&'static str> {
    ROLE_LABELS
        .iter()
        .find(|(id, _)| *id == t)
        .map(|(_, label)| *label)
}

/// The text selected in whatever element has keyboard focus, when that
/// element exposes a selection through the Text pattern (browsers, most
/// editors, Windows Terminal).
///
/// Call before DeskHand's panel takes focus. Password fields are never read.
/// Best effort: anything that fails or exceeds `timeout` yields `None`, so a
/// slow app never delays the panel by more than `timeout`.
pub fn read_focused_selection(max_chars: usize, timeout: Duration) -> Option<String> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("deskhand-uia-selection".into())
        .spawn(move || {
            let _serial = READ_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = tx.send(selection_on_this_thread(max_chars));
        })
        .ok()?;
    rx.recv_timeout(timeout).ok().flatten()
}

fn selection_on_this_thread(max_chars: usize) -> Option<String> {
    let _apartment = ComApartment::init().ok()?;
    let max = i32::try_from(max_chars).unwrap_or(i32::MAX);
    // SAFETY: COM is initialized on this thread for the apartment guard's
    // lifetime; every call is on valid interface pointers created here, and
    // failures surface as errors mapped to `None`.
    unsafe {
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
        if let Ok(a2) = automation.cast::<IUIAutomation2>() {
            let _ = a2.SetConnectionTimeout(1_000);
            let _ = a2.SetTransactionTimeout(1_000);
        }
        let focused = automation.GetFocusedElement().ok()?;
        // Unknown counts as a password field.
        if focused
            .CurrentIsPassword()
            .map(|b| b.as_bool())
            .unwrap_or(true)
        {
            return None;
        }
        let pattern = focused
            .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
            .ok()?;
        let ranges = pattern.GetSelection().ok()?;
        let mut parts = Vec::new();
        for i in 0..ranges.Length().ok()? {
            if let Ok(text) = ranges.GetElement(i).and_then(|r| r.GetText(max)) {
                let text = text.to_string();
                if !text.trim().is_empty() {
                    parts.push(text);
                }
            }
        }
        let joined = parts.join("\n");
        (!joined.is_empty()).then(|| joined.chars().take(max_chars).collect())
    }
}
