//! Editing and deleting trades, headless on the real engine: a trade the trader typed is edited and
//! shows its new facts everywhere, a trade of the terminal keeps its facts, deleting asks first and
//! takes the journal with it, several trades go at once, a journal kept in R is typed in R, a time is
//! typed in a field, prices keep their decimals, and the figures of the list fit their columns.

use nsh_core::{ApproxTextEngine, TextEngine, platform::Os};
use std::time::Duration;

use nsh_testing::{DialogAnswer, Headless};
use tradelens_core::{
    Db, Deal, DealBatch, DealEntry, DealKind, Gaps, JournalEntry, ManualTrade, Origin, Side, TradeKey, import::notion,
    model::AccountInfo, time::parse_mt5_time, trades::build_trades,
};

mod support;
use support::{Fixture, WAIT, wait_until};

/// How long the app runs to show that a question left unanswered, or cancelled, did nothing.
const QUIET: Duration = Duration::from_millis(500);

fn time(text: &str) -> i64 {
    parse_mt5_time(text).expect("fixture times parse")
}

/// A trade the trader typed: EURUSD, long, 5-digit prices.
fn typed(symbol: &str, open: &str, close: &str) -> ManualTrade {
    ManualTrade {
        symbol: symbol.into(),
        side: Side::Long,
        volume: Some(0.5),
        open_time: time(open),
        open_price: Some(1.0850),
        close_time: time(close),
        close_price: Some(1.08505),
        date_only: false,
        sl: 1.0830,
        tp: 0.0,
        profit: Some(25.0),
        commission: 0.0,
        swap: 0.0,
        result_r: None,
    }
}

fn info() -> AccountInfo {
    AccountInfo { login: "5012345".into(), server: "Demo".into(), currency: "USD".into(), ..AccountInfo::default() }
}

/// A trade of the terminal: XAUUSD, an entry and an exit.
fn terminal_trade(db: &mut Db) -> TradeKey {
    let deal = |ticket: i64, entry: DealEntry, at: &str| Deal {
        ticket,
        order: ticket,
        position_id: 700,
        time_msc: time(at),
        symbol: "XAUUSD".into(),
        kind: if entry == DealEntry::In { DealKind::Buy } else { DealKind::Sell },
        entry,
        volume: 0.1,
        price: 2648.2,
        commission: 0.0,
        swap: 0.0,
        fee: 0.0,
        profit: if entry == DealEntry::In { 0.0 } else { 40.0 },
        magic: 0,
        comment: String::new(),
        sl: 2640.0,
        tp: 0.0,
        risk: None,
        result_r: None,
        gaps: Gaps::NONE,
        reason: None,
    };
    let batch = DealBatch {
        account: info(),
        deals: vec![deal(701, DealEntry::In, "2026.11.18 10:00:00"), deal(702, DealEntry::Out, "2026.11.18 11:00:00")],
        positions: Vec::new(),
    };
    let account_id = db.upsert_batch(&batch, "html").expect("stored").account_id;
    TradeKey { account_id, position_id: 700, segment: 0 }
}

fn typed_in(db: &mut Db, trade: &ManualTrade) -> TradeKey {
    let account =
        db.accounts().expect("accounts").first().map_or_else(|| db.upsert_account(&info()).expect("account"), |a| a.id);
    db.add_manual_trade(account, trade).expect("stored")
}

fn start(fixture: &Fixture) -> Headless {
    let mut app = fixture.start(1400.0, 1000.0, |_| {});
    app.press("role:button[Lệnh]").expect("the trades row");
    app
}

fn open_first(app: &mut Headless, symbol: &str) {
    app.wait_for_within(&format!("role:button[Mở nhật ký lệnh {symbol}]"), WAIT).expect("the list shows the trade");
    app.press(&format!("role:button[Mở nhật ký lệnh {symbol}] >> nth=0")).expect("the trade opens");
    app.wait_for_within("Chi tiết lệnh", WAIT).expect("the trade's page");
}

/// Replaces what a number field holds.
fn retype(app: &mut Headless, label: &str, value: &str) {
    app.clear(&format!("label:{label}")).expect("the field clears");
    app.type_text(&format!("label:{label}"), value).expect("the field is typed in");
}

fn deal_count(db: &Db) -> usize {
    db.deals(None).expect("deals").len()
}

#[test]
fn a_trade_the_trader_typed_is_edited_and_shows_its_new_facts_everywhere() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    let key = typed_in(&mut db, &typed("EURUSD", "2026.11.19 08:00:00", "2026.11.19 09:00:00"));
    db.save_journal(&key, &JournalEntry { notes: "vào theo trend".into(), ..JournalEntry::default() })
        .expect("a journal");
    let tickets: Vec<i64> = db.deals(None).expect("deals").iter().map(|(_, d)| d.ticket).collect();
    drop(db);
    let mut app = start(&fixture);
    open_first(&mut app, "EURUSD");

    // 5-digit prices keep their decimals, and the symbol is told apart from the account in "nguồn".
    app.wait_for_within("1.08500", WAIT).expect("the entry price has the symbol's five decimals");
    app.wait_for_within("1.08505", WAIT).expect("so does the exit price");
    app.wait_for_within("text:/nguồn: Nhập tay$/", WAIT).expect("the source comes from how the trade was written");

    app.press("role:button[Sửa lệnh]").expect("the edit button");
    app.wait_for_within("Số liệu của lệnh; nhật ký, ảnh và liên kết của lệnh giữ nguyên", WAIT)
        .expect("the form opens");
    wait_until("the form being filled", || {
        app.screen().input_text("label:Symbol").is_ok_and(|text| text == Some("EURUSD"))
    });
    assert_eq!(app.screen().input_text("label:Giờ vào").expect("the field"), Some("08:00"), "the time is filled in");
    assert_eq!(app.screen().input_text("label:Giờ ra").expect("the field"), Some("09:00"));

    retype(&mut app, "Giá ra", "1.0910");
    retype(&mut app, "Kết quả (R)", "2.5");
    app.clear("label:Giờ ra").expect("the time clears");
    app.type_text("label:Giờ ra", "1130").expect("a time is typed without a colon");
    app.press_key("enter").expect("the time is kept");
    wait_until("the time being read", || app.screen().input_text("label:Giờ ra").is_ok_and(|t| t == Some("11:30")));
    app.press("role:button[Lưu thay đổi]").expect("saves");
    app.wait_for_within("Chi tiết lệnh", WAIT).expect("back on the trade's page");

    let db = fixture.db();
    wait_until("the edit being stored", || {
        db.manual_trade(&key).expect("reads").is_some_and(|t| t.result_r == Some(2.5))
    });
    let stored = db.manual_trade(&key).expect("reads").expect("still the trader's trade");
    assert_eq!(stored.close_price, Some(1.0910));
    assert_eq!(tradelens_core::time::naive(stored.close_time).format("%H:%M").to_string(), "11:30");
    assert_eq!(db.deals(None).expect("deals").iter().map(|(_, d)| d.ticket).collect::<Vec<_>>(), tickets);
    assert_eq!(db.journal(&key).expect("reads").map(|j| j.notes).as_deref(), Some("vào theo trend"));

    // The page, the list and the overview read the new facts.
    app.wait_for_within("1.091", WAIT).expect("the page shows the new exit price");
    app.wait_for_within("+2.5R", WAIT).expect("and the R that was typed");
    app.press("role:button[‹ Lệnh]").expect("back to the list");
    app.wait_for_within("+2.5R", WAIT).expect("the list shows the R");
    app.wait_for_within("text:/^19\\/11 11:30$/", WAIT).expect("and the new closing time");
}

#[test]
fn ctrl_e_opens_the_edit_form_of_a_trade_the_trader_owns() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    typed_in(&mut db, &typed("EURUSD", "2026.11.19 08:00:00", "2026.11.19 09:00:00"));
    drop(db);
    let mut app = start(&fixture);
    open_first(&mut app, "EURUSD");
    app.press_key("mod+e").expect("the shortcut");
    app.wait_for_within("Số liệu của lệnh; nhật ký, ảnh và liên kết của lệnh giữ nguyên", WAIT)
        .expect("the form opens");
    app.press("role:button[Huỷ]").expect("cancels");
    app.wait_for_within("Chi tiết lệnh", WAIT).expect("back on the trade");
}

#[test]
fn a_trade_of_the_terminal_has_no_edit_button_and_says_why() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    let key = terminal_trade(&mut db);
    drop(db);
    let mut app = start(&fixture);
    open_first(&mut app, "XAUUSD");
    app.wait_for_within("text:/nguồn: MT5/", WAIT).expect("the source");
    app.wait_for_within("Số liệu của lệnh lấy từ MT5 nên không sửa được; nhật ký và ảnh chart vẫn sửa được.", WAIT)
        .expect("the one-line note");
    assert!(!app.screen().matches("role:button[Sửa lệnh]").expect("a selector"), "no button to edit the facts");
    app.press_key("mod+e").expect("the shortcut is pressed");
    app.wait_for_gone(
        "Số liệu của lệnh; nhật ký, ảnh và liên kết của lệnh giữ nguyên",
        std::time::Duration::from_secs(1),
    )
    .expect("and does nothing");
    // The journal stays editable as ever.
    app.type_text("label:Ghi chú", "vẫn sửa được").expect("the notes");
    app.press("role:button[Lưu]").expect("saves");
    let db = fixture.db();
    wait_until("the journal being stored", || {
        db.journal(&key).expect("reads").is_some_and(|j| j.notes == "vẫn sửa được")
    });
}

#[test]
fn deleting_a_trade_asks_first_and_takes_its_journal_with_it() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    let key = typed_in(&mut db, &typed("EURUSD", "2026.11.19 08:00:00", "2026.11.19 09:00:00"));
    let other = typed_in(&mut db, &typed("GBPUSD", "2026.11.18 08:00:00", "2026.11.18 09:00:00"));
    db.save_journal(&key, &JournalEntry { notes: "sẽ mất".into(), ..JournalEntry::default() }).expect("a journal");
    drop(db);
    let mut app = start(&fixture);
    open_first(&mut app, "EURUSD");

    // The question is the system's alert: it is answered through the headless dialog API.
    // Never answered, it waits and nothing is deleted.
    app.answer_dialogs_myself();
    app.press("role:button[Xoá lệnh]").expect("the delete button");
    app.wait(QUIET).expect("the app runs");
    let question = app.pending_dialog().expect("the question is asked and waits");
    assert_eq!(question.title.as_deref(), Some("Xoá lệnh?"));
    assert_eq!(
        question.message.as_deref(),
        Some("Xoá lệnh EURUSD cùng nhật ký và ảnh chart của nó. Không hoàn tác được."),
        "the question names the trade"
    );
    assert_eq!(deal_count(&fixture.db()), 4, "an unanswered question deletes nothing");

    // "Huỷ" leaves everything where it is.
    app.answer_dialog(DialogAnswer::Press("Huỷ".to_owned())).expect("cancels");
    app.wait(QUIET).expect("the app runs");
    assert_eq!(deal_count(&fixture.db()), 4, "nothing was deleted");
    assert!(app.pending_dialog().is_none(), "the question is closed");
    assert!(app.screen().shows("Chi tiết lệnh"), "the trade stays open");

    app.press("role:button[Xoá lệnh]").expect("the delete button again");
    app.answer_dialog(DialogAnswer::Press("Xoá lệnh".to_owned())).expect("confirms");
    app.wait_for_within("Hiển thị 1 / 1 lệnh", WAIT).expect("back on the list, which reloads");
    let db = fixture.db();
    wait_until("the trade being deleted", || deal_count(&db) == 2);
    assert_eq!(db.journal(&key).expect("reads"), None, "the journal went with it");
    assert!(db.manual_trade(&other).expect("reads").is_some(), "the other trade stays");
    app.wait_for_within("GBPUSD", WAIT).expect("the list shows what is left");
}

#[test]
fn the_question_about_a_trade_of_the_terminal_says_that_importing_again_brings_it_back() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    terminal_trade(&mut db);
    drop(db);
    let mut app = start(&fixture);
    open_first(&mut app, "XAUUSD");
    app.answer_dialogs_myself();
    app.press("role:button[Xoá lệnh]").expect("the delete button");
    let question = app.pending_dialog().expect("the question is asked");
    assert!(
        question.message.as_deref().is_some_and(|said| said.contains("import lại cùng báo cáo sẽ đưa lệnh trở lại")),
        "the warning: {question:?}"
    );
    app.answer_dialog(DialogAnswer::Press("Xoá lệnh".to_owned())).expect("confirms");
    wait_until("the trade being deleted", || deal_count(&fixture.db()) == 0);
}

#[test]
fn several_trades_are_ticked_and_deleted_with_one_question() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    for (symbol, day) in [("EURUSD", 15), ("GBPUSD", 16), ("USDJPY", 17), ("AUDUSD", 18), ("NZDUSD", 19)] {
        let (open, close) = (format!("2026.11.{day} 08:00:00"), format!("2026.11.{day} 09:00:00"));
        typed_in(&mut db, &typed(symbol, &open, &close));
    }
    drop(db);
    let mut app = start(&fixture);
    app.wait_for_within("Hiển thị 5 / 5 lệnh", WAIT).expect("five trades");
    // Newest first: NZDUSD, AUDUSD, USDJPY, GBPUSD, EURUSD. Tick one, then Shift-tick two further down.
    app.press("label:Chọn lệnh NZDUSD 19/11 09:00").expect("a box");
    app.wait_for_within("Đã chọn 1 lệnh", WAIT).expect("the bar counts");
    app.hold_keys("shift", true).expect("shift goes down");
    app.press("label:Chọn lệnh GBPUSD 16/11 09:00").expect("a box with shift");
    app.hold_keys("shift", false).expect("shift goes up");
    app.wait_for_within("Đã chọn 4 lệnh", WAIT).expect("the range between them is ticked");
    // Unticking one by a plain press.
    app.press("label:Chọn lệnh AUDUSD 18/11 09:00").expect("a box");
    app.wait_for_within("Đã chọn 3 lệnh", WAIT).expect("one fewer");

    app.answer_dialogs_myself();
    app.press("role:button[Xoá]").expect("the bar's delete");
    app.wait(QUIET).expect("the app runs");
    let question = app.pending_dialog().expect("one question for all, and it waits");
    assert_eq!(question.title.as_deref(), Some("Xoá 3 lệnh?"));
    assert_eq!(
        question.message.as_deref(),
        Some("Xoá 3 lệnh cùng nhật ký và ảnh chart của chúng. Không hoàn tác được.")
    );
    assert_eq!(deal_count(&fixture.db()), 10, "an unanswered question deletes nothing");
    app.answer_dialog(DialogAnswer::Press("Huỷ".to_owned())).expect("cancels");
    app.wait(QUIET).expect("the app runs");
    assert_eq!(deal_count(&fixture.db()), 10, "cancelling deletes nothing");
    assert!(app.screen().shows("Đã chọn 3 lệnh"), "the selection stays");
    app.press("role:button[Xoá]").expect("the bar's delete again");
    app.answer_dialog(DialogAnswer::Press("Xoá lệnh".to_owned())).expect("confirms");
    app.wait_for_within("Hiển thị 2 / 2 lệnh", WAIT).expect("the list reloads");
    let db = fixture.db();
    wait_until("three trades being deleted", || deal_count(&db) == 4);
    let left: Vec<String> = db.deals(None).expect("deals").into_iter().map(|(_, d)| d.symbol).collect();
    assert!(left.iter().all(|s| s == "AUDUSD" || s == "EURUSD"), "the ticked ones went: {left:?}");
    assert!(!app.screen().shows("Đã chọn"), "the bar goes with the selection");
}

#[test]
fn a_journal_kept_in_r_is_typed_in_r_with_a_symbol_in_order_and_its_own_source() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    let (plan, _) = notion::read_source(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/core/tests/fixtures/notion_r_journal.zip"),
    )
    .expect("the zip is understood");
    db.import_notion(&plan).expect("imports");
    drop(db);
    let mut app = fixture.start(1400.0, 1300.0, |_| {});
    app.press("role:button[Lệnh]").expect("the trades row");
    app.wait_for_within("BTCUSD", WAIT).expect("the journal's trades");
    app.press("role:button[Thêm lệnh]").expect("the add button");
    app.wait_for_within("Nhập lệnh & nhật ký thủ công", WAIT).expect("the form opens");
    app.wait_for_within("text:/^Nhật ký ghi bằng R: lot, giá và lãi\\/lỗ không bắt buộc/", WAIT)
        .expect("the form leads with R");

    app.type_text("label:Symbol", "eth/usd").expect("a symbol as it is written by hand");
    let this_month = |day: u32| {
        let today = chrono::Local::now().date_naive();
        chrono::NaiveDate::from_ymd_opt(chrono::Datelike::year(&today), chrono::Datelike::month(&today), day)
            .expect("a day of the month")
            .format("%Y-%m-%d")
            .to_string()
    };
    app.press("label:Thời gian vào").expect("the calendar");
    app.press(&format!("role:button[{}]", this_month(3))).expect("a day");
    app.press("label:Thời gian ra").expect("the calendar");
    app.press(&format!("role:button[{}]", this_month(3))).expect("a day");
    app.press("role:button[Lưu lệnh]").expect("saves with nothing to say how it went");
    app.wait_for_within("text:/Nhập kết quả \\(R\\) của lệnh/", WAIT).expect("the form says what is missing");

    app.type_text("label:Kết quả (R)", "1.5").expect("the result in R");
    app.wait_for_within("text:/^Tự tính: R nhập tay \\+1\\.5R$/", WAIT).expect("the line follows");
    app.press("role:button[Lưu lệnh]").expect("saves");

    let db = fixture.db();
    wait_until("the trade being stored", || db.deals(None).expect("deals").iter().any(|(_, d)| d.symbol == "ETHUSD"));
    let deals: Vec<(i64, Deal)> =
        db.deals(None).expect("deals").into_iter().filter(|(_, d)| d.symbol == "ETHUSD").collect();
    let exit = deals.iter().map(|(_, d)| d).find(|d| d.entry == DealEntry::Out).expect("an exit");
    assert_eq!((exit.result_r, exit.profit), (Some(1.5), 0.0), "kept by its R, with no money");
    let origin = db.trade_origins(None).expect("origins")[&(deals[0].0, deals[0].1.position_id)];
    assert_eq!(origin, Origin::Manual, "typed by hand, though filed in the journal's account");
    let trade = &build_trades(deals[0].0, &deals.iter().map(|(_, d)| d.clone()).collect::<Vec<_>>())[0];
    assert!(!trade.money_known() && !trade.volume_known() && !trade.prices_known() && !trade.time_known());
    app.wait_for_within("Chi tiết lệnh", WAIT).expect("the new trade opens");
    app.wait_for_within("text:/nguồn: Nhập tay$/", WAIT)
        .expect("its source is told by how it was written, not by the account");
    app.wait_for_within("+1.5R", WAIT).expect("its result");
}

#[test]
fn a_time_is_typed_in_its_own_field_and_a_wrong_one_is_said_in_words() {
    let fixture = Fixture::new();
    fixture.db().upsert_account(&info()).expect("an account");
    let mut app = fixture.start(1400.0, 1300.0, |_| {});
    app.press("role:button[Lệnh]").expect("the trades row");
    app.press("role:button[Thêm lệnh]").expect("the add button");
    app.wait_for_within("Nhập lệnh & nhật ký thủ công", WAIT).expect("the form opens");

    app.type_text("label:Giờ vào", "9:30").expect("a time with a colon");
    app.press_key("enter").expect("kept");
    wait_until("the time being read", || app.screen().input_text("label:Giờ vào").is_ok_and(|t| t == Some("09:30")));
    app.clear("label:Giờ vào").expect("clears");
    app.type_text("label:Giờ vào", "2599").expect("a time that is not one");
    app.press_key("enter").expect("kept");
    app.wait_for_within("text:/^Giờ không hợp lệ: gõ 0930 hoặc 9:30/", WAIT).expect("the reason, in Vietnamese");
    app.clear("label:Giờ vào").expect("clears");
    app.type_text("label:Giờ vào", "0715").expect("a time without a colon");
    app.press_key("enter").expect("kept");
    app.wait_for_gone("text:/^Giờ không hợp lệ/", WAIT).expect("the reason goes with the mistake");
    wait_until("the time being read", || app.screen().input_text("label:Giờ vào").is_ok_and(|t| t == Some("07:15")));
}

#[test]
fn the_figures_of_the_list_fit_their_columns_and_their_titles() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    // The widest figures a trade has: a lot of 12.50, a result of +12.5R, 12 hours 30 minutes held, a
    // five-figure win with its thousands separator.
    typed_in(
        &mut db,
        &ManualTrade {
            volume: Some(12.5),
            open_time: time("2026.11.19 08:00:00"),
            close_time: time("2026.11.19 20:30:00"),
            profit: Some(12_345.67),
            result_r: Some(12.5),
            ..typed("EURUSD", "2026.11.19 08:00:00", "2026.11.19 20:30:00")
        },
    );
    typed_in(
        &mut db,
        &ManualTrade {
            profit: Some(-1_126.4),
            result_r: Some(-1.2),
            ..typed("XAUUSD", "2026.11.18 08:00:00", "2026.11.18 08:42:00")
        },
    );
    drop(db);
    let mut app = fixture.start(1280.0, 900.0, |_| {});
    app.press("role:button[Lệnh]").expect("the trades row");
    app.wait_for_within("Hiển thị 2 / 2 lệnh", WAIT).expect("the trades");
    app.wait_for_within("12g 30p", WAIT).expect("the hold time");

    let engine = ApproxTextEngine;
    let nodes = app.screen().layout_nodes();
    let figures = ["12.50", "0.50", "+$12,345.67", "−$1,126.40", "+12.5R", "−1.2R", "12g 30p", "42p"];
    let titles = ["Lot", "Thời gian giữ", "Net", "R", "Đóng lúc", "Symbol", "Chiều", "Nhật ký"];
    let mut checked = 0;
    for node in &nodes {
        let Some(shown) = node.text.as_ref() else { continue };
        if !figures.contains(&shown.text.text.as_str()) && !titles.contains(&shown.text.text.as_str()) {
            continue;
        }
        checked += 1;
        let prepared = engine.prepare(&shown.text);
        let needs = engine.measure(&prepared, None).width.0;
        assert!(
            shown.content.size.width.0 + 0.5 >= needs,
            "{:?} is cut: it needs {needs} points and its column gives {}",
            shown.text.text,
            shown.content.size.width.0
        );
    }
    assert!(checked >= 12, "the figures and the titles were found on screen: {checked}");
}

#[test]
fn a_price_typed_with_five_decimals_is_stored_whole_and_a_symbol_in_lower_case_is_normalised() {
    let fixture = Fixture::new();
    fixture.db().upsert_account(&info()).expect("an account");
    let mut app = fixture.start(1400.0, 1300.0, |_| {});
    app.press("role:button[Lệnh]").expect("the trades row");
    app.press("role:button[Thêm lệnh]").expect("the add button");
    app.wait_for_within("Nhập lệnh & nhật ký thủ công", WAIT).expect("the form opens");
    app.type_text("label:Symbol", "eurusd").expect("symbol");
    let today = chrono::Local::now().date_naive();
    let day = chrono::NaiveDate::from_ymd_opt(chrono::Datelike::year(&today), chrono::Datelike::month(&today), 3)
        .expect("a day of the month");
    app.press("label:Thời gian vào").expect("the calendar");
    app.press(&format!("role:button[{}]", day.format("%Y-%m-%d"))).expect("a day");
    app.type_text("label:Giờ vào", "0900").expect("a time");
    app.press_key("enter").expect("kept");
    app.press("label:Thời gian ra").expect("the calendar");
    app.press(&format!("role:button[{}]", day.format("%Y-%m-%d"))).expect("a day");
    app.type_text("label:Giờ ra", "1000").expect("a time");
    app.press_key("enter").expect("kept");
    app.type_text("label:Khối lượng", "0.5").expect("volume");
    app.type_text("label:Giá vào", "1.0850").expect("open price");
    app.type_text("label:Giá ra", "1.08505").expect("close price");
    app.type_text("label:Lãi/lỗ", "25").expect("profit");
    app.press("role:button[Lưu lệnh]").expect("saves");
    app.wait_for_within("Chi tiết lệnh", WAIT).expect("the trade opens");

    let db = fixture.db();
    // The page can show the trade before the background write has stored it: wait for the rows.
    let stored_key = || {
        let position_id = db.deals(None).expect("deals").iter().map(|(_, d)| d.position_id).next()?;
        let key = TradeKey { account_id: db.accounts().expect("accounts")[0].id, position_id, segment: 0 };
        db.manual_trade(&key).expect("reads").is_some().then_some(key)
    };
    wait_until("the manual trade being stored", || stored_key().is_some());
    let key = stored_key().expect("the trade was stored");
    let trade = db.manual_trade(&key).expect("reads").expect("the trade");
    assert_eq!(trade.symbol, "EURUSD");
    assert_eq!((trade.open_price, trade.close_price), (Some(1.0850), Some(1.08505)), "no price is rounded");
    assert_eq!(tradelens_core::time::naive(trade.open_time).format("%H:%M").to_string(), "09:00");
    app.wait_for_within("1.08500", WAIT).expect("shown with the five decimals the symbol has");
    app.wait_for_within("1.08505", WAIT).expect("both prices line up");
}

#[test]
fn enter_on_the_row_the_keyboard_selected_opens_that_trade() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    for (symbol, day) in [("EURUSD", 17), ("GBPUSD", 18)] {
        let (open, close) = (format!("2026.11.{day} 08:00:00"), format!("2026.11.{day} 09:00:00"));
        typed_in(&mut db, &typed(symbol, &open, &close));
    }
    drop(db);
    // Keys reach the focused view only on a desktop.
    let mut app = fixture.start(1400.0, 1000.0, |config| config.environment.os = Os::Linux);
    app.press("role:button[Lệnh]").expect("the trades row");
    app.wait_for_within("Hiển thị 2 / 2 lệnh", WAIT).expect("two trades");
    // Ticking a box gives the table the keyboard, on the row of that box: the newest trade, GBPUSD.
    app.press("label:Chọn lệnh GBPUSD 18/11 09:00").expect("a box");
    app.wait_for_within("Đã chọn 1 lệnh", WAIT).expect("the bar counts");
    app.press_key("enter").expect("Enter on the selected row");
    app.wait_for_within("Chi tiết lệnh", WAIT).expect("the trade of that row opens");
    assert!(app.screen().shows("GBPUSD"), "the trade is the one of the row: {:?}", app.screen().texts());
    // One row further down: the keyboard moves the selection, Enter opens that one.
    app.press("role:button[‹ Lệnh]").expect("back to the list");
    app.wait_for_within("Hiển thị 2 / 2 lệnh", WAIT).expect("the list is back");
    app.press("label:Chọn lệnh GBPUSD 18/11 09:00").expect("the box again");
    app.press_key("down").expect("the selection moves down");
    app.press_key("enter").expect("Enter on the next row");
    app.wait_for_within("Chi tiết lệnh", WAIT).expect("a trade opens");
    assert!(app.screen().shows("EURUSD"), "the trade is the next row's: {:?}", app.screen().texts());
}

#[test]
fn the_free_room_of_the_list_is_shared_between_columns_and_the_last_one_ends_at_the_table_edge() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    typed_in(&mut db, &typed("EURUSD", "2026.11.19 08:00:00", "2026.11.19 09:00:00"));
    drop(db);
    // The distance from the last title to the table's right edge, and the widths of two columns that
    // start at their title: the closing time (title "Đóng lúc" to "Symbol") and the setup ("Setup" to
    // "Nhật ký").
    let measure = |width: f32| {
        let mut app = fixture.start(width, 900.0, |_| {});
        app.press("role:button[Lệnh]").expect("the trades row");
        app.wait_for_within("Hiển thị 1 / 1 lệnh", WAIT).expect("the trade");
        app.wait_for_within("Nhật ký", WAIT).expect("the last title");
        let table = app.screen().layout("label:Danh sách lệnh").expect("one table").expect("the table is on screen");
        let nodes = app.screen().layout_nodes();
        let left = |title: &str| {
            nodes
                .iter()
                .find(|node| node.text.as_ref().is_some_and(|shown| shown.text.text == title))
                .unwrap_or_else(|| panic!("the title {title:?} is on screen"))
                .bounds
                .origin
                .x
                .0
        };
        let edge = table.bounds.origin.x.0 + table.bounds.size.width.0 - left("Nhật ký");
        (edge, left("Symbol") - left("Đóng lúc"), left("Nhật ký") - left("Setup"))
    };
    let (narrow_edge, ..) = measure(1400.0);
    let (wide_edge, time, setup) = measure(1700.0);
    assert!(
        (wide_edge - narrow_edge).abs() < 2.0,
        "the last title is {narrow_edge} points from the edge, then {wide_edge}"
    );
    assert!(narrow_edge < 120.0, "the last column is its own width from the edge: {narrow_edge}");
    assert!(setup < 2.0 * time, "Setup takes {setup} points where the closing time takes {time}");
    assert!(setup > time * 0.5, "the room is shared, not taken from Setup: {setup} against {time}");
}
