//! Tin quan trọng, headless on the real engine. A feed shaped like Forex Factory's is served on this
//! machine and the setting `news.url` points the app at it, so nothing reaches the network.

use std::{sync::Arc, thread::JoinHandle};

use chrono::{DateTime, Datelike, Duration, FixedOffset, Local, Utc};
use nsh_testing::Headless;

mod support;
use support::{Fixture, WAIT};

/// The sidebar row of the page.
const ROW: &str = "role:button[Tin quan trọng]";

/// A host answering every request with `status` and `body`.
struct Feed {
    server: Arc<tiny_http::Server>,
    thread: Option<JoinHandle<()>>,
}

impl Feed {
    fn serve(status: u16, body: String) -> Self {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").expect("a local port"));
        let thread = {
            let server = Arc::clone(&server);
            std::thread::spawn(move || {
                while let Ok(request) = server.recv() {
                    drop(request.respond(tiny_http::Response::from_string(body.clone()).with_status_code(status)));
                }
            })
        };
        Self { server, thread: Some(thread) }
    }

    fn url(&self) -> String {
        format!("http://{}/ff_calendar_thisweek.json", self.server.server_addr())
    }
}

impl Drop for Feed {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(thread) = self.thread.take() {
            drop(thread.join());
        }
    }
}

/// The week the page shows, Monday 00:00 to the next Monday on this machine's clock.
fn local_week(now: DateTime<Local>) -> (DateTime<Local>, DateTime<Local>) {
    let monday = now.date_naive() - Duration::days(i64::from(now.weekday().num_days_from_monday()));
    let start = |day: chrono::NaiveDate| {
        day.and_hms_opt(0, 0, 0)
            .and_then(|midnight| midnight.and_local_timezone(Local).earliest())
            .expect("midnight exists on this machine's clock")
    };
    (start(monday), start(monday + Duration::days(7)))
}

/// One feed row at `at`, written in New York's winter offset as Forex Factory writes it.
fn row(title: &str, country: &str, at: DateTime<Local>, impact: &str) -> serde_json::Value {
    let new_york = FixedOffset::west_opt(5 * 3_600).expect("a valid offset");
    serde_json::json!({
        "title": title,
        "country": country,
        "date": at.with_timezone(&new_york).to_rfc3339(),
        "impact": impact,
        "forecast": "0.3%",
        "previous": "0.2%",
    })
}

/// A feed with a red news already out, a red one to come and a yellow one, all inside the week the
/// page shows whatever the day and the time zone.
fn this_week() -> String {
    let now = Utc::now().with_timezone(&Local);
    let (start, end) = local_week(now);
    let past = start + (now - start) / 2;
    let next = now + (end - now) / 2;
    serde_json::Value::Array(vec![
        row("Core PCE Price Index m/m", "USD", past, "High"),
        row("CPI y/y", "AUD", next, "High"),
        row("Flash Services PMI", "EUR", next, "Low"),
    ])
    .to_string()
}

/// Whether a text on screen holds `part`.
fn shows_part(app: &Headless, part: &str) -> bool {
    app.screen().texts().iter().any(|text| text.contains(part))
}

fn open(fixture: &Fixture, feed: &Feed) -> Headless {
    fixture.db().set_setting(nsh_app_tradelens::NEWS_URL_SETTING, &feed.url()).expect("the setting is stored");
    let mut app = fixture.start(1280.0, 900.0, |_| {});
    app.press(ROW).expect("the sidebar has the news row");
    app.wait_for("text:/^Tin đỏ trong lịch tuần của Forex Factory/").expect("the news page opens");
    app
}

#[test]
fn the_weeks_red_news_shows_with_the_next_one_counted_down() {
    let fixture = Fixture::new();
    let feed = Feed::serve(200, this_week());
    let mut app = open(&fixture, &feed);
    app.wait_for_within("CPI y/y", WAIT).expect("the coming red news shows");
    assert!(app.screen().shows("Core PCE Price Index m/m"), "the red news already out stays in the week");
    assert!(app.screen().shows("Sắp tới"), "the next red news has its card");
    assert!(shows_part(&app, "còn "), "with a countdown");
    assert!(shows_part(&app, "Cập nhật lúc"), "the page says when it fetched");
    assert!(!app.screen().shows("Flash Services PMI"), "only red news is kept");
}

#[test]
fn a_failed_fetch_says_so_and_keeps_the_last_week() {
    let fixture = Fixture::new();
    let good = Feed::serve(200, this_week());
    let mut app = open(&fixture, &good);
    app.wait_for_within("CPI y/y", WAIT).expect("the week is fetched");
    let refused = Feed::serve(403, "Request Denied".to_owned());
    fixture.db().set_setting(nsh_app_tradelens::NEWS_URL_SETTING, &refused.url()).expect("the setting is stored");
    app.press("role:button[Tải lại]").expect("the page can fetch again");
    app.wait_for_within("text:/^Không tải được lịch tin/", WAIT).expect("the failure is shown");
    assert!(shows_part(&app, "Đang hiện bản tải"), "the page says which fetch it shows");
    assert!(app.screen().shows("CPI y/y"), "the last week stays");
}
