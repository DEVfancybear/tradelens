//! The trades list, a trade's journal and the overview, headless on the real engine over a
//! database in a temporary data directory.

mod common;

use nsh_testing::Headless;
use tradelens_core::{Db, Deal, DealBatch, DealEntry, DealKind, TradeKey, model::AccountInfo};

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
        self.0.start(1280.0, 900.0, |_| {})
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

/// A closed trade as an entry and an exit `hours` apart, ending `days_ago` days ago.
fn closed_trade(position: i64, symbol: &str, days_ago: i64, profit: f64) -> Vec<Deal> {
    let exit = now_ms() - days_ago * DAY_MS;
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

/// Three closed trades: EURUSD +100, XAUUSD −40, GBPJPY +20 (net +80).
fn seed(db: &mut Db) {
    let mut deals = closed_trade(1, "EURUSD", 3, 100.0);
    deals.extend(closed_trade(2, "XAUUSD", 2, -40.0));
    deals.extend(closed_trade(3, "GBPJPY", 1, 20.0));
    db.upsert_batch(&batch(deals), "test").expect("the batch is stored");
}

fn open_trades(app: &mut Headless) {
    common::press_until(app, "role:button[Lệnh]", |screen| {
        screen.shows("Gom từ các deal theo position; bấm một lệnh để mở nhật ký")
    });
}

fn trade_button(symbol: &str) -> String {
    format!("role:button[Mở nhật ký lệnh {symbol}]")
}

fn key_of(db: &Db, position: i64) -> TradeKey {
    let account = db.accounts().expect("accounts read").remove(0);
    TradeKey { account_id: account.id, position_id: position, segment: 0 }
}

#[test]
fn the_trades_page_lists_the_trades_of_the_database() {
    let fixture = Fixture::new();
    seed(&mut fixture.db());
    let mut app = fixture.start();
    open_trades(&mut app);
    for symbol in ["EURUSD", "XAUUSD", "GBPJPY"] {
        app.wait_for(&trade_button(symbol)).expect("each trade has a row");
    }
    app.wait_for("Hiển thị 3 / 3 lệnh").expect("the footer counts the trades");
    assert!(app.screen().shows("+$100.00"), "a gain shows with its sign");
    assert!(app.screen().shows("\u{2212}$40.00"), "a loss shows with its sign");
}

#[test]
fn an_empty_database_says_so() {
    let fixture = Fixture::new();
    let mut app = fixture.start();
    open_trades(&mut app);
    app.wait_for("Chưa có lệnh nào").expect("the empty state shows");
}

#[test]
fn filtering_narrows_the_list_and_clearing_brings_it_back() {
    let fixture = Fixture::new();
    seed(&mut fixture.db());
    let mut app = fixture.start();
    open_trades(&mut app);
    app.wait_for(&trade_button("GBPJPY")).expect("the rows load");

    app.type_text("label:Tìm lệnh", "xau").expect("the search box takes text");
    app.wait_for_gone(&trade_button("GBPJPY"), WAIT).expect("GBPJPY is filtered out");
    app.wait_for("Hiển thị 1 / 3 lệnh").expect("the footer follows the filter");
    app.wait_for(&trade_button("XAUUSD")).expect("XAUUSD stays");

    app.press("role:button[Xoá lọc]").expect("a filter can be cleared");
    app.wait_for(&trade_button("GBPJPY")).expect("the list is whole again");

    app.press("role:button[Kết quả: Thua]").expect("the outcome switch is a button");
    app.wait_for("Hiển thị 1 / 3 lệnh").expect("only the loser is left");
    app.wait_for(&trade_button("XAUUSD")).expect("the loser is XAUUSD");
}

#[test]
fn a_trades_journal_is_written_and_saved() {
    let fixture = Fixture::new();
    seed(&mut fixture.db());
    let mut app = fixture.start();
    open_trades(&mut app);
    app.wait_for_within(&trade_button("XAUUSD"), WAIT).expect("the trades load");
    app.press(&trade_button("XAUUSD")).expect("a trade opens");
    app.wait_for("Nhật ký lệnh").expect("the journal card shows");
    app.wait_for("Deal của lệnh").expect("the trade's deals show");

    app.type_text("label:Setup", "Breakout phiên Á").expect("the setup is typed");
    common::toggle_on(&mut app, "role:button[Tag: Tin tức]");
    common::toggle_on(&mut app, "role:button[Lỗi: Vào sớm]");
    app.press("role:button[Điểm thực thi: 4 sao]").expect("a grade is given");
    app.type_text("label:Ghi chú", "Vào sớm vì sợ lỡ").expect("notes are typed");
    app.wait_for("Có thay đổi chưa lưu").expect("the page notices the edits");

    app.press("role:button[Lưu]").expect("the save button works");
    let db = fixture.db();
    let key = key_of(&db, 2);
    wait_until("the journal being saved", || db.journal(&key).expect("the journal reads").is_some());
    let entry = db.journal(&key).expect("the journal reads").expect("an entry was saved");
    assert_eq!(entry.setup, "Breakout phiên Á");
    assert_eq!(entry.tags, ["news"]);
    assert_eq!(entry.mistakes, ["early_entry"]);
    assert_eq!(entry.rating, 4);
    assert_eq!(entry.notes, "Vào sớm vì sợ lỡ");
    app.wait_for("Đã lưu").expect("the page says it is saved");

    // Back on the list the trade reads as journalled.
    app.press("role:button[‹ Lệnh]").expect("back to the list");
    app.wait_for("Đã ghi").expect("the journal column follows the saved entry");
    app.wait_for("Breakout phiên Á").expect("the setup shows in the table");
}

#[test]
fn leaving_a_trade_with_edits_saves_them() {
    let fixture = Fixture::new();
    seed(&mut fixture.db());
    let mut app = fixture.start();
    open_trades(&mut app);
    app.wait_for_within(&trade_button("GBPJPY"), WAIT).expect("the trades load");
    app.press(&trade_button("GBPJPY")).expect("a trade opens");
    app.wait_for("Nhật ký lệnh").expect("the journal card shows");
    app.type_text("label:Ghi chú", "chốt đúng kế hoạch").expect("notes are typed");
    app.press("role:button[‹ Lệnh]").expect("back to the list");
    app.wait_for("Hiển thị 3 / 3 lệnh").expect("the list is back");

    let db = fixture.db();
    let key = key_of(&db, 3);
    wait_until("the notes being saved on leaving", || {
        db.journal(&key).expect("the journal reads").is_some_and(|e| e.notes == "chốt đúng kế hoạch")
    });
}

#[test]
fn the_overview_shows_the_figures_and_the_latest_trades() {
    let fixture = Fixture::new();
    seed(&mut fixture.db());
    let mut app = fixture.start();
    app.wait_for("+$80.00").expect("net profit is shown with its sign");
    app.wait_for("66.7%").expect("the win rate counts two of three");
    app.wait_for("3.00").expect("the profit factor is 120 over 40");
    app.wait_for("2 thắng · 1 thua").expect("the tile's note counts wins and losses");
    app.wait_for("Lệnh gần đây").expect("the latest trades card shows");
    app.wait_for(&trade_button("GBPJPY")).expect("the latest trade is listed");
}

#[test]
fn a_range_without_trades_says_the_curve_is_missing() {
    let fixture = Fixture::new();
    // A trade a year ago: outside every range but "Tất cả".
    fixture.db().upsert_batch(&batch(closed_trade(1, "EURUSD", 400, 50.0)), "test").expect("stored");
    let mut app = fixture.start();
    app.wait_for("Chưa có đủ lệnh đóng để vẽ đường số dư").expect("the 30 day curve is empty");
    app.press("role:button[Khoảng thời gian: Tất cả]").expect("the range switch is a button");
    app.wait_for("+$50.00").expect("the whole history counts the old trade");
}

#[test]
fn a_trade_stored_elsewhere_appears_after_a_revision_bump() {
    let fixture = Fixture::new();
    seed(&mut fixture.db());
    let mut app = fixture.start();
    open_trades(&mut app);
    app.wait_for("Hiển thị 3 / 3 lệnh").expect("the trades load");

    // Another connection writes, as an import does, then the revision moves. The store looks at the
    // revision every 400 ms (`store::POLL`), so the page follows well inside the 5 seconds the
    // acceptance criterion allows; the test does not time that on a machine that may be busy.
    fixture.db().upsert_batch(&batch(closed_trade(4, "USDJPY", 0, 15.0)), "test").expect("stored");
    nsh_app_tradelens::data_changed();
    app.wait_for_within(&trade_button("USDJPY"), WAIT).expect("the new trade appears");
    app.wait_for("Hiển thị 4 / 4 lệnh").expect("the footer counts it");
}
