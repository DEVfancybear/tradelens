//! The playbook, the missed trades, the reviews, the search and the importers, headless on the
//! real engine over a database in a temporary data directory.

mod common;

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::Local;
use nsh_app_tradelens::ui::properties::kind_name;
use nsh_testing::Headless;
use tradelens_core::{
    DayNote, Db, Deal, DealBatch, DealEntry, DealKind, JournalEntry, Side, TradeKey, catalog::FieldScope,
    import::notion, missed::MissedTrade, model::AccountInfo, playbook::Playbook, review::Period,
};

mod support;
use support::{WAIT, wait_until};

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
        self.0.start(1400.0, 1000.0, |_| {})
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
    symbol: &str,
    at_ms: i64,
    kind: DealKind,
    entry: DealEntry,
    price: f64,
    profit: f64,
) -> Deal {
    Deal {
        ticket,
        order: ticket,
        position_id: position,
        time_msc: at_ms,
        symbol: symbol.into(),
        kind,
        entry,
        volume: 0.5,
        price,
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

/// A closed trade that ended just now, an hour after it began.
fn closed_now(position: i64, symbol: &str, profit: f64) -> Vec<Deal> {
    let exit = now_ms();
    let entry = exit - 3_600_000;
    vec![
        deal(position * 10, position, symbol, entry, DealKind::Buy, DealEntry::In, 1.1, 0.0),
        deal(position * 10 + 1, position, symbol, exit, DealKind::Sell, DealEntry::Out, 1.1, profit),
    ]
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

/// Three trades that closed just now: EURUSD +100, XAUUSD −40, GBPJPY +20 (net +80).
fn seed(db: &mut Db) {
    let mut deals = closed_now(1, "EURUSD", 100.0);
    deals.extend(closed_now(2, "XAUUSD", -40.0));
    deals.extend(closed_now(3, "GBPJPY", 20.0));
    db.upsert_batch(&batch(deals), "test").expect("the batch is stored");
}

fn key_of(db: &Db, position: i64) -> TradeKey {
    let account = db.accounts().expect("accounts read").remove(0);
    TradeKey { account_id: account.id, position_id: position, segment: 0 }
}

fn open_page(app: &mut Headless, row: &str, marker: &str) {
    app.press(&format!("role:button[{row}]")).expect("the sidebar row is a button");
    app.wait_for(marker).expect("the page opens");
}

fn fixture_file(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/core/tests/fixtures").join(name)
}

fn shows_text_containing(app: &mut Headless, what: &str, needle: &str) {
    app.wait_until(what, WAIT, |screen| screen.texts().iter().any(|t| t.contains(needle)))
        .unwrap_or_else(|error| panic!("{what}: {error}"));
}

// --- Playbook -------------------------------------------------------------------------------------

#[test]
fn a_playbook_lists_the_statistics_of_the_trades_linked_to_it() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    seed(&mut db);
    let setup = Playbook {
        name: "Breakout phiên Á".into(),
        entry_rules: vec!["Retest giữ được".into()],
        checklist: vec!["Cùng chiều trend H4".into()],
        ..Playbook::default()
    };
    let id = db.save_playbook(&setup, 0).expect("the setup is stored");
    for position in [1, 2] {
        let entry = JournalEntry { playbook_id: Some(id), ..JournalEntry::default() };
        db.save_journal(&key_of(&db, position), &entry).expect("the journal is stored");
    }
    drop(db);

    let mut app = fixture.start();
    open_page(&mut app, "Playbook", "Playbook của bạn");
    app.wait_for("2 lệnh · 50% thắng · Kỳ vọng —").expect("the list counts the two linked trades");
    app.wait_for("1 lệnh · 100% thắng · Kỳ vọng —").expect("the trade with no setup is counted apart");
    app.wait_for("Retest giữ được").expect("the setup's rules show");
    app.wait_for("Cùng chiều trend H4").expect("and its checklist");
    assert!(app.screen().shows("+$60.00"), "net of the two trades: {:?}", app.screen().texts());
}

#[test]
fn a_playbook_is_created_edited_and_deleted() {
    let fixture = Fixture::new();
    let mut app = fixture.start();
    open_page(&mut app, "Playbook", "Playbook của bạn");
    app.wait_for("Chưa có playbook nào").expect("the empty state shows");

    app.press("role:button[Thêm playbook]").expect("the add button");
    app.type_text("label:Tên playbook", "Pullback EMA20").expect("the name is typed");
    app.type_text("label:Symbol áp dụng", "eurusd, xauusd").expect("the symbols are typed");
    app.press("role:button[Thêm điều kiện vào]").expect("a rule is added");
    app.type_text("label:Điều kiện vào 1", "Giá chạm EMA20").expect("the rule is typed");
    app.press("role:button[Lưu]").expect("the save button");
    wait_until("the setup being stored", || fixture.db().playbooks().is_ok_and(|p| p.len() == 1));
    let stored = fixture.db().playbooks().expect("setups read").remove(0);
    assert_eq!(stored.name, "Pullback EMA20");
    assert_eq!(stored.symbols, ["EURUSD", "XAUUSD"]);
    assert_eq!(stored.entry_rules, ["Giá chạm EMA20"]);
    app.wait_for("Giá chạm EMA20").expect("the saved setup shows in the detail");
    assert_eq!(fixture.db().search("pullback", 5).expect("searches").len(), 1, "a saved setup is searchable");

    // The detail (and its edit button) shows once the store has read the setup back; the rule
    // text above may still be the form's own field on a slow runner.
    app.wait_for_within("role:button[Sửa]", WAIT).expect("the saved setup's detail shows");
    common::press_until(&mut app, "role:button[Sửa]", |screen| screen.shows("Sửa playbook"));
    app.type_text("label:Tên playbook", " H1").expect("the name is extended");
    app.press("role:button[Lưu]").expect("the save button");
    wait_until("the setup being renamed", || {
        fixture.db().playbooks().is_ok_and(|p| p.first().is_some_and(|p| p.name == "Pullback EMA20 H1"))
    });

    // The form closes once the rename is written; the next press must open a fresh one.
    app.wait_until("the edit form closing", WAIT, |screen| !screen.shows("Sửa playbook")).expect("the form closes");
    common::press_until(&mut app, "role:button[Sửa]", |screen| screen.shows("Sửa playbook"));
    // Cancelled, or never answered: the setup stays.
    app.on_dialog(|_| nsh_testing::DialogAnswer::Press("Huỷ".to_owned()));
    app.press("role:button[Xoá playbook]").expect("the delete button");
    app.wait(Duration::from_millis(500)).expect("the app runs");
    assert_eq!(fixture.db().playbooks().expect("reads").len(), 1, "cancelling the question deletes nothing");
    app.answer_dialogs_myself();
    app.press("role:button[Xoá playbook]").expect("the delete button again");
    app.wait(Duration::from_millis(500)).expect("the app runs");
    assert!(app.pending_dialog().is_some(), "the question waits");
    assert_eq!(fixture.db().playbooks().expect("reads").len(), 1, "an unanswered question deletes nothing");
    app.answer_dialog(nsh_testing::DialogAnswer::Press("Xoá hẳn".to_owned())).expect("confirms");
    wait_until("the setup being deleted", || fixture.db().playbooks().is_ok_and(|p| p.is_empty()));
    app.wait_for("Chưa có playbook nào").expect("the list is empty again");
}

#[test]
fn a_second_playbook_cannot_take_a_name_in_use() {
    let fixture = Fixture::new();
    fixture
        .db()
        .save_playbook(&Playbook { name: "Breakout".into(), ..Playbook::default() }, 0)
        .expect("the setup is stored");
    let mut app = fixture.start();
    open_page(&mut app, "Playbook", "Playbook của bạn");
    app.press("role:button[Thêm playbook]").expect("the add button");
    app.type_text("label:Tên playbook", "breakout").expect("the name is typed");
    app.press("role:button[Lưu]").expect("the save button");
    app.wait_for("Đã có playbook tên “breakout”.").expect("the form says why it did not save");
    assert_eq!(fixture.db().playbooks().expect("setups read").len(), 1);
}

// --- Lệnh bỏ lỡ -------------------------------------------------------------------------------------

#[test]
fn a_missed_trade_is_recorded_with_its_reason() {
    let fixture = Fixture::new();
    let mut app = fixture.start();
    open_page(&mut app, "Lệnh bỏ lỡ", "Ghi lệnh bỏ lỡ");
    app.wait_for("Chưa có lệnh bỏ lỡ nào").expect("the empty state shows");

    app.type_text("label:Symbol", "xauusd").expect("the symbol is typed");
    common::toggle_on(&mut app, "role:button[Chiều: Sell]");
    common::toggle_on(&mut app, "role:button[Lý do: Sợ sau lệnh thua]");
    app.type_text("label:Ghi chú", "Đợi thêm nến xác nhận").expect("the notes are typed");
    app.press("role:button[Lưu]").expect("the save button");

    wait_until("the missed trade being stored", || fixture.db().missed_trades(0, i64::MAX).is_ok_and(|m| m.len() == 1));
    let stored = fixture.db().missed_trades(0, i64::MAX).expect("missed trades read").remove(0);
    assert_eq!((stored.symbol.as_str(), stored.side), ("XAUUSD", Side::Short));
    assert_eq!(stored.reason.as_deref(), Some("fear"));
    assert_eq!(stored.notes, "Đợi thêm nến xác nhận");
    app.wait_for("Lệnh bỏ lỡ · 1 gần nhất / 1").expect("the table counts it");
    assert_eq!(fixture.db().search("nen xac nhan", 5).expect("searches").len(), 1, "its notes are searchable");
}

#[test]
fn missed_trades_add_up_to_the_r_left_on_the_table() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    let missed = |reason: &str, outcome: f64| MissedTrade {
        id: 0,
        account_id: None,
        symbol: "XAUUSD".into(),
        side: Side::Long,
        seen_at: now_ms(),
        playbook_id: None,
        setup_grade: None,
        reason: Some(reason.into()),
        entry: Some(100.0),
        stop_loss: Some(99.0),
        take_profit: None,
        outcome_price: Some(outcome),
        notes: String::new(),
        screenshots: Vec::new(),
        custom: std::collections::BTreeMap::default(),
    };
    for trade in [missed("fear", 103.0), missed("fear", 102.0), missed("limit_reached", 104.0), missed("away", 99.0)] {
        db.save_missed_trade(&trade).expect("the missed trade is stored");
    }
    drop(db);

    let mut app = fixture.start();
    open_page(&mut app, "Lệnh bỏ lỡ", "Ghi lệnh bỏ lỡ");
    app.wait_for("Lệnh bỏ lỡ · 4 gần nhất / 4").expect("all four are listed");
    // +3R and +2R missed; the blocked one is a right miss and the −1R one a loss avoided.
    app.wait_for("+5.0R").expect("the R left on the table");
    app.wait_for("tránh được 1.0R nhờ không vào lệnh thua").expect("and the loss avoided");
    app.wait_for("bị chặn bởi quy tắc").expect("the rule-blocked miss is counted apart");
    app.wait_for("Sợ sau lệnh thua").expect("the most frequent reason is named");
    app.wait_for("2 / 4 lần").expect("with how often it came up");
}

// --- Tổng kết & xem lại -------------------------------------------------------------------------------

#[test]
fn the_review_queue_lists_the_trades_that_need_a_look() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    seed(&mut db);
    let flagged =
        JournalEntry { review_later: true, notes: "xem lại điểm vào".into(), ..JournalEntry::default() };
    db.save_journal(&key_of(&db, 2), &flagged).expect("the journal is stored");
    drop(db);

    let mut app = fixture.start();
    open_page(&mut app, "Tổng kết & xem lại", "Hàng chờ xem lại · 3");
    app.wait_for("2 lệnh chưa ghi nhật ký").expect("the trades with nothing written");
    app.wait_for("1 lệnh đánh dấu “Xem lại sau”").expect("the flagged one");
    app.wait_for("+$80.00").expect("the week's net");
    app.wait_for("3 lệnh").expect("and how many trades made it");
    app.press("role:button[Mở]").expect("the flagged trade opens");
    app.wait_for("Nhật ký lệnh").expect("on the trade's page");
}

#[test]
fn a_review_is_written_saved_and_found_again() {
    let fixture = Fixture::new();
    let mut app = fixture.start();
    open_page(&mut app, "Tổng kết & xem lại", "Câu hỏi tổng kết");
    app.wait_for("3 điều làm tốt tuần này").expect("the questions of the template show");
    app.type_text("label:3 điều làm tốt tuần này", "Chờ retest đúng playbook").expect("an answer is typed");
    app.press("role:button[Lưu tổng kết]").expect("the save button");

    // No account is known, so no server offset either: the page counts days in UTC, which is
    // another week than the machine's between its midnight and UTC's on a Monday.
    let start = Period::Week.bounds(chrono::Utc::now().date_naive()).0;
    wait_until("the review being stored", || fixture.db().review(None, Period::Week, start).is_ok_and(|r| r.is_some()));
    let stored = fixture.db().review(None, Period::Week, start).expect("review reads").expect("a review");
    assert_eq!(stored.answers[0], ("3 điều làm tốt tuần này".to_owned(), "Chờ retest đúng playbook".to_owned()));
    app.wait_for("Đã lưu").expect("the form says it is saved");
    assert_eq!(fixture.db().search("retest", 5).expect("searches").len(), 1, "the review is searchable");
}

#[test]
fn a_template_is_edited_and_asks_one_more_question() {
    let fixture = Fixture::new();
    let mut app = fixture.start();
    open_page(&mut app, "Tổng kết & xem lại", "Mẫu ghi chú");
    common::press_until(&mut app, "label:Sửa mẫu Tổng kết tuần", |screen| screen.shows("Thêm câu hỏi"));
    app.press("role:button[Thêm câu hỏi]").expect("a question is added");
    app.type_text("label:Câu hỏi 5", "Điều gì làm bạn bất ngờ?").expect("the question is typed");
    app.press("role:button[Lưu mẫu]").expect("the template is saved");
    // The form asks the new question once the template is stored and read again.
    app.wait_for("5 câu").expect("the template list counts five questions");
    app.wait_for("Điều gì làm bạn bất ngờ?").expect("the new question shows in the form");
    assert_eq!(fixture.db().review_template(Period::Week).expect("template reads").prompts.len(), 5);
}

// --- Tìm kiếm -----------------------------------------------------------------------------------------

#[test]
fn ctrl_k_finds_a_journal_note_without_accents_and_opens_its_trade() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    seed(&mut db);
    let entry = JournalEntry { notes: "Vào sớm trước khi nến M15 đóng".into(), ..JournalEntry::default() };
    db.save_journal(&key_of(&db, 2), &entry).expect("the journal is stored");
    drop(db);

    let mut app = fixture.start();
    app.press_key("mod+k").expect("Ctrl+K opens the palette");
    app.type_text("label:Tìm trong nhật ký", "vao som").expect("the query is typed");
    app.wait_for("role:button[Loại kết quả: Lệnh · 1]").expect("the trade is found, accents or not");
    app.wait_for("Lệnh").expect("under its heading");
    app.press_key("enter").expect("Enter opens the first result");
    app.wait_for("Nhật ký lệnh").expect("the trade's page opens");
    app.wait_for_gone("role:button[Đóng tìm kiếm]", WAIT).expect("and the palette closes");
}

#[test]
fn a_day_note_is_found_whether_it_belongs_to_an_account_or_to_all() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    seed(&mut db);
    let account = db.accounts().expect("accounts read").remove(0);
    let day = Local::now().date_naive();
    let note = |text: &str| DayNote { plan: text.into(), review: String::new(), mood: String::new() };
    db.save_day_note(account.id, day, &note("Giữ kỷ luật hôm nay")).expect("the note is stored");
    db.save_day_note(0, day, &note("Nghỉ vì tin tức mạnh")).expect("the note of all accounts is stored");
    drop(db);

    let mut app = fixture.start();
    app.press("role:button[Mở tìm kiếm]").expect("the sidebar's search box opens the palette");
    app.type_text("label:Tìm trong nhật ký", "ky luat").expect("the query is typed");
    app.wait_for("role:button[Loại kết quả: Nhật ký ngày · 1]").expect("the account's note is found");
    app.clear("label:Tìm trong nhật ký").expect("the field is cleared");
    app.type_text("label:Tìm trong nhật ký", "tin tuc").expect("the other query is typed");
    app.wait_for("role:button[Loại kết quả: Nhật ký ngày · 1]").expect("the note of all accounts is found");
    app.press_key("enter").expect("Enter opens the day");
    app.wait_for_gone("role:button[Đóng tìm kiếm]", WAIT).expect("the palette closes");
}

#[test]
fn a_query_with_no_match_says_so() {
    let fixture = Fixture::new();
    let mut app = fixture.start();
    app.press_key("mod+k").expect("Ctrl+K opens the palette");
    app.wait_for("Gõ để tìm trong lệnh, nhật ký ngày, tổng kết, lệnh bỏ lỡ và playbook — không cần gõ dấu.")
        .expect("an empty field says what to type");
    app.type_text("label:Tìm trong nhật ký", "zzzz").expect("the query is typed");
    app.wait_for("Không có kết quả cho “zzzz”.").expect("a miss is said");
    app.press_key("escape").expect("Escape clears the query");
    app.press_key("escape").expect("and closes the palette");
    app.wait_for_gone("role:button[Đóng tìm kiếm]", WAIT).expect("it is gone");
}

// --- Nguồn dữ liệu ---------------------------------------------------------------------------------------

#[test]
fn the_data_sources_page_shows_how_to_export_the_history_with_python() {
    let fixture = Fixture::new();
    let mut app = fixture.start();
    open_page(&mut app, "Nguồn dữ liệu", "Nhật ký import");
    app.wait_for("Script Python (MetaTrader5)").expect("the script's card shows");
    app.wait_for("python tradelens_sync.py --days 365 --out deals.json")
        .expect("with the command that writes the file");
    assert!(!app.screen().shows("Cài EA TradeLensSync"), "the EA is gone");
}

#[test]
fn an_mt5_html_report_adds_its_trades() {
    let fixture = Fixture::new();
    let mut app = fixture.start();
    open_page(&mut app, "Nguồn dữ liệu", "Nhật ký import");
    app.wait_for("Chưa import gì").expect("the log is empty");

    app.on_pick_paths(vec![fixture_file("mt5_en_hedge.html")]).expect("answers the dialog");
    app.press("label:Kéo thả báo cáo MT5 vào đây").expect("the drop zone opens the dialog");
    shows_text_containing(&mut app, "the result line", "mt5_en_hedge.html: ");
    shows_text_containing(&mut app, "the deals counted", " deal mới, ");
    let db = fixture.db();
    assert!(!db.deals(None).expect("deals read").is_empty(), "the report's deals are stored");
    assert_eq!(db.accounts().expect("accounts read").len(), 1);
    app.wait_for("Import HTML").expect("the import log names the source");

    // The same report again stores nothing new.
    app.on_pick_paths(vec![fixture_file("mt5_en_hedge.html")]).expect("answers the dialog");
    app.press("label:Kéo thả báo cáo MT5 vào đây").expect("the drop zone opens the dialog");
    shows_text_containing(&mut app, "the second result", " 0 deal mới, 0 deal cập nhật");
}

#[test]
fn a_file_that_is_not_a_report_is_refused_with_a_reason() {
    let fixture = Fixture::new();
    let broken = fixture.path().join("broken.html");
    std::fs::write(&broken, "<html><body>không có bảng deal</body></html>").expect("writes");
    let mut app = fixture.start();
    open_page(&mut app, "Nguồn dữ liệu", "Nhật ký import");
    app.on_pick_paths(vec![broken]).expect("answers the dialog");
    app.press("label:Kéo thả báo cáo MT5 vào đây").expect("the drop zone opens the dialog");
    shows_text_containing(&mut app, "the reason", "broken.html: không tìm thấy bảng Deals trong báo cáo");
}

#[test]
fn a_notion_export_is_previewed_then_imported_with_no_column_mapping() {
    let fixture = Fixture::new();
    let file = fixture_file("notion_extras.csv");
    let plan = notion::read_file(&file).expect("the fixture reads");
    let rows = plan.rows.len();
    assert!(rows > 0 && !plan.properties.is_empty(), "the fixture has trades and extra columns");

    let mut app = fixture.start();
    open_page(&mut app, "Nguồn dữ liệu", "Nhật ký import");
    app.on_pick_paths(vec![file]).expect("answers the dialog");
    app.press("label:Kéo thả file ZIP hoặc CSV của Notion vào đây").expect("the drop zone opens the dialog");
    shows_text_containing(&mut app, "the preview", &format!("{rows} lệnh đọc được"));
    app.wait_for("Cột tự nhận ra (từ nội dung, không cần ghép tay)").expect("the detected fields are listed");
    for property in &plan.properties {
        let line = format!("{} · {} · {} lệnh có giá trị", property.label, kind_name(property.kind), property.filled);
        app.wait_for(&line).unwrap_or_else(|error| panic!("{line}: {error}"));
    }
    assert!(fixture.db().deals(None).expect("deals read").is_empty(), "nothing is stored before the button");
    assert!(!app.screen().shows("Cột Notion"), "there is no mapping table to fill in");

    app.press(&format!("role:button[Nhập {rows} lệnh]")).expect("the one import button");
    app.wait_for(&format!("Đã nhập {rows} lệnh mới · 0 cập nhật · 0 không đổi")).expect("the summary shows");
    shows_text_containing(&mut app, "the created properties", "Thuộc tính đã tạo: ");

    let db = fixture.db();
    assert_eq!(db.deals(None).expect("deals read").len(), rows * 2, "an entry and an exit deal per trade");
    let labels: Vec<String> =
        db.field_defs(FieldScope::Trade).expect("fields read").into_iter().map(|def| def.label).collect();
    for property in &plan.properties {
        assert!(labels.contains(&property.label), "{} was created: {labels:?}", property.label);
    }
    app.wait_for("Import Notion").expect("the import log names the source");
}

#[test]
fn a_notion_export_can_go_to_an_mt5_account() {
    let fixture = Fixture::new();
    seed(&mut fixture.db());
    let account = fixture.db().accounts().expect("accounts read").remove(0);
    let file = fixture_file("notion_en.csv");
    let rows = notion::read_file(&file).expect("the fixture reads").rows.len();
    let mut app = fixture.start();
    open_page(&mut app, "Nguồn dữ liệu", "Nhật ký import");
    app.on_pick_paths(vec![file]).expect("answers the dialog");
    app.press("label:Kéo thả file ZIP hoặc CSV của Notion vào đây").expect("the drop zone opens the dialog");
    app.wait_for("Nhập vào tài khoản").expect("the account choice shows");
    app.press("label:Nhập vào tài khoản").expect("the choice opens");
    app.press(&format!("text:{}", account.title())).expect("the MT5 account is chosen");
    app.press(&format!("role:button[Nhập {rows} lệnh]")).expect("the import button");
    app.wait_for(&format!("Đã nhập {rows} lệnh mới · 0 cập nhật · 0 không đổi")).expect("the summary shows");
    let after = fixture.db();
    let accounts = after.accounts().expect("accounts read");
    assert_eq!(accounts.len(), 1, "no Notion account was made: {accounts:?}");
    assert_eq!(after.deals(Some(account.id)).expect("deals read").len(), 6 + rows * 2);
}
