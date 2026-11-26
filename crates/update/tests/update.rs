//! The updater against a release served on this machine: a manifest signed with a key made for
//! the test, its signature and an archive, as `nsh package` and `nsh update-manifest` publish them.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread::JoinHandle,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use tradelens_update::{Installation, Origin, Source, UpdateError, platform, sha256, sign};

const ID: &str = "vn.dat.tradelens";
/// A signing key for these tests only (32 bytes, base64): never one a release is signed with.
const SECRET: &str = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=";
/// Another key, for a manifest someone else signed.
const OTHER_SECRET: &str = "CQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk=";

fn public_key(secret: &str) -> String {
    let seed: [u8; 32] = B64.decode(secret).expect("base64").try_into().expect("32 bytes");
    B64.encode(ed25519_dalek::SigningKey::from_bytes(&seed).verifying_key().to_bytes())
}

/// Files served at `http://127.0.0.1:<port>/<name>`; anything else is a 404.
struct Host {
    server: Arc<tiny_http::Server>,
    files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    thread: Option<JoinHandle<()>>,
}

impl Host {
    fn start() -> Self {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").expect("a local port"));
        let files: Arc<Mutex<BTreeMap<String, Vec<u8>>>> = Arc::default();
        let thread = {
            let (server, files) = (Arc::clone(&server), Arc::clone(&files));
            std::thread::spawn(move || {
                while let Ok(request) = server.recv() {
                    let name = request.url().trim_start_matches('/').to_owned();
                    let body = files.lock().expect("lock poisoned").get(&name).cloned();
                    let answered = match body {
                        Some(bytes) => request.respond(tiny_http::Response::from_data(bytes)),
                        None => request.respond(tiny_http::Response::empty(404)),
                    };
                    drop(answered);
                }
            })
        };
        Self { server, files, thread: Some(thread) }
    }

    fn url(&self, name: &str) -> String {
        format!("http://{}/{name}", self.server.server_addr())
    }

    fn put(&self, name: &str, bytes: impl Into<Vec<u8>>) {
        self.files.lock().expect("lock poisoned").insert(name.to_owned(), bytes.into());
    }

    /// Publishes release `version` with `archive` for this machine, signed with `secret`.
    fn release(&self, version: &str, archive: &[u8], secret: &str) {
        let name = format!("TradeLens-{version}-{}.tar", platform());
        let manifest = serde_json::to_vec(&serde_json::json!({
            "id": ID,
            "version": version,
            "notes": "Sửa lỗi",
            "platforms": { platform(): {
                "url": name,
                "sha256": sha256(archive),
                "size": archive.len(),
            } },
        }))
        .expect("the manifest is JSON");
        self.put("latest.json.sig", sign(secret, &manifest).expect("the test key signs"));
        self.put("latest.json", manifest);
        self.put(&name, archive);
    }

    fn source(&self, version: &str) -> Source {
        let key = public_key(SECRET);
        Source::new(Origin::Url(self.url("latest.json")), ID, version, Some(&key)).expect("a valid source")
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(thread) = self.thread.take() {
            drop(thread.join());
        }
    }
}

/// The executable's name in a package on this system.
fn exe_name() -> &'static str {
    if cfg!(windows) { "TradeLens.exe" } else { "TradeLens" }
}

/// The configuration `nsh package` would ship.
const CONFIG: &[u8] = br#"{"id":"vn.dat.tradelens","version":"0.1.0","github":"DEVfancybear/tradelens"}"#;

/// A package of version 0.1.0 laid out as `nsh package` makes it on Windows and Linux: the
/// executable beside `nsh/app.json` and `nsh/assets/nsh-updater.json`. Returns its executable.
fn installed_package(dir: &Path) -> PathBuf {
    let root = dir.join("TradeLens");
    fs::create_dir_all(root.join("nsh/assets")).expect("the package directory");
    fs::write(root.join("nsh/assets/nsh-updater.json"), CONFIG).expect("the configuration");
    fs::write(root.join("nsh/app.json"), format!(r#"{{"id":"{ID}","version":"0.1.0"}}"#)).expect("app.json");
    let exe = root.join(exe_name());
    fs::write(&exe, "bản cũ").expect("the executable");
    exe
}

/// A `.tar` of the app at `version`, holding one directory as `nsh package`'s archives do.
fn archive(version: &str) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    let mut add = |path: String, data: &[u8], mode: u32| {
        let mut header = tar::Header::new_gnu();
        header.set_size(u64::try_from(data.len()).expect("small"));
        header.set_mode(mode);
        header.set_cksum();
        builder.append_data(&mut header, path, data).expect("the entry is written");
    };
    add(format!("TradeLens/{}", exe_name()), format!("bản {version}").as_bytes(), 0o755);
    add("TradeLens/nsh/app.json".into(), format!(r#"{{"id":"{ID}","version":"{version}"}}"#).as_bytes(), 0o644);
    add("TradeLens/nsh/assets/nsh-updater.json".into(), CONFIG, 0o644);
    builder.into_inner().expect("the archive is finished")
}

#[test]
fn a_newer_release_is_offered_and_the_same_or_an_older_one_is_not() {
    let server = Host::start();
    server.release("0.4.1", &archive("0.4.1"), SECRET);
    let found = server.source("0.1.0").check().expect("the manifest is read").expect("0.4.1 is newer");
    assert_eq!(found.version, "0.4.1");
    assert_eq!(found.notes.as_deref(), Some("Sửa lỗi"));
    assert_eq!(found.url, server.url(&format!("TradeLens-0.4.1-{}.tar", platform())), "next to the manifest");
    assert!(found.sha256.is_some(), "an archive, not a page");
    assert!(server.source("0.4.1").check().expect("read").is_none(), "the same version");
    assert!(server.source("0.5.0").check().expect("read").is_none(), "an older one");
}

#[test]
fn a_manifest_signed_with_another_key_or_not_signed_is_refused() {
    let server = Host::start();
    server.release("0.4.1", &archive("0.4.1"), OTHER_SECRET);
    let error = server.source("0.1.0").check().expect_err("another key signed it");
    assert!(matches!(error, UpdateError::Refused(_)), "{error}");
    server.files.lock().expect("lock poisoned").remove("latest.json.sig");
    let error = server.source("0.1.0").check().expect_err("nobody signed it");
    assert!(error.to_string().contains("not signed"), "{error}");
}

#[test]
fn no_release_at_all_is_a_404_of_the_manifest() {
    let server = Host::start();
    let error = server.source("0.1.0").check().expect_err("nothing is published");
    assert!(matches!(error, UpdateError::Status { status: 404, .. }), "{error}");
}

#[test]
fn a_download_that_does_not_match_the_manifest_is_refused_and_removed() {
    let server = Host::start();
    server.release("0.4.1", &archive("0.4.1"), SECRET);
    let found = server.source("0.1.0").check().expect("read").expect("newer");
    // The archive on the server is swapped after the manifest was signed.
    server.put(&format!("TradeLens-0.4.1-{}.tar", platform()), archive("6.6.6"));
    let dir = tempfile::tempdir().expect("a temp dir");
    let error = server.source("0.1.0").download(&found, dir.path()).expect_err("the hash differs");
    assert!(matches!(error, UpdateError::Refused(_)), "{error}");
    let left: Vec<_> = fs::read_dir(dir.path()).expect("the directory").collect();
    assert!(left.is_empty(), "nothing is kept of a refused download: {left:?}");
}

#[test]
fn a_page_instead_of_an_archive_cannot_be_downloaded() {
    let server = Host::start();
    let manifest = serde_json::to_vec(&serde_json::json!({
        "id": ID, "version": "0.4.1",
        "platforms": { platform(): { "url": "https://github.com/DEVfancybear/tradelens/releases" } },
    }))
    .expect("JSON");
    server.put("latest.json.sig", sign(SECRET, &manifest).expect("signed"));
    server.put("latest.json", manifest);
    let found = server.source("0.1.0").check().expect("read").expect("newer");
    let dir = tempfile::tempdir().expect("a temp dir");
    let error = server.source("0.1.0").download(&found, dir.path()).expect_err("a page");
    assert!(matches!(error, UpdateError::NotInstallable), "{error}");
}

#[test]
fn without_an_installation_nothing_is_staged() {
    let server = Host::start();
    server.release("0.4.1", &archive("0.4.1"), SECRET);
    let source = server.source("0.1.0");
    let found = source.check().expect("read").expect("newer");
    let dir = tempfile::tempdir().expect("a temp dir");
    let archive = source.download(&found, dir.path()).expect("downloaded and checked");
    assert!(!source.is_packaged());
    assert!(matches!(source.stage(&archive), Err(UpdateError::NotInstallable)));
}

// macOS packages are `.app` bundles that `codesign` must accept: the layout here is Windows' and Linux's.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_downloaded_release_is_staged_beside_the_app_and_swapped_in_when_finished() {
    let server = Host::start();
    server.release("0.4.1", &archive("0.4.1"), SECRET);
    let apps = tempfile::tempdir().expect("a temp dir");
    let exe = installed_package(apps.path());
    let installation = Installation::of(&exe, ID, CONFIG).expect("a package as nsh package lays it out");
    let source = server.source("0.1.0").with_installation(installation);
    assert!(source.is_packaged() && source.installable());

    let found = source.check().expect("read").expect("newer");
    let downloads = apps.path().join("downloads");
    let archive = source.download(&found, &downloads).expect("downloaded and checked");
    assert_eq!(source.download(&found, &downloads).expect("already there"), archive, "not fetched twice");
    let staged = source.stage(&archive).expect("the archive holds the app");
    assert_eq!(fs::read_to_string(&exe).expect("the app"), "bản cũ", "staging leaves the installed app alone");

    let new = staged.finish().expect("swapped in");
    assert_eq!(new, exe, "the same executable is started again");
    assert_eq!(fs::read_to_string(&exe).expect("the app"), "bản 0.4.1");
    let app_json = fs::read_to_string(exe.with_file_name("nsh").join("app.json")).expect("app.json");
    assert!(app_json.contains("0.4.1"), "{app_json}");
    source.remove_leftovers();
    assert_eq!(names(apps.path()), ["TradeLens", "downloads"], "nothing of the update is left beside the app");
    assert_eq!(names(&exe.with_file_name("")), [exe_name(), "nsh"], "nor in it");
}

#[cfg(not(target_os = "macos"))]
#[test]
fn an_archive_without_an_app_is_not_staged() {
    let apps = tempfile::tempdir().expect("a temp dir");
    let exe = installed_package(apps.path());
    let source = Source::new(Origin::Github("DEVfancybear/tradelens".into()), ID, "0.1.0", None)
        .expect("a valid source")
        .with_installation(Installation::of(&exe, ID, CONFIG).expect("a package"));
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(4);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append_data(&mut header, "README.txt", &b"text"[..]).expect("written");
    let bad = apps.path().join("bad.tar");
    fs::write(&bad, builder.into_inner().expect("finished")).expect("written");
    assert!(matches!(source.stage(&bad), Err(UpdateError::Refused(_))));
    assert_eq!(fs::read_to_string(&exe).expect("the app"), "bản cũ");
}

/// The names in `dir`, sorted.
#[cfg(not(target_os = "macos"))]
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("the directory")
        .filter_map(|entry| entry.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect();
    names.sort();
    names
}

#[test]
fn the_test_key_is_the_one_the_screen_tests_trust() {
    // `app/tests/update_screens.rs` signs with `SECRET` and trusts this public key.
    assert_eq!(public_key(SECRET), "6kpsY+KcUgq+9VB7Ey7F+ZVHdq6+vnuSQh7qaRRG0iw=");
}
