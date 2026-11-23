//! Importing from Notion on Nguồn dữ liệu, headless on the real engine: the `.zip` Notion exports
//! (with its images) dropped in the same zone as a CSV, and the Notion API (a local server plays Notion;
//! the demo mode keeps the token in memory instead of the operating system's credential store).

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

use nsh_testing::Headless;
use serde_json::{Value, json};
use tiny_http::{Header, Response, Server, StatusCode};
use tradelens_core::Db;

mod support;
use support::{Fixture, WAIT, wait_until};

const DROPZONE: &str = "label:Kéo thả file ZIP hoặc CSV của Notion vào đây";

fn core_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/core/tests/fixtures").join(name)
}

fn start(fixture: &Fixture) -> Headless {
    let mut app = fixture.start(1400.0, 1300.0, |_| {});
    app.press("role:button[Nguồn dữ liệu]").expect("the data sources row is a button");
    app.wait_for("Nhật ký import").expect("the page opens");
    app
}

/// The text of the first thing on screen that contains `needle`.
fn shows_containing(app: &mut Headless, what: &str, needle: &str) {
    app.wait_until(what, WAIT, |screen| screen.texts().iter().any(|t| t.contains(needle)))
        .unwrap_or_else(|error| panic!("{what}: {error}"));
}

#[test]
fn a_zip_is_previewed_with_what_each_column_was_taken_for_and_imported_with_its_images() {
    let fixture = Fixture::new();
    let mut app = start(&fixture);
    app.on_pick_paths(vec![core_fixture("notion_r_journal.zip")]).expect("answers the dialog");
    app.press(DROPZONE).expect("the drop zone opens the dialog");

    shows_containing(&mut app, "the preview", "5 lệnh đọc được");
    app.wait_for("Cột tự nhận ra (từ nội dung, không cần ghép tay)").expect("the detected roles are listed");
    app.wait_for("Cột «Risk/Reward» → Kết quả (R)").expect("the result column is named");
    app.wait_for("Cột «Cặp tiền» → Symbol").expect("the symbol column is named");
    app.wait_for("Cột «Loại lệnh» → Buy/Sell").expect("the side column is named");
    app.wait_for("Cột «Ngày giao dịch» → Thời gian vào").expect("the date column is named");
    app.wait_for("Cột «Ảnh» → Ảnh").expect("the files column is named");
    shows_containing(&mut app, "the status column", "Cột «Trạng thái» → thắng/thua (");
    shows_containing(&mut app, "the total in R", "tổng kết quả trong file +1.4R · 2 thắng, 2 thua, 1 hòa vốn");
    app.wait_for("Ảnh: 4 tìm thấy trong file zip, 1 không có trong zip").expect("the images are counted");
    assert!(fixture.db().deals(None).expect("deals read").is_empty(), "nothing is stored before the button");

    app.press("role:button[Nhập 5 lệnh]").expect("the one import button");
    app.wait_for("Đã nhập 5 lệnh mới · 0 cập nhật · 0 không đổi").expect("the summary shows");
    app.wait_for("Ảnh gắn vào lệnh: 4 · không có trong file zip: 1").expect("the summary counts the images");

    let db = fixture.db();
    let account = db.accounts().expect("accounts").pop().expect("the Notion account");
    let deals: Vec<_> = db.deals(Some(account.id)).expect("deals").into_iter().map(|(_, d)| d).collect();
    assert_eq!(deals.len(), 10);
    assert!(deals.iter().all(|d| d.profit == 0.0), "no money was invented for trades kept in R");
    assert_eq!(deals.iter().filter_map(|d| d.result_r).sum::<f64>(), 1.4);
    let pictures = fixture.path().join("attachments").join("images");
    assert_eq!(std::fs::read_dir(&pictures).expect("the images folder").count(), 2, "one copy of each picture");
    let attached: usize =
        db.journals(Some(account.id)).expect("journals").iter().map(|(_, entry)| entry.screenshots.len()).sum();
    assert_eq!(attached, 4);

    // The pages show R, never "$0.00": the trades list has a dash for the money.
    app.press("role:button[Lệnh]").expect("the trades row");
    app.wait_for("Hiển thị 5 / 5 lệnh").expect("the trades are listed");
    assert!(!app.screen().shows("$0.00") && !app.screen().shows("+$0.00"), "no zero dollars on screen");
    shows_containing(&mut app, "the R of a trade", "+2.0R");
}

#[test]
fn importing_the_same_zip_twice_stores_nothing_new() {
    let fixture = Fixture::new();
    let mut app = start(&fixture);
    for expected in ["Đã nhập 5 lệnh mới · 0 cập nhật · 0 không đổi", "Đã nhập 0 lệnh mới · 0 cập nhật · 5 không đổi"]
    {
        app.on_pick_paths(vec![core_fixture("notion_r_journal.zip")]).expect("answers the dialog");
        app.press(DROPZONE).expect("the drop zone opens the dialog");
        shows_containing(&mut app, "the preview", "5 lệnh đọc được");
        app.press("role:button[Nhập 5 lệnh]").expect("the import button");
        app.wait_for(expected).expect("the summary shows");
    }
    assert_eq!(fixture.db().deals(None).expect("deals").len(), 10);
}

// --- the Notion API ------------------------------------------------------------------------------------------

/// A server that answers as Notion does, for one database of three trades.
struct NotionServer {
    base: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    seen: Arc<std::sync::Mutex<Vec<(String, String)>>>,
}

fn text(kind: &str, value: &str) -> Value {
    json!({"type": kind, kind: [{"plain_text": value}]})
}

fn page(title: &str, date: &str, pair: &str, side: &str, lot: f64, pnl: f64) -> Value {
    json!({
        "object": "page",
        "properties": {
            "Name": text("title", title),
            "Date": {"type": "date", "date": {"start": date}},
            "Pair": {"type": "select", "select": {"name": pair}},
            "Direction": {"type": "select", "select": {"name": side}},
            "Lot": {"type": "number", "number": lot},
            "P&L": {"type": "number", "number": pnl},
        },
    })
}

impl NotionServer {
    fn start() -> Self {
        let server = Server::http("127.0.0.1:0").expect("a free port");
        let base = format!("http://{}/v1", server.server_addr().to_ip().expect("an IP address"));
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let thread = {
            let (stop, seen) = (Arc::clone(&stop), Arc::clone(&seen));
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let Ok(Some(request)) = server.recv_timeout(Duration::from_millis(50)) else { continue };
                    let token = request
                        .headers()
                        .iter()
                        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case("authorization"))
                        .map_or_default(|h| h.value.as_str().to_owned());
                    let path = request.url().to_owned();
                    seen.lock().expect("lock poisoned").push((path.clone(), token.clone()));
                    let (status, body) = if token != "Bearer ntn_test_token" {
                        (401, json!({"object": "error", "status": 401, "message": "API token is invalid."}))
                    } else if path == "/v1/search" {
                        (
                            200,
                            json!({"results": [{"object": "database", "id": "db-1",
                                "title": [{"plain_text": "Nhật ký lệnh Notion"}], "properties": {}}],
                                "has_more": false, "next_cursor": null}),
                        )
                    } else if path == "/v1/databases/db-1/query" {
                        (
                            200,
                            json!({"results": [
                                page("EURUSD breakout", "2026-11-16T09:00:00.000+07:00", "EURUSD", "Long", 0.2, 150.0),
                                page("Gold fade", "2026-11-17T08:15:00.000+07:00", "XAUUSD", "Short", 0.1, -100.0),
                                page("Gold swing", "2026-11-18T10:00:00.000+07:00", "XAUUSD", "Long", 0.3, 1200.0),
                            ], "has_more": false, "next_cursor": null}),
                        )
                    } else {
                        (404, json!({"object": "error", "status": 404, "message": "unknown path"}))
                    };
                    let mut response = Response::from_data(body.to_string()).with_status_code(StatusCode(status));
                    response.add_header(Header::from_bytes("Content-Type", "application/json").expect("a header"));
                    request.respond(response).ok();
                }
            })
        };
        Self { base, stop, thread: Some(thread), seen }
    }

    fn paths(&self) -> Vec<String> {
        self.seen.lock().expect("lock poisoned").iter().map(|(path, _)| path.clone()).collect()
    }
}

impl Drop for NotionServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }
}

/// A fixture in the demo mode (the token stays in memory) whose Notion is `server`.
fn fixture_with_notion(server: &NotionServer, drive: &str) -> Fixture {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    db.set_setting("cloud.mode", "memory").expect("the demo is chosen");
    db.set_setting("cloud.demo_drive", drive).expect("the drive is named");
    db.set_setting("cloud.notion.base_url", &server.base).expect("the server is named");
    fixture
}

#[test]
fn a_notion_database_is_listed_picked_previewed_and_imported_like_a_file() {
    let server = NotionServer::start();
    let fixture = fixture_with_notion(&server, "notion một");
    let mut app = start(&fixture);
    app.wait_for("Kết nối Notion (API)").expect("the API section is next to the file drop zone");
    assert_eq!(app.screen().enabled("role:button[Lưu token & tải danh sách]"), Ok(false), "no token yet");

    app.type_text("label:Token tích hợp Notion", "ntn_test_token").expect("the token is pasted");
    app.press("role:button[Lưu token & tải danh sách]").expect("keeps the token and lists the databases");
    app.wait_for("label:Database Notion").expect("the databases are offered");
    app.wait_for("text:Nhật ký lệnh Notion").expect("the database is listed by its title");
    app.press("role:button[Xem trước database]").expect("reads the chosen database");
    shows_containing(&mut app, "the preview", "Nhật ký lệnh Notion (Notion API) · 3 dòng: 3 lệnh đọc được");
    app.wait_for("Cột tự nhận ra (từ nội dung, không cần ghép tay)").expect("the same preview as a file");
    shows_containing(&mut app, "the money total", "tổng lãi/lỗ trong file +$1,250.00");
    assert!(fixture.db().deals(None).expect("deals").is_empty(), "nothing is stored before the button");

    app.press("role:button[Nhập 3 lệnh]").expect("the same import button");
    app.wait_for("Đã nhập 3 lệnh mới · 0 cập nhật · 0 không đổi").expect("the same summary");
    let db: Db = fixture.db();
    let account = db.accounts().expect("accounts").pop().expect("the Notion account");
    assert_eq!(db.deals(Some(account.id)).expect("deals").len(), 6);

    // The token was kept for the next time (in memory in the demo, the credential store otherwise): the
    // section offers to load the list again, and "Xoá token" forgets it.
    app.wait_until("the kept token", WAIT, |s| s.enabled("role:button[Tải lại danh sách]") == Ok(true))
        .expect("a kept token can list again");
    app.press("role:button[Xoá token]").expect("forgets the token");
    app.wait_until("the forgotten token", WAIT, |s| s.enabled("role:button[Tải lại danh sách]") == Ok(false))
        .expect("without a token there is nothing to list");
    assert!(server.paths().iter().any(|p| p == "/v1/databases/db-1/query"), "{:?}", server.paths());
}

#[test]
fn a_token_notion_refuses_is_reported_in_vietnamese_and_nothing_crashes() {
    let server = NotionServer::start();
    let fixture = fixture_with_notion(&server, "notion hai");
    let mut app = start(&fixture);
    app.type_text("label:Token tích hợp Notion", "sai_token").expect("a wrong token is pasted");
    app.press("role:button[Lưu token & tải danh sách]").expect("tries it");
    shows_containing(&mut app, "the refusal", "401");
    assert!(!app.screen().shows("label:Database Notion"), "no database is offered");
    wait_until("the server having been asked", || !server.paths().is_empty());
}
