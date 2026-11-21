//! The trades list, a trade's journal and the overview, headless on the real engine over a
//! database in a temporary data directory.

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use nsh_core::{Size, prelude::px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use tradelens_core::{Db, Deal, DealBatch, DealEntry, DealKind, TradeKey, model::AccountInfo};

const DAY_MS: i64 = 86_400_000;

/// A data directory with a database in it, and the engine's view of it.
struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut db = Db::open(dir.path().join("tradelens.db")).expect("the database opens");
        // An ephemeral port, so no test claims the bridge's default one.
        db.set_setting("bridge_port", "0").expect("the port is stored");
        Self { dir }
    }

    fn db_path(&self) -> PathBuf {
        self.dir.path().join("tradelens.db")
    }

    fn db(&self) -> Db {
        Db::open(self.db_path()).expect("the database opens")
    }

    /// Starts the app on a desktop-sized window with this fixture as its data directory.
    fn start(&self) -> Headless {
        let config = EngineConfig {
            viewport: Size::new(px(1280.0), px(900.0)),
            data_dir: Some(self.dir.path().to_path_buf()),
            ..EngineConfig::new(Root::of::<nsh_app_tradelens::Workspace>())
        };
        let mut app = Headless::start(config).expect("the app starts");
        app.wait_for("Tổng quan").expect("the app opens");
        app
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
    app.press("role:button[Lệnh]").expect("the trades row is a button");
    app.wait_for("Gom từ deal MT5 theo position; bấm một lệnh để mở nhật ký").expect("the trades page opens");
}

fn trade_button(symbol: &str) -> String {
    format!("role:button[Mở nhật ký lệnh {symbol}]")
}

/// Waits until `check` holds, polling at most `within`.
fn wait_for_database(what: &str, within: Duration, check: impl Fn() -> bool) {
    let started = Instant::now();
    while !check() {
        assert!(started.elapsed() < within, "{what} did not happen within {within:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
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
    app.wait_for_gone(&trade_button("GBPJPY"), Duration::from_secs(5)).expect("GBPJPY is filtered out");
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
    app.press(&trade_button("XAUUSD")).expect("a trade opens");
    app.wait_for("Nhật ký lệnh").expect("the journal card shows");
    app.wait_for("Deal của lệnh").expect("the trade's deals show");

    app.type_text("label:Setup", "Breakout phiên Á").expect("the setup is typed");
    app.press("role:button[Tag: Tin tức]").expect("a tag toggles");
    app.press("role:button[Lỗi: Vào sớm]").expect("a mistake toggles");
    app.press("role:button[Điểm thực thi: 4 sao]").expect("a grade is given");
    app.type_text("label:Ghi chú", "Vào sớm vì sợ lỡ").expect("notes are typed");
    app.wait_for("Có thay đổi chưa lưu").expect("the page notices the edits");

    app.press("role:button[Lưu]").expect("the save button works");
    let db = fixture.db();
    let key = key_of(&db, 2);
    wait_for_database("the journal being saved", Duration::from_secs(5), || {
        db.journal(&key).expect("the journal reads").is_some()
    });
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
    app.press(&trade_button("GBPJPY")).expect("a trade opens");
    app.wait_for("Nhật ký lệnh").expect("the journal card shows");
    app.type_text("label:Ghi chú", "chốt đúng kế hoạch").expect("notes are typed");
    app.press("role:button[‹ Lệnh]").expect("back to the list");
    app.wait_for("Hiển thị 3 / 3 lệnh").expect("the list is back");

    let db = fixture.db();
    let key = key_of(&db, 3);
    wait_for_database("the notes being saved on leaving", Duration::from_secs(5), || {
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
fn a_trade_the_bridge_stores_appears_after_a_revision_bump() {
    let fixture = Fixture::new();
    seed(&mut fixture.db());
    let mut app = fixture.start();
    open_trades(&mut app);
    app.wait_for("Hiển thị 3 / 3 lệnh").expect("the trades load");

    // Another connection writes, as the bridge does, then the revision moves.
    fixture.db().upsert_batch(&batch(closed_trade(4, "USDJPY", 0, 15.0)), "bridge").expect("stored");
    let stored_at = Instant::now();
    nsh_app_tradelens::data_changed();
    app.wait_for_within(&trade_button("USDJPY"), Duration::from_secs(5)).expect("the new trade appears");
    app.wait_for("Hiển thị 4 / 4 lệnh").expect("the footer counts it");
    assert!(stored_at.elapsed() < Duration::from_secs(5), "within the 5 seconds of the acceptance criterion");
}

#[test]
fn the_data_sources_page_shows_the_bridge_state_and_token() {
    let fixture = Fixture::new();
    let mut app = fixture.start();
    app.press("role:button[Nguồn dữ liệu]").expect("the data sources row is a button");
    app.wait_for("Bridge MT5").expect("the bridge card shows");
    // Until the bridge's server exists it cannot start; with it, it listens.
    app.wait_until("the bridge state", Duration::from_secs(5), |screen| {
        screen.shows("Lý do: bridge chưa có trong bản này")
            || screen.texts().iter().any(|t| t.starts_with("Bridge MT5: đang nghe cổng"))
    })
    .expect("the page says whether the bridge listens");
    let token = app.screen().texts().into_iter().find(|t| t.len() == 32 && t.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(token.is_some(), "the token the EA must send is shown");
}
