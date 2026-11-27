//! The HTTPS plumbing shared by the providers: one `ureq` agent that hands back every status
//! as a [`Response`] (never as an error), with retries on 429 and 5xx that honour `Retry-After`.

use std::borrow::Cow;
use std::time::Duration;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::de::DeserializeOwned;
use ureq::http;

use crate::CloudError;

/// How long to wait for the connection, and for any single read.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(90);
/// The largest response body held in memory (a database snapshot is the biggest).
const MAX_BODY: u64 = 1 << 30;

/// Everything but the RFC 3986 unreserved characters.
const FORM_SET: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');
/// As [`FORM_SET`] but `/` stays, to encode a path whose segments are already split.
const PATH_SET: &AsciiSet = &FORM_SET.remove(b'/');

/// Percent-encodes `text` for a query value or a form field.
pub(crate) fn encode(text: &str) -> String {
    utf8_percent_encode(text, FORM_SET).to_string()
}

/// Percent-encodes a `/`-separated path, keeping the slashes.
pub(crate) fn encode_path(path: &str) -> String {
    utf8_percent_encode(path, PATH_SET).to_string()
}

/// `a=1&b=2` with both sides encoded.
pub(crate) fn query<K: AsRef<str>, V: AsRef<str>>(pairs: &[(K, V)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", encode(k.as_ref()), encode(v.as_ref()))).collect::<Vec<_>>().join("&")
}

/// The current time in Unix seconds.
pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or_default()
}

/// How often a request that got 429 or a 5xx answer (or no answer) is sent again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Tries in all, the first included.
    pub max_attempts: u32,
    /// The wait before the second try; it doubles after each failure.
    pub base_delay: Duration,
    /// The longest wait, whatever `Retry-After` asks.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { max_attempts: 5, base_delay: Duration::from_millis(500), max_delay: Duration::from_secs(60) }
    }
}

impl RetryPolicy {
    /// No second chance: for tests that look at the first answer.
    pub const NONE: Self = Self { max_attempts: 1, base_delay: Duration::ZERO, max_delay: Duration::ZERO };

    fn delay(&self, attempt: u32, retry_after: Option<Duration>) -> Duration {
        let backoff = self.base_delay.saturating_mul(1_u32 << attempt.saturating_sub(1).min(16));
        retry_after.unwrap_or(backoff).min(self.max_delay)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    fn as_http(self) -> http::Method {
        match self {
            Self::Get => http::Method::GET,
            Self::Post => http::Method::POST,
            Self::Put => http::Method::PUT,
            Self::Patch => http::Method::PATCH,
            Self::Delete => http::Method::DELETE,
        }
    }
}

/// A request that can be sent more than once: its body is held, not streamed.
pub(crate) struct Request<'a> {
    method: Method,
    url: String,
    headers: Vec<(String, String)>,
    body: Cow<'a, [u8]>,
}

impl<'a> Request<'a> {
    fn new(method: Method, url: impl Into<String>) -> Self {
        Self { method, url: url.into(), headers: Vec::new(), body: Cow::Borrowed(&[]) }
    }

    pub(crate) fn get(url: impl Into<String>) -> Self {
        Self::new(Method::Get, url)
    }

    pub(crate) fn post(url: impl Into<String>) -> Self {
        Self::new(Method::Post, url)
    }

    pub(crate) fn put(url: impl Into<String>) -> Self {
        Self::new(Method::Put, url)
    }

    pub(crate) fn patch(url: impl Into<String>) -> Self {
        Self::new(Method::Patch, url)
    }

    pub(crate) fn delete(url: impl Into<String>) -> Self {
        Self::new(Method::Delete, url)
    }

    pub(crate) fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_owned(), value.into()));
        self
    }

    pub(crate) fn bearer(self, token: &str) -> Self {
        self.header("Authorization", format!("Bearer {token}"))
    }

    pub(crate) fn bytes(mut self, content_type: &str, data: &'a [u8]) -> Self {
        self.body = Cow::Borrowed(data);
        self.header("Content-Type", content_type)
    }

    pub(crate) fn json(mut self, value: &serde_json::Value) -> Self {
        self.body = Cow::Owned(value.to_string().into_bytes());
        self.header("Content-Type", "application/json")
    }

    pub(crate) fn form<K: AsRef<str>, V: AsRef<str>>(mut self, fields: &[(K, V)]) -> Self {
        self.body = Cow::Owned(query(fields).into_bytes());
        self.header("Content-Type", "application/x-www-form-urlencoded")
    }
}

/// An answer, whatever its status.
#[derive(Debug)]
pub(crate) struct Response {
    pub(crate) status: u16,
    headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl Response {
    pub(crate) fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// A header's value, the name compared without regard to case.
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The body as JSON.
    pub(crate) fn json<T: DeserializeOwned>(&self) -> crate::Result<T> {
        serde_json::from_slice(&self.body)
            .map_err(|e| CloudError::Network(crate::tr!("cloud.http.unreadable_body", error = e)))
    }

    /// The body as a generic JSON value; `Null` when it is empty or not JSON.
    pub(crate) fn value(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_default()
    }

    /// Passes a 2xx answer through and turns anything else into [`CloudError::Service`].
    pub(crate) fn ok(self) -> crate::Result<Self> {
        if self.is_success() { Ok(self) } else { Err(self.service_error()) }
    }

    pub(crate) fn service_error(&self) -> CloudError {
        let body = self.value();
        if self.status == 403 && google_lacks_drive_scope(&body) {
            return CloudError::DriveNotGranted;
        }
        CloudError::Service { status: self.status, message: error_message(&body, &self.text()) }
    }
}

/// Whether a 403 body is Google saying the token lacks the Drive scope (the user unticked it on
/// the consent screen): the English message, the `insufficientPermissions` reason, or the
/// `ACCESS_TOKEN_SCOPE_INSUFFICIENT` detail. These are Google's own words, so the message that
/// follows may name Google Drive; no other service's 403 is taken for it.
fn google_lacks_drive_scope(body: &serde_json::Value) -> bool {
    let has_reason = |list: &serde_json::Value, wanted: &str| {
        list.as_array().is_some_and(|items| items.iter().any(|item| item["reason"] == wanted))
    };
    body["error"]["message"]
        .as_str()
        .is_some_and(|message| message.to_ascii_lowercase().contains("insufficient authentication scopes"))
        || has_reason(&body["error"]["errors"], "insufficientPermissions")
        || has_reason(&body["error"]["details"], "ACCESS_TOKEN_SCOPE_INSUFFICIENT")
}

/// The human part of an error body: Google and Graph (`error.message`), OAuth
/// (`error_description`), Notion (`message`); the body's start when it is none of them.
fn error_message(body: &serde_json::Value, raw: &str) -> String {
    let pick = |v: &serde_json::Value| v.as_str().map(str::to_owned);
    let found = pick(&body["error"]["message"])
        .or_else(|| pick(&body["error_description"]))
        .or_else(|| pick(&body["message"]))
        .or_else(|| pick(&body["error"]));
    found.unwrap_or_else(|| raw.chars().take(200).collect())
}

/// Sends requests, retrying the ones the service asks to repeat.
#[derive(Debug, Clone)]
pub(crate) struct Http {
    agent: ureq::Agent,
    retry: RetryPolicy,
}

impl Default for Http {
    fn default() -> Self {
        Self::new(RetryPolicy::default())
    }
}

impl Http {
    pub(crate) fn new(retry: RetryPolicy) -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            // 3xx answers are data here: Drive's resumable upload says 308 for "send more".
            .max_redirects(0)
            .max_redirects_will_error(false)
            .user_agent("TradeLens")
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_per_read(Some(READ_TIMEOUT))
            .build();
        Self { agent: config.into(), retry }
    }

    /// Sends `request`; 429 and 5xx answers and network failures are tried again.
    ///
    /// # Errors
    ///
    /// [`CloudError::Network`] when no answer came after the last try.
    pub(crate) fn send(&self, request: &Request<'_>) -> crate::Result<Response> {
        let mut attempt = 1;
        loop {
            let last = attempt >= self.retry.max_attempts;
            match self.once(request) {
                Ok(response) if !last && retriable(response.status) => {
                    let wait = response.header("retry-after").and_then(|v| v.trim().parse::<u64>().ok());
                    std::thread::sleep(self.retry.delay(attempt, wait.map(Duration::from_secs)));
                }
                Err(e) if !last => {
                    log::warn!("yêu cầu {} lỗi, thử lại: {e}", request.url);
                    std::thread::sleep(self.retry.delay(attempt, None));
                }
                done => return done,
            }
            attempt += 1;
        }
    }

    fn once(&self, request: &Request<'_>) -> crate::Result<Response> {
        let mut builder = http::Request::builder().method(request.method.as_http()).uri(&request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        let network = |e: &dyn std::fmt::Display| CloudError::Network(e.to_string());
        let mut response = if request.body.is_empty() && matches!(request.method, Method::Get | Method::Delete) {
            self.agent.run(builder.body(()).map_err(|e| network(&e))?)
        } else {
            self.agent.run(builder.body(request.body.as_ref()).map_err(|e| network(&e))?)
        }
        .map_err(|e| network(&e))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned())))
            .collect();
        let body = response.body_mut().with_config().limit(MAX_BODY).read_to_vec().map_err(|e| network(&e))?;
        Ok(Response { status, headers, body })
    }
}

fn retriable(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_everything_but_unreserved() {
        assert_eq!(encode("a b&c=d/é~-._"), "a%20b%26c%3Dd%2F%C3%A9~-._");
        assert_eq!(encode_path("changes/dev 1/1-5.json"), "changes/dev%201/1-5.json");
        assert_eq!(query(&[("q", "name = 'x'"), ("n", "1")]), "q=name%20%3D%20%27x%27&n=1");
    }

    #[test]
    fn the_wait_doubles_and_never_passes_the_cap() {
        let policy = RetryPolicy {
            max_attempts: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(350),
        };
        assert_eq!(policy.delay(1, None), Duration::from_millis(100));
        assert_eq!(policy.delay(2, None), Duration::from_millis(200));
        assert_eq!(policy.delay(3, None), Duration::from_millis(350));
        assert_eq!(policy.delay(1, Some(Duration::from_secs(9))), Duration::from_millis(350), "capped");
        assert_eq!(policy.delay(1, Some(Duration::ZERO)), Duration::ZERO, "Retry-After wins over the backoff");
    }

    #[test]
    fn error_bodies_of_each_service_are_read() {
        let google = serde_json::json!({"error": {"code": 403, "message": "Quota exceeded"}});
        let oauth = serde_json::json!({"error": "invalid_grant", "error_description": "Token revoked"});
        let notion = serde_json::json!({"object": "error", "message": "API token is invalid."});
        assert_eq!(error_message(&google, ""), "Quota exceeded");
        assert_eq!(error_message(&oauth, ""), "Token revoked");
        assert_eq!(error_message(&notion, ""), "API token is invalid.");
        assert_eq!(error_message(&serde_json::Value::Null, "boom"), "boom");
    }
}
