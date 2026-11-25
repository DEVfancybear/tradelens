//! The calendar, the analytics and the overview's discipline block, headless on the real engine
//! over a database in a temporary data directory.

use chrono::Datelike;
use nsh_testing::Headless;
use tradelens_core::{
    Db, Deal, DealBatch, DealEntry, DealKind, JournalEntry, Rules, TradeKey, model::AccountInfo, time::parse_mt5_time,
};

#[expect(dead_code, reason = "this file waits on the screen only, never on a background write")]
mod support;
use support::WAIT;

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
        self.0.start(1280.0, 1000.0, |_| {})
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

fn deal(ticket: i64, position: i64, symbol: &str, at_ms: i64, entry: DealEntry, profit: f64) -> Deal {
    Deal {
        ticket,
        order: ticket,
        position_id: position,
        time_msc: at_ms,
        symbol: symbol.into(),
        kind: if entry == DealEntry::In { DealKind::Buy } else { DealKind::Sell },
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

/// A closed trade that exits at `exit_ms` an hour after it entered.
fn closed_at(position: i64, symbol: &str, exit_ms: i64, profit: f64) -> Vec<Deal> {
    vec![
        deal(position * 10, position, symbol, exit_ms - 3_600_000, DealEntry::In, 0.0),
        deal(position * 10 + 1, position, symbol, exit_ms, DealEntry::Out, profit),
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

fn key_of(db: &Db, position: i64) -> TradeKey {
    let account = db.accounts().expect("accounts read").remove(0);
    TradeKey { account_id: account.id, position_id: position, segment: 0 }
}

fn trade_button(symbol: &str) -> String {
    format!("role:button[Mở nhật ký lệnh {symbol}]")
}

/// Two trades on 10 March 2026 (EURUSD +100, XAUUSD −40) and one on the 11th (GBPJPY +20).
fn seed_march(db: &mut Db) {
    let at = |s: &str| parse_mt5_time(s).expect("fixture times parse");
    let mut deals = closed_at(1, "EURUSD", at("2026.03.10 10:00:00"), 100.0);
    deals.extend(closed_at(2, "XAUUSD", at("2026.03.10 15:00:00"), -40.0));
    deals.extend(closed_at(3, "GBPJPY", at("2026.03.11 09:00:00"), 20.0));
    db.upsert_batch(&batch(deals), "test").expect("the batch is stored");
}

#[test]
fn the_calendar_shows_a_days_result_and_moves_between_months() {
    let fixture = Fixture::new();
    seed_march(&mut fixture.db());
    let mut app = fixture.start();
    app.press("role:button[Lịch P/L]").expect("the calendar row is a button");
    // The current month has no trades: the calendar opens on the latest month that has some.
    app.wait_for("Tháng 3, 2026").expect("it opens on March, the month of the latest trades");
    app.wait_for("Thứ 4, 11/03").expect("with the latest day of trades picked");
    // "Hôm nay" still goes to today's month, and back to March by the buttons.
    let now = chrono::Local::now();
    app.press("role:button[Hôm nay]").expect("the today button");
    app.wait_for(&format!("Tháng {}, {}", now.month(), now.year())).expect("it jumps to the current month");
    let back = (now.year() - 2026) * 12 + i32::try_from(now.month()).expect("a month fits") - 3;
    for _ in 0..back {
        app.press("role:button[Tháng trước]").expect("the previous month button");
    }
    app.wait_for("Tháng 3, 2026").expect("the month moved back to March");
    app.wait_for("+$60").expect("the 10th shows its net result in whole dollars");
    app.wait_for("1/2 lệnh thắng").expect("and how many of its trades won");
    app.wait_for("+$80.00").expect("the month's total is the three trades");
    app.wait_for("Ngày lãi / lỗ").expect("the month's summary lists the days");
    app.wait_for("2 / 0").expect("both days were green");

    app.press("role:button[Xem ngày 10/03]").expect("a day is a button");
    app.wait_for("Thứ 3, 10/03").expect("the day card names the day");
    app.wait_for(&trade_button("EURUSD")).expect("the day's first trade is listed");
    app.wait_for(&trade_button("XAUUSD")).expect("and its second");
    assert!(
        !app.screen().texts().iter().any(|t| t.contains("GBPJPY")),
        "the trade of the 11th is not in the 10th's list"
    );

    app.press("role:button[Tháng sau]").expect("the next month button");
    app.wait_for("Tháng 4, 2026").expect("the month moved on");
}

#[test]
fn the_calendar_of_an_empty_database_still_draws_the_month() {
    let fixture = Fixture::new();
    let mut app = fixture.start();
    app.press("role:button[Lịch P/L]").expect("the calendar row is a button");
    app.wait_for("Kết quả tháng").expect("the month summary shows");
    app.wait_for("Không có lệnh đóng").expect("the selected day says it has no trades");
}

/// Trades of the last days: EURUSD +100 (setup Breakout), XAUUSD −40 (mistake: early entry), GBPJPY +20.
fn seed_recent(db: &mut Db) {
    let mut deals = closed_at(1, "EURUSD", now_ms() - 3 * DAY_MS, 100.0);
    deals.extend(closed_at(2, "XAUUSD", now_ms() - 2 * DAY_MS, -40.0));
    deals.extend(closed_at(3, "GBPJPY", now_ms() - DAY_MS, 20.0));
    db.upsert_batch(&batch(deals), "test").expect("the batch is stored");
    let winner = JournalEntry { setup: "Breakout".into(), ..JournalEntry::default() };
    db.save_journal(&key_of(db, 1), &winner).expect("saves");
    let loser = JournalEntry { mistakes: vec!["early_entry".into()], ..JournalEntry::default() };
    db.save_journal(&key_of(db, 2), &loser).expect("saves");
}

#[test]
fn the_analytics_page_breaks_results_down_and_prices_the_mistakes() {
    let fixture = Fixture::new();
    seed_recent(&mut fixture.db());
    let mut app = fixture.start();
    app.press("role:button[Phân tích]").expect("the analytics row is a button");
    app.wait_for("Chi phí của lỗi").expect("the mistake cost card shows");
    app.wait_for("Vào sớm").expect("the mistake is named by the trader's catalog");
    app.wait_for("\u{2212}$40.00").expect("the loss it cost shows with its sign");
    app.wait_for("1 lệnh").expect("and how many trades carried it");
    app.wait_for("Theo phiên").expect("the sessions table shows");

    app.press("role:tab[Phân tích: Symbol]").expect("the symbol tab");
    app.wait_for("Theo symbol").expect("the symbol breakdown shows");
    app.wait_for("EURUSD").expect("a symbol has its row");
    app.wait_for("+$100.00").expect("with its net result");
    app.wait_for("100.0%").expect("and its win rate");

    app.press("role:tab[Phân tích: Setup & tag]").expect("the setup tab");
    app.wait_for("Breakout").expect("the setup the trader journalled shows");

    app.press("role:tab[Phân tích: Rủi ro (R)]").expect("the risk tab");
    app.wait_for("Phân phối R").expect("the R distribution shows");
    app.wait_for("Chưa có lệnh nào đo được R: đặt SL hoặc ghi rủi ro dự định.")
        .expect("trades without a stop have no R to show");
}

#[test]
fn the_analytics_page_of_an_empty_range_says_so() {
    let fixture = Fixture::new();
    let mut app = fixture.start();
    app.press("role:button[Phân tích]").expect("the analytics row is a button");
    app.wait_for("Chưa có lệnh nào gắn lỗi").expect("the mistake card is empty");
    app.press("role:tab[Phân tích: Symbol]").expect("the symbol tab");
    app.wait_for("Chưa có lệnh nào trong khoảng này").expect("the table is empty");
}

#[test]
fn the_overview_shows_todays_discipline_the_month_and_the_symbols() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    let mut deals = closed_at(1, "EURUSD", now_ms(), 30.0);
    deals.extend(closed_at(2, "XAUUSD", now_ms(), -50.0));
    db.upsert_batch(&batch(deals), "test").expect("the batch is stored");
    // A server on UTC: the times above are the clock's own, so they are today on the server's day
    // whatever the machine's zone is. Without an offset the page uses the machine's day, which
    // between its midnight and UTC's is another day than the one these trades closed on.
    let mut account = db.accounts().expect("accounts read").remove(0);
    account.server_offset_min = Some(0);
    db.update_account(&account).expect("the server offset is stored");
    let account = account.id;
    let rules = Rules { max_daily_loss: Some(300.0), max_daily_trades: Some(5), ..Rules::default() };
    db.save_rules(account, &rules).expect("the rules are stored");
    drop(db);

    let mut app = fixture.start();
    app.wait_for("Kỷ luật hôm nay").expect("the discipline card shows");
    app.wait_for_within("2 / 5", WAIT).expect("two of five trades today");
    app.wait_for("Số lệnh").expect("the trade count rule is named");
    app.wait_for("$20 / $300").expect("today lost 20 of the 300 allowed");
    app.wait_for("0 vi phạm").expect("nothing was broken");
    app.wait_for(&chrono::Local::now().format("Tháng %-m/%Y").to_string())
        .expect("the small month is titled by its month");
    app.wait_for("Theo symbol").expect("the symbol card shows");
    app.wait_for("+$30.00").expect("a symbol's net result shows with its sign");

    app.press("role:button[Biểu đồ: R]").expect("the chart can show R");
    app.wait_for("R tích luỹ").expect("the card follows the tab");
    app.wait_for("Chưa có lệnh nào đo được R").expect("these trades have no stop");
}

#[test]
fn without_rules_the_overview_offers_to_set_them() {
    let fixture = Fixture::new();
    let mut app = fixture.start();
    app.wait_for("Chưa đặt quy tắc kỷ luật").expect("the discipline card says there are no rules");
    app.press("role:button[Đặt quy tắc]").expect("the button leads to the rules");
    app.wait_for("Tài khoản & kỷ luật").expect("the accounts page is current");
}
