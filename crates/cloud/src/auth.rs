//! OAuth 2.0 for installed apps: PKCE, the system browser, a redirect to
//! `http://127.0.0.1:<random port>` that the app listens on once.

use serde::{Deserialize, Serialize};

/// Where the user signs in, and where the data goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Provider {
    /// Google account → Google Drive `appDataFolder`.
    Google,
    /// Microsoft account → OneDrive `approot`.
    Microsoft,
}

impl Provider {
    pub const ALL: [Self; 2] = [Self::Google, Self::Microsoft];

    pub fn label(self) -> &'static str {
        match self {
            Self::Google => "Google Drive",
            Self::Microsoft => "OneDrive",
        }
    }

    pub fn authorize_url(self) -> &'static str {
        match self {
            Self::Google => "https://accounts.google.com/o/oauth2/v2/auth",
            Self::Microsoft => "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
        }
    }

    pub fn token_url(self) -> &'static str {
        match self {
            Self::Google => "https://oauth2.googleapis.com/token",
            Self::Microsoft => "https://login.microsoftonline.com/common/oauth2/v2.0/token",
        }
    }

    /// The narrowest scopes that reach the app folder and the user's name.
    pub fn scopes(self) -> &'static [&'static str] {
        match self {
            Self::Google => &["openid", "email", "profile", "https://www.googleapis.com/auth/drive.appdata"],
            Self::Microsoft => &["openid", "email", "profile", "offline_access", "Files.ReadWrite.AppFolder"],
        }
    }
}

/// The app's registration with a provider (client ids are not secrets for
/// installed apps; they ship in the binary or come from the environment in dev).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    pub provider: Provider,
    pub client_id: String,
}

/// Who is signed in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub provider: Provider,
    pub email: String,
    pub name: String,
}

/// Tokens for one provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix seconds.
    pub expires_at: i64,
}

/// Opens the browser at the consent page, waits for the redirect, exchanges the code.
///
/// # Errors
///
/// [`crate::CloudError`] when the provider, the network or the store fails.
pub fn sign_in(_client: &ClientConfig) -> crate::Result<(Identity, Tokens)> {
    Err(crate::CloudError::NotImplemented("v0.1: PKCE verifier/challenge, loopback listener, open_url, token exchange"))
}

/// Fresh tokens from the refresh token.
///
/// # Errors
///
/// [`crate::CloudError`] when the provider, the network or the store fails.
pub fn refresh(_client: &ClientConfig, _tokens: &Tokens) -> crate::Result<Tokens> {
    Err(crate::CloudError::NotImplemented("v0.1"))
}

/// Revokes the tokens with the provider and forgets them.
///
/// # Errors
///
/// [`crate::CloudError`] when the provider, the network or the store fails.
pub fn sign_out(_client: &ClientConfig, _tokens: &Tokens) -> crate::Result<()> {
    Err(crate::CloudError::NotImplemented("v0.1"))
}
