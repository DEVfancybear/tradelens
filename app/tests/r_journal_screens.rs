//! A journal kept in R (the Notion export `notion_r_journal.zip`: no money, no lots, no prices, dates
//! without a time of day), walked screen by screen on the real engine. Whatever the source did not tell
//! shows as a dash or as R, never as a placeholder: no `$0.00`, no lot of 1, no 00:00.
//!
//! Also the layout checks that need the boxes the engine laid out: one line where the design has one.

use std::path::{Path, PathBuf};

use nsh_core::{Bounds, Points};
use nsh_testing::{Headless, LayoutNode};
use tradelens_core::{
    Db, TradeKey,
    import::notion,
    model::{AccountInfo, Deal, DealBatch, DealEntry, DealKind},
};

#[expect(dead_code, reason = "this file turns no toggle on")]
mod common;
mod support;
use support::{Fixture, wait_until};

fn core_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/core/tests/fixtures").join(name)
}

/// A database with the five trades of the R fixture, as the import stores them.
fn seed_r_journal(db: &mut Db) {
    let (plan, _) = notion::read_source(&core_fixture("notion_r_journal.zip")).expect("the zip is understood");
    db.import_notion(&plan).expect("imports");
}

fn start_on(fixture: &Fixture, page: &str) -> Headless {
    let mut app = fixture.start(1400.0, 1300.0, |_| {});
    app.press(&format!("role:button[{page}]")).expect("a sidebar row");
    app
}

/// Shows every date, however old.
fn all_time(app: &mut Headless) {
    app.press("role:button[Khoảng thời gian: Tất cả]").expect("the range switch");
}

fn shows_nothing_like(app: &Headless, what: &str) {
    let found: Vec<&str> = app.screen().texts().into_iter().filter(|t| t.contains(what)).collect();
    assert!(found.is_empty(), "no text should contain {what:?}: {found:?}");
}

#[test]
fn the_overview_of_a_journal_in_r_shows_r_never_dollars_and_the_trades_of_its_range() {
    let fixture = Fixture::new();
    seed_r_journal(&mut fixture.db());
    let mut app = fixture.start(1400.0, 1300.0, |_| {});
    all_time(&mut app);
    app.wait_for("Net R").expect("the first tile is in R");
    app.wait_for("+1.4R").expect("the total R of the five trades");
    app.wait_for("Lệnh đã đóng").expect("the last tile counts trades, not stops");
    app.wait_for("R tích luỹ").expect("the chart opens on the R tab, not the empty balance");
    shows_nothing_like(&app, "$");
    shows_nothing_like(&app, "tài khoản MT5");
    app.wait_for("text:/ · 1 tài khoản$/").expect("the header counts accounts, not MT5 accounts");

    // The balance tab has no money to draw: it says so, in place of a flat line at zero.
    app.press("role:button[Biểu đồ: Balance]").expect("the balance tab");
    app.wait_for("Nhật ký này ghi kết quả theo R — xem tab R").expect("the empty state");
    shows_nothing_like(&app, "$");
    app.press("role:button[Biểu đồ: R]").expect("the R tab");
    app.wait_for("R tích luỹ").expect("back on the R curve");
    app.wait_for("Theo symbol").expect("the symbol card");
    app.wait_for("text:/^\\+2\\.0R$/").expect("a symbol's bar is in R");
}

#[test]
fn the_recent_trades_follow_the_range_like_the_figures_do() {
    let fixture = Fixture::new();
    seed_r_journal(&mut fixture.db());
    let mut app = fixture.start(1400.0, 1300.0, |_| {});
    // 30N: the journal is from April, the range reaches back 30 days from today.
    app.wait_for("Chưa có lệnh nào trong khoảng này").expect("the list is empty like the figures are");
    all_time(&mut app);
    app.wait_for("BTCUSD").expect("with every date, the trades come");
    app.wait_for("Hiển thị 5").ok();
}

#[test]
fn a_trade_in_r_shows_dashes_for_what_its_source_never_had() {
    let fixture = Fixture::new();
    seed_r_journal(&mut fixture.db());
    let mut app = start_on(&fixture, "Lệnh");
    app.wait_for("BTCUSD").expect("the list");
    // The lot is not 1.00, the hold time is not 0 minutes, the date has no time of day.
    shows_nothing_like(&app, "1.00");
    shows_nothing_like(&app, "0p");
    shows_nothing_like(&app, "00:00");
    app.press("role:button[Mở nhật ký lệnh BTCUSD] >> nth=0").expect("a trade opens");
    app.wait_for("text:/nguồn: Notion$/").expect("the header names the source, not an internal ticket");
    app.wait_for("BTCUSD · Notion").expect("and so does the page title");
    shows_nothing_like(&app, "Deal của lệnh");
    shows_nothing_like(&app, "Ticket position");
    for fact in ["Khối lượng", "Giá vào", "Giá ra TB", "Stop loss", "Take profit", "Commission", "Swap"] {
        app.wait_for(fact).unwrap_or_else(|e| panic!("{fact}: {e}"));
    }
    shows_nothing_like(&app, "1.00 lot");
    shows_nothing_like(&app, "$");
    shows_nothing_like(&app, "00:00");
    assert!(app.screen().texts().iter().all(|t| !t.contains(" · -")), "no internal negative ticket");
}

#[test]
fn the_plan_prompt_of_a_trade_in_r_shows_no_lot_and_names_its_source() {
    let fixture = Fixture::new();
    seed_r_journal(&mut fixture.db());
    let mut app = start_on(&fixture, "Lệnh");
    app.wait_for("BTCUSD").expect("the list");
    app.press("role:button[Mở nhật ký lệnh BTCUSD] >> nth=0").expect("a trade opens");
    app.wait_for("Chi tiết lệnh").expect("the trade's page");
    // The page draws again once the trade's data is read; a press that lands then is lost.
    common::press_until(&mut app, "role:button[Viết kế hoạch]", |screen| screen.shows("Kế hoạch vào lệnh"));
    app.wait_for("text:/ · Notion$/").expect("the first line names the source of the deals, not a ticket");
    assert!(
        !app.screen().matches("text:/^[0-9.]+ lot/").expect("a selector"),
        "the header says no lot size: the source had none"
    );
    shows_nothing_like(&app, "1.00");
}

#[test]
fn the_calendar_opens_on_the_latest_month_with_trades_and_counts_in_r() {
    let fixture = Fixture::new();
    seed_r_journal(&mut fixture.db());
    let mut app = start_on(&fixture, "Lịch P/L");
    app.wait_for("Tháng 4, 2026").expect("the journal is from April: the calendar opens there");
    app.wait_for("Kết quả tháng").expect("the month summary");
    app.wait_for("+1.4R").expect("the month's result is in R");
    shows_nothing_like(&app, "$");
    app.press("role:button[Hôm nay]").expect("the today button");
    app.wait_for(&format!("Tháng {}, {}", chrono::Local::now().format("%-m"), chrono::Local::now().format("%Y")))
        .expect("it still jumps to today");
}

#[test]
fn the_analysis_of_a_journal_in_r_plots_r_and_leaves_out_trades_with_no_hour() {
    let fixture = Fixture::new();
    seed_r_journal(&mut fixture.db());
    let mut app = start_on(&fixture, "Phân tích");
    all_time(&mut app);
    app.press("label:Phân tích: Thời gian").expect("the time tab");
    app.wait_for("Dữ liệu không có giờ vào lệnh").expect("every trade lacks an hour: no grid of one column");
    app.wait_for("5 lệnh chỉ có ngày").expect("and how many are left out");
    shows_nothing_like(&app, "00:00");
    shows_nothing_like(&app, "$");
    // The weekday bars carry the R of each weekday.
    app.wait_for("Theo thứ trong tuần").expect("the weekday card");
    app.wait_for("text:/^[+\\u2212]\\d\\.\\dR$/").expect("a bar says its R");

    // The tables name their rows and keep the figures inside their card.
    app.wait_for("Theo phiên").expect("the session table");
    app.press("label:Phân tích: Symbol").expect("the symbol tab");
    app.wait_for("BTCUSD").expect("a row is named");
    app.wait_for("Tổng R").expect("the results column is in R");
    let nodes = app.screen().layout_nodes();
    let right_edge = |text: &str| -> Option<f32> {
        nodes.iter().find(|n| n.text.as_ref().is_some_and(|t| t.text.text == text)).map(|n| n.bounds.right().0)
    };
    let card = nodes
        .iter()
        .filter(|n| n.text.is_none() && n.bounds.size.width.0 > 250.0 && n.bounds.size.width.0 < 700.0)
        .find(|n| {
            nodes
                .iter()
                .any(|m| m.text.as_ref().is_some_and(|t| t.text.text == "BTCUSD") && contains(&n.bounds, &m.bounds))
        })
        .expect("the card holding the row");
    let r_title = right_edge("R TB").expect("the last title");
    assert!(
        r_title <= card.bounds.right().0 + 0.5,
        "the last column ends inside its card: {r_title} vs {:?}",
        card.bounds
    );
}

fn contains(outer: &Bounds<Points>, inner: &Bounds<Points>) -> bool {
    inner.origin.x >= outer.origin.x && inner.right() <= outer.right() && inner.origin.y >= outer.origin.y
}

/// The text view showing exactly `text`.
fn text_node(app: &Headless, text: &str) -> LayoutNode {
    app.screen()
        .layout_nodes()
        .into_iter()
        .find(|n| n.text.as_ref().is_some_and(|t| t.text.text == text))
        .unwrap_or_else(|| panic!("no text view shows {text:?}"))
}

#[test]
fn the_search_box_of_the_sidebar_is_one_line_with_its_key_beside_the_placeholder() {
    let fixture = Fixture::new();
    let mut app = fixture.start(1280.0, 800.0, |_| {});
    app.wait_for("Tìm trong nhật ký").expect("the sidebar search");
    let placeholder = text_node(&app, "Tìm trong nhật ký");
    // One line of 14-point text is about 20 points tall; two would be about 40.
    let height = placeholder.text.as_ref().expect("a text view").content.size.height.0;
    assert!(height <= 24.0, "the placeholder wraps onto several lines: {height} points tall");
    let nodes = app.screen().layout_nodes();
    let key = nodes
        .iter()
        .find(|n| n.text.as_ref().is_some_and(|t| t.text.text.contains('K') && t.text.text.len() <= 8))
        .expect("the key that opens it");
    assert!(
        key.bounds.origin.x.0 >= placeholder.bounds.right().0 - 0.5,
        "the key sits beside the placeholder, not over it: {:?} vs {:?}",
        key.bounds,
        placeholder.bounds
    );
    let trigger =
        nodes.iter().find(|n| n.parent == placeholder.parent && n.text.is_none() && n.bounds.size.height.0 > 20.0);
    if let Some(trigger) = trigger {
        assert!(trigger.bounds.size.height.0 <= 36.0, "a field of one line: {:?}", trigger.bounds);
    }
}

#[test]
fn a_property_name_longer_than_its_column_wraps_inside_it_and_its_chip_stays_beside_the_editor() {
    let fixture = Fixture::new();
    let mut db = fixture.db();
    seed_r_journal(&mut db);
    drop(db);
    let mut app = start_on(&fixture, "Lệnh");
    app.wait_for("BTCUSD").expect("the list");
    app.press("role:button[Mở nhật ký lệnh BTCUSD] >> nth=0").expect("a trade opens");
    app.wait_for("riêng").expect("the imported properties carry the badge");
    let nodes = app.screen().layout_nodes();
    for badge in nodes.iter().filter(|n| n.text.as_ref().is_some_and(|t| t.text.text == "riêng")) {
        // The column of names is 168 points wide: nothing of a name column goes past it.
        let row = nodes.iter().find(|n| n.id == badge.parent).expect("the chip's parent");
        assert!(
            badge.bounds.right().0 <= row.bounds.right().0 + 0.5 || row.bounds.size.width.0 <= 1.0,
            "a chip outside its column: {:?} in {:?}",
            badge.bounds,
            row.bounds
        );
    }
}

#[test]
fn a_sync_that_cannot_run_says_what_to_do() {
    let fixture = Fixture::new();
    let mut app = start_on(&fixture, "Đăng nhập & sync");
    app.wait_for("Đăng nhập Google Drive hoặc OneDrive để đồng bộ").expect("the hint under a disabled button");
    assert_eq!(app.screen().enabled("role:button[Sync ngay]"), Ok(false));
}

/// One closed trade, the way an import leaves it.
fn one_trade(db: &mut Db) -> TradeKey {
    let deal = |ticket: i64, entry: DealEntry, kind: DealKind, at: i64, profit: f64| Deal {
        ticket,
        order: ticket,
        position_id: 7,
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
    };
    let now = chrono::Utc::now().timestamp_millis();
    let batch = DealBatch {
        account: AccountInfo {
            login: "1".into(),
            server: "Demo".into(),
            currency: "USD".into(),
            ..AccountInfo::default()
        },
        deals: vec![
            deal(10, DealEntry::In, DealKind::Buy, now - 7_200_000, 0.0),
            deal(11, DealEntry::Out, DealKind::Sell, now - 3_600_000, 25.0),
        ],
        positions: Vec::new(),
    };
    db.upsert_batch(&batch, "test").expect("stored");
    let account = db.accounts().expect("accounts").remove(0);
    TradeKey { account_id: account.id, position_id: 7, segment: 0 }
}

#[test]
fn a_trade_with_money_keeps_all_its_facts_and_its_dollars() {
    let fixture = Fixture::new();
    one_trade(&mut fixture.db());
    let mut app = fixture.start(1400.0, 1300.0, |_| {});
    let db = fixture.db();
    wait_until("the trade being stored", || db.deals(None).is_ok_and(|d| d.len() == 2));
    app.wait_for("Net P/L").expect("a journal with money keeps its money tile");
    app.wait_for("+$25.00").expect("and its amount");
    app.wait_for("R trung bình").expect("and the stop tile");
}
