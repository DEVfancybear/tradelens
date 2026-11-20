//! Tokens live in the OS credential store (Windows Credential Manager, macOS
//! Keychain, Secret Service on Linux), keyed by provider.

use crate::auth::{Provider, Tokens};

pub trait TokenStore {
    /// # Errors
    ///
    /// [`crate::CloudError`] when the provider, the network or the store fails.
    fn load(&self, provider: Provider) -> crate::Result<Option<Tokens>>;
    /// # Errors
    ///
    /// [`crate::CloudError`] when the provider, the network or the store fails.
    fn save(&self, provider: Provider, tokens: &Tokens) -> crate::Result<()>;
    /// # Errors
    ///
    /// [`crate::CloudError`] when the provider, the network or the store fails.
    fn clear(&self, provider: Provider) -> crate::Result<()>;
}

/// The OS store (v0.1, `keyring` crate, service name `vn.dat.tradelens`).
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyringStore;

impl TokenStore for KeyringStore {
    /// # Errors
    ///
    /// [`crate::CloudError`] when the provider, the network or the store fails.
    fn load(&self, _provider: Provider) -> crate::Result<Option<Tokens>> {
        Err(crate::CloudError::NotImplemented("v0.1"))
    }

    /// # Errors
    ///
    /// [`crate::CloudError`] when the provider, the network or the store fails.
    fn save(&self, _provider: Provider, _tokens: &Tokens) -> crate::Result<()> {
        Err(crate::CloudError::NotImplemented("v0.1"))
    }

    /// # Errors
    ///
    /// [`crate::CloudError`] when the provider, the network or the store fails.
    fn clear(&self, _provider: Provider) -> crate::Result<()> {
        Err(crate::CloudError::NotImplemented("v0.1"))
    }
}
