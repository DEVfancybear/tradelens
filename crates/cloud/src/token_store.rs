//! Tokens live in the OS credential store (Windows Credential Manager, macOS
//! Keychain, Secret Service on Linux), keyed by provider.

use crate::auth::{Provider, Tokens};

pub trait TokenStore {
    fn load(&self, provider: Provider) -> crate::Result<Option<Tokens>>;
    fn save(&self, provider: Provider, tokens: &Tokens) -> crate::Result<()>;
    fn clear(&self, provider: Provider) -> crate::Result<()>;
}

/// The OS store (v0.1, `keyring` crate, service name `vn.dat.tradelens`).
pub struct KeyringStore;

impl TokenStore for KeyringStore {
    fn load(&self, _provider: Provider) -> crate::Result<Option<Tokens>> {
        todo!("v0.1")
    }

    fn save(&self, _provider: Provider, _tokens: &Tokens) -> crate::Result<()> {
        todo!("v0.1")
    }

    fn clear(&self, _provider: Provider) -> crate::Result<()> {
        todo!("v0.1")
    }
}
