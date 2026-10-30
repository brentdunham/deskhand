//! Windows platform layer: Win32, UI Automation, window management, and hotkeys.
//!
//! All unsafe code in DeskHand lives in this crate. Every `unsafe` block must
//! carry a `// SAFETY:` comment (enforced by `clippy::undocumented_unsafe_blocks`).
//!
//! The public API is safe. Window and monitor identifiers are plain handle
//! values: a stale or invalid one produces an error, never undefined behavior.
//!
//! Coordinates are physical pixels. The host process must be per-monitor DPI
//! aware (Tauri apps are) for positions to be correct on mixed-DPI setups.
//!
//! [`geometry`] is platform-independent so its math can be tested anywhere.

pub mod geometry;
pub mod image;

#[cfg(windows)]
pub mod capture;
#[cfg(windows)]
pub mod clipboard;
#[cfg(windows)]
mod error;
#[cfg(windows)]
pub mod input;
#[cfg(windows)]
pub mod monitor;
#[cfg(windows)]
pub mod shell;
#[cfg(all(windows, feature = "test-support"))]
pub mod test_support;
#[cfg(windows)]
pub mod uia;
#[cfg(windows)]
pub mod window;

#[cfg(windows)]
pub use error::{Result, WinError};
pub use geometry::{Placement, Rect, ShowState, SnapTarget};
