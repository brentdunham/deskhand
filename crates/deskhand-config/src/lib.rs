//! Configuration: the TOML schema for DeskHand settings, validation, and hot
//! reload. Secrets (API keys) never live here; they are in the OS credential
//! store.

mod config;
mod hotkey;
mod watch;

pub use config::{
    Config, ConfigError, DEFAULT_ANTHROPIC_MODEL, DEFAULT_MAX_TOKENS, FILE_NAME, ProviderConfig,
    SearchConfig, TEMPLATE,
};
pub use hotkey::{Hotkey, Key};
pub use watch::Watcher;
