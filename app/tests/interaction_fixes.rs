//! Fixes found by hand on the real window, each pinned headless: a question asked from a card that is
//! built again, the pointing hand over what can be pressed, the columns of the trades table under
//! their titles, and the date picker.

use std::sync::{Arc, Mutex};

use chrono::NaiveDate;
use nsh_core::{Cursor, MountItem, NodeId};
use nsh_testing::{DialogAnswer, Headless};
use tradelens_core::{Db, Deal, DealBatch, DealEntry, DealKind, model::AccountInfo};

mod support;
use support::{Fixture, WAIT, wait_until};

const EMAIL: &str = "demo@tradelens.local";

/// A machine signed in to the demo drive.
fn signed_in() -> Fixture {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    db.set_setting("cloud.mode", "memory").expect("the demo is chosen");
    db.set_setting("cloud.demo_drive", "câu hỏi").expect("the drive is named");
    db.set_setting("cloud.demo_user", "google").expect("signed in");
    db.set_setting("cloud.provider", "google").expect("the provider is chosen");
    fixture
}

#[test]
fn a_question_asked_by_a_card_that_is_built_again_before_the_answer_still_acts_on_it() {
    let fixture = signed_in();
    let mut app = fixture.start(1280.0, 1100.0, |_| {});
    app.press("role:button[Đăng nhập & sync]").expect("the cloud row");
    app.wait_for(EMAIL).expect("signed in");
    app.answer_dialogs_myself();
    app.press("role:button[Đăng xuất]").expect("the sign-out button");
    assert!(app.pending_dialog().is_some(), "the question is shown");

    // The state changes while the question is open: the card with the button is built again.
    app.press("label:Tự động sync").expect("the switch");
    let db = fixture.db();
    wait_until("the switch being kept", || db.setting("cloud.auto_sync").is_ok_and(|v| v.as_deref() == Some("1")));
    assert!(app.pending_dialog().is_some(), "the question is still open after the card was rebuilt");

    app.answer_dialog(DialogAnswer::Press("Đăng xuất".to_owned())).expect("answers");
    app.wait_for_gone(EMAIL, WAIT).expect("the answer signed out, though the card that asked is gone");
    wait_until("the session being forgotten", || db.setting("cloud.demo_user").is_ok_and(|v| v.as_deref() == Some("")));
}

/// The cursor of every view that got a style, by node.
type Cursors = Arc<Mutex<std::collections::HashMap<NodeId, Cursor>>>;

fn watch_cursors(app: &mut Headless) -> Cursors {
    let seen: Cursors = Arc::default();
    let log = Arc::clone(&seen);
    app.observe(move |_, batch| {
        for item in &batch.items {
            if let MountItem::ViewStyle { id, style } = item {
                log.lock().expect("lock").insert(*id, style.cursor);
            }
        }
    });
    seen
}

#[test]
fn what_can_be_pressed_shows_the_pointing_hand_and_a_disabled_button_does_not() {
    let fixture = Fixture::new();
    let mut app = fixture.start(1280.0, 800.0, |_| {});
    let cursors = watch_cursors(&mut app);
    app.press("role:button[Đăng nhập & sync]").expect("the cloud row");
    app.wait_for("Sync ngay").expect("the page");
    app.press("role:button[Tài khoản & kỷ luật]").expect("the accounts row");
    app.press("role:button[Đăng nhập & sync]").expect("back");
    app.wait_for("Sync ngay").expect("the page again");
    let cursor_of = |selector: &str| {
        let id = app.screen().id(selector).unwrap_or_else(|e| panic!("{selector}: {e}"));
        cursors.lock().expect("lock").get(&id).copied()
    };
    assert_eq!(cursor_of("role:button[Đăng nhập & sync]"), Some(Cursor::Pointer), "a sidebar row");
    assert_eq!(cursor_of("role:button[Đăng nhập với Google]"), Some(Cursor::Pointer), "a button that can be pressed");
    assert_eq!(cursor_of("role:button[Sync ngay]"), Some(Cursor::Default), "a disabled button keeps the arrow");
    let _ = cursor_of;
}

fn deal(ticket: i64, position: i64, entry: DealEntry, kind: DealKind, at: i64, profit: f64) -> Deal {
    Deal {
        ticket,
        order: ticket,
        position_id: position,
        time_msc: at,
        symbol: "EURUSD".into(),
        kind,
        entry,
        volume: 0.5,
        price: 1.1,
        commission: 0.0,
        swap: 0.0,
        fee: 0.0,
        profit,
        magic: 0,
        comment: String::new(),
        sl: 0.0,
        tp: 0.0,
        risk: None,
        result_r: None,
        gaps: tradelens_core::Gaps::NONE,
        reason: None,
    }
}

fn seed_recent(db: &mut Db) {
    let now = chrono::Utc::now().timestamp_millis();
    let mut deals = Vec::new();
    for n in 0..3_i64 {
        let at = now - (n + 1) * 86_400_000;
        deals.push(deal(10 + 2 * n, 100 + n, DealEntry::In, DealKind::Buy, at - 3_600_000, 0.0));
        deals.push(deal(
            11 + 2 * n,
            100 + n,
            DealEntry::Out,
            DealKind::Sell,
            at,
            20.0 + f64::from(i32::try_from(n).expect("small")),
        ));
    }
    let batch = DealBatch {
        account: AccountInfo {
            login: "1".into(),
            server: "Demo".into(),
            currency: "USD".into(),
            ..AccountInfo::default()
        },
        deals,
        positions: Vec::new(),
    };
    db.upsert_batch(&batch, "test").expect("stored");
}

/// The box a text view with exactly `text` was laid out in: the `n`th, from the top.
fn nth_text_box(app: &Headless, text: &str, n: usize) -> nsh_testing::LayoutNode {
    let mut found: Vec<_> = app
        .screen()
        .layout_nodes()
        .into_iter()
        .filter(|node| node.text.as_ref().is_some_and(|t| t.text.text == text))
        .collect();
    found.sort_by(|a, b| a.bounds.origin.y.0.total_cmp(&b.bounds.origin.y.0));
    found.into_iter().nth(n).unwrap_or_else(|| panic!("no {n}th text {text:?}"))
}

#[test]
fn the_figures_of_the_recent_trades_stand_under_their_titles_and_the_rows_reach_the_card_edge() {
    let fixture = Fixture::new();
    seed_recent(&mut fixture.db());
    let mut app = fixture.start(1400.0, 1000.0, |_| {});
    app.press("role:button[Khoảng thời gian: 7N]").expect("the range");
    app.wait_for("EURUSD").expect("the trades");
    for title in ["Lot", "Net"] {
        let header = nth_text_box(&app, title, 0);
        // The first figure of the column: the cell right under its title.
        let figure_text = match title {
            "Lot" => "0.50",
            "Net" => "+$20.00",
            _ => unreachable!("only the two titles above"),
        };
        let cell = nth_text_box(&app, figure_text, 0);
        let (header_end, cell_end) = (header.bounds.right().0, cell.bounds.right().0);
        assert!(
            (header_end - cell_end).abs() <= 1.0,
            "{title}: the title ends at {header_end} and its figure at {cell_end}"
        );
    }
    // The last column's title is where the rows end: no empty band on the right of the table.
    let setup = nth_text_box(&app, "Setup", 0);
    let table_right = setup.bounds.origin.x.0 + setup.bounds.size.width.0;
    assert!(table_right > 600.0, "the setup column takes the room that is left: {table_right}");
}

/// The title the journal gives a day: `Thứ 2, 16/11/2026`.
fn journal_title(day: NaiveDate) -> String {
    format!("{}, {}", tradelens_core::time::weekday_vi(chrono::Datelike::weekday(&day)), day.format("%d/%m/%Y"))
}

/// The 15th of the month it is, which the calendar of any day of the month shows once.
fn fifteenth() -> NaiveDate {
    let today = chrono::Local::now().date_naive();
    NaiveDate::from_ymd_opt(chrono::Datelike::year(&today), chrono::Datelike::month(&today), 15).expect("a date")
}

#[test]
fn the_day_of_the_journal_is_picked_from_a_calendar_and_the_page_follows() {
    let fixture = Fixture::new();
    seed_recent(&mut fixture.db());
    let mut app = fixture.start(1400.0, 1000.0, |_| {});
    app.press("role:button[Nhật ký ngày]").expect("the journal row");
    app.wait_for("Hôm nay").expect("the page");
    app.press("label:Chọn ngày của nhật ký").expect("the date opens the calendar");
    app.wait_for("T2").expect("Monday comes first, named in Vietnamese");
    app.wait_for("CN").expect("and Sunday last");
    app.wait_for("text:/^Tháng \\d+$/").expect("the month has its name");
    let day = fifteenth();
    app.press(&format!("role:button[{}]", day.format("%Y-%m-%d"))).expect("a day of the calendar");
    app.wait_for(&journal_title(day)).expect("the page follows the day that was picked");
    // Whether the calendar closes on a pick is the platform's; on macOS it stays open, and its own
    // "Hôm nay" preset would then match too. Escape closes it either way.
    app.press_key("Escape").expect("Escape closes the calendar");
    app.wait_until("one today button", WAIT, |screen| screen.count("role:button[Hôm nay]") == Ok(1))
        .expect("only the page's today button is left");
    app.press("role:button[Hôm nay]").expect("the today button");
    app.wait_for(&journal_title(chrono::Local::now().date_naive())).expect("back to today");
}

#[test]
fn a_date_property_is_set_from_the_calendar() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    seed_recent(&mut db);
    let account = db.accounts().expect("accounts").remove(0);
    let key = tradelens_core::TradeKey { account_id: account.id, position_id: 100, segment: 0 };
    let field = tradelens_core::catalog::FieldDef {
        id: 0,
        scope: tradelens_core::catalog::FieldScope::Trade,
        key: "reviewed_on".into(),
        label: "Ngày xem lại".into(),
        kind: tradelens_core::catalog::FieldKind::Date,
        options: Vec::new(),
        max: None,
        required: tradelens_core::catalog::Requirement::Optional,
        analysed: false,
        position: 0,
        archived: false,
    };
    db.save_field_def(&field).expect("the property is defined");
    drop(db);
    let mut app = fixture.start(1400.0, 1000.0, |_| {});
    app.press("role:button[Khoảng thời gian: 7N]").expect("the range");
    app.press("role:button[Lệnh]").expect("the trades row");
    app.press("role:button[Mở nhật ký lệnh EURUSD] >> nth=0").expect("a trade opens");
    app.wait_for("Ngày xem lại").expect("the property is on the form");
    app.wait_for("Chọn ngày").expect("it has no date yet, and says to choose one");
    app.press("label:Ngày xem lại").expect("the field opens the calendar");
    app.wait_for("T2").expect("the calendar");
    let day = fifteenth();
    app.press(&format!("role:button[{}]", day.format("%Y-%m-%d"))).expect("a day");
    app.wait_for(&day.format("%d/%m/%Y").to_string()).expect("the field shows the day");
    app.press("role:button[Lưu]").expect("saves");
    let reader = fixture.db();
    wait_until("the date being stored", || {
        reader.journal(&key).is_ok_and(|j| {
            j.is_some_and(|j| {
                j.custom.get("reviewed_on")
                    == Some(&tradelens_core::catalog::FieldValue::Date(day.format("%Y-%m-%d").to_string()))
            })
        })
    });
}
