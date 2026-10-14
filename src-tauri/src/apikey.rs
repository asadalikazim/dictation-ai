//! The OpenRouter API key. It lives in the macOS Keychain and is cached in
//! memory, so requests never touch the Keychain (each read can prompt).
//!
//! The key is never written to settings.json or the logs, and the UI only
//! ever receives whether one is set and its last four characters.

use std::sync::{OnceLock, RwLock};

use keyring_core::{Entry, Error as KeyError};
use serde::Serialize;

const SERVICE: &str = "com.asadkazim.dictationai";
const ACCOUNT: &str = "openrouter-api-key";

static CACHE: RwLock<Option<String>> = RwLock::new(None);
static STORE: OnceLock<Result<(), String>> = OnceLock::new();

fn entry_for(service: &str, account: &str) -> Result<Entry, String> {
    STORE
        .get_or_init(|| {
            let store = apple_native_keyring_store::keychain::Store::new()
                .map_err(|e| format!("could not open the Keychain: {e}"))?;
            keyring_core::set_default_store(store);
            Ok(())
        })
        .as_ref()
        .map_err(Clone::clone)?;
    Entry::new(service, account).map_err(|e| format!("could not open the Keychain entry: {e}"))
}

fn entry() -> Result<Entry, String> {
    entry_for(SERVICE, ACCOUNT)
}

/// Read the saved key into memory. Called once at startup. A missing item is
/// normal; any other failure is logged and treated as "no key".
pub fn load() {
    let found = entry().and_then(|e| match e.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(KeyError::NoEntry) => Ok(None),
        Err(e) => Err(format!("could not read the saved key: {e}")),
    });
    match found {
        Ok(key) => *CACHE.write().unwrap() = key,
        Err(e) => log::warn!("{e}"),
    }
}

/// The key to send with requests, if one has been saved.
pub fn current() -> Option<String> {
    CACHE.read().unwrap().clone()
}

/// Trim surrounding whitespace and reject anything that cannot be a key.
pub fn validate(raw: &str) -> Result<String, String> {
    let key = raw.trim();
    if key.is_empty() {
        return Err("Enter your OpenRouter API key.".to_string());
    }
    if key.chars().any(char::is_whitespace) {
        return Err("An API key has no spaces or line breaks.".to_string());
    }
    Ok(key.to_string())
}

/// "…ab12": enough to recognise a key without revealing it.
pub fn hint(key: &str) -> String {
    let tail: Vec<char> = key.chars().rev().take(4).collect();
    format!("…{}", tail.into_iter().rev().collect::<String>())
}

#[derive(Serialize)]
pub struct KeyStatus {
    pub set: bool,
    pub hint: Option<String>,
}

pub fn status() -> KeyStatus {
    let current = current();
    KeyStatus { set: current.is_some(), hint: current.as_deref().map(hint) }
}

fn save(raw: &str) -> Result<(), String> {
    let key = validate(raw)?;
    entry()?
        .set_password(&key)
        .map_err(|e| format!("could not save to the Keychain: {e}"))?;
    *CACHE.write().unwrap() = Some(key);
    Ok(())
}

fn clear() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) | Err(KeyError::NoEntry) => {}
        Err(e) => return Err(format!("could not remove the key from the Keychain: {e}")),
    }
    *CACHE.write().unwrap() = None;
    Ok(())
}

#[tauri::command]
pub fn get_api_key_status() -> KeyStatus {
    status()
}

#[tauri::command]
pub fn set_api_key(key: String) -> Result<KeyStatus, String> {
    save(&key)?;
    log::info!("OpenRouter API key saved to the Keychain");
    Ok(status())
}

#[tauri::command]
pub fn clear_api_key() -> Result<KeyStatus, String> {
    clear()?;
    log::info!("OpenRouter API key removed from the Keychain");
    Ok(status())
}

#[tauri::command]
pub async fn check_api_key() -> crate::openrouter::KeyCheck {
    crate::openrouter::check_key().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation() {
        assert_eq!(validate("  sk-or-v1-abc123  ").unwrap(), "sk-or-v1-abc123");
        assert!(validate("").is_err());
        assert!(validate("   ").is_err());
        assert!(validate("sk-or v1").is_err());
        assert!(validate("sk-or-v1\nabc").is_err());
    }

    #[test]
    fn hint_shows_only_the_last_four_characters() {
        assert_eq!(hint("sk-or-v1-0123456789abcdef"), "…cdef");
        assert_eq!(hint("abc"), "…abc");
        assert!(!hint("sk-or-v1-secretvalue1234").contains("secret"));
    }

    /// Round-trips a value through the real login Keychain under a throwaway
    /// service name. Run: cargo test --lib keychain_round_trip -- --ignored
    #[test]
    #[ignore = "touches the real macOS Keychain"]
    fn keychain_round_trip() {
        let service = format!("com.asadkazim.dictationai.test-{}", std::process::id());
        let entry = entry_for(&service, "round-trip").expect("entry");
        let _ = entry.delete_credential();

        assert!(matches!(entry.get_password(), Err(KeyError::NoEntry)));
        entry.set_password("sk-or-v1-test-value").expect("set");
        assert_eq!(entry.get_password().expect("get"), "sk-or-v1-test-value");
        entry.set_password("sk-or-v1-replaced").expect("replace");
        assert_eq!(entry.get_password().expect("get"), "sk-or-v1-replaced");
        entry.delete_credential().expect("delete");
        assert!(matches!(entry.get_password(), Err(KeyError::NoEntry)));
    }
}
