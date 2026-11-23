//! Authenticated calls: a request built for the current access token, sent again with a
//! fresh token when the provider answers 401.

use std::sync::Arc;

use crate::CloudError;
use crate::http::{Http, Request, Response};
use crate::token_store::TokenSource;

/// When a file goes up in one request and when in pieces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UploadLimits {
    /// The largest file sent in a single request, in bytes.
    pub simple_max: usize,
    /// The size of each piece of a larger file, in bytes. The providers want a multiple of
    /// 256 KiB (Google) or 320 KiB (Microsoft).
    pub chunk: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct Api {
    http: Http,
    source: Arc<TokenSource>,
}

impl Api {
    pub(crate) fn new(source: Arc<TokenSource>, http: Http) -> Self {
        Self { http, source }
    }

    /// Sends what `build` makes from the access token. A 401 renews the token once and sends
    /// again; a second 401 means the session is gone.
    ///
    /// # Errors
    ///
    /// [`CloudError::SignedOut`] when the provider rejects the renewed token too;
    /// [`CloudError::Network`] when there is no answer.
    pub(crate) fn call<'a>(&self, build: impl Fn(&str) -> Request<'a>) -> crate::Result<Response> {
        let token = self.source.access_token()?;
        let response = self.http.send(&build(&token))?;
        if response.status != 401 {
            return Ok(response);
        }
        let token = self.source.renewed_after_rejection(&token)?;
        let response = self.http.send(&build(&token))?;
        if response.status == 401 {
            return Err(CloudError::SignedOut);
        }
        Ok(response)
    }

    /// [`Api::call`], with anything but 2xx turned into an error.
    ///
    /// # Errors
    ///
    /// As [`Api::call`]; [`CloudError::Service`] for another status.
    pub(crate) fn call_ok<'a>(&self, build: impl Fn(&str) -> Request<'a>) -> crate::Result<Response> {
        self.call(build)?.ok()
    }

    /// A request that carries its own authorisation (an upload session address).
    pub(crate) fn send(&self, request: &Request<'_>) -> crate::Result<Response> {
        self.http.send(request)
    }
}

/// An RFC 3339 time as Unix seconds; 0 when the provider sent none or something unreadable.
pub(crate) fn rfc3339_to_unix(text: Option<&str>) -> i64 {
    text.and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map_or_default(|t| t.timestamp())
}
