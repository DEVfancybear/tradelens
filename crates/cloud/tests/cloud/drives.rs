//! Google Drive and OneDrive through [`CloudDrive`], and the token source behind them.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tradelens_cloud::auth::{ClientConfig, Provider, Tokens};
use tradelens_cloud::drive::{CloudDrive, RemoteFile};
use tradelens_cloud::google::{GoogleDrive, GoogleUrls};
use tradelens_cloud::onedrive::{OneDrive, OneDriveUrls};
use tradelens_cloud::token_store::{MemoryStore, TokenSource, TokenStore};
use tradelens_cloud::{CloudError, RetryPolicy, UploadLimits};

use crate::auth::{endpoints, now};
use crate::mock::{Mock, Reply, Seen};

const FAST_RETRY: RetryPolicy =
    RetryPolicy { max_attempts: 4, base_delay: Duration::from_millis(1), max_delay: Duration::from_millis(20) };

fn tokens(access: &str, valid_for: i64) -> Tokens {
    Tokens { access_token: access.into(), refresh_token: Some("rt".into()), expires_at: now() + valid_for }
}

fn session(provider: Provider, base: &str, tokens: &Tokens) -> (Arc<TokenSource>, Arc<MemoryStore>) {
    let store = Arc::new(MemoryStore::new());
    store.save(provider, tokens).expect("saves");
    let client = ClientConfig::new(provider, "client").with_endpoints(endpoints(base));
    let handle = Arc::clone(&store);
    let shared: Arc<dyn TokenStore> = handle;
    (Arc::new(TokenSource::new(client, shared)), store)
}

fn refresh_reply() -> Reply {
    Reply::ok(&json!({"access_token": "fresh", "expires_in": 3600}))
}

fn google(base: &str, source: Arc<TokenSource>) -> GoogleDrive {
    GoogleDrive::new(source)
        .with_urls(GoogleUrls { api: format!("{base}/drive/v3"), upload: format!("{base}/upload/drive/v3") })
        .with_retry(FAST_RETRY)
}

fn onedrive(base: &str, source: Arc<TokenSource>) -> OneDrive {
    OneDrive::new(source).with_urls(OneDriveUrls { graph: format!("{base}/v1.0") }).with_retry(FAST_RETRY)
}

fn gfile(id: &str, name: &str, size: usize) -> Value {
    json!({"id": id, "name": name, "size": size.to_string(), "modifiedTime": "2026-11-22T10:30:00.000Z"})
}

fn remote(id: &str, name: &str) -> RemoteFile {
    RemoteFile { id: id.into(), name: name.into(), size: 0, modified: 0 }
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| u8::try_from(i % 251).expect("below 251")).collect()
}

// --- the token source --------------------------------------------------------------

#[test]
fn an_expired_token_is_refreshed_before_the_call_and_saved() {
    let mock = Mock::start(|req: &Seen, _| match req.path() {
        "/token" => refresh_reply(),
        "/drive/v3/files" => Reply::ok(&json!({"files": []})),
        _ => Reply::status(404),
    });
    let (source, store) = session(Provider::Google, &mock.base, &tokens("stale", -100));
    google(&mock.base, source).list("changes/").expect("lists");

    assert_eq!(mock.requests_to("/token").len(), 1);
    assert_eq!(mock.requests_to("/drive/v3/files")[0].header("authorization"), Some("Bearer fresh"));
    assert_eq!(store.load(Provider::Google).expect("loads").expect("saved").access_token, "fresh");
}

#[test]
fn a_401_refreshes_once_and_repeats_the_call() {
    let mock = Mock::start(|req: &Seen, _| match req.path() {
        "/token" => refresh_reply(),
        "/drive/v3/files" if req.header("authorization") == Some("Bearer fresh") => {
            Reply::ok(&json!({"files": [gfile("1", "changes/a/1-1.json", 5)]}))
        }
        "/drive/v3/files" => Reply::json(401, &json!({"error": {"code": 401, "message": "Invalid Credentials"}})),
        _ => Reply::status(404),
    });
    let (source, _) = session(Provider::Google, &mock.base, &tokens("revoked-but-unexpired", 3000));
    let files = google(&mock.base, source).list("changes/").expect("lists after the refresh");
    assert_eq!(files.len(), 1);
    assert_eq!(mock.requests_to("/token").len(), 1);
    assert_eq!(mock.requests_to("/drive/v3/files").len(), 2);
}

#[test]
fn a_second_401_means_signed_out() {
    let mock = Mock::start(|req: &Seen, _| match req.path() {
        "/token" => refresh_reply(),
        _ => Reply::json(401, &json!({"error": {"message": "nope"}})),
    });
    let (source, _) = session(Provider::Google, &mock.base, &tokens("t", 3000));
    let result = google(&mock.base, source).list("");
    assert!(matches!(result, Err(CloudError::SignedOut)), "{result:?}");
    assert_eq!(mock.requests_to("/token").len(), 1, "one refresh, not a loop");
}

#[test]
fn a_revoked_session_is_cleared_from_the_store() {
    let mock = Mock::start(|_, _| Reply::json(400, &json!({"error": "invalid_grant"})));
    let (source, store) = session(Provider::Google, &mock.base, &tokens("old", -5));
    assert!(matches!(source.access_token(), Err(CloudError::SignedOut)));
    assert_eq!(store.load(Provider::Google).expect("loads"), None);
    assert!(!source.is_signed_in().expect("reads"));
}

// --- Google Drive ------------------------------------------------------------------

#[test]
fn google_lists_every_page_and_keeps_the_prefix() {
    let mock = Mock::start(|req: &Seen, _| {
        assert_eq!(req.param("spaces").as_deref(), Some("appDataFolder"));
        match req.param("pageToken").as_deref() {
            None => Reply::ok(&json!({
                "nextPageToken": "p2",
                "files": [gfile("1", "changes/a/1-2.json", 10), gfile("2", "snapshots/2026-11-22.db", 99)],
            })),
            Some("p2") => Reply::ok(&json!({
                "files": [gfile("3", "changes/b/1-1.json", 20), gfile("4", "changes/a/3-4.json", 30)],
            })),
            Some(other) => panic!("unexpected page {other}"),
        }
    });
    let (source, _) = session(Provider::Google, &mock.base, &tokens("tok", 3000));
    let files = google(&mock.base, source).list("changes/").expect("lists");

    let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["changes/a/1-2.json", "changes/b/1-1.json", "changes/a/3-4.json"]);
    assert_eq!((files[0].id.as_str(), files[0].size), ("1", 10));
    assert_eq!(files[0].modified, 1_795_343_400, "2026-11-22T10:30:00Z");
    assert_eq!(mock.requests().len(), 2);
    assert_eq!(mock.requests()[0].header("authorization"), Some("Bearer tok"));
}

#[test]
fn google_creates_a_small_file_with_one_multipart_request() {
    let mock = Mock::start(|req: &Seen, _| match (req.method.as_str(), req.path()) {
        ("GET", "/drive/v3/files") => Reply::ok(&json!({"files": []})),
        ("POST", "/upload/drive/v3/files") => Reply::ok(&gfile("new", "changes/dev1/1-5.json", 7)),
        other => panic!("unexpected {other:?}"),
    });
    let (source, _) = session(Provider::Google, &mock.base, &tokens("tok", 3000));
    let file = google(&mock.base, source).upload("changes/dev1/1-5.json", b"{\"a\":1}").expect("uploads");
    assert_eq!((file.id.as_str(), file.size), ("new", 7));

    let find = &mock.requests()[0];
    assert_eq!(find.param("q").as_deref(), Some("name = 'changes/dev1/1-5.json' and trashed = false"));
    let post = &mock.requests()[1];
    assert_eq!(post.param("uploadType").as_deref(), Some("multipart"));
    assert!(post.header("content-type").is_some_and(|t| t.starts_with("multipart/related; boundary=")));
    let body = post.text();
    assert!(body.contains(r#"{"name":"changes/dev1/1-5.json","parents":["appDataFolder"]}"#), "{body}");
    assert!(body.contains("\r\n\r\n{\"a\":1}\r\n--"), "{body}");
}

#[test]
fn google_replaces_a_file_that_exists_by_updating_it() {
    let mock = Mock::start(|req: &Seen, _| match (req.method.as_str(), req.path()) {
        ("GET", "/drive/v3/files") => Reply::ok(&json!({"files": [gfile("F1", "devices.json", 2)]})),
        ("PATCH", "/upload/drive/v3/files/F1") => Reply::ok(&gfile("F1", "devices.json", 4)),
        other => panic!("unexpected {other:?}"),
    });
    let (source, _) = session(Provider::Google, &mock.base, &tokens("tok", 3000));
    let file = google(&mock.base, source).upload("devices.json", b"[ 1 ]").expect("uploads");
    assert_eq!((file.id.as_str(), file.size), ("F1", 4));
    let patch = &mock.requests()[1];
    assert_eq!((patch.param("uploadType").as_deref(), patch.text().as_str()), (Some("media"), "[ 1 ]"));
}

#[test]
fn google_sends_a_big_file_through_a_resumable_session_in_pieces() {
    let data = pattern(600_000);
    let received = Arc::new(Mutex::new(Vec::<u8>::new()));
    let sink = Arc::clone(&received);
    let mock = Mock::start(move |req: &Seen, base: &str| match (req.method.as_str(), req.path()) {
        ("GET", "/drive/v3/files") => Reply::ok(&json!({"files": []})),
        ("POST", "/upload/drive/v3/files") => {
            Reply::status(200).header("Location", &format!("{base}/session/1")).body(Vec::new())
        }
        ("PUT", "/session/1") => {
            sink.lock().expect("lock poisoned").extend_from_slice(&req.body);
            let range = req.header("content-range").expect("a Content-Range").to_owned();
            let (span, total) = range.trim_start_matches("bytes ").split_once('/').expect("span/total");
            let last: usize = span.split_once('-').expect("a-b").1.parse().expect("a number");
            if last + 1 < total.parse::<usize>().expect("a number") {
                Reply::status(308).header("Range", &format!("bytes=0-{last}"))
            } else {
                Reply::ok(&gfile("big", "snapshots/x.db", last + 1))
            }
        }
        other => panic!("unexpected {other:?}"),
    });
    let (source, _) = session(Provider::Google, &mock.base, &tokens("tok", 3000));
    let drive = google(&mock.base, source).with_limits(UploadLimits { simple_max: 1000, chunk: 262_144 });
    let file = drive.upload("snapshots/x.db", &data).expect("uploads");
    assert_eq!((file.id.as_str(), file.size), ("big", 600_000));

    assert_eq!(*received.lock().expect("lock poisoned"), data, "the pieces add up to the file");
    let open = &mock.requests()[1];
    assert_eq!(open.param("uploadType").as_deref(), Some("resumable"));
    assert_eq!(open.header("x-upload-content-length"), Some("600000"));
    assert_eq!(open.json()["parents"], json!(["appDataFolder"]));
    let ranges: Vec<_> = mock
        .requests_to("/session/1")
        .iter()
        .map(|r| r.header("content-range").unwrap_or_default().to_owned())
        .collect();
    assert_eq!(ranges, ["bytes 0-262143/600000", "bytes 262144-524287/600000", "bytes 524288-599999/600000"]);
    assert!(
        mock.requests_to("/session/1").iter().all(|r| r.header("authorization").is_none()),
        "the session is its own authorisation"
    );
}

#[test]
fn google_retries_429_after_the_time_the_service_asks() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let mock = Mock::start(move |_, _| {
        if counter.fetch_add(1, Ordering::SeqCst) < 2 {
            Reply::json(429, &json!({"error": {"message": "rate limit"}})).header("Retry-After", "0")
        } else {
            Reply::ok(&json!({"files": [gfile("1", "a", 1)]}))
        }
    });
    let (source, _) = session(Provider::Google, &mock.base, &tokens("tok", 3000));
    let files = google(&mock.base, source).list("").expect("succeeds on the third try");
    assert_eq!(files.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[test]
fn google_gives_up_with_the_service_message_after_the_last_try() {
    let mock = Mock::start(|_, _| Reply::json(503, &json!({"error": {"message": "backend unavailable"}})));
    let (source, _) = session(Provider::Google, &mock.base, &tokens("tok", 3000));
    let result = google(&mock.base, source).list("");
    assert!(
        matches!(&result, Err(CloudError::Service { status: 503, message }) if message == "backend unavailable"),
        "{result:?}"
    );
    assert_eq!(mock.requests().len(), 4, "FAST_RETRY allows four tries");
}

#[test]
fn google_downloads_and_deletes() {
    let mock = Mock::start(|req: &Seen, _| match (req.method.as_str(), req.path()) {
        ("GET", "/drive/v3/files/abc") => Reply::status(200).body(vec![0, 1, 2, 255]),
        ("DELETE", "/drive/v3/files/abc") => Reply::status(204),
        ("DELETE", "/drive/v3/files/gone") => Reply::status(404),
        other => panic!("unexpected {other:?}"),
    });
    let (source, _) = session(Provider::Google, &mock.base, &tokens("tok", 3000));
    let drive = google(&mock.base, source);
    assert_eq!(drive.download(&remote("abc", "x")).expect("downloads"), [0, 1, 2, 255]);
    assert_eq!(mock.requests()[0].param("alt").as_deref(), Some("media"));
    drive.delete(&remote("abc", "x")).expect("deletes");
    drive.delete(&remote("gone", "y")).expect("already gone is fine");
}

// --- OneDrive ----------------------------------------------------------------------

fn item(id: &str, name: &str, size: usize) -> Value {
    json!({"id": id, "name": name, "size": size, "lastModifiedDateTime": "2026-11-22T10:30:00Z"})
}

fn folder(id: &str, name: &str) -> Value {
    json!({"id": id, "name": name, "size": 0, "lastModifiedDateTime": "2026-11-22T10:30:00Z", "folder": {"childCount": 1}})
}

#[test]
fn onedrive_lists_folders_recursively_and_follows_next_links() {
    let mock = Mock::start(|req: &Seen, base: &str| match req.path() {
        "/v1.0/me/drive/special/approot:/changes:/children" => Reply::ok(&json!({
            "value": [folder("d1", "dev1"), item("n", "notes.txt", 3)],
            "@odata.nextLink": format!("{base}/next-page"),
        })),
        "/next-page" => Reply::ok(&json!({"value": [folder("d2", "dev2")]})),
        "/v1.0/me/drive/special/approot:/changes/dev1:/children" => {
            Reply::ok(&json!({"value": [item("f1", "1-5.json", 40), item("f2", "6-9.json", 50)]}))
        }
        "/v1.0/me/drive/special/approot:/changes/dev2:/children" => {
            Reply::ok(&json!({"value": [item("f3", "1-1.json", 60)]}))
        }
        other => panic!("unexpected {other}"),
    });
    let (source, _) = session(Provider::Microsoft, &mock.base, &tokens("tok", 3000));
    let drive = onedrive(&mock.base, source);

    let files = drive.list("changes/").expect("lists");
    let mut names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["changes/dev1/1-5.json", "changes/dev1/6-9.json", "changes/dev2/1-1.json", "changes/notes.txt"]);
    assert_eq!(files.iter().find(|f| f.id == "f3").map(|f| (f.size, f.modified)), Some((60, 1_795_343_400)));
    assert_eq!(mock.requests()[0].header("authorization"), Some("Bearer tok"));

    // A prefix inside a device's folder opens only that folder and filters by name.
    let narrow = drive.list("changes/dev1/6-").expect("lists");
    assert_eq!(narrow.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["changes/dev1/6-9.json"]);
}

#[test]
fn onedrive_lists_nothing_below_a_folder_that_does_not_exist() {
    let mock = Mock::start(|_, _| Reply::json(404, &json!({"error": {"code": "itemNotFound", "message": "no"}})));
    let (source, _) = session(Provider::Microsoft, &mock.base, &tokens("tok", 3000));
    assert_eq!(onedrive(&mock.base, source).list("changes/").expect("not an error"), Vec::new());
}

#[test]
fn onedrive_puts_a_small_file_at_its_path() {
    let mock = Mock::start(|req: &Seen, _| match (req.method.as_str(), req.path()) {
        ("PUT", "/v1.0/me/drive/special/approot:/changes/dev%201/1-5.json:/content") => {
            Reply::json(201, &item("new", "1-5.json", 3))
        }
        other => panic!("unexpected {other:?}"),
    });
    let (source, _) = session(Provider::Microsoft, &mock.base, &tokens("tok", 3000));
    let file = onedrive(&mock.base, source).upload("changes/dev 1/1-5.json", b"abc").expect("uploads");
    assert_eq!((file.id.as_str(), file.name.as_str(), file.size), ("new", "changes/dev 1/1-5.json", 3));
    assert_eq!(mock.requests()[0].body, b"abc");
}

#[test]
fn onedrive_sends_a_big_file_through_an_upload_session() {
    let data = pattern(700_000);
    let received = Arc::new(Mutex::new(Vec::<u8>::new()));
    let sink = Arc::clone(&received);
    let mock = Mock::start(move |req: &Seen, base: &str| match (req.method.as_str(), req.path()) {
        ("POST", "/v1.0/me/drive/special/approot:/snapshots/x.db:/createUploadSession") => {
            Reply::ok(&json!({"uploadUrl": format!("{base}/up/1"), "expirationDateTime": "2026-12-01T00:00:00Z"}))
        }
        ("PUT", "/up/1") => {
            sink.lock().expect("lock poisoned").extend_from_slice(&req.body);
            let range = req.header("content-range").expect("a Content-Range").to_owned();
            let (span, total) = range.trim_start_matches("bytes ").split_once('/').expect("span/total");
            let last: usize = span.split_once('-').expect("a-b").1.parse().expect("a number");
            if last + 1 < total.parse::<usize>().expect("a number") {
                Reply::json(202, &json!({"nextExpectedRanges": [format!("{}-", last + 1)]}))
            } else {
                Reply::json(201, &item("big", "x.db", last + 1))
            }
        }
        other => panic!("unexpected {other:?}"),
    });
    let (source, _) = session(Provider::Microsoft, &mock.base, &tokens("tok", 3000));
    let drive = onedrive(&mock.base, source).with_limits(UploadLimits { simple_max: 1000, chunk: 327_680 });
    let file = drive.upload("snapshots/x.db", &data).expect("uploads");
    assert_eq!((file.id.as_str(), file.size), ("big", 700_000));

    assert_eq!(*received.lock().expect("lock poisoned"), data);
    let ranges: Vec<_> =
        mock.requests_to("/up/1").iter().map(|r| r.header("content-range").unwrap_or_default().to_owned()).collect();
    assert_eq!(ranges, ["bytes 0-327679/700000", "bytes 327680-655359/700000", "bytes 655360-699999/700000"]);
    assert!(
        mock.requests_to("/up/1").iter().all(|r| r.header("authorization").is_none()),
        "Graph refuses a bearer token on the session"
    );
    assert_eq!(
        mock.requests()[0].json()["item"]["@microsoft.graph.conflictBehavior"],
        json!("replace"),
        "a snapshot of the same day replaces the earlier one"
    );
}

#[test]
fn onedrive_follows_the_download_redirect_without_the_token() {
    let mock = Mock::start(|req: &Seen, base: &str| match req.path() {
        "/v1.0/me/drive/items/ID1/content" => Reply::status(302).header("Location", &format!("{base}/cdn/blob")),
        "/cdn/blob" => Reply::status(200).body(vec![9, 8, 7]),
        "/v1.0/me/drive/items/ID1" if req.method == "DELETE" => Reply::status(204),
        "/v1.0/me/drive/items/GONE" => Reply::json(404, &json!({"error": {"code": "itemNotFound"}})),
        other => panic!("unexpected {other}"),
    });
    let (source, _) = session(Provider::Microsoft, &mock.base, &tokens("tok", 3000));
    let drive = onedrive(&mock.base, source);
    assert_eq!(drive.download(&remote("ID1", "x")).expect("downloads"), [9, 8, 7]);
    assert_eq!(mock.requests_to("/cdn/blob")[0].header("authorization"), None, "the CDN must not see the token");
    drive.delete(&remote("ID1", "x")).expect("deletes");
    drive.delete(&remote("GONE", "y")).expect("already gone is fine");
}

#[test]
fn onedrive_refreshes_on_401_like_google() {
    let mock = Mock::start(|req: &Seen, _| match req.path() {
        "/token" => refresh_reply(),
        _ if req.header("authorization") == Some("Bearer fresh") => Reply::ok(&json!({"value": []})),
        _ => Reply::json(401, &json!({"error": {"code": "InvalidAuthenticationToken", "message": "expired"}})),
    });
    let (source, store) = session(Provider::Microsoft, &mock.base, &tokens("stale", 3000));
    onedrive(&mock.base, source).list("changes/").expect("lists after the refresh");
    assert_eq!(store.load(Provider::Microsoft).expect("loads").expect("saved").access_token, "fresh");
}

#[test]
fn google_403_for_a_missing_drive_scope_is_the_vietnamese_permission_message() {
    // What Google answers when the token was granted without the Drive permission.
    let body = json!({"error": {
        "code": 403,
        "message": "Request had insufficient authentication scopes.",
        "errors": [{"message": "Insufficient Permission", "domain": "global", "reason": "insufficientPermissions"}],
        "status": "PERMISSION_DENIED"
    }});
    let mock = Mock::start(move |_, _| Reply::json(403, &body));
    let (source, _) = session(Provider::Google, &mock.base, &tokens("tok", 3000));
    let error = google(&mock.base, source).list("changes/").expect_err("refused");
    assert!(matches!(error, CloudError::DriveNotGranted), "{error:?}");
    assert!(error.to_string().contains("tick ô quyền Google Drive"), "{error}");

    // Any other 403 keeps the service's own words.
    let other = Mock::start(|_, _| Reply::json(403, &json!({"error": {"code": 403, "message": "Quota exceeded"}})));
    let (source, _) = session(Provider::Google, &other.base, &tokens("tok", 3000));
    let error = google(&other.base, source).list("changes/").expect_err("refused");
    assert!(matches!(error, CloudError::Service { status: 403, .. }), "{error:?}");
}

#[test]
fn google_creates_a_file_known_to_be_new_without_looking_its_name_up() {
    let mock = Mock::start(|req: &Seen, _| match (req.method.as_str(), req.path()) {
        ("POST", "/upload/drive/v3/files") => Reply::ok(&gfile("new", "attachments/ab.png", 3)),
        other => panic!("unexpected {other:?}"),
    });
    let (source, _) = session(Provider::Google, &mock.base, &tokens("tok", 3000));
    let file = google(&mock.base, source).upload_new("attachments/ab.png", b"png").expect("uploads");
    assert_eq!(file.id, "new");
    assert_eq!(mock.requests().len(), 1, "one request, no lookup: {:?}", mock.requests());
    assert_eq!(mock.requests()[0].param("uploadType").as_deref(), Some("multipart"));
}
