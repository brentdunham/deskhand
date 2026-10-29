//! API key storage in Windows Credential Manager.
//!
//! The key never touches config files, logs, or the frontend after it is
//! saved: the UI can set or clear it and learn whether one exists, but cannot
//! read it back.

use keyring::{Entry, Error};

const SERVICE: &str = "dev.bd.deskhand";
const ACCOUNT: &str = "anthropic-api-key";

fn entry() -> Result<Entry, String> {
    Entry::new(SERVICE, ACCOUNT).map_err(|e| format!("credential store unavailable: {e}"))
}

pub fn get() -> Result<Option<String>, String> {
    match entry()?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("could not read the API key: {e}")),
    }
}

pub fn set(key: &str) -> Result<(), String> {
    entry()?
        .set_password(key)
        .map_err(|e| format!("could not save the API key: {e}"))
}

pub fn clear() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("could not remove the API key: {e}")),
    }
}
