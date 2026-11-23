//! Microsoft Graph, `/me/drive/special/approot`: the app's own folder in the signed-in user's
//! OneDrive. Unlike Google's, it has real directories, so the app's relative paths
//! (`changes/<device>/1-5.json`) are real paths below the folder.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;

use crate::CloudError;
use crate::api::{Api, UploadLimits, rfc3339_to_unix};
use crate::drive::{CloudDrive, RemoteFile};
use crate::http::{Http, Request, RetryPolicy, encode, encode_path};
use crate::token_store::TokenSource;

const OCTET_STREAM: &str = "application/octet-stream";
/// What of an item the app reads.
const ITEM_FIELDS: &str = "id,name,size,lastModifiedDateTime,folder";
/// The page size asked of a listing.
const PAGE_SIZE: &str = "200";
/// How many redirects a download follows (Graph answers with one to the content server).
const MAX_REDIRECTS: usize = 3;

/// Where the API lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OneDriveUrls {
    /// `https://graph.microsoft.com/v1.0`.
    pub graph: String,
}

impl Default for OneDriveUrls {
    fn default() -> Self {
        Self { graph: "https://graph.microsoft.com/v1.0".to_owned() }
    }
}

/// The app folder of the signed-in user's OneDrive.
#[derive(Debug, Clone)]
pub struct OneDrive {
    api: Api,
    urls: OneDriveUrls,
    limits: UploadLimits,
    source: Arc<TokenSource>,
}

#[derive(Deserialize)]
struct Children {
    #[serde(default)]
    value: Vec<Item>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
}

#[derive(Deserialize)]
struct Item {
    id: String,
    name: String,
    size: Option<u64>,
    #[serde(rename = "lastModifiedDateTime")]
    modified: Option<String>,
    /// Present on folders only.
    folder: Option<serde_json::Value>,
}

impl Item {
    fn into_file(self, path: String) -> RemoteFile {
        RemoteFile {
            id: self.id,
            name: path,
            size: self.size.unwrap_or_default(),
            modified: rfc3339_to_unix(self.modified.as_deref()),
        }
    }
}

#[derive(Deserialize)]
struct UploadSession {
    #[serde(rename = "uploadUrl")]
    upload_url: String,
}

#[derive(Deserialize)]
struct Progress {
    #[serde(rename = "nextExpectedRanges", default)]
    next_expected_ranges: Vec<String>,
}

impl OneDrive {
    /// The drive of the account `source` is signed in to.
    pub fn new(source: Arc<TokenSource>) -> Self {
        Self {
            api: Api::new(Arc::clone(&source), Http::default()),
            urls: OneDriveUrls::default(),
            // Pieces of an upload session must be multiples of 320 KiB.
            limits: UploadLimits { simple_max: 4 << 20, chunk: 32 * 320 * 1024 },
            source,
        }
    }

    #[must_use]
    pub fn with_urls(mut self, urls: OneDriveUrls) -> Self {
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
        self.api = Api::new(Arc::clone(&self.source), Http::new(retry));
        self
    }

    fn root(&self) -> String {
        format!("{}/me/drive/special/approot", self.urls.graph)
    }

    /// The address of a path below the app folder, `suffix` being `:/content` and the like.
    fn path_url(&self, path: &str, suffix: &str) -> String {
        format!("{}:/{}{suffix}", self.root(), encode_path(path))
    }

    fn children_url(&self, dir: &str) -> String {
        let params = format!("$select={ITEM_FIELDS}&$top={PAGE_SIZE}");
        if dir.is_empty() {
            format!("{}/children?{params}", self.root())
        } else {
            self.path_url(dir, &format!(":/children?{params}"))
        }
    }

    /// Adds to `found` the files below `dir` (a path, empty for the folder itself) whose full
    /// path starts with `prefix`, going into the folders that can hold such files.
    fn walk(&self, dir: &str, prefix: &str, found: &mut Vec<RemoteFile>) -> crate::Result<()> {
        let mut url = self.children_url(dir);
        loop {
            let response = self.api.call(|token| Request::get(&url).bearer(token))?;
            if response.status == 404 {
                return Ok(());
            }
            let page: Children = response.ok()?.json()?;
            for item in page.value {
                let path = if dir.is_empty() { item.name.clone() } else { format!("{dir}/{}", item.name) };
                if item.folder.is_some() {
                    let inside = format!("{path}/");
                    if inside.starts_with(prefix) || prefix.starts_with(&inside) {
                        self.walk(&path, prefix, found)?;
                    }
                } else if path.starts_with(prefix) {
                    found.push(item.into_file(path));
                }
            }
            match page.next_link {
                Some(next) => url = next,
                None => return Ok(()),
            }
        }
    }

    fn upload_simple(&self, name: &str, bytes: &[u8]) -> crate::Result<RemoteFile> {
        let url = self.path_url(name, ":/content");
        let item: Item =
            self.api.call_ok(|token| Request::put(&url).bearer(token).bytes(OCTET_STREAM, bytes))?.json()?;
        Ok(item.into_file(name.to_owned()))
    }

    /// An upload session, then the bytes in pieces.
    fn upload_session(&self, name: &str, bytes: &[u8]) -> crate::Result<RemoteFile> {
        let url = self.path_url(name, ":/createUploadSession");
        let body = json!({ "item": { "@microsoft.graph.conflictBehavior": "replace" } });
        let session: UploadSession = self.api.call_ok(|token| Request::post(&url).bearer(token).json(&body))?.json()?;

        let total = bytes.len();
        let mut sent = 0;
        loop {
            let end = (sent + self.limits.chunk).min(total);
            let range = format!("bytes {sent}-{}/{total}", end - 1);
            // The session address carries its own authorisation: Graph refuses a bearer token there.
            let response = self.api.send(
                &Request::put(&session.upload_url)
                    .header("Content-Range", range)
                    .bytes(OCTET_STREAM, &bytes[sent..end]),
            )?;
            match response.status {
                200 | 201 => return Ok(response.json::<Item>()?.into_file(name.to_owned())),
                202 => {
                    // `nextExpectedRanges: ["26-"]` is where the server wants the next byte.
                    let next = response
                        .json::<Progress>()?
                        .next_expected_ranges
                        .first()
                        .and_then(|r| r.split('-').next().and_then(|n| n.parse::<usize>().ok()))
                        .unwrap_or(end);
                    if next <= sent {
                        return Err(CloudError::Network("OneDrive không nhận phần dữ liệu vừa gửi".to_owned()));
                    }
                    sent = next.min(total);
                }
                _ => return Err(response.service_error()),
            }
        }
    }
}

impl CloudDrive for OneDrive {
    /// Lists the files whose path below the app folder starts with `prefix`. Only the folders
    /// that can hold such files are opened.
    fn list(&self, prefix: &str) -> crate::Result<Vec<RemoteFile>> {
        let dir = prefix.rsplit_once('/').map_or("", |(dir, _)| dir);
        let mut found = Vec::new();
        self.walk(dir, prefix, &mut found)?;
        Ok(found)
    }

    /// Creates or replaces the file at `name`, the folders in its path included.
    fn upload(&self, name: &str, bytes: &[u8]) -> crate::Result<RemoteFile> {
        if bytes.len() <= self.limits.simple_max {
            self.upload_simple(name, bytes)
        } else {
            self.upload_session(name, bytes)
        }
    }

    fn download(&self, file: &RemoteFile) -> crate::Result<Vec<u8>> {
        let url = format!("{}/me/drive/items/{}/content", self.urls.graph, encode(&file.id));
        let mut response = self.api.call(|token| Request::get(&url).bearer(token))?;
        for _ in 0..MAX_REDIRECTS {
            if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
                break;
            }
            let Some(location) = response.header("location").map(str::to_owned) else { break };
            // The content server is another host: it must not see the bearer token.
            response = self.api.send(&Request::get(location))?;
        }
        Ok(response.ok()?.body)
    }

    /// Deletes the file; one that is already gone counts as deleted.
    fn delete(&self, file: &RemoteFile) -> crate::Result<()> {
        let url = format!("{}/me/drive/items/{}", self.urls.graph, encode(&file.id));
        let response = self.api.call(|token| Request::delete(&url).bearer(token))?;
        if response.is_success() || response.status == 404 { Ok(()) } else { Err(response.service_error()) }
    }
}
