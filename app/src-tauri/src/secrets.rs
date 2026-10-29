//! API key storage in Windows Credential Manager.
//!
//! Keys never touch config files, logs, or the frontend after they are saved:
//! the UI can set or clear a key and learn whether one exists, but cannot
//! read it back.

use keyring::{Entry, Error};
use serde::Deserialize;

const SERVICE: &str = "dev.bd.deskhand";

/// Which provider a key belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeySlot {
    Anthropic,
    OpenaiCompatible,
}

impl KeySlot {
    fn account(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic-api-key",
            Self::OpenaiCompatible => "openai-compatible-api-key",
        }
    }
}

fn entry(slot: KeySlot) -> Result<Entry, String> {
    Entry::new(SERVICE, slot.account()).map_err(|e| format!("credential store unavailable: {e}"))
}

pub fn get(slot: KeySlot) -> Result<Option<String>, String> {
    match entry(slot)?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("could not read the API key: {e}")),
    }
}

pub fn set(slot: KeySlot, key: &str) -> Result<(), String> {
    entry(slot)?
        .set_password(key)
        .map_err(|e| format!("could not save the API key: {e}"))
}

pub fn clear(slot: KeySlot) -> Result<(), String> {
    match entry(slot)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("could not remove the API key: {e}")),
    }
}
