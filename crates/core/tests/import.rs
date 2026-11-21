//! The MT5 report importers against the fixtures in `tests/fixtures/` (written by
//! `gen_fixtures.rs` from the scenario in `common`): the deals, the account, the stops and
//! targets, the trades the deals make and the report's own totals.

mod common;

use std::fs;
use std::path::PathBuf;

use common::{
    CLOSED, CLOSED_TRADES, COMPANY, CURRENCY, Closed, EVENTS, FINAL_BALANCE, LOGIN, NAME, REPORT_DATE, SERVER,
    TOTAL_NET_PROFIT,
};
use tradelens_core::import::{self, Format, ImportError, mt5_html};
use tradelens_core::metrics::{self, Sample};
use tradelens_core::model::{Deal, DealBatch, DealEntry, DealKind, Side, Trade};
use tradelens_core::time::{Clock, parse_mt5_time};
use tradelens_core::trades::{build_trades, cash_flows};

/// The English report as the terminal saves it: UTF-16 LE with a BOM.
const ENGLISH_HTML: &str = "mt5_en_hedge.html";
/// The same report with typed cells: numbers, and real Excel date-times.
const ENGLISH_XLSX: &str = "mt5_en_hedge.xlsx";
/// The same report with every cell written as text.
const ENGLISH_XLSX_TEXT: &str = "mt5_en_hedge_text.xlsx";
/// The same report with Russian titles (UTF-16 LE).
const RUSSIAN_HTML: &str = "mt5_ru_hedge.html";
/// The same report with Vietnamese titles, UTF-8 and sloppy uppercase markup.
const VIETNAMESE_HTML: &str = "mt5_vi_hedge.html";

const ALL: [&str; 5] = [ENGLISH_HTML, ENGLISH_XLSX, ENGLISH_XLSX_TEXT, RUSSIAN_HTML, VIETNAMESE_HTML];

/// The GBPUSD position that pays a fee, held over two nights.
const GBPUSD_POSITION: i64 = 91_000_006;
/// The EURUSD position closed in two halves.
const PARTIAL_POSITION: i64 = 91_000_001;

fn path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
}

fn load(name: &str) -> DealBatch {
    let (format, batch) = import::read_file(&path(name)).unwrap_or_else(|e| panic!("{name} should import: {e}"));
    let expected = if matches!(name, ENGLISH_XLSX | ENGLISH_XLSX_TEXT) { Format::Mt5Xlsx } else { Format::Mt5Html };
    assert_eq!(format, expected, "format of {name}");
    batch
}

fn time(at: &str) -> i64 {
    parse_mt5_time(at).expect("scenario times parse")
}

#[track_caller]
fn assert_close(got: f64, want: f64, what: &str) {
    assert!((got - want).abs() < 1e-9, "{what}: got {got}, want {want}");
}

fn net(deal: &Deal) -> f64 {
    deal.profit + deal.commission + deal.swap + deal.fee
}

fn levels_of(position: i64) -> (f64, f64) {
    CLOSED.iter().find(|c| c.position == position).map_or((0.0, 0.0), |c| (c.sl, c.tp))
}

#[test]
fn every_fixture_reads_the_deals_of_the_scenario() {
    for name in ALL {
        let batch = load(name);
        assert_eq!(batch.deals.len(), EVENTS.len(), "deal count of {name}");
        for (deal, event) in batch.deals.iter().zip(&EVENTS) {
            let what = format!("{name} deal {}", event.ticket);
            assert_eq!(deal.ticket, event.ticket, "{what}: ticket");
            assert_eq!(deal.order, event.order, "{what}: order");
            assert_eq!(deal.position_id, event.position, "{what}: position");
            assert_eq!(deal.time_msc, time(event.at), "{what}: time");
            assert_eq!(deal.symbol, event.symbol, "{what}: symbol");
            assert_eq!(deal.kind, DealKind::parse(event.kind), "{what}: type");
            let entry = if event.entry.is_empty() { DealEntry::None } else { DealEntry::parse(event.entry) };
            assert_eq!(deal.entry, entry, "{what}: direction");
            assert_close(deal.volume, event.volume, &format!("{what}: volume"));
            assert_close(deal.price, event.price, &format!("{what}: price"));
            assert_close(deal.commission, event.commission, &format!("{what}: commission"));
            assert_close(deal.fee, event.fee, &format!("{what}: fee"));
            assert_close(deal.swap, event.swap, &format!("{what}: swap"));
            assert_close(deal.profit, event.profit, &format!("{what}: profit"));
            assert_eq!(deal.comment, event.comment, "{what}: comment");
            assert_eq!((deal.magic, deal.risk), (0, None), "{what}: the report has no magic or risk");
            let reason = match event.comment.get(..3) {
                Some("[sl") => Some("sl"),
                Some("[tp") => Some("tp"),
                _ => None,
            };
            assert_eq!(deal.reason.as_deref(), reason, "{what}: reason");
        }
    }
}

#[test]
fn the_scenario_covers_the_deal_kinds_the_importers_must_tell_apart() {
    let batch = load(ENGLISH_HTML);
    let count = |keep: &dyn Fn(&Deal) -> bool| batch.deals.iter().filter(|d| keep(d)).count();
    assert_eq!(count(&|d| d.kind == DealKind::Balance), 2, "a deposit and a withdrawal");
    assert_eq!(count(&|d| d.entry == DealEntry::OutBy), 2, "a close-by");
    assert_eq!(count(&|d| d.reason.as_deref() == Some("sl")), 1, "a stop-out of a loser");
    assert_eq!(count(&|d| d.reason.as_deref() == Some("tp")), 2, "two targets");
    assert_eq!(count(&|d| d.position_id == PARTIAL_POSITION), 3, "one entry and two partial exits");
}

#[test]
fn the_account_header_is_read_in_every_language() {
    for name in ALL {
        let account = load(name).account;
        assert_eq!(account.login, LOGIN, "{name}");
        assert_eq!(account.currency, CURRENCY, "{name}");
        assert_eq!(account.server, SERVER, "{name}");
        assert_eq!(account.broker, COMPANY, "{name}");
        assert_eq!(account.name, NAME, "{name}");
        assert_eq!(account.server_offset_min, None, "{name}: a report does not say the server's offset");
        assert_close(account.balance.expect("the last Balance cell"), FINAL_BALANCE, &format!("{name}: balance"));
    }
}

#[test]
fn stops_and_targets_come_from_the_positions_table() {
    let mut wanted: Vec<(i64, f64, f64)> =
        CLOSED.iter().filter(|c| c.sl > 0.0 || c.tp > 0.0).map(|c| (c.position, c.sl, c.tp)).collect();
    wanted.sort_by_key(|w| w.0);
    assert_eq!(wanted.len(), 5, "five positions have a stop or a target");
    for name in ALL {
        let batch = load(name);
        let mut got: Vec<(i64, f64, f64)> = batch.positions.iter().map(|p| (p.position_id, p.sl, p.tp)).collect();
        got.sort_by_key(|g| g.0);
        assert_eq!(got.len(), wanted.len(), "{name}: positions with levels");
        for (got, want) in got.iter().zip(&wanted) {
            assert_eq!(got.0, want.0, "{name}: position");
            assert_close(got.1, want.1, &format!("{name} {}: sl", want.0));
            assert_close(got.2, want.2, &format!("{name} {}: tp", want.0));
        }
        // The entry deal carries them too, the way the EA sends them.
        for deal in batch.deals.iter().filter(|d| d.entry == DealEntry::In) {
            let (sl, tp) = levels_of(deal.position_id);
            assert_close(deal.sl, sl, &format!("{name} deal {}: sl", deal.ticket));
            assert_close(deal.tp, tp, &format!("{name} deal {}: tp", deal.ticket));
        }
    }
}

#[test]
fn html_and_xlsx_of_the_same_report_make_equal_batches() {
    let html = load(ENGLISH_HTML);
    for name in [ENGLISH_XLSX, ENGLISH_XLSX_TEXT, RUSSIAN_HTML, VIETNAMESE_HTML] {
        assert!(load(name) == html, "{name} reads differently from {ENGLISH_HTML}");
    }
}

#[test]
fn the_deals_net_to_the_reports_total_net_profit() {
    let printed = mt5_html::decode(&fs::read(path(ENGLISH_HTML)).expect("the fixture exists"));
    assert!(printed.contains(&format!("{TOTAL_NET_PROFIT:.2}")), "the report prints its Total Net Profit");
    assert!(printed.contains(REPORT_DATE), "the report prints the date it was made");
    for name in ALL {
        let total: f64 = load(name).deals.iter().filter(|d| d.kind.is_trade()).map(net).sum();
        assert_close(total, TOTAL_NET_PROFIT, &format!("{name}: sum of net profit"));
    }
}

fn trade_of(trades: &[Trade], position: i64) -> &Trade {
    trades.iter().find(|t| t.key.position_id == position).unwrap_or_else(|| panic!("no trade for {position}"))
}

#[test]
fn the_deals_make_the_trades_of_the_report() {
    for name in ALL {
        let batch = load(name);
        let trades = build_trades(1, &batch.deals);
        assert_eq!(trades.len(), CLOSED_TRADES, "{name}: trades");
        assert!(trades.iter().all(Trade::is_closed), "{name}: every trade is closed");
        for closed in &CLOSED {
            check_trade(name, closed, trade_of(&trades, closed.position));
        }
        let total: f64 = trades.iter().map(Trade::net).sum();
        assert_close(total, TOTAL_NET_PROFIT, &format!("{name}: sum of trades"));
    }
}

fn check_trade(name: &str, closed: &Closed, trade: &Trade) {
    let what = format!("{name} position {}", closed.position);
    let side = if closed.kind == "buy" { Side::Long } else { Side::Short };
    assert_eq!((trade.side, trade.symbol.as_str()), (side, closed.symbol), "{what}");
    assert_eq!(trade.key.segment, 0, "{what}: segment");
    assert_close(trade.volume, closed.volume, &format!("{what}: volume"));
    assert_close(trade.open_price, closed.open_price, &format!("{what}: open price"));
    assert_close(trade.profit, closed.profit, &format!("{what}: profit"));
    assert_close(trade.commission, closed.commission, &format!("{what}: commission"));
    assert_close(trade.swap, closed.swap, &format!("{what}: swap"));
    assert_close(trade.sl, closed.sl, &format!("{what}: sl"));
    assert_close(trade.tp, closed.tp, &format!("{what}: tp"));
    assert_eq!(trade.open_time, time(closed.opened_at), "{what}: open time");
    assert_eq!(trade.close_time, Some(time(closed.ended_at)), "{what}: close time");
    // The report prints the last exit; the trade averages the two halves of the partial close.
    let close_price = if closed.position == PARTIAL_POSITION { 1.0925 } else { closed.close_price };
    assert_close(trade.close_price.expect("closed"), close_price, &format!("{what}: close price"));
}

#[test]
fn the_fee_and_the_swap_stay_on_their_trade() {
    let trades = build_trades(1, &load(ENGLISH_HTML).deals);
    let gbp = trade_of(&trades, GBPUSD_POSITION);
    assert_close(gbp.fee, -0.25, "fee");
    assert_close(gbp.swap, -3.20, "swap");
    assert_close(gbp.net(), 200.0 - 3.50 - 3.20 - 0.25, "net");
    assert_eq!(gbp.duration_secs(), Some(time("2026.11.19 13:20:44") / 1000 - time("2026.11.17 22:50:00") / 1000));
}

#[test]
fn hedged_positions_on_one_symbol_close_each_at_its_own_deal() {
    // D and E are EURUSD longs opened together; E is closed first, at 13:00.
    let trades = build_trades(1, &load(VIETNAMESE_HTML).deals);
    let (d, e) = (trade_of(&trades, 91_000_007), trade_of(&trades, 91_000_008));
    assert_close(d.profit, -10.0, "D");
    assert_close(e.profit, 30.0, "E");
    assert_eq!(e.close_time, Some(time("2026.11.18 13:00:00")));
    assert_eq!(d.close_time, Some(time("2026.11.18 16:00:00")));
}

#[test]
fn the_statistics_match_the_reports_summary() {
    for name in ALL {
        let batch = load(name);
        let trades = build_trades(1, &batch.deals);
        let samples: Vec<Sample<'_>> = trades.iter().map(|trade| Sample { trade, r: None }).collect();
        let stats = metrics::stats(&samples, 0.0, &cash_flows(&batch.deals), Clock::default());
        assert_eq!((stats.trades, stats.wins, stats.losses), (CLOSED_TRADES, 3, 4), "{name}");
        assert_close(stats.net, TOTAL_NET_PROFIT, &format!("{name}: net"));
        assert_close(stats.gross_profit, 370.95, &format!("{name}: gross profit"));
        assert_close(stats.gross_loss, -132.10, &format!("{name}: gross loss"));
        assert_close(stats.deposits, 10_000.0, &format!("{name}: deposits"));
    }
}

#[test]
fn a_truncated_report_keeps_the_deals_it_has() {
    let html = mt5_html::decode(&fs::read(path(ENGLISH_HTML)).expect("the fixture exists"));
    let cut = html.find("81000010").expect("the tenth deal is in the report");
    let batch = mt5_html::parse(&html.as_bytes()[..cut]).expect("the first nine deals are readable");
    let tickets: Vec<i64> = batch.deals.iter().map(|d| d.ticket).collect();
    assert_eq!(tickets, (81_000_001..=81_000_009).collect::<Vec<_>>());
}

#[test]
fn a_file_without_a_deals_table_is_refused() {
    let header_only = "<html><table><tr><th>Name:</th><td>Someone</td></tr>\
                       <tr><th>Account:</th><td>1 (USD, Server, demo, Hedge)</td></tr></table></html>";
    for bytes in [header_only.as_bytes(), b"not a report".as_slice(), b"".as_slice()] {
        assert!(matches!(mt5_html::parse(bytes), Err(ImportError::NoDeals)), "{}", String::from_utf8_lossy(bytes));
    }
}

#[test]
fn files_that_are_not_reports_are_refused() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let fake = dir.path().join("broken.xlsx");
    fs::write(&fake, b"this is not a zip archive").expect("the file is written");
    assert!(matches!(import::read_file(&fake), Err(ImportError::Invalid(_))), "a broken workbook");
    let missing = dir.path().join("missing.html");
    assert!(matches!(import::read_file(&missing), Err(ImportError::Io(_))), "a missing file");
    let other = dir.path().join("report.pdf");
    assert!(matches!(import::read_file(&other), Err(ImportError::Unsupported(_))), "an unknown extension");
}
