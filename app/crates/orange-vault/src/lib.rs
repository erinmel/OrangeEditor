//! Credential storage for an application: the operating system's own store, and a
//! store in memory for tests and for machines without one.
//!
//! A document never holds a secret. It names one as `credential://<name>` in a
//! connection's settings, and the host replaces that reference with the secret from the
//! application's [`SecretStore`] when it opens the connection. [`OsVault`] keeps secrets
//! in Windows Credential Manager, the macOS Keychain or, on Linux, the desktop's Secret
//! Service (GNOME Keyring, KWallet), so they are kept across restarts, protected by the
//! user's login and never written by this project to a file.
//!
//! On Linux the Secret Service is reached over D-Bus. A machine without one, such as
//! a server with no desktop session, reports the store as unavailable instead of
//! keeping secrets somewhere weaker.
// Product code reports failures as typed errors; each remaining panic is a documented
// internal invariant (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

use orange_sdk::{SecretError, SecretStore, is_secret_name};
use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

/// Longest secret that is stored, in bytes. Operating-system stores have limits of
/// their own near this size.
pub const MAX_SECRET_BYTES: usize = 2048;

fn check(name: &str) -> Result<(), SecretError> {
    if is_secret_name(name) {
        Ok(())
    } else {
        Err(SecretError::InvalidName)
    }
}

/// The operating system's credential store, under one service name.
#[derive(Debug, Clone)]
pub struct OsVault {
    service: String,
}

impl OsVault {
    /// A vault whose entries the operating system lists under `service`, such as the
    /// application's name. Nothing is contacted until a secret is read or written.
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, name: &str) -> Result<keyring::Entry, SecretError> {
        check(name)?;
        keyring::Entry::new(&self.service, name).map_err(|error| failure(&error))
    }

    /// Stores `secret` under `name`, replacing what was there.
    pub fn set(&self, name: &str, secret: &str) -> Result<(), SecretError> {
        if secret.is_empty() || secret.len() > MAX_SECRET_BYTES {
            return Err(SecretError::InvalidSecret);
        }
        self.entry(name)?
            .set_password(secret)
            .map_err(|error| failure(&error))
    }

    /// Deletes the secret under `name`. Deleting one that does not exist is
    /// [`SecretError::NotFound`].
    pub fn remove(&self, name: &str) -> Result<(), SecretError> {
        self.entry(name)?
            .delete_credential()
            .map_err(|error| failure(&error))
    }
}

/// The store's own message can name accounts and paths; only the kind is kept.
fn failure(error: &keyring::Error) -> SecretError {
    match error {
        keyring::Error::NoEntry => SecretError::NotFound,
        keyring::Error::NoStorageAccess(_) => SecretError::Unavailable("locked or not reachable"),
        keyring::Error::TooLong(..) | keyring::Error::Invalid(..) => SecretError::InvalidSecret,
        _ => SecretError::Unavailable("the operating system's store reported a failure"),
    }
}

impl SecretStore for OsVault {
    fn get(&self, name: &str) -> Result<String, SecretError> {
        self.entry(name)?
            .get_password()
            .map_err(|error| failure(&error))
    }
    fn replace(&self, name: &str, secret: &str) -> Result<(), SecretError> {
        self.set(name, secret)
    }
}

/// Secrets held in memory for the life of the value: for tests, and for an application
/// that asks the user for them at each start.
#[derive(Default)]
pub struct MemoryVault {
    secrets: Mutex<BTreeMap<String, String>>,
}
/// The secrets are left out.
impl std::fmt::Debug for MemoryVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self
            .secrets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        f.debug_struct("MemoryVault")
            .field("secrets", &count)
            .finish()
    }
}

impl MemoryVault {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores `secret` under `name`, replacing what was there.
    pub fn set(&self, name: &str, secret: &str) -> Result<(), SecretError> {
        check(name)?;
        if secret.is_empty() || secret.len() > MAX_SECRET_BYTES {
            return Err(SecretError::InvalidSecret);
        }
        let _ = self
            .secrets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned(), secret.to_owned());
        Ok(())
    }

    /// Deletes the secret under `name`.
    pub fn remove(&self, name: &str) -> Result<(), SecretError> {
        check(name)?;
        self.secrets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(name)
            .map(|_| ())
            .ok_or(SecretError::NotFound)
    }
}

impl SecretStore for MemoryVault {
    fn get(&self, name: &str) -> Result<String, SecretError> {
        check(name)?;
        self.secrets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
            .ok_or(SecretError::NotFound)
    }
    fn replace(&self, name: &str, secret: &str) -> Result<(), SecretError> {
        self.set(name, secret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_memory_vault_stores_replaces_and_removes_without_showing_secrets() {
        let vault = MemoryVault::new();
        assert_eq!(vault.get("api/token"), Err(SecretError::NotFound));
        vault.set("api/token", "first").unwrap();
        vault.set("api/token", "second").unwrap();
        assert_eq!(vault.get("api/token").as_deref(), Ok("second"));
        assert!(!format!("{vault:?}").contains("second"));
        assert_eq!(vault.set("Bad Name", "x"), Err(SecretError::InvalidName));
        assert_eq!(vault.set("empty", ""), Err(SecretError::InvalidSecret));
        assert_eq!(
            vault.set("long", &"x".repeat(MAX_SECRET_BYTES + 1)),
            Err(SecretError::InvalidSecret)
        );
        vault.remove("api/token").unwrap();
        assert_eq!(vault.remove("api/token"), Err(SecretError::NotFound));
    }

    /// Writes to the real store of the machine it runs on, so it runs only on request:
    /// `cargo test -p orange-vault -- --ignored`.
    #[test]
    #[ignore = "uses the operating system's credential store"]
    fn the_operating_system_store_keeps_a_secret() {
        let vault = OsVault::new("orange-editor-test");
        let name = format!("test/{}", std::process::id());
        assert_eq!(vault.get(&name), Err(SecretError::NotFound));
        vault.set(&name, "generated-test-value").unwrap();
        assert_eq!(vault.get(&name).as_deref(), Ok("generated-test-value"));
        vault.remove(&name).unwrap();
        assert_eq!(vault.get(&name), Err(SecretError::NotFound));
    }
}
