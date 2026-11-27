//! Google Drive API v3 in `appDataFolder` (`spaces=appDataFolder`).
//!
//! The folder is flat: a file has a name and no directory. The app's relative paths
//! (`changes/<device>/1-5.json`) are therefore stored as the whole file name, `/` included,
//! which Drive allows, and a prefix is matched on that name.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;

use crate::CloudError;
use crate::api::{Api, UploadLimits, rfc3339_to_unix};
use crate::auth::random_token;
use crate::drive::{CloudDrive, RemoteFile};
use crate::http::{Http, Request, RetryPolicy, encode, query};
use crate::token_store::TokenSource;

/// The fields asked of every file, so answers stay small.
const FILE_FIELDS: &str = "id,name,size,modifiedTime";
/// The most files Drive returns per page.
const PAGE_SIZE: &str = "1000";
const OCTET_STREAM: &str = "application/octet-stream";

/// Where the API lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleUrls {
    /// `https://www.googleapis.com/drive/v3`.
    pub api: String,
    /// `https://www.googleapis.com/upload/drive/v3`.
    pub upload: String,
}

impl Default for GoogleUrls {
    fn default() -> Self {
        Self {
            api: "https://www.googleapis.com/drive/v3".to_owned(),
            upload: "https://www.googleapis.com/upload/drive/v3".to_owned(),
        }
    }
}

/// The app's hidden folder in the signed-in user's Google Drive.
#[derive(Debug, Clone)]
pub struct GoogleDrive {
    api: Api,
    urls: GoogleUrls,
    limits: UploadLimits,
    retry: RetryPolicy,
    source: Arc<TokenSource>,
}

#[derive(Deserialize)]
struct FileList {
    #[serde(default)]
    files: Vec<DriveFile>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct DriveFile {
    id: String,
    name: String,
    /// Drive writes sizes as strings.
    size: Option<String>,
    #[serde(rename = "modifiedTime")]
    modified_time: Option<String>,
}

impl From<DriveFile> for RemoteFile {
    fn from(file: DriveFile) -> Self {
        Self {
            id: file.id,
            name: file.name,
            size: file.size.and_then(|s| s.parse().ok()).unwrap_or_default(),
            modified: rfc3339_to_unix(file.modified_time.as_deref()),
        }
    }
}

impl GoogleDrive {
    /// The drive of the account `source` is signed in to.
    pub fn new(source: Arc<TokenSource>) -> Self {
        let retry = RetryPolicy::default();
        Self {
            api: Api::new(Arc::clone(&source), Http::new(retry)),
            urls: GoogleUrls::default(),
            // Resumable pieces must be multiples of 256 KiB.
            limits: UploadLimits { simple_max: 5 << 20, chunk: 8 << 20 },
            retry,
            source,
        }
    }

    #[must_use]
    pub fn with_urls(mut self, urls: GoogleUrls) -> Self {
        self.urls = urls;
        self
    }

    #[must_use]
    pub fn with_limits(mut self, limits: UploadLimits) -> Self {
        self.limits = limits;
        self
    }

    #[must_use]
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self.api = Api::new(Arc::clone(&self.source), Http::new(retry));
        self
    }

    fn files_url(&self, params: &[(&str, &str)]) -> String {
        format!("{}/files?{}", self.urls.api, query(params))
    }

    /// The file named exactly `name`, if any (the first when several share it).
    fn find(&self, name: &str) -> crate::Result<Option<RemoteFile>> {
        let q = format!("name = '{}' and trashed = false", escape_literal(name));
        let fields = format!("files({FILE_FIELDS})");
        let url = self.files_url(&[("spaces", "appDataFolder"), ("q", &q), ("fields", &fields), ("pageSize", "10")]);
        let list: FileList = self.api.call_ok(|token| Request::get(&url).bearer(token))?.json()?;
        Ok(list.files.into_iter().next().map(RemoteFile::from))
    }

    /// Writes `bytes` over `existing`, or as a new file `name` when there is none.
    fn put(&self, existing: Option<&RemoteFile>, name: &str, bytes: &[u8]) -> crate::Result<RemoteFile> {
        if bytes.len() <= self.limits.simple_max {
            self.upload_simple(existing, name, bytes)
        } else {
            self.upload_resumable(existing, name, bytes)
        }
    }

    fn metadata(name: &str) -> serde_json::Value {
        json!({ "name": name, "parents": ["appDataFolder"] })
    }

    /// One request: the metadata and the bytes together (`uploadType=multipart`).
    fn upload_simple(&self, existing: Option<&RemoteFile>, name: &str, bytes: &[u8]) -> crate::Result<RemoteFile> {
        let file: DriveFile = if let Some(file) = existing {
            let url = format!(
                "{}/files/{}?{}",
                self.urls.upload,
                encode(&file.id),
                query(&[("uploadType", "media"), ("fields", FILE_FIELDS)])
            );
            self.api.call_ok(|token| Request::patch(&url).bearer(token).bytes(OCTET_STREAM, bytes))?.json()?
        } else {
            let url = format!(
                "{}/files?{}",
                self.urls.upload,
                query(&[("uploadType", "multipart"), ("fields", FILE_FIELDS)])
            );
            let boundary = boundary()?;
            let body = multipart_related(&boundary, &Self::metadata(name), bytes);
            let content_type = format!("multipart/related; boundary={boundary}");
            self.api.call_ok(|token| Request::post(&url).bearer(token).bytes(&content_type, &body))?.json()?
        };
        Ok(file.into())
    }

    /// A session, then the bytes in pieces (`uploadType=resumable`).
    fn upload_resumable(&self, existing: Option<&RemoteFile>, name: &str, bytes: &[u8]) -> crate::Result<RemoteFile> {
        let params = query(&[("uploadType", "resumable"), ("fields", FILE_FIELDS)]);
        let total = bytes.len();
        let start = |request: Request<'static>| {
            request.header("X-Upload-Content-Type", OCTET_STREAM).header("X-Upload-Content-Length", total.to_string())
        };
        let opened = if let Some(file) = existing {
            let url = format!("{}/files/{}?{params}", self.urls.upload, encode(&file.id));
            self.api.call_ok(|token| start(Request::patch(&url).bearer(token).json(&json!({}))))?
        } else {
            let url = format!("{}/files?{params}", self.urls.upload);
            self.api.call_ok(|token| start(Request::post(&url).bearer(token).json(&Self::metadata(name))))?
        };
        let session = opened
            .header("location")
            .ok_or_else(|| CloudError::Network(crate::tr!("cloud.drive.no_session_url")))?
            .to_owned();
        self.send_pieces(&session, bytes)
    }

    fn send_pieces(&self, session: &str, bytes: &[u8]) -> crate::Result<RemoteFile> {
        let total = bytes.len();
        let mut sent = 0;
        loop {
            let end = (sent + self.limits.chunk).min(total);
            let range = format!("bytes {sent}-{}/{total}", end - 1);
            // The session address is its own authorisation.
            let response = self
                .api
                .send(&Request::put(session).header("Content-Range", range).bytes(OCTET_STREAM, &bytes[sent..end]))?;
            match response.status {
                200 | 201 => return Ok(response.json::<DriveFile>()?.into()),
                308 => {
                    // `Range: bytes=0-N` is what Drive has kept; none means nothing yet.
                    let kept = response
                        .header("range")
                        .and_then(|r| r.rsplit('-').next())
                        .and_then(|n| n.trim().parse::<usize>().ok())
                        .map_or(0, |last| last + 1);
                    if kept <= sent {
                        return Err(CloudError::Network(crate::tr!("cloud.drive.chunk_refused")));
                    }
                    sent = kept.min(total);
                }
                _ => return Err(response.service_error()),
            }
        }
    }
}

/// A string for a Drive `q` expression: quotes and backslashes escaped.
fn escape_literal(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\'', "\\'")
}

/// A boundary no file content will contain.
fn boundary() -> crate::Result<String> {
    Ok(format!("tradelens{}", random_token(12)?))
}

/// The body of `uploadType=multipart`: the metadata as JSON, then the content.
fn multipart_related(boundary: &str, metadata: &serde_json::Value, bytes: &[u8]) -> Vec<u8> {
    let mut body = format!(
        "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n--{boundary}\r\nContent-Type: {OCTET_STREAM}\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--").as_bytes());
    body
}

impl CloudDrive for GoogleDrive {
    /// Lists every file of the app folder (Drive cannot filter on a name's start) and keeps
    /// the ones whose name starts with `prefix`.
    fn list(&self, prefix: &str) -> crate::Result<Vec<RemoteFile>> {
        let fields = format!("nextPageToken,files({FILE_FIELDS})");
        let mut found = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let mut params = vec![
                ("spaces", "appDataFolder"),
                ("q", "trashed = false"),
                ("fields", fields.as_str()),
                ("pageSize", PAGE_SIZE),
            ];
            if let Some(token) = &page {
                params.push(("pageToken", token));
            }
            let url = self.files_url(&params);
            let list: FileList = self.api.call_ok(|token| Request::get(&url).bearer(token))?.json()?;
            found.extend(list.files.into_iter().filter(|f| f.name.starts_with(prefix)).map(RemoteFile::from));
            match list.next_page_token {
                Some(next) => page = Some(next),
                None => return Ok(found),
            }
        }
    }

    fn upload(&self, name: &str, bytes: &[u8]) -> crate::Result<RemoteFile> {
        let existing = self.find(name)?;
        self.put(existing.as_ref(), name, bytes)
    }

    /// One request for a small file: no lookup of a name the caller knows is free.
    fn upload_new(&self, name: &str, bytes: &[u8]) -> crate::Result<RemoteFile> {
        self.put(None, name, bytes)
    }

    fn download(&self, file: &RemoteFile) -> crate::Result<Vec<u8>> {
        let url = format!("{}/files/{}?alt=media", self.urls.api, encode(&file.id));
        Ok(self.api.call_ok(|token| Request::get(&url).bearer(token))?.body)
    }

    /// Deletes the file; one that is already gone counts as deleted.
    fn delete(&self, file: &RemoteFile) -> crate::Result<()> {
        let url = format!("{}/files/{}", self.urls.api, encode(&file.id));
        let response = self.api.call(|token| Request::delete(&url).bearer(token))?;
        if response.is_success() || response.status == 404 { Ok(()) } else { Err(response.service_error()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_literals_are_escaped() {
        assert_eq!(escape_literal(r"it's a\b"), r"it\'s a\\b");
    }

    #[test]
    fn the_multipart_body_has_the_metadata_then_the_bytes() {
        let body = multipart_related("B", &json!({"name": "a/b"}), b"\x00\x01DATA");
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.starts_with(
                "--B\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{\"name\":\"a/b\"}\r\n--B\r\n"
            )
        );
        assert!(text.ends_with("\x00\x01DATA\r\n--B--"), "{text:?}");
    }
}
