//! Accounts and discipline, the daily journal and the settings, headless on the real engine
//! over a database in a temporary data directory.

mod common;

use std::time::Duration;

use chrono::{Datelike, Local, NaiveDate};
use nsh_core::Checked;
use nsh_testing::{Headless, Screen};
use tradelens_core::{
    ChecklistItem, DayNote, Db, Deal, DealBatch, DealEntry, DealKind,
    catalog::{FieldDef, FieldKind, FieldScope, Requirement},
    model::AccountInfo,
};

mod support;
use support::{WAIT, wait_until};

const HOUR_MS: i64 = 3_600_000;

struct Fixture(support::Fixture);

impl std::ops::Deref for Fixture {
    type Target = support::Fixture;

    fn deref(&self) -> &support::Fixture {
        &self.0
    }
}

impl Fixture {
    fn new() -> Self {
        Self(support::Fixture::new())
    }

    fn start(&self) -> Headless {
        self.0.start(1280.0, 1100.0, |_| {})
    }
}

fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after 1970")
            .as_millis(),
    )
    .expect("milliseconds fit")
}

fn deal(ticket: i64, position: i64, at_ms: i64, kind: DealKind, entry: DealEntry, profit: f64) -> Deal {
    Deal {
        ticket,
        order: ticket,
        position_id: position,
        time_msc: at_ms,
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

/// One account whose server runs on UTC, so the times below are the machine's today, and one
/// trade that closed a moment ago with `profit`.
fn seed_today(db: &mut Db, profit: f64) -> i64 {
    let exit = now_ms();
    let batch = DealBatch {
        account: AccountInfo {
            login: "52012345".into(),
            server: "Demo".into(),
            currency: "USD".into(),
            balance: Some(10_000.0),
            ..AccountInfo::default()
        },
        deals: vec![
            deal(10, 1, exit - HOUR_MS / 2, DealKind::Buy, DealEntry::In, 0.0),
            deal(11, 1, exit, DealKind::Sell, DealEntry::Out, profit),
        ],
        positions: Vec::new(),
    };
    db.upsert_batch(&batch, "test").expect("the batch is stored");
    let mut account = db.accounts().expect("accounts read").remove(0);
    account.server_offset_min = Some(0);
    db.update_account(&account).expect("the server offset is stored");
    account.id
}

fn open_accounts(app: &mut Headless) {
    app.press("role:button[Tài khoản & kỷ luật]").expect("the accounts row is a button");
    app.wait_for("Quy tắc kỷ luật").expect("the page opens");
}

#[test]
fn saving_the_rules_updates_the_status_of_today() {
    let fixture = Fixture::new();
    let account = seed_today(&mut fixture.db(), -40.0);
    let mut app = fixture.start();
    open_accounts(&mut app);
    app.wait_for("Chưa bật quy tắc nào: bật ở khối Quy tắc kỷ luật.").expect("no rule is on yet");

    app.press("label:Lỗ tối đa trong ngày").expect("the loss rule has a switch");
    app.press("role:button[Lưu quy tắc]").expect("the save button works");
    let db = fixture.db();
    wait_until("the rules being saved", || db.rules(account).expect("rules read").max_daily_loss.is_some());
    assert_eq!(db.rules(account).expect("rules read").max_daily_loss, Some(300.0));

    app.wait_for("Lỗ trong ngày").expect("the status shows the rule that is on now");
    app.wait_for("\u{2212}$40.00 / $300.00").expect("today's loss against the limit");
    app.wait_for("Đã lưu quy tắc").expect("the page says so");
}

#[test]
fn a_rule_that_is_broken_is_listed_with_its_day() {
    let fixture = Fixture::new();
    let account = seed_today(&mut fixture.db(), -400.0);
    fixture
        .db()
        .save_rules(account, &tradelens_core::Rules { max_daily_loss: Some(300.0), ..Default::default() })
        .expect("rules are stored");
    let mut app = fixture.start();
    open_accounts(&mut app);
    app.wait_for("Lỗ trong ngày 400.00 vượt giới hạn 300.00").expect("the violation is listed");
    app.wait_for("vi phạm · tuân thủ 0.0% số ngày").expect("the day traded was broken");
    assert!(app.screen().shows(&Local::now().format("%d/%m").to_string()), "the day is shown");
}

#[test]
fn editing_the_checklist_keeps_the_ids_of_the_items() {
    let fixture = Fixture::new();
    seed_today(&mut fixture.db(), 10.0);
    let mut db = fixture.db();
    db.save_checklist(
        None,
        &[
            ChecklistItem { id: 0, text: "Cùng chiều trend H4".into() },
            ChecklistItem { id: 0, text: "Có vùng cản".into() },
        ],
    )
    .expect("the checklist is stored");
    let before = db.checklist(None).expect("reads");
    let mut app = fixture.start();
    open_accounts(&mut app);
    app.wait_for("Cùng chiều trend H4").expect("the items show");

    app.type_text("label:Checklist dùng chung: mục 1", " (đã sửa)").expect("renames");
    app.submit("label:Checklist dùng chung: mục 1").expect("Enter keeps the name");
    app.type_text("label:Checklist dùng chung: mục mới", "Risk 1.5%").expect("types a new item");
    app.press("role:button[Thêm vào Checklist dùng chung]").expect("adds it");

    wait_until("the checklist being saved", || db.checklist(None).expect("reads").len() == 3);
    let after = db.checklist(None).expect("reads");
    assert_eq!(after[0].id, before[0].id, "the renamed item keeps its id");
    assert_eq!(after[0].text, "Cùng chiều trend H4 (đã sửa)");
    assert_eq!(after[1], before[1], "the untouched item is the same");
    assert_eq!(after[2].text, "Risk 1.5%");
    assert!(after[2].id > before[1].id, "the new item has an id of its own");
}

#[test]
fn account_details_are_saved() {
    let fixture = Fixture::new();
    let account = seed_today(&mut fixture.db(), 5.0);
    let mut app = fixture.start();
    open_accounts(&mut app);
    app.wait_for("Thông tin tài khoản").expect("the details card shows");
    app.type_text("label:Tên hiển thị", "IC Markets").expect("types a name");
    app.press("role:button[Lưu thông tin]").expect("saves");
    let db = fixture.db();
    wait_until("the account being renamed", || {
        db.accounts().expect("reads").iter().any(|a| a.id == account && a.name == "IC Markets")
    });
    assert_eq!(db.accounts().expect("reads")[0].server_offset_min, Some(0), "the server offset is kept as it was");
    app.wait_for("Đã lưu thông tin").expect("the page says so");
}

fn open_journal(app: &mut Headless) {
    app.press("role:button[Nhật ký ngày]").expect("the journal row is a button");
    app.wait_for("Kế hoạch và tổng kết").expect("the page opens");
    // The form appears once the day is read; typing before would be overwritten by what was read.
    app.wait_for("label:Kế hoạch ngày").expect("the day is read and the form is there");
}

#[test]
fn a_daily_note_is_saved_and_comes_back_on_the_next_run() {
    let fixture = Fixture::new();
    seed_today(&mut fixture.db(), 25.0);
    let today = Local::now().date_naive();
    {
        let mut app = fixture.start();
        open_journal(&mut app);
        app.wait_for("Lệnh trong ngày").expect("the day's trades card shows");
        app.wait_for("+$25.00").expect("the day's result shows");
        app.type_text("label:Kế hoạch ngày", "Chờ tin 19:30, không vào lệnh trước đó").expect("types the plan");
        app.press("role:button[Lưu nhật ký]").expect("saves");
        let db = fixture.db();
        wait_until("the note being saved", || db.day_note(0, today).expect("reads").plan.starts_with("Chờ tin"));
        app.wait_until("the saved note", WAIT, |screen| screen.texts().iter().any(|t| t.starts_with("Đã lưu lúc")))
            .expect("the page says when");
    }
    let mut again = fixture.start();
    open_journal(&mut again);
    again
        .wait_until("the plan back in its box", WAIT, |screen| {
            screen.input_text("label:Kế hoạch ngày") == Ok(Some("Chờ tin 19:30, không vào lệnh trước đó"))
        })
        .expect("the plan is there again");
}

#[test]
fn the_form_of_the_day_shows_the_traders_own_properties() {
    let fixture = Fixture::new();
    seed_today(&mut fixture.db(), 5.0);
    fixture
        .db()
        .save_field_def(&FieldDef {
            id: 0,
            scope: FieldScope::Day,
            key: "sleep".into(),
            label: "Giấc ngủ".into(),
            kind: FieldKind::Text,
            options: Vec::new(),
            max: None,
            required: Requirement::Optional,
            analysed: false,
            position: 0,
            archived: false,
        })
        .expect("a day property is defined");
    let mut app = fixture.start();
    open_journal(&mut app);
    app.wait_for("Giấc ngủ").expect("the property is in the form");
    app.type_text("label:Giấc ngủ", "7 tiếng").expect("types a value");
    app.press("role:button[Lưu nhật ký]").expect("saves");
    let db = fixture.db();
    let today = Local::now().date_naive();
    wait_until("the value being saved", || !db.day_values(0, today).expect("reads").is_empty());
    assert_eq!(
        db.day_values(0, today).expect("reads").get("sleep"),
        Some(&tradelens_core::catalog::FieldValue::Text("7 tiếng".into()))
    );
}

#[test]
fn going_to_another_day_saves_the_edits_of_this_one() {
    let fixture = Fixture::new();
    seed_today(&mut fixture.db(), 5.0);
    let today = Local::now().date_naive();
    let mut app = fixture.start();
    open_journal(&mut app);
    app.type_text("label:Tổng kết cuối ngày", "Giữ kỷ luật").expect("types the review");
    app.press("role:button[Ngày trước]").expect("goes to yesterday");
    let db = fixture.db();
    wait_until("the review being saved on leaving", || db.day_note(0, today).expect("reads").review == "Giữ kỷ luật");
}

#[test]
fn settings_are_kept_in_the_database() {
    let fixture = Fixture::new();
    let mut app = fixture.start();
    app.press("role:button[Cài đặt]").expect("the settings row is a button");
    app.wait_for("Chế độ màu").expect("the page opens");

    common::toggle_on(&mut app, "role:button[Chế độ màu: Sáng]");
    common::toggle_on(&mut app, "role:button[Giờ hiển thị: Giờ server]");
    let db = fixture.db();
    wait_until("the settings being saved", || {
        db.setting("appearance").expect("reads").as_deref() == Some("light")
            && db.setting("time_display").expect("reads").as_deref() == Some("server")
    });
}

#[test]
fn opening_a_day_of_the_calendar_shows_that_days_journal() {
    let fixture = Fixture::new();
    // The 15th of last month, whatever today is.
    let first_of_month = Local::now().date_naive().with_day(1).expect("every month has a first");
    let day = first_of_month.pred_opt().expect("a day before").with_day(15).expect("every month has a 15th");
    let note = DayNote { plan: "Ghi chú của ngày cũ".into(), ..DayNote::default() };
    fixture.db().save_day_note(0, day, &note).expect("the note is stored");
    let mut app = fixture.start();

    app.press("role:button[Lịch P/L]").expect("the calendar row is a button");
    app.wait_for("Kết quả tháng").expect("the page opens");
    app.press("role:button[Tháng trước]").expect("goes to last month");
    let cell = format!("role:button[Xem ngày {}]", day.format("%d/%m"));
    app.wait_for(&cell).expect("last month is drawn");
    // The day card's title ("Thứ 5, 15/10") names the picked day; a press that lands while the
    // month is drawn again is lost, so the day is picked again until the card follows.
    // "." stands for the "/" between day and month, which would end the pattern.
    let title = format!("text:/, {}$/", day.format("%d.%m"));
    let picked = (0..5).any(|_| {
        app.press(&cell).expect("picks the day");
        app.wait_for_within(&title, Duration::from_secs(2)).is_ok()
    });
    assert!(picked, "the day card shows {day}");
    app.press("role:button[Mở nhật ký ngày]").expect("opens its journal");

    app.wait_for("Kế hoạch và tổng kết").expect("the day journal opens");
    app.wait_until("the day's note", WAIT, |screen| {
        screen.input_text("label:Kế hoạch ngày") == Ok(Some("Ghi chú của ngày cũ"))
    })
    .expect("the note of the day asked for is there, not today's");
    app.wait_for(&format!("text:/{}$/", day.format("%d.%m.%Y"))).expect("the header names the day");
}

#[test]
fn the_time_display_setting_changes_the_times_the_pages_show() {
    let fixture = Fixture::new();
    // A server three hours ahead of the machine, so that the two clocks never agree.
    let machine_offset = Local::now().offset().local_minus_utc() / 60;
    let closed_on_server =
        NaiveDate::from_ymd_opt(2026, 11, 20).and_then(|d| d.and_hms_opt(10, 0, 0)).expect("a date and time");
    let closed_ms = closed_on_server.and_utc().timestamp_millis();
    {
        let mut db = fixture.db();
        let batch = DealBatch {
            account: AccountInfo {
                login: "52012345".into(),
                server: "Demo".into(),
                currency: "USD".into(),
                balance: Some(10_000.0),
                server_offset_min: Some(machine_offset + 180),
                ..AccountInfo::default()
            },
            deals: vec![
                deal(10, 1, closed_ms - HOUR_MS, DealKind::Buy, DealEntry::In, 0.0),
                deal(11, 1, closed_ms, DealKind::Sell, DealEntry::Out, 25.0),
            ],
            positions: Vec::new(),
        };
        db.upsert_batch(&batch, "test").expect("the batch is stored");
    }
    let on_server = closed_on_server.format("%d/%m %H:%M").to_string();
    let on_machine = (closed_on_server - chrono::Duration::minutes(180)).format("%d/%m %H:%M").to_string();
    assert_ne!(on_server, on_machine);

    let mut app = fixture.start();
    app.press("role:button[Lệnh]").expect("the trades row is a button");
    app.wait_for(&on_machine).expect("the trade's time is on the trader's clock");

    app.press("role:button[Cài đặt]").expect("the settings row is a button");
    app.wait_for("Chế độ màu").expect("the page opens");
    common::toggle_on(&mut app, "role:button[Giờ hiển thị: Giờ server]");
    app.press("role:button[Lệnh]").expect("back to the trades");
    app.wait_for(&on_server).expect("the same trade now shows the time MT5 reports");
    app.wait_for_gone(&on_machine, WAIT).expect("and no longer the machine's");

    app.press("role:button[Cài đặt]").expect("the settings row is a button");
    // Until the stored choice is read the switch shows the default, the trader's clock, and
    // pressing an option already on changes nothing.
    let server_chosen = |screen: &Screen| {
        screen
            .accessibility_state("role:button[Giờ hiển thị: Giờ server]")
            .is_ok_and(|state| state.pressed == Some(Checked::True))
    };
    app.wait_until("the stored choice being shown", Duration::from_secs(5), server_chosen)
        .expect("the settings show the server's clock as chosen");
    app.press("role:button[Giờ hiển thị: Giờ của bạn]").expect("the trader's clock is chosen again");
    app.press("role:button[Lệnh]").expect("back to the trades");
    app.wait_for(&on_machine).expect("the trader's clock is back");
}
