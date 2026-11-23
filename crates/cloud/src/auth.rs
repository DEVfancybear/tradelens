//! OAuth 2.0 for installed apps (RFC 6749 + PKCE, RFC 7636): the system browser, a redirect to
//! `http://127.0.0.1:<random port>` that the app listens on once, then the code is exchanged
//! for tokens. Tokens are kept by [`crate::token_store`], not here.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::CloudError;
use crate::http::{Http, Request, query, unix_now};
use crate::token_store::TokenStore;

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

    /// A short stable name: keys of the credential store and of the settings.
    pub fn key(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Microsoft => "microsoft",
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

    /// The userinfo endpoint (`OpenID` Connect): the e-mail and the name behind a token.
    pub fn userinfo_url(self) -> &'static str {
        match self {
            Self::Google => "https://openidconnect.googleapis.com/v1/userinfo",
            Self::Microsoft => "https://graph.microsoft.com/oidc/userinfo",
        }
    }

    /// Where a token is revoked; Microsoft has no such endpoint for the v2 flow.
    pub fn revoke_url(self) -> Option<&'static str> {
        match self {
            Self::Google => Some("https://oauth2.googleapis.com/revoke"),
            Self::Microsoft => None,
        }
    }

    /// The narrowest scopes that reach the app folder and the user's name.
    pub fn scopes(self) -> &'static [&'static str] {
        match self {
            Self::Google => &["openid", "email", "profile", "https://www.googleapis.com/auth/drive.appdata"],
            Self::Microsoft => &["openid", "email", "profile", "offline_access", "Files.ReadWrite.AppFolder"],
        }
    }

    /// Extra parameters of the consent page. Google hands out a refresh token only on
    /// consent, so ask for it every time; Microsoft lets the user pick the account.
    fn authorize_extras(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Google => &[("access_type", "offline"), ("prompt", "consent")],
            Self::Microsoft => &[("prompt", "select_account")],
        }
    }

    /// The setting that overrides the client id (`cloud.google.client_id`).
    pub fn client_id_setting(self) -> String {
        format!("cloud.{}.client_id", self.key())
    }

    /// The setting that overrides Google's desktop "secret" (`cloud.google.client_secret`).
    pub fn client_secret_setting(self) -> String {
        format!("cloud.{}.client_secret", self.key())
    }

    /// The environment variable of the client id, at run time and at build time.
    pub fn client_id_env(self) -> &'static str {
        match self {
            Self::Google => "TRADELENS_GOOGLE_CLIENT_ID",
            Self::Microsoft => "TRADELENS_MICROSOFT_CLIENT_ID",
        }
    }

    /// The environment variable of the client secret, at run time and at build time.
    pub fn client_secret_env(self) -> &'static str {
        match self {
            Self::Google => "TRADELENS_GOOGLE_CLIENT_SECRET",
            Self::Microsoft => "TRADELENS_MICROSOFT_CLIENT_SECRET",
        }
    }

    /// The id and secret compiled into the binary (`TRADELENS_*` set when building).
    fn built_in(self) -> (Option<&'static str>, Option<&'static str>) {
        match self {
            Self::Google => (option_env!("TRADELENS_GOOGLE_CLIENT_ID"), option_env!("TRADELENS_GOOGLE_CLIENT_SECRET")),
            Self::Microsoft => {
                (option_env!("TRADELENS_MICROSOFT_CLIENT_ID"), option_env!("TRADELENS_MICROSOFT_CLIENT_SECRET"))
            }
        }
    }
}

/// The provider's URLs; replaced in tests by a local server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    pub authorize: String,
    pub token: String,
    pub userinfo: String,
    pub revoke: Option<String>,
}

impl Endpoints {
    /// The real ones.
    pub fn of(provider: Provider) -> Self {
        Self {
            authorize: provider.authorize_url().to_owned(),
            token: provider.token_url().to_owned(),
            userinfo: provider.userinfo_url().to_owned(),
            revoke: provider.revoke_url().map(str::to_owned),
        }
    }
}

/// The app's registration with a provider (client ids are not secrets for
/// installed apps; they ship in the binary or come from the environment in dev).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    pub provider: Provider,
    pub client_id: String,
    /// Google's desktop clients come with a "secret" that Google itself calls not confidential;
    /// the token endpoint still asks for it. Microsoft's public clients have none.
    pub client_secret: Option<String>,
    pub endpoints: Endpoints,
    /// The host of the redirect, `127.0.0.1` unless the registration only accepts `localhost`.
    pub redirect_host: String,
}

impl ClientConfig {
    pub fn new(provider: Provider, client_id: impl Into<String>) -> Self {
        Self {
            provider,
            client_id: client_id.into(),
            client_secret: None,
            endpoints: Endpoints::of(provider),
            redirect_host: "127.0.0.1".to_owned(),
        }
    }

    #[must_use]
    pub fn with_secret(mut self, secret: impl Into<String>) -> Self {
        self.client_secret = Some(secret.into());
        self
    }

    #[must_use]
    pub fn with_endpoints(mut self, endpoints: Endpoints) -> Self {
        self.endpoints = endpoints;
        self
    }

    /// Finds the registration: the settings (`setting` returns the value of a settings key)
    /// first, then the environment, then what was compiled into the binary.
    ///
    /// # Errors
    ///
    /// [`CloudError::Config`] when no source has a client id.
    pub fn resolve(provider: Provider, setting: impl Fn(&str) -> Option<String>) -> crate::Result<Self> {
        let given = |value: Option<String>| value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        let (built_id, built_secret) = provider.built_in();
        let pick = |key: String, env: &str, built: Option<&str>| {
            given(setting(&key)).or_else(|| given(std::env::var(env).ok())).or_else(|| given(built.map(str::to_owned)))
        };
        let id = pick(provider.client_id_setting(), provider.client_id_env(), built_id).ok_or_else(|| {
            CloudError::Config(format!(
                "chưa có client id của {}: đặt {} hoặc biến môi trường {} (xem docs/cloud-sync.md)",
                provider.label(),
                provider.client_id_setting(),
                provider.client_id_env()
            ))
        })?;
        let secret = pick(provider.client_secret_setting(), provider.client_secret_env(), built_secret);
        let mut config = Self::new(provider, id);
        if let Some(host) = given(setting(&format!("cloud.{}.redirect_host", provider.key()))) {
            config.redirect_host = host;
        }
        Ok(match secret {
            Some(secret) => config.with_secret(secret),
            None => config,
        })
    }

    fn check(&self) -> crate::Result<()> {
        if self.client_id.trim().is_empty() {
            return Err(CloudError::Config(format!("chưa có client id của {}", self.provider.label())));
        }
        Ok(())
    }
}

/// Who is signed in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub provider: Provider,
    pub email: String,
    pub name: String,
}

/// Tokens for one provider.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix seconds.
    pub expires_at: i64,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("access_token", &"<redacted>")
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "<redacted>"))
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// A token this close to its end counts as ended: a request takes time.
const EXPIRY_MARGIN_SECS: i64 = 60;
/// The lifetime assumed when the provider does not say.
const DEFAULT_LIFETIME_SECS: i64 = 3600;

impl Tokens {
    /// Whether the access token has ended (or is about to) at Unix time `now`.
    pub fn is_expired(&self, now: i64) -> bool {
        self.access_token.is_empty() || self.expires_at - EXPIRY_MARGIN_SECS <= now
    }
}

/// How long to wait, and how to stop waiting.
#[derive(Debug, Clone)]
pub struct SignInOptions {
    /// How long the browser has to come back with the redirect.
    pub timeout: Duration,
    /// Set from another thread (a Cancel button) to stop the wait with [`CloudError::Cancelled`].
    pub cancel: Arc<AtomicBool>,
}

impl Default for SignInOptions {
    fn default() -> Self {
        Self { timeout: Duration::from_secs(300), cancel: Arc::new(AtomicBool::new(false)) }
    }
}

/// How often the wait looks at the listener, the cancel flag and the clock.
const POLL: Duration = Duration::from_millis(40);
/// How long a browser connection may take to send its request line.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
/// The most of a browser request that is read; the interesting part is the first line.
const MAX_REQUEST: usize = 16 * 1024;

/// The PKCE code challenge of `verifier`: `BASE64URL(SHA256(verifier))` (RFC 7636, section 4.2).
pub(crate) fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// `bytes` random bytes from the OS, as unpadded base64url text.
pub(crate) fn random_token(bytes: usize) -> crate::Result<String> {
    let mut buf = vec![0_u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| CloudError::Auth(format!("không lấy được số ngẫu nhiên: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

/// The consent page's address for this attempt.
fn authorize_url(client: &ClientConfig, redirect_uri: &str, state: &str, challenge: &str) -> String {
    let scope = client.provider.scopes().join(" ");
    let mut pairs: Vec<(&str, &str)> = vec![
        ("client_id", &client.client_id),
        ("redirect_uri", redirect_uri),
        ("response_type", "code"),
        ("scope", &scope),
        ("state", state),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
    ];
    pairs.extend(client.provider.authorize_extras());
    let separator = if client.endpoints.authorize.contains('?') { '&' } else { '?' };
    format!("{}{separator}{}", client.endpoints.authorize, query(&pairs))
}

/// Opens the browser at the consent page, waits for the redirect, exchanges the code and reads
/// who signed in.
///
/// `open_browser` receives the address to open; the crate opens no windows. The wait ends
/// when the redirect comes, when `options.cancel` is set ([`CloudError::Cancelled`]) or after
/// `options.timeout`.
///
/// # Errors
///
/// [`CloudError::Cancelled`] when the user cancels, here or on the consent page;
/// [`CloudError::Auth`] when the provider refuses or the wait times out;
/// [`CloudError::Network`] when it cannot be reached.
pub fn sign_in(
    client: &ClientConfig,
    options: &SignInOptions,
    open_browser: impl FnOnce(&str),
) -> crate::Result<(Identity, Tokens)> {
    client.check()?;
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| CloudError::Network(format!("không mở được cổng nhận đăng nhập: {e}")))?;
    listener.set_nonblocking(true).map_err(|e| CloudError::Network(e.to_string()))?;
    let port = listener.local_addr().map_err(|e| CloudError::Network(e.to_string()))?.port();
    let redirect_uri = format!("http://{}:{port}", client.redirect_host);

    let verifier = random_token(32)?;
    let state = random_token(16)?;
    open_browser(&authorize_url(client, &redirect_uri, &state, &pkce_challenge(&verifier)));

    let code = wait_for_code(&listener, &state, options)?;
    let http = Http::default();
    let tokens = exchange(
        &http,
        client,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("code_verifier", &verifier),
            ("redirect_uri", &redirect_uri),
        ],
        None,
    )?;
    let identity = userinfo(&http, client, &tokens.access_token)?;
    Ok((identity, tokens))
}

/// Fresh tokens from the refresh token. A provider that does not send a new refresh token
/// (Google) keeps the old one.
///
/// # Errors
///
/// [`CloudError::SignedOut`] when there is no refresh token or the provider no longer accepts
/// it (the user revoked the app); [`CloudError::Network`] / [`CloudError::Service`] otherwise.
pub fn refresh(client: &ClientConfig, tokens: &Tokens) -> crate::Result<Tokens> {
    client.check()?;
    let refresh_token = tokens.refresh_token.as_deref().ok_or(CloudError::SignedOut)?;
    exchange(
        &Http::default(),
        client,
        &[("grant_type", "refresh_token"), ("refresh_token", refresh_token)],
        Some(refresh_token),
    )
}

/// Revokes the tokens with the provider (where it can be done) and forgets them. The tokens are
/// forgotten even when the provider cannot be reached.
///
/// # Errors
///
/// [`CloudError::Store`] when the stored tokens cannot be removed.
pub fn sign_out(client: &ClientConfig, store: &dyn TokenStore) -> crate::Result<()> {
    let provider = client.provider;
    match store.load(provider) {
        Ok(Some(tokens)) => revoke(client, &tokens),
        Ok(None) => {}
        Err(e) => log::warn!("không đọc được token để thu hồi: {e}"),
    }
    store.clear(provider)
}

/// Best effort: a failure is logged, never returned, because signing out must always work.
fn revoke(client: &ClientConfig, tokens: &Tokens) {
    let Some(url) = &client.endpoints.revoke else { return };
    let token = tokens.refresh_token.as_deref().unwrap_or(&tokens.access_token);
    if token.is_empty() {
        return;
    }
    match Http::default().send(&Request::post(url).form(&[("token", token)])) {
        Ok(response) if response.is_success() => {}
        Ok(response) => log::warn!("thu hồi token bị từ chối: {}", response.status),
        Err(e) => log::warn!("không thu hồi được token: {e}"),
    }
}

// --- the token endpoint ------------------------------------------------------------

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
}

/// Posts a grant to the token endpoint. `keep_refresh` is the refresh token to keep when the
/// answer carries none.
fn exchange(
    http: &Http,
    client: &ClientConfig,
    grant: &[(&str, &str)],
    keep_refresh: Option<&str>,
) -> crate::Result<Tokens> {
    let mut fields: Vec<(&str, &str)> = vec![("client_id", &client.client_id)];
    fields.extend(grant);
    if let Some(secret) = &client.client_secret {
        fields.push(("client_secret", secret));
    }
    let response = http.send(&Request::post(&client.endpoints.token).form(&fields))?;
    if !response.is_success() {
        let body = response.value();
        let code = body["error"].as_str().unwrap_or_default();
        // The grant is dead: the user revoked the app, or the refresh token expired.
        if code == "invalid_grant" && keep_refresh.is_some() {
            return Err(CloudError::SignedOut);
        }
        return Err(CloudError::Auth(match body["error_description"].as_str() {
            Some(description) => format!("{code}: {description}"),
            None => format!("dịch vụ trả lỗi {}", response.status),
        }));
    }
    let parsed: TokenResponse = response.json()?;
    Ok(Tokens {
        access_token: parsed.access_token,
        refresh_token: parsed.refresh_token.or_else(|| keep_refresh.map(str::to_owned)),
        expires_at: unix_now() + parsed.expires_in.unwrap_or(DEFAULT_LIFETIME_SECS),
    })
}

#[derive(Deserialize)]
struct UserInfo {
    email: Option<String>,
    preferred_username: Option<String>,
    name: Option<String>,
    given_name: Option<String>,
    family_name: Option<String>,
}

fn userinfo(http: &Http, client: &ClientConfig, access_token: &str) -> crate::Result<Identity> {
    let info: UserInfo = http.send(&Request::get(&client.endpoints.userinfo).bearer(access_token))?.ok()?.json()?;
    let email = info.email.or(info.preferred_username).unwrap_or_default();
    let joined = [info.given_name, info.family_name].into_iter().flatten().collect::<Vec<_>>().join(" ");
    let name = info.name.filter(|n| !n.is_empty()).unwrap_or(joined);
    if email.is_empty() && name.is_empty() {
        return Err(CloudError::Auth("không đọc được tên tài khoản từ nhà cung cấp".to_owned()));
    }
    let name = if name.is_empty() { email.clone() } else { name };
    Ok(Identity { provider: client.provider, email, name })
}

// --- the loopback listener ---------------------------------------------------------

/// The page the browser lands on, in Vietnamese.
fn page(title: &str, message: &str) -> String {
    format!(
        "<!doctype html><html lang=\"vi\"><head><meta charset=\"utf-8\"><title>TradeLens</title>\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"></head>\
         <body style=\"font-family:system-ui,sans-serif;max-width:32rem;margin:15vh auto;padding:0 1rem;\
         text-align:center;color:#1f2933\"><h1 style=\"font-size:1.4rem\">{title}</h1><p>{message}</p></body></html>"
    )
}

fn respond(mut stream: TcpStream, status: &str, html: &str) {
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
        html.len()
    );
    // The browser may already have gone; the sign-in does not depend on this page arriving.
    if stream.write_all(reply.as_bytes()).is_err() || stream.flush().is_err() {
        log::debug!("trình duyệt đã đóng trước khi nhận trang kết quả");
    }
}

/// The first line of the request the browser sent: `GET /?code=… HTTP/1.1`.
fn read_request_line(stream: &mut TcpStream) -> Option<String> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(REQUEST_TIMEOUT)).ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 1024];
    while buf.len() < MAX_REQUEST && !buf.contains(&b'\n') {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let text = String::from_utf8_lossy(&buf);
    text.lines().next().map(str::to_owned)
}

/// The decoded `name=value` pairs of the query in `GET /?a=b HTTP/1.1`.
fn query_of(request_line: &str) -> Vec<(String, String)> {
    let target = request_line.split_whitespace().nth(1).unwrap_or_default();
    let raw = target.split_once('?').map_or("", |(_, q)| q);
    let decode = |s: &str| percent_decode_str(&s.replace('+', " ")).decode_utf8_lossy().into_owned();
    raw.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(name), decode(value))
        })
        .collect()
}

fn wait_for_code(listener: &TcpListener, state: &str, options: &SignInOptions) -> crate::Result<String> {
    let deadline = Instant::now() + options.timeout;
    loop {
        if options.cancel.load(Ordering::Relaxed) {
            return Err(CloudError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(CloudError::Auth("hết thời gian chờ đăng nhập".to_owned()));
        }
        let mut stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(POLL);
                continue;
            }
            Err(e) => return Err(CloudError::Network(format!("cổng nhận đăng nhập bị lỗi: {e}"))),
        };
        let Some(line) = read_request_line(&mut stream) else { continue };
        let params = query_of(&line);
        let get = |name: &str| params.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        // Anything without our state (a favicon, a stray request) is not the answer: ignore it.
        if get("state") != Some(state) {
            respond(
                stream,
                "400 Bad Request",
                &page("Yêu cầu không hợp lệ", "Trang này chỉ dùng để đăng nhập TradeLens."),
            );
            continue;
        }
        if let Some(error) = get("error") {
            respond(
                stream,
                "200 OK",
                &page("Đăng nhập chưa hoàn tất", "Bạn đã từ chối hoặc có lỗi. Hãy quay lại TradeLens và thử lại."),
            );
            return Err(if error == "access_denied" {
                CloudError::Cancelled
            } else {
                CloudError::Auth(format!("{error}: {}", get("error_description").unwrap_or_default()))
            });
        }
        if let Some(code) = get("code") {
            let code = code.to_owned();
            respond(stream, "200 OK", &page("Đăng nhập thành công", "Bạn có thể đóng tab này và quay lại TradeLens."));
            return Ok(code);
        }
        respond(stream, "400 Bad Request", &page("Thiếu mã đăng nhập", "Hãy quay lại TradeLens và thử lại."));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_the_rfc_7636_appendix_b_vector() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn random_tokens_are_url_safe_and_differ() {
        let (a, b) = (random_token(32).expect("random"), random_token(32).expect("random"));
        assert_eq!(a.len(), 43, "32 bytes are 43 base64url characters, the RFC's minimum verifier length");
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'), "{a}");
    }

    #[test]
    fn the_consent_url_carries_pkce_and_the_scopes() {
        let client = ClientConfig::new(Provider::Google, "id 1");
        let url = authorize_url(&client, "http://127.0.0.1:5555", "st", "ch");
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?client_id=id%201&"), "{url}");
        for part in [
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A5555",
            "response_type=code",
            "scope=openid%20email%20profile%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fdrive.appdata",
            "state=st",
            "code_challenge=ch",
            "code_challenge_method=S256",
            "access_type=offline",
        ] {
            assert!(url.contains(part), "{part} in {url}");
        }
    }

    #[test]
    fn tokens_expire_a_minute_early_and_hide_in_debug() {
        let tokens = Tokens { access_token: "secret".into(), refresh_token: Some("also".into()), expires_at: 1000 };
        assert!(!tokens.is_expired(900));
        assert!(tokens.is_expired(940), "inside the margin");
        assert!(Tokens { access_token: String::new(), ..tokens.clone() }.is_expired(0), "no access token is expired");
        let shown = format!("{tokens:?}");
        assert!(!shown.contains("secret") && !shown.contains("also"), "{shown}");
    }

    #[test]
    fn the_query_of_a_request_line_is_decoded() {
        let line = "GET /?code=4%2F0A&state=a+b&error_description=x%20y HTTP/1.1";
        let params = query_of(line);
        assert_eq!(params[0], ("code".to_owned(), "4/0A".to_owned()));
        assert_eq!(params[1], ("state".to_owned(), "a b".to_owned()));
        assert_eq!(params[2].1, "x y");
        assert_eq!(query_of("GET /favicon.ico HTTP/1.1").len(), 0, "no query, no pairs");
    }

    #[test]
    fn the_client_is_found_in_settings_then_the_environment() {
        let from_settings = ClientConfig::resolve(Provider::Microsoft, |key| match key {
            "cloud.microsoft.client_id" => Some(" abc ".to_owned()),
            "cloud.microsoft.redirect_host" => Some("localhost".to_owned()),
            _ => None,
        })
        .expect("a setting is enough");
        assert_eq!(from_settings.client_id, "abc");
        assert_eq!(from_settings.client_secret, None);
        assert_eq!(from_settings.redirect_host, "localhost");
        let missing = ClientConfig::resolve(Provider::Microsoft, |_| None);
        // Unless the build or the environment of this machine provides one.
        if std::env::var(Provider::Microsoft.client_id_env()).is_err() && Provider::Microsoft.built_in().0.is_none() {
            assert!(matches!(missing, Err(CloudError::Config(_))), "{missing:?}");
        }
    }
}
