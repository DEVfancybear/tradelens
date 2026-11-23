//! Sign-in, refresh and sign-out against the mock provider.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use percent_encoding::percent_decode_str;
use serde_json::json;
use sha2::{Digest, Sha256};
use tradelens_cloud::CloudError;
use tradelens_cloud::auth::{self, ClientConfig, Endpoints, Identity, Provider, SignInOptions, Tokens};
use tradelens_cloud::token_store::{MemoryStore, TokenStore};

use crate::mock::{Mock, Reply, Seen};

pub(crate) fn endpoints(base: &str) -> Endpoints {
    Endpoints {
        authorize: format!("{base}/authorize"),
        token: format!("{base}/token"),
        userinfo: format!("{base}/userinfo"),
        revoke: Some(format!("{base}/revoke")),
    }
}

pub(crate) fn now() -> i64 {
    i64::try_from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("after 1970").as_secs())
        .expect("fits")
}

fn params_of(url: &str) -> HashMap<String, String> {
    let query = url.split_once('?').expect("a query").1;
    query
        .split('&')
        .map(|pair| {
            let (k, v) = pair.split_once('=').expect("name=value");
            (k.to_owned(), percent_decode_str(v).decode_utf8_lossy().into_owned())
        })
        .collect()
}

/// What a browser does on the redirect: one GET to the app's loopback port. Returns the raw reply.
fn visit(redirect_uri: &str, path_and_query: &str) -> String {
    let address = redirect_uri.trim_start_matches("http://");
    let mut stream = TcpStream::connect(address).expect("the loopback listener is up");
    write!(stream, "GET {path_and_query} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n").expect("sent");
    let mut reply = String::new();
    stream.read_to_string(&mut reply).expect("the app answers the browser");
    reply
}

fn provider_mock() -> Mock {
    Mock::start(|req: &Seen, _| match req.path() {
        "/token" if req.form_value("grant_type").as_deref() == Some("refresh_token") => {
            Reply::ok(&json!({"access_token": "at-2", "expires_in": 3599, "token_type": "Bearer"}))
        }
        "/token" => Reply::ok(
            &json!({"access_token": "at-1", "refresh_token": "rt-1", "expires_in": 3600, "token_type": "Bearer"}),
        ),
        "/userinfo" => Reply::ok(&json!({"email": "dat@example.com", "name": "Dat Le"})),
        "/revoke" => Reply::status(200),
        _ => Reply::status(404),
    })
}

fn quick() -> SignInOptions {
    SignInOptions { timeout: Duration::from_secs(20), ..SignInOptions::default() }
}

#[test]
fn google_sign_in_runs_the_whole_flow() {
    let mock = provider_mock();
    let client =
        ClientConfig::new(Provider::Google, "client-1").with_secret("sec-1").with_endpoints(endpoints(&mock.base));

    let mut authorize_url = String::new();
    let mut browser: Option<JoinHandle<(String, String)>> = None;
    let (identity, tokens) = auth::sign_in(&client, &quick(), |url| {
        authorize_url = url.to_owned();
        let params = params_of(url);
        let (redirect, state) = (params["redirect_uri"].clone(), params["state"].clone());
        browser = Some(thread::spawn(move || {
            // A stray request first (a favicon, another tab): it must not end the wait.
            let stray = visit(&redirect, "/?code=evil&state=wrong");
            let real = visit(&redirect, &format!("/?code=the-code&state={state}"));
            (stray, real)
        }));
    })
    .expect("signs in");
    let (stray, real) = browser.expect("the browser opened").join().expect("the fake browser finished");

    assert_eq!(
        identity,
        Identity { provider: Provider::Google, email: "dat@example.com".into(), name: "Dat Le".into() }
    );
    assert_eq!(tokens.access_token, "at-1");
    assert_eq!(tokens.refresh_token.as_deref(), Some("rt-1"));
    assert!((tokens.expires_at - (now() + 3600)).abs() < 10, "expires in an hour: {}", tokens.expires_at);

    assert!(stray.starts_with("HTTP/1.1 400"), "{stray}");
    assert!(real.starts_with("HTTP/1.1 200") && real.contains("Đăng nhập thành công"), "{real}");

    let params = params_of(&authorize_url);
    assert!(authorize_url.starts_with(&format!("{}/authorize?", mock.base)), "{authorize_url}");
    assert_eq!(params["client_id"], "client-1");
    assert_eq!(params["response_type"], "code");
    assert_eq!(params["code_challenge_method"], "S256");
    assert!(params["scope"].contains("https://www.googleapis.com/auth/drive.appdata"), "{}", params["scope"]);
    assert!(params["redirect_uri"].starts_with("http://127.0.0.1:"), "{}", params["redirect_uri"]);

    let exchange = &mock.requests_to("/token")[0];
    assert_eq!(exchange.form_value("grant_type").as_deref(), Some("authorization_code"));
    assert_eq!(exchange.form_value("code").as_deref(), Some("the-code"), "the stray code was ignored");
    assert_eq!(exchange.form_value("client_id").as_deref(), Some("client-1"));
    assert_eq!(exchange.form_value("client_secret").as_deref(), Some("sec-1"));
    assert_eq!(exchange.form_value("redirect_uri"), Some(params["redirect_uri"].clone()));
    let verifier = exchange.form_value("code_verifier").expect("a verifier");
    assert_eq!(URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())), params["code_challenge"], "S256");

    let info = &mock.requests_to("/userinfo")[0];
    assert_eq!(info.header("authorization"), Some("Bearer at-1"));
}

#[test]
fn microsoft_sign_in_has_no_secret_and_reads_the_username() {
    let mock = Mock::start(|req: &Seen, _| match req.path() {
        "/token" => Reply::ok(&json!({"access_token": "ms-at", "refresh_token": "ms-rt", "expires_in": 3000})),
        "/userinfo" => Reply::ok(
            &json!({"sub": "x", "preferred_username": "dat@outlook.com", "given_name": "Dat", "family_name": "Le"}),
        ),
        _ => Reply::status(404),
    });
    let client = ClientConfig::new(Provider::Microsoft, "app-id").with_endpoints(endpoints(&mock.base));
    let mut browser = None;
    let (identity, tokens) = auth::sign_in(&client, &quick(), |url| {
        let params = params_of(url);
        assert_eq!(params["prompt"], "select_account");
        assert!(params["scope"].contains("Files.ReadWrite.AppFolder") && params["scope"].contains("offline_access"));
        let (redirect, state) = (params["redirect_uri"].clone(), params["state"].clone());
        browser = Some(thread::spawn(move || visit(&redirect, &format!("/?code=c&state={state}"))));
    })
    .expect("signs in");
    browser.expect("opened").join().expect("done");

    assert_eq!((identity.email.as_str(), identity.name.as_str()), ("dat@outlook.com", "Dat Le"));
    assert_eq!(tokens.refresh_token.as_deref(), Some("ms-rt"));
    assert_eq!(mock.requests_to("/token")[0].form_value("client_secret"), None);
}

#[test]
fn cancelling_stops_the_wait_at_once() {
    let mock = provider_mock();
    let client = ClientConfig::new(Provider::Google, "c").with_endpoints(endpoints(&mock.base));
    let options = SignInOptions { timeout: Duration::from_secs(60), ..SignInOptions::default() };
    let flag = Arc::clone(&options.cancel);
    let started = Instant::now();
    let result = auth::sign_in(&client, &options, |_| flag.store(true, Ordering::Relaxed));
    assert!(matches!(result, Err(CloudError::Cancelled)), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(5), "took {:?}", started.elapsed());
    assert!(mock.requests().is_empty(), "nothing was exchanged");
}

#[test]
fn the_wait_ends_after_the_timeout() {
    let mock = provider_mock();
    let client = ClientConfig::new(Provider::Google, "c").with_endpoints(endpoints(&mock.base));
    let options = SignInOptions { timeout: Duration::from_millis(200), ..SignInOptions::default() };
    let result = auth::sign_in(&client, &options, |_| {});
    assert!(matches!(&result, Err(CloudError::Auth(m)) if m.contains("hết thời gian")), "{result:?}");
}

#[test]
fn refusing_the_consent_page_is_a_cancel() {
    let mock = provider_mock();
    let client = ClientConfig::new(Provider::Google, "c").with_endpoints(endpoints(&mock.base));
    let mut browser = None;
    let result = auth::sign_in(&client, &quick(), |url| {
        let params = params_of(url);
        let (redirect, state) = (params["redirect_uri"].clone(), params["state"].clone());
        browser = Some(thread::spawn(move || visit(&redirect, &format!("/?error=access_denied&state={state}"))));
    });
    let page = browser.expect("opened").join().expect("done");
    assert!(matches!(result, Err(CloudError::Cancelled)), "{result:?}");
    assert!(page.contains("chưa hoàn tất"), "{page}");
    assert!(mock.requests_to("/token").is_empty());
}

#[test]
fn a_missing_client_id_is_reported_before_anything_opens() {
    let client = ClientConfig::new(Provider::Google, "  ");
    let result = auth::sign_in(&client, &quick(), |_| panic!("the browser must not open"));
    assert!(matches!(result, Err(CloudError::Config(_))), "{result:?}");
}

fn old_tokens() -> Tokens {
    Tokens { access_token: "at-0".into(), refresh_token: Some("rt-1".into()), expires_at: now() - 10 }
}

#[test]
fn refresh_keeps_the_refresh_token_google_does_not_resend() {
    let mock = provider_mock();
    let client = ClientConfig::new(Provider::Google, "c").with_secret("s").with_endpoints(endpoints(&mock.base));
    let fresh = auth::refresh(&client, &old_tokens()).expect("refreshes");
    assert_eq!(fresh.access_token, "at-2");
    assert_eq!(fresh.refresh_token.as_deref(), Some("rt-1"));
    assert!(!fresh.is_expired(now()));
    let request = &mock.requests_to("/token")[0];
    assert_eq!(request.form_value("grant_type").as_deref(), Some("refresh_token"));
    assert_eq!(request.form_value("refresh_token").as_deref(), Some("rt-1"));
    assert_eq!(request.form_value("client_secret").as_deref(), Some("s"));
}

#[test]
fn a_revoked_refresh_token_means_signed_out() {
    let mock = Mock::start(|_, _| {
        Reply::json(400, &json!({"error": "invalid_grant", "error_description": "Token has been expired or revoked."}))
    });
    let client = ClientConfig::new(Provider::Google, "c").with_endpoints(endpoints(&mock.base));
    assert!(matches!(auth::refresh(&client, &old_tokens()), Err(CloudError::SignedOut)));
    let without = Tokens { refresh_token: None, ..old_tokens() };
    assert!(matches!(auth::refresh(&client, &without), Err(CloudError::SignedOut)));
    assert_eq!(mock.requests().len(), 1, "no refresh token, no request");
}

#[test]
fn sign_out_revokes_and_forgets() {
    let mock = provider_mock();
    let client = ClientConfig::new(Provider::Google, "c").with_endpoints(endpoints(&mock.base));
    let store = MemoryStore::new();
    store.save(Provider::Google, &old_tokens()).expect("saves");
    auth::sign_out(&client, &store).expect("signs out");
    assert_eq!(store.load(Provider::Google).expect("loads"), None);
    assert_eq!(mock.requests_to("/revoke")[0].form_value("token").as_deref(), Some("rt-1"));
}

#[test]
fn sign_out_forgets_even_when_the_provider_refuses() {
    let mock = Mock::start(|_, _| Reply::json(400, &json!({"error": "invalid_token"})));
    let client = ClientConfig::new(Provider::Google, "c").with_endpoints(endpoints(&mock.base));
    let store = MemoryStore::new();
    store.save(Provider::Google, &old_tokens()).expect("saves");
    auth::sign_out(&client, &store).expect("still signs out");
    assert_eq!(store.load(Provider::Google).expect("loads"), None);

    // Microsoft has nothing to revoke: no request at all.
    let offline = Mock::start(|_, _| Reply::status(500));
    let mut config = ClientConfig::new(Provider::Microsoft, "c");
    config.endpoints = Endpoints { revoke: None, ..endpoints(&offline.base) };
    store.save(Provider::Microsoft, &old_tokens()).expect("saves");
    auth::sign_out(&config, &store).expect("signs out");
    assert_eq!(store.load(Provider::Microsoft).expect("loads"), None);
    assert!(offline.requests().is_empty());
}
