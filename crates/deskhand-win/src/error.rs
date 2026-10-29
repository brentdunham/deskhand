use std::fmt;

use crate::window::WindowId;

pub type Result<T> = std::result::Result<T, WinError>;

#[derive(Debug)]
pub enum WinError {
    /// A Win32 call failed.
    Api {
        op: &'static str,
        source: windows::core::Error,
    },
    /// The window no longer exists.
    WindowGone(WindowId),
    /// Windows refused to bring the window to the foreground. This happens when
    /// the calling process is not itself in the foreground.
    ForegroundDenied(WindowId),
    /// The window's application did not answer within the time limit.
    NotResponding(WindowId),
}

impl fmt::Display for WinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Api { op, source } => write!(f, "{op} failed: {source}"),
            Self::WindowGone(id) => write!(f, "window {} no longer exists", id.raw()),
            Self::ForegroundDenied(id) => {
                write!(f, "Windows refused to focus window {}", id.raw())
            }
            Self::NotResponding(id) => {
                write!(f, "window {} did not respond in time", id.raw())
            }
        }
    }
}

impl std::error::Error for WinError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Api { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub(crate) trait Context<T> {
    fn ctx(self, op: &'static str) -> Result<T>;
}

impl<T> Context<T> for windows::core::Result<T> {
    fn ctx(self, op: &'static str) -> Result<T> {
        self.map_err(|source| WinError::Api { op, source })
    }
}
