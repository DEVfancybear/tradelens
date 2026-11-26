//! Tự động cập nhật on the settings page, headless on the real engine. A release is served on this
//! machine (`latest.json` signed with a key made for these tests, its signature and an archive) and
//! the settings point the app at it (`update.url`, `update.public_key`). The tests run from the
//! code, not from a package, so the app checks and offers the release page but installs nothing.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::{Duration, Instant},
};

use nsh_core::Checked;
use nsh_testing::Headless;

mod support;
use support::{Fixture, WAIT, wait_until};

/// A signing key for these tests only, and its public half (`crates/update` checks they match).
const SECRET: &str = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=";
const PUBLIC: &str = "6kpsY+KcUgq+9VB7Ey7F+ZVHdq6+vnuSQh7qaRRG0iw=";
/// Someone else's key.
const OTHER_SECRET: &str = "CQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk=";

/// The version the app runs.
const RUNNING: &str = env!("CARGO_PKG_VERSION");

/// The switch of automatic updates.
const SWITCH: &str = "label:Tự động cập nhật";
/// The button that checks now.
const BUTTON: &str = "role:button[Kiểm tra cập nhật]";

/// A release host on `127.0.0.1`: files by name, a 404 for anything else.
struct Host {
    server: Arc<tiny_http::Server>,
    thread: Option<JoinHandle<()>>,
}

impl Host {
    /// Serves release `version`, its manifest signed with `secret`.
    fn release(version: &str, secret: &str) -> Self {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").expect("a local port"));
        let files: Arc<Mutex<BTreeMap<String, Vec<u8>>>> = Arc::default();
        let archive = b"not opened: a run from the code installs nothing".to_vec();
        let name = format!("TradeLens-{version}-{}.zip", tradelens_update::platform());
        let manifest = serde_json::to_vec(&serde_json::json!({
            "id": "vn.dat.tradelens",
            "version": version,
            "platforms": { tradelens_update::platform(): {
                "url": name,
                "sha256": tradelens_update::sha256(&archive),
                "size": archive.len(),
            } },
        }))
        .expect("the manifest is JSON");
        {
            let mut files = files.lock().expect("lock poisoned");
            files.insert("latest.json.sig".into(), tradelens_update::sign(secret, &manifest).expect("signed").into());
            files.insert("latest.json".into(), manifest);
            files.insert(name, archive);
        }
        let thread = {
            let (server, files) = (Arc::clone(&server), Arc::clone(&files));
            std::thread::spawn(move || {
                while let Ok(request) = server.recv() {
                    let name = request.url().trim_start_matches('/').to_owned();
                    let body = files.lock().expect("lock poisoned").get(&name).cloned();
                    drop(match body {
                        Some(bytes) => request.respond(tiny_http::Response::from_data(bytes)),
                        None => request.respond(tiny_http::Response::empty(404)),
                    });
                }
            })
        };
        Self { server, thread: Some(thread) }
    }

    fn manifest_url(&self) -> String {
        format!("http://{}/latest.json", self.server.server_addr())
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

/// A data directory whose app checks `host`; `auto` is the stored `update.auto`, if any.
fn fixture_for(host: &Host, auto: Option<&str>) -> Fixture {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    db.set_setting("update.url", &host.manifest_url()).expect("the source is stored");
    db.set_setting("update.public_key", PUBLIC).expect("the key is stored");
    if let Some(auto) = auto {
        db.set_setting("update.auto", auto).expect("the switch is stored");
    }
    fixture
}

fn open_settings(fixture: &Fixture) -> Headless {
    let mut app = fixture.start(1280.0, 1400.0, |_| {});
    app.press("role:button[Cài đặt]").expect("the settings row is a button");
    app.wait_for_within("Chế độ màu", WAIT).expect("the settings page opens");
    app.wait_for_within("Về TradeLens", WAIT).expect("the about card");
    app
}

/// Presses "Kiểm tra cập nhật" until the status line matches `status` (a press made before the
/// settings are read does nothing; checking twice is harmless).
fn check_until(app: &mut Headless, status: &str) {
    let started = Instant::now();
    while app.screen().matches(status) != Ok(true) {
        assert!(started.elapsed() < WAIT, "`{status}` did not show within {WAIT:?}");
        if app.screen().enabled(BUTTON) == Ok(true) {
            app.press(BUTTON).expect("the check button");
        }
        drop(app.wait_for_within(status, Duration::from_secs(1)));
    }
}

#[test]
fn checking_finds_a_newer_release_and_sends_a_run_from_the_code_to_its_page() {
    let host = Host::release("99.0.0", SECRET);
    let fixture = fixture_for(&host, Some("0"));
    let mut app = open_settings(&fixture);
    app.wait_for_within("text:/^Chưa kiểm tra bản mới/", WAIT).expect("nothing checked by itself while off");
    check_until(&mut app, "text:/^Có bản 99\\.0\\.0: tải ở trang phát hành/");
    app.wait_for("text:/^Chỉ bản cài đặt tự tải và cài bản mới/").expect("only a package installs itself");
    app.wait_for("role:button[Ghi chú phát hành]").expect("the way to the release page");
    assert_eq!(app.screen().matches("role:button[Khởi động lại để cập nhật]"), Ok(false), "nothing to restart into");
    app.press("role:button[Ghi chú phát hành]").expect("the link");
    app.wait_until("the release page opening", WAIT, |screen| {
        screen.opened_urls().iter().any(|url| url == "https://github.com/DEVfancybear/tradelens/releases")
    })
    .expect("a source of its own has no release page: the list of releases opens");
}

#[test]
fn the_running_version_is_the_newest() {
    let host = Host::release(RUNNING, SECRET);
    let fixture = fixture_for(&host, Some("0"));
    let mut app = open_settings(&fixture);
    check_until(&mut app, "text:/^Bản mới nhất · kiểm tra lúc \\d\\d:\\d\\d$/");
    app.wait_for(&format!("TradeLens {RUNNING}")).expect("the version runs above it");
}

#[test]
fn the_app_checks_by_itself_soon_after_it_opens() {
    let host = Host::release("99.0.0", SECRET);
    // Automatic updates are on unless turned off: nothing is stored for them.
    let fixture = fixture_for(&host, None);
    let mut app = open_settings(&fixture);
    app.wait_for_within("text:/^Có bản 99\\.0\\.0/", WAIT).expect("found without a press");
}

#[test]
fn a_release_signed_with_another_key_is_refused() {
    let host = Host::release("99.0.0", OTHER_SECRET);
    let fixture = fixture_for(&host, Some("0"));
    let mut app = open_settings(&fixture);
    check_until(&mut app, "text:/^Bản phát hành bị từ chối/");
}

#[test]
fn turning_automatic_updates_off_is_kept_on_this_device() {
    let host = Host::release(RUNNING, SECRET);
    let fixture = fixture_for(&host, Some("1"));
    let mut app = open_settings(&fixture);
    let db = fixture.db();
    let stored = |value: &str| db.setting("update.auto").is_ok_and(|v| v.as_deref() == Some(value));
    // Once its settings are read the app says it will check by itself (or already has).
    app.wait_for_within("text:/^(Sẽ tự kiểm tra|Đang kiểm tra|Bản mới nhất)/", WAIT).expect("the settings are read");
    app.press(SWITCH).expect("the switch");
    wait_until("the switch being turned off", || stored("0"));
    app.wait_until("the switch showing off", WAIT, |screen| {
        screen.accessibility_state(SWITCH).is_ok_and(|state| state.checked == Some(Checked::False))
    })
    .expect("the switch follows");
    app.press(SWITCH).expect("the switch again");
    wait_until("the switch being turned on", || stored("1"));
}
