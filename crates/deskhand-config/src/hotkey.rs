//! Global hotkey parsing.

use std::fmt;
use std::str::FromStr;

/// A global hotkey such as `Ctrl+Alt+D`.
///
/// At least one of Ctrl, Alt, or Win is required: a bare key, or Shift plus a
/// key, would capture ordinary typing in every application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: Key,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// `A` to `Z`, stored uppercase.
    Letter(char),
    /// `0` to `9`.
    Digit(u8),
    /// `F1` to `F24`.
    Function(u8),
    Space,
}

impl Hotkey {
    /// The accelerator string `tauri-plugin-global-shortcut` parses, e.g.
    /// `ctrl+alt+KeyD`.
    pub fn accelerator(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.ctrl {
            parts.push("ctrl".into());
        }
        if self.alt {
            parts.push("alt".into());
        }
        if self.shift {
            parts.push("shift".into());
        }
        if self.win {
            parts.push("super".into());
        }
        parts.push(match self.key {
            Key::Letter(c) => format!("Key{c}"),
            Key::Digit(d) => format!("Digit{d}"),
            Key::Function(n) => format!("F{n}"),
            Key::Space => "Space".into(),
        });
        parts.join("+")
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.win, "Win"),
        ] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        match self.key {
            Key::Letter(c) => write!(f, "{c}"),
            Key::Digit(d) => write!(f, "{d}"),
            Key::Function(n) => write!(f, "F{n}"),
            Key::Space => f.write_str("Space"),
        }
    }
}

impl FromStr for Hotkey {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let (key, modifiers) = parts
            .split_last()
            .filter(|(k, _)| !k.is_empty())
            .ok_or_else(|| format!("hotkey `{s}` has no key"))?;

        let mut h = Hotkey {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            key: parse_key(key).ok_or_else(|| {
                format!("hotkey key `{key}` is not supported; use A-Z, 0-9, F1-F24, or Space")
            })?,
        };
        for m in modifiers {
            let flag = match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut h.ctrl,
                "alt" => &mut h.alt,
                "shift" => &mut h.shift,
                "win" | "super" | "meta" => &mut h.win,
                _ => {
                    return Err(format!(
                        "unknown hotkey modifier `{m}`; use Ctrl, Alt, Shift, or Win"
                    ));
                }
            };
            if *flag {
                return Err(format!("hotkey `{s}` repeats `{m}`"));
            }
            *flag = true;
        }
        if !(h.ctrl || h.alt || h.win) {
            return Err(format!(
                "hotkey `{s}` needs Ctrl, Alt, or Win; otherwise it would capture normal typing"
            ));
        }
        Ok(h)
    }
}

fn parse_key(k: &str) -> Option<Key> {
    let upper = k.to_ascii_uppercase();
    let mut chars = upper.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_uppercase() => return Some(Key::Letter(c)),
        (Some(c), None) if c.is_ascii_digit() => return Some(Key::Digit(c as u8 - b'0')),
        _ => {}
    }
    if upper == "SPACE" {
        return Some(Key::Space);
    }
    let n: u8 = upper.strip_prefix('F')?.parse().ok()?;
    (1..=24).contains(&n).then_some(Key::Function(n))
}

impl TryFrom<String> for Hotkey {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<Hotkey> for String {
    fn from(h: Hotkey) -> Self {
        h.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hk(s: &str) -> Hotkey {
        s.parse().unwrap()
    }

    #[test]
    fn parses_and_canonicalizes() {
        let h = hk(" ctrl + ALT + d ");
        assert_eq!(h.to_string(), "Ctrl+Alt+D");
        assert_eq!(h.accelerator(), "ctrl+alt+KeyD");
        assert_eq!(hk("Win+Shift+F12").to_string(), "Shift+Win+F12");
        assert_eq!(hk("Control+Super+space").accelerator(), "ctrl+super+Space");
        assert_eq!(hk("Alt+7").accelerator(), "alt+Digit7");
    }

    #[test]
    fn rejects_hotkeys_that_would_capture_typing() {
        assert!("D".parse::<Hotkey>().is_err());
        assert!("Shift+D".parse::<Hotkey>().is_err());
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in [
            "",
            "Ctrl+",
            "Ctrl+Ctrl+D",
            "Hyper+D",
            "Ctrl+Enter",
            "Ctrl+F25",
            "Ctrl+F0",
            "Ctrl+DD",
        ] {
            assert!(bad.parse::<Hotkey>().is_err(), "{bad} should fail");
        }
    }
}
