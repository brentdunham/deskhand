//! Windows platform layer: Win32, UI Automation, window management, and hotkeys.
//!
//! All unsafe code in DeskHand lives in this crate. Every `unsafe` block must
//! carry a `// SAFETY:` comment (enforced by `clippy::undocumented_unsafe_blocks`).
