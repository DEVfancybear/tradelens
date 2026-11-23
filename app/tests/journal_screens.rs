//! The advanced trade journal, the form editor, the entry-plan prompt and manual entry,
//! headless on the real engine over a database in a temporary data directory.

mod common;

use std::time::Duration;

use nsh_core::platform::Os;
use nsh_testing::{DialogAnswer, Headless};
use tradelens_core::{
    Db, Deal, DealBatch, DealEntry, DealKind, JournalEntry, TradeKey,
    attachments::{Attachment, Source},
    catalog::{FieldDef, FieldKind, FieldScope, FieldValue, Requirement},
    journal::EntryPlan,
    model::AccountInfo,
};

mod support;
use support::{WAIT, wait_until};

const DAY_MS: i64 = 86_400_000;

/// The shared fixture with this file's window.
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
        // A desktop that delivers keys to the focused view, and takes dropped files.
        self.0.start(1280.0, 1000.0, |config| config.environment.os = Os::Linux)
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

#[expect(clippy::too_many_arguments, reason = "a fixture that spells out one deal")]
fn deal(
    ticket: i64,
    position: i64,
    at_ms: i64,
    kind: DealKind,
    entry: DealEntry,
    price: f64,
    sl: f64,
    tp: f64,
) -> Deal {
    Deal {
        ticket,
        order: ticket,
        position_id: position,
        time_msc: at_ms,
        symbol: "XAUUSD".into(),
        kind,
        entry,
        volume: 0.5,
        price,
        commission: 0.0,
        swap: 0.0,
        fee: 0.0,
        profit: if entry == DealEntry::Out { 100.0 } else { 0.0 },
        magic: 0,
        comment: String::new(),
        sl,
        tp,
        risk: None,
        result_r: None,
        gaps: tradelens_core::Gaps::NONE,
        reason: None,
    }
}

fn batch(deals: Vec<Deal>) -> DealBatch {
    DealBatch {
        account: AccountInfo {
            login: "52012345".into(),
            server: "Demo".into(),
            currency: "USD".into(),
            balance: Some(10_000.0),
            ..AccountInfo::default()
        },
        deals,
        positions: Vec::new(),
    }
}

/// Two closed XAUUSD trades (positions 1 and 2), the entry at 2648.2 with a stop at 2640.
fn seed(db: &mut Db) {
    let exit = now_ms() - 2 * DAY_MS;
    let mut deals = Vec::new();
    for position in [1_i64, 2] {
        let at = exit + position * 60_000;
        deals.push(deal(position * 10, position, at - 3_600_000, DealKind::Buy, DealEntry::In, 2648.2, 2640.0, 2660.0));
        deals.push(deal(position * 10 + 1, position, at, DealKind::Sell, DealEntry::Out, 2656.0, 2640.0, 2660.0));
    }
    db.upsert_batch(&batch(deals), "test").expect("the batch is stored");
}

fn key_of(db: &Db, position: i64) -> TradeKey {
    let account = db.accounts().expect("accounts read").remove(0);
    TradeKey { account_id: account.id, position_id: position, segment: 0 }
}

fn open_trade(app: &mut Headless, symbol: &str) {
    app.press("role:button[Lệnh]").expect("the trades row is a button");
    app.wait_for(&format!("role:button[Mở nhật ký lệnh {symbol}]")).expect("the trades load");
    press_until(app, &format!("role:button[Mở nhật ký lệnh {symbol}] >> nth=0"), "Nhật ký lệnh");
}

/// Presses `press` until `done` shows: the editor draws its rows again after each stored change, and a
/// press that lands on a button that is being replaced is lost. `done` is the state the press leads
/// to, so a press that did land is never repeated.
fn press_until(app: &mut Headless, press: &str, done: &str) {
    for _ in 0..5 {
        if app.screen().matches(done) == Ok(true) {
            return;
        }
        app.press(press).expect("the button is there");
        if app.wait_for_within(done, Duration::from_secs(2)).is_ok() {
            return;
        }
    }
    panic!("`{press}` did not lead to `{done}`");
}

/// Presses "Viết kế hoạch" on the open trade until the plan prompt is up: a press that lands while
/// the page is being read again reaches a button that is about to be replaced. Pressing twice is
/// harmless, since a trade waits in the prompt's queue only once.
fn open_plan_prompt(app: &mut Headless) {
    for _ in 0..5 {
        app.press("role:button[Viết kế hoạch]").expect("the trade has no plan yet");
        if app.wait_for_within("Lưu kế hoạch", Duration::from_secs(2)).is_ok() {
            return;
        }
    }
    panic!("the plan prompt did not open");
}

fn focus_property(scope: FieldScope, kind: FieldKind) -> FieldDef {
    FieldDef {
        id: 0,
        scope,
        key: "focus".into(),
        label: "Khung vào lệnh".into(),
        kind,
        options: Vec::new(),
        max: None,
        required: Requirement::Optional,
        analysed: false,
        position: 0,
        archived: false,
    }
}

#[test]
fn a_custom_property_edited_on_a_trade_is_saved_with_the_journal() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    seed(&mut db);
    db.apply_field_def(&focus_property(FieldScope::Trade, FieldKind::Text)).expect("the property is added");
    let mut app = fixture.start();
    open_trade(&mut app, "XAUUSD");
    app.wait_for("label:Sửa thuộc tính Khung vào lệnh").expect("the trader's own property is in the form");

    app.type_text("label:Khung vào lệnh", "M15").expect("types in the property");
    app.press("role:button[Điểm thực thi: 3 sao]").expect("a built-in is edited next to it");
    app.press("role:button[Lưu]").expect("saves");
    wait_until("the journal being saved", || {
        let key = key_of(&db, 2);
        db.journal(&key).expect("reads").is_some_and(|e| e.custom.contains_key("focus"))
    });
    let entry = db.journal(&key_of(&db, 2)).expect("reads").expect("an entry");
    assert_eq!(entry.custom["focus"], FieldValue::Text("M15".into()));
    assert_eq!(entry.rating, 3);
}

#[test]
fn the_exit_deals_reason_prefills_the_exit_reason_without_creating_an_entry() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    let exit = now_ms() - DAY_MS;
    let mut closing = deal(11, 1, exit, DealKind::Sell, DealEntry::Out, 2640.0, 2640.0, 2660.0);
    closing.reason = Some("DEAL_REASON_SL".into());
    let opening = deal(10, 1, exit - 3_600_000, DealKind::Buy, DealEntry::In, 2648.2, 2640.0, 2660.0);
    db.upsert_batch(&batch(vec![opening, closing]), "test").expect("stored");
    let mut app = fixture.start();
    open_trade(&mut app, "XAUUSD");
    app.wait_for("Gợi ý từ deal đóng: Chạm SL").expect("the deal's reason is suggested");
    app.wait_for("Đã lưu").expect("nothing is pending");
    assert!(db.journal(&key_of(&db, 1)).expect("reads").is_none(), "opening a trade writes no entry");
}

#[test]
fn the_plan_is_set_against_what_the_trade_did() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    seed(&mut db);
    let planned = JournalEntry {
        plan: Some(EntryPlan {
            thesis: "Phá đỉnh phiên Á, retest giữ được".into(),
            entry: Some(2648.0),
            stop_loss: Some(2640.0),
            take_profit: Some(2660.0),
            timeframe: "M15".into(),
            written_at: 1,
            ..EntryPlan::default()
        }),
        ..JournalEntry::default()
    };
    db.save_journal(&key_of(&db, 2), &planned).expect("the plan is stored");
    let mut app = fixture.start();
    open_trade(&mut app, "XAUUSD");
    app.wait_for("Kế hoạch lúc vào vs thực tế").expect("the comparison shows");
    app.wait_for("Phá đỉnh phiên Á, retest giữ được").expect("the thesis shows");
    app.wait_for("+0.20 (trượt giá)").expect("the entry slippage is named");
    app.wait_for("Viết trước khi đóng").expect("the plan was written before the close");
    app.wait_for("đúng kế hoạch").expect("the stop was kept");

    // Another trade has no plan: it offers to write one.
    app.press("role:button[‹ Lệnh trước]").expect("steps to the other trade");
    app.wait_for("Chưa có kế hoạch cho lệnh này").expect("the empty state says so");
    app.press("role:button[Viết kế hoạch]").expect("the plan form opens over the page");
    app.wait_for("Lưu kế hoạch").expect("the plan dialog shows");
}

#[test]
fn a_picture_added_to_a_trade_is_saved_into_its_journal() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    seed(&mut db);
    let source = fixture.path().join("h1.png");
    std::fs::write(&source, b"png one").expect("writes the source");
    let uri = url::Url::from_file_path(&source).map(String::from).expect("absolute");
    let mut app = fixture.start();
    open_trade(&mut app, "XAUUSD");
    app.wait_for("label:Thêm ảnh vào Ảnh chart").expect("the gallery shows");
    app.drop_files("label:Thêm ảnh vào Ảnh chart", vec![uri]).expect("drops a picture");
    wait_until("the picture being saved", || {
        db.journal(&key_of(&db, 2)).expect("reads").is_some_and(|e| e.screenshots.len() == 1)
    });
    let entry = db.journal(&key_of(&db, 2)).expect("reads").expect("an entry");
    assert!(entry.screenshots[0].path.starts_with("images/"), "{}", entry.screenshots[0].path);
}

#[test]
fn a_trades_pictures_open_full_size_and_a_removal_can_be_undone() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    seed(&mut db);
    let shot = |path: &str, caption: &str| Attachment {
        path: path.to_owned(),
        caption: caption.to_owned(),
        timeframe: Some("h4".to_owned()),
        source: Source::Upload,
        added_at: 0,
    };
    let pictures = vec![shot("images/one.png", "bối cảnh"), shot("images/two.png", "retest")];
    let entry = JournalEntry { screenshots: pictures.clone(), ..JournalEntry::default() };
    db.save_journal(&key_of(&db, 2), &entry).expect("the pictures are stored");
    let mut app = fixture.start();
    open_trade(&mut app, "XAUUSD");

    // A picture opens over the whole window (the sidebar is under the scrim), and Escape closes it.
    app.wait_for("label:Phóng to Ảnh chart 2").expect("the gallery shows both pictures");
    app.press("label:Phóng to Ảnh chart 2").expect("the second picture");
    app.wait_for("2 / 2").expect("the viewer opens on it");
    app.press_key("left").expect("previous");
    app.wait_for("1 / 2").expect("the first picture");
    app.press_key("escape").expect("Escape");
    app.wait_for_gone("1 / 2", WAIT).expect("the viewer closes");

    // The removal is saved at once, and so is its undo, in the same place.
    press_until(&mut app, "label:Xoá Ảnh chart 1", "Đã xoá ảnh ·");
    wait_until("the removal being saved", || {
        db.journal(&key_of(&db, 2)).expect("reads").is_some_and(|e| e.screenshots.len() == 1)
    });
    app.wait_for("Đã xoá ảnh ·").expect("the undo line shows");
    press_until(&mut app, "role:button[Hoàn tác]", "label:Xoá Ảnh chart 2");
    wait_until("the undo being saved", || {
        db.journal(&key_of(&db, 2)).expect("reads").is_some_and(|e| e.screenshots == pictures)
    });
}

#[test]
fn changing_a_propertys_type_in_the_form_editor_converts_every_entry() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    seed(&mut db);
    db.apply_field_def(&focus_property(FieldScope::Trade, FieldKind::Text)).expect("the property is added");
    for (position, text) in [(1_i64, "7"), (2, "cao")] {
        let mut entry = JournalEntry::default();
        entry.custom.insert("focus".into(), FieldValue::Text(text.into()));
        let key = key_of(&db, position);
        db.save_journal(&key, &entry).expect("stored");
    }
    let mut app = fixture.start();
    open_trade(&mut app, "XAUUSD");
    app.press("role:button[Tuỳ biến form]").expect("the link to the form editor");
    app.wait_for("Tuỳ biến form nhập liệu").expect("the editor opens over the page");
    app.wait_for("label:Sửa thuộc tính Khung vào lệnh").expect("the property is in the table");

    app.on_dialog(|_| DialogAnswer::Press("Đổi kiểu".to_owned()));
    app.press("label:Sửa thuộc tính Khung vào lệnh").expect("picks the property");
    app.press("label:Kiểu").expect("the kind drop-down");
    app.press("text:# Số").expect("the number kind");
    wait_until("the stored values following the new type", || {
        let value =
            |position| db.journal(&key_of(&db, position)).expect("reads").and_then(|e| e.custom.get("focus").cloned());
        value(1) == Some(FieldValue::Number(7.0)) && value(2).is_none()
    });
    assert_eq!(db.field_defs(FieldScope::Trade).expect("reads")[0].kind, FieldKind::Number);

    // Hiding a property is saved with the form's arrangement; closing the editor shows the trade again.
    press_until(&mut app, "label:Ẩn Khung vào lệnh", "label:Hiện Khung vào lệnh");
    wait_until("the layout being saved", || {
        db.form_layout(FieldScope::Trade).expect("reads").items.iter().any(|i| i.hidden)
    });
    app.press("Xong").expect("closes the editor");
    app.wait_for("Chi tiết lệnh").expect("back on the trade");
}

#[test]
fn a_trade_typed_in_by_hand_is_stored_with_its_journal() {
    let fixture = Fixture::new();
    seed(&mut fixture.db());
    let mut app = fixture.start();
    app.press("role:button[Lệnh]").expect("the trades row is a button");
    app.wait_for("Hiển thị 2 / 2 lệnh").expect("the trades load");
    app.press("role:button[Thêm lệnh]").expect("the add button");
    app.wait_for("Nhập lệnh & nhật ký thủ công").expect("the form opens");

    app.press("role:button[Lưu lệnh]").expect("saves an empty form");
    app.wait_for("text:/Nhập mã giao dịch/").expect("the form says what is missing");

    app.type_text("label:Symbol", "GBPUSD").expect("symbol");
    common::toggle_on(&mut app, "role:button[Chiều: Sell]");
    // The times are chosen from the calendar: the 3rd and the 4th of this month, both at 00:00.
    let this_month = |day: u32| {
        let today = chrono::Local::now().date_naive();
        chrono::NaiveDate::from_ymd_opt(chrono::Datelike::year(&today), chrono::Datelike::month(&today), day)
            .expect("a day of the month")
            .format("%Y-%m-%d")
            .to_string()
    };
    app.press("label:Thời gian vào").expect("the opening time opens the calendar");
    app.press(&format!("role:button[{}]", this_month(3))).expect("opened");
    app.press("label:Thời gian ra").expect("the closing time opens the calendar");
    app.press(&format!("role:button[{}]", this_month(4))).expect("closed");
    app.type_text("label:Khối lượng", "0.8").expect("volume");
    app.type_text("label:Giá vào", "1.2684").expect("open price");
    app.type_text("label:Giá ra", "1.2651").expect("close price");
    app.type_text("label:Lãi/lỗ", "264").expect("profit");
    app.type_text("label:Commission", "-5.6").expect("commission");
    app.wait_for("text:/^Tự tính: kết quả ròng [+]\\$258[.]40 · giữ /").expect("the computed line follows");
    common::toggle_on(&mut app, "role:button[Tag: Tin tức]");
    app.type_text("label:Ghi chú", "Vào sớm vì sợ lỡ").expect("notes");
    app.press("role:button[Lưu lệnh]").expect("saves");

    let db = fixture.db();
    wait_until("the trade being stored", || db.deals(None).expect("reads").iter().any(|(_, d)| d.symbol == "GBPUSD"));
    let deals: Vec<Deal> =
        db.deals(None).expect("reads").into_iter().map(|(_, d)| d).filter(|d| d.symbol == "GBPUSD").collect();
    assert_eq!(deals.len(), 2, "an entry and an exit deal");
    assert!(deals.iter().all(|d| d.ticket < 0), "hand-entered deals have negative tickets");
    let key =
        TradeKey { account_id: db.accounts().expect("reads")[0].id, position_id: deals[0].position_id, segment: 0 };
    wait_until("the journal being stored", || db.journal(&key).expect("reads").is_some());
    let entry = db.journal(&key).expect("reads").expect("an entry");
    assert_eq!(entry.notes, "Vào sớm vì sợ lỡ");
    assert_eq!(entry.tags, ["news"]);
    app.wait_for("Chi tiết lệnh").expect("the new trade opens");
}

#[test]
fn ctrl_n_opens_the_manual_entry_form_from_the_trades_list() {
    let fixture = Fixture::new();
    let mut app = fixture.start();
    app.press("role:button[Lệnh]").expect("the trades row is a button");
    app.wait_for("Chưa có lệnh nào").expect("the trades page opens");
    app.press_key("ctrl+n").expect("the shortcut");
    app.wait_for("Nhập lệnh & nhật ký thủ công").expect("the form opens");
    app.press("role:button[Huỷ]").expect("cancels");
    app.wait_for("Chưa có lệnh nào").expect("back on the list");
}

#[test]
fn the_plan_prompt_warns_when_the_planned_risk_breaks_the_accounts_rule() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    seed(&mut db);
    let account = db.accounts().expect("accounts read").remove(0).id;
    db.save_rules(account, &tradelens_core::Rules { max_risk_pct: Some(1.5), ..tradelens_core::Rules::default() })
        .expect("the rules are stored");
    drop(db);
    let mut app = fixture.start();
    open_trade(&mut app, "XAUUSD");
    open_plan_prompt(&mut app);
    app.wait_for_gone("text:/vượt quy tắc/", Duration::from_secs(1)).expect("no risk is planned yet, so no warning");

    // 300 of the balance before this trade (a little under 10,000) is about 3%, over the 1.5% the
    // account allows.
    app.type_text("label:Risk", "300").expect("the risk is typed");
    app.wait_for(r"text:/^Risk 3\.\d% vượt quy tắc 1\.5% của tài khoản 52012345 · Demo\.$/")
        .expect("the rule is warned about");

    // Within the rule the warning goes.
    app.clear("label:Risk").expect("the risk is cleared");
    app.type_text("label:Risk", "100").expect("the risk is retyped");
    app.wait_for_gone("text:/vượt quy tắc/", WAIT).expect("the warning goes with the risk");
}

#[test]
fn the_plan_prompt_saves_the_plan_and_closing_it_writes_nothing() {
    let fixture = Fixture::new();
    seed(&mut fixture.db());
    let mut app = fixture.start();
    open_trade(&mut app, "XAUUSD");
    let db = fixture.db();
    let journals = |db: &Db| -> Vec<JournalEntry> {
        [1, 2].into_iter().filter_map(|p| db.journal(&key_of(db, p)).expect("reads")).collect()
    };
    let before = journals(&db);

    // There is no "Ghi sau": closing writes no plan and flags nothing for review.
    open_plan_prompt(&mut app);
    assert!(!app.screen().matches("role:button[Ghi sau]").expect("a selector"), "no postponing button");
    app.press("role:button[Đóng]").expect("closes the prompt");
    app.wait_for_gone("Lưu kế hoạch", WAIT).expect("the dialog closes");
    app.wait(Duration::from_millis(400)).expect("the app runs");
    assert_eq!(journals(&db), before, "closing wrote nothing");

    // Written, the plan starts from the trade's own levels.
    open_plan_prompt(&mut app);
    app.type_text("label:Lý do vào lệnh", "Phá đỉnh phiên Á").expect("the reason is typed");
    app.press("Lưu kế hoạch").expect("saves the plan");
    wait_until("the plan being saved", || journals(&db).iter().any(|e| e.plan.is_some()));
    let entry = journals(&db).into_iter().find(|e| e.plan.is_some()).expect("an entry with a plan");
    let plan = entry.plan.expect("a plan");
    assert_eq!(plan.thesis, "Phá đỉnh phiên Á");
    assert_eq!(plan.entry, Some(2648.2), "the levels start from the trade's");
    assert_eq!(plan.stop_loss, Some(2640.0));
    assert!(!entry.review_later, "saving a plan does not flag the trade either");
}

/// A point of the scrim that the plan's panel does not cover.
fn scrim_corner() -> nsh_core::Point<nsh_core::Points> {
    nsh_core::Point { x: nsh_core::Points(4.0), y: nsh_core::Points(4.0) }
}

#[test]
fn the_plan_prompt_shows_its_form_and_escape_or_the_close_button_close_it_writing_nothing() {
    let fixture = Fixture::new();
    seed(&mut fixture.db());
    let mut app = fixture.start();
    open_trade(&mut app, "XAUUSD");
    let db = fixture.db();
    let no_plan = |db: &Db| {
        [1, 2].into_iter().all(|p| db.journal(&key_of(db, p)).expect("reads").is_none_or(|e| e.plan.is_none()))
    };
    let no_flag = |db: &Db| {
        [1, 2].into_iter().all(|p| db.journal(&key_of(db, p)).expect("reads").is_none_or(|e| !e.review_later))
    };

    open_plan_prompt(&mut app);
    for field in ["label:Lý do vào lệnh", "label:Giá vào", "label:Stop loss", "label:Take profit", "label:Risk"] {
        app.wait_for(field).unwrap_or_else(|error| panic!("the form has {field}: {error}"));
    }
    app.type_text("label:Lý do vào lệnh", "chưa lưu").expect("the reason is typed");
    // A press outside the panel must not throw the typing away.
    app.press_at("label:Đóng hộp thoại", scrim_corner()).expect("a press on the scrim");
    app.wait(Duration::from_millis(400)).expect("the app runs");
    assert!(app.screen().shows("Lưu kế hoạch"), "the scrim leaves a form with edits open");

    // Escape closes it from inside the text area, where the typing is, and writes nothing: no plan,
    // and no flag for review either.
    app.press_key("escape").expect("Escape");
    app.wait_for_gone("Lưu kế hoạch", WAIT).expect("the dialog closes");
    app.wait(Duration::from_millis(400)).expect("the app runs");
    assert!(no_plan(&db), "Escape wrote no plan");
    assert!(no_flag(&db), "and flagged nothing");

    // The close button says the same.
    open_plan_prompt(&mut app);
    app.press("role:button[Đóng]").expect("the close button");
    app.wait_for_gone("Lưu kế hoạch", WAIT).expect("the dialog closes again");
    app.wait(Duration::from_millis(400)).expect("the app runs");
    assert!(no_plan(&db) && no_flag(&db), "the close button wrote nothing");
}
