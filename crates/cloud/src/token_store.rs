//! Tokens live in the OS credential store (Windows Credential Manager, macOS
//! Keychain, Secret Service on Linux), keyed by provider, never in the database.
//!
//! [`TokenSource`] is what the drives talk to: it holds the signed-in session of one provider
//! and hands out a valid access token, refreshing it when it has ended or the provider
//! refused it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::CloudError;
use crate::auth::{self, ClientConfig, Identity, Provider, SignInOptions, Tokens};
use crate::http::unix_now;

/// Where a provider's session is kept between runs.
pub trait TokenStore: Send + Sync {
    /// The saved tokens, `None` when nobody is signed in.
    ///
    /// A persistent store may hold only the refresh token: then `access_token` is empty and
    /// `expires_at` is 0, which [`Tokens::is_expired`] reads as "refresh first".
    ///
    /// # Errors
    ///
    /// [`CloudError::Store`] when the store cannot be read.
    fn load(&self, provider: Provider) -> crate::Result<Option<Tokens>>;

    /// # Errors
    ///
    /// [`CloudError::Store`] when the store cannot be written.
    fn save(&self, provider: Provider, tokens: &Tokens) -> crate::Result<()>;

    /// Forgets the tokens and the identity.
    ///
    /// # Errors
    ///
    /// [`CloudError::Store`] when the store cannot be written.
    fn clear(&self, provider: Provider) -> crate::Result<()>;

    /// Who the saved tokens belong to, to show after a restart.
    ///
    /// # Errors
    ///
    /// [`CloudError::Store`] when the store cannot be read.
    fn load_identity(&self, provider: Provider) -> crate::Result<Option<Identity>>;

    /// # Errors
    ///
    /// [`CloudError::Store`] when the store cannot be written.
    fn save_identity(&self, identity: &Identity) -> crate::Result<()>;
}

// --- the OS store ------------------------------------------------------------------

/// The service name under which the entries are filed.
const SERVICE: &str = "vn.dat.tradelens";

/// What is persisted of [`Tokens`]: the refresh token only. The access token is short-lived,
/// and a Microsoft one can be longer than a Windows credential may be; it is fetched again
/// with the refresh token after a restart.
#[derive(Serialize, Deserialize)]
struct Persisted {
    refresh_token: Option<String>,
}

/// The OS credential store.
#[derive(Debug, Clone)]
pub struct KeyringStore {
    service: String,
}

impl Default for KeyringStore {
    fn default() -> Self {
        Self::new()
    }
}

fn store_error(e: &keyring::Error) -> CloudError {
    CloudError::Store(e.to_string())
}

impl KeyringStore {
    pub fn new() -> Self {
        Self { service: SERVICE.to_owned() }
    }

    /// A store under another service name, so tests never touch the real entries.
    pub fn with_service(service: impl Into<String>) -> Self {
        Self { service: service.into() }
    }

    fn entry(&self, name: &str) -> crate::Result<keyring::Entry> {
        keyring::Entry::new(&self.service, name).map_err(|e| store_error(&e))
    }

    /// Any secret of the app by name (the Notion token, an AI key).
    ///
    /// # Errors
    ///
    /// [`CloudError::Store`] when the store cannot be read.
    pub fn load_secret(&self, name: &str) -> crate::Result<Option<String>> {
        match self.entry(name)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(store_error(&e)),
        }
    }

    /// # Errors
    ///
    /// [`CloudError::Store`] when the store cannot be written.
    pub fn save_secret(&self, name: &str, secret: &str) -> crate::Result<()> {
        self.entry(name)?.set_password(secret).map_err(|e| store_error(&e))
    }

    /// Removes a secret; one that is not there is not an error.
    ///
    /// # Errors
    ///
    /// [`CloudError::Store`] when the store cannot be written.
    pub fn clear_secret(&self, name: &str) -> crate::Result<()> {
        match self.entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(store_error(&e)),
        }
    }

    fn tokens_name(provider: Provider) -> String {
        format!("{}-tokens", provider.key())
    }

    fn identity_name(provider: Provider) -> String {
        format!("{}-identity", provider.key())
    }
}

impl TokenStore for KeyringStore {
    fn load(&self, provider: Provider) -> crate::Result<Option<Tokens>> {
        let Some(json) = self.load_secret(&Self::tokens_name(provider))? else { return Ok(None) };
        let persisted: Persisted = serde_json::from_str(&json)
            .map_err(|e| CloudError::Store(crate::tr!("cloud.store.unreadable_token", error = e)))?;
        Ok(Some(Tokens { access_token: String::new(), refresh_token: persisted.refresh_token, expires_at: 0 }))
    }

    fn save(&self, provider: Provider, tokens: &Tokens) -> crate::Result<()> {
        let json = serde_json::to_string(&Persisted { refresh_token: tokens.refresh_token.clone() })
            .map_err(|e| CloudError::Store(e.to_string()))?;
        self.save_secret(&Self::tokens_name(provider), &json)
    }

    fn clear(&self, provider: Provider) -> crate::Result<()> {
        self.clear_secret(&Self::tokens_name(provider))?;
        self.clear_secret(&Self::identity_name(provider))
    }

    fn load_identity(&self, provider: Provider) -> crate::Result<Option<Identity>> {
        let Some(json) = self.load_secret(&Self::identity_name(provider))? else { return Ok(None) };
        Ok(serde_json::from_str(&json).ok())
    }

    fn save_identity(&self, identity: &Identity) -> crate::Result<()> {
        let json = serde_json::to_string(identity).map_err(|e| CloudError::Store(e.to_string()))?;
        self.save_secret(&Self::identity_name(identity.provider), &json)
    }
}

// --- the in-memory store -----------------------------------------------------------

/// Tokens kept in memory only: for tests, and for a run that must leave nothing behind.
#[derive(Debug, Default)]
pub struct MemoryStore {
    tokens: Mutex<HashMap<Provider, Tokens>>,
    identities: Mutex<HashMap<Provider, Identity>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().expect("lock poisoned")
}

impl TokenStore for MemoryStore {
    fn load(&self, provider: Provider) -> crate::Result<Option<Tokens>> {
        Ok(locked(&self.tokens).get(&provider).cloned())
    }

    fn save(&self, provider: Provider, tokens: &Tokens) -> crate::Result<()> {
        locked(&self.tokens).insert(provider, tokens.clone());
        Ok(())
    }

    fn clear(&self, provider: Provider) -> crate::Result<()> {
        locked(&self.tokens).remove(&provider);
        locked(&self.identities).remove(&provider);
        Ok(())
    }

    fn load_identity(&self, provider: Provider) -> crate::Result<Option<Identity>> {
        Ok(locked(&self.identities).get(&provider).cloned())
    }

    fn save_identity(&self, identity: &Identity) -> crate::Result<()> {
        locked(&self.identities).insert(identity.provider, identity.clone());
        Ok(())
    }
}

// --- the session -------------------------------------------------------------------

/// The signed-in session of one provider: tokens from a [`TokenStore`], refreshed on demand.
/// Share it between threads in an [`Arc`]; refreshes are serialised, so a burst of requests
/// with an expired token refreshes once.
pub struct TokenSource {
    client: ClientConfig,
    store: Arc<dyn TokenStore>,
    current: Mutex<Option<Tokens>>,
}

impl std::fmt::Debug for TokenSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenSource").field("provider", &self.client.provider).finish_non_exhaustive()
    }
}

impl TokenSource {
    pub fn new(client: ClientConfig, store: Arc<dyn TokenStore>) -> Self {
        Self { client, store, current: Mutex::new(None) }
    }

    pub fn provider(&self) -> Provider {
        self.client.provider
    }

    /// Signs in through the browser and keeps the tokens and the identity in the store.
    ///
    /// # Errors
    ///
    /// As [`auth::sign_in`]; [`CloudError::Store`] when the result cannot be saved.
    pub fn sign_in(&self, options: &SignInOptions, open_browser: impl FnOnce(&str)) -> crate::Result<Identity> {
        let (identity, tokens) = auth::sign_in(&self.client, options, open_browser)?;
        self.store.save(self.client.provider, &tokens)?;
        self.store.save_identity(&identity)?;
        *locked(&self.current) = Some(tokens);
        Ok(identity)
    }

    /// Revokes where the provider allows, and forgets the session. Always forgets.
    ///
    /// # Errors
    ///
    /// [`CloudError::Store`] when the store cannot be cleared.
    pub fn sign_out(&self) -> crate::Result<()> {
        *locked(&self.current) = None;
        auth::sign_out(&self.client, self.store.as_ref())
    }

    /// Whether a session is saved (it may still turn out to be revoked at the next refresh).
    ///
    /// # Errors
    ///
    /// [`CloudError::Store`] when the store cannot be read.
    pub fn is_signed_in(&self) -> crate::Result<bool> {
        Ok(self.store.load(self.client.provider)?.is_some())
    }

    /// Who is signed in, `None` when nobody is.
    ///
    /// # Errors
    ///
    /// [`CloudError::Store`] when the store cannot be read.
    pub fn identity(&self) -> crate::Result<Option<Identity>> {
        self.store.load_identity(self.client.provider)
    }

    /// An access token valid for the next minute, refreshed first when it is not.
    ///
    /// # Errors
    ///
    /// [`CloudError::SignedOut`] when nobody is signed in or the provider revoked the session;
    /// [`CloudError::Network`] when the refresh cannot reach the provider.
    pub fn access_token(&self) -> crate::Result<String> {
        let mut current = locked(&self.current);
        let tokens = self.loaded(&mut current)?;
        if !tokens.is_expired(unix_now()) {
            return Ok(tokens.access_token.clone());
        }
        self.renew(&mut current)
    }

    /// A new access token after the provider answered 401 to `rejected`. When another thread
    /// has already replaced `rejected`, that one is returned without another refresh.
    ///
    /// # Errors
    ///
    /// As [`TokenSource::access_token`].
    pub fn renewed_after_rejection(&self, rejected: &str) -> crate::Result<String> {
        let mut current = locked(&self.current);
        let tokens = self.loaded(&mut current)?;
        if tokens.access_token != rejected && !tokens.is_expired(unix_now()) {
            return Ok(tokens.access_token.clone());
        }
        self.renew(&mut current)
    }

    /// The tokens in memory, read from the store the first time.
    fn loaded<'a>(&self, current: &'a mut Option<Tokens>) -> crate::Result<&'a Tokens> {
        if current.is_none() {
            *current = self.store.load(self.client.provider)?;
        }
        current.as_ref().ok_or(CloudError::SignedOut)
    }

    fn renew(&self, current: &mut Option<Tokens>) -> crate::Result<String> {
        let old = current.as_ref().ok_or(CloudError::SignedOut)?;
        match auth::refresh(&self.client, old) {
            Ok(fresh) => {
                self.store.save(self.client.provider, &fresh)?;
                let token = fresh.access_token.clone();
                *current = Some(fresh);
                Ok(token)
            }
            Err(CloudError::SignedOut) => {
                // Revoked or expired for good: forget it so the app asks to sign in again.
                *current = None;
                if let Err(e) = self.store.clear(self.client.provider) {
                    log::warn!("không xoá được phiên đã hết hạn: {e}");
                }
                Err(CloudError::SignedOut)
            }
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_memory_store_keeps_tokens_and_identity_until_cleared() {
        let store = MemoryStore::new();
        let tokens = Tokens { access_token: "a".into(), refresh_token: Some("r".into()), expires_at: 5 };
        let me = Identity { provider: Provider::Google, email: "a@b.c".into(), name: "A".into() };
        assert_eq!(store.load(Provider::Google).expect("loads"), None);
        store.save(Provider::Google, &tokens).expect("saves");
        store.save_identity(&me).expect("saves");
        assert_eq!(store.load(Provider::Google).expect("loads"), Some(tokens));
        assert_eq!(store.load(Provider::Microsoft).expect("loads"), None, "providers are separate");
        assert_eq!(store.load_identity(Provider::Google).expect("loads"), Some(me));
        store.clear(Provider::Google).expect("clears");
        assert_eq!(store.load(Provider::Google).expect("loads"), None);
        assert_eq!(store.load_identity(Provider::Google).expect("loads"), None);
    }

    #[test]
    fn nobody_signed_in_is_signed_out() {
        let source = TokenSource::new(ClientConfig::new(Provider::Google, "id"), Arc::new(MemoryStore::new()));
        assert!(matches!(source.access_token(), Err(CloudError::SignedOut)));
        assert!(!source.is_signed_in().expect("reads"));
    }

    #[test]
    fn a_valid_token_is_handed_out_without_a_refresh() {
        let store = Arc::new(MemoryStore::new());
        let valid = Tokens { access_token: "good".into(), refresh_token: None, expires_at: unix_now() + 3600 };
        store.save(Provider::Google, &valid).expect("saves");
        let source = TokenSource::new(ClientConfig::new(Provider::Google, "id"), store);
        assert_eq!(source.access_token().expect("valid"), "good");
        assert_eq!(source.renewed_after_rejection("other").expect("already replaced"), "good");
    }
}
