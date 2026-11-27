//! What the pages show, read from the database in one go on a background thread: the
//! accounts, every trade with its journal summary, the statistics and the balance curve of
//! the chosen accounts and range. A [`Snapshot`] is plain data (`Send`), swapped into the
//! store whole, so the UI thread never waits on SQLite.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{Datelike, NaiveDate, NaiveDateTime};
use nsh_core::t;
use tradelens_core::{
    Account, Db, DbError, Deal, JournalEntry, Origin, Trade, TradeKey,
    catalog::{CatalogItem, CatalogKind},
    metrics::{self, Basis, EquityPoint, Sample, Stats, Unit},
    playbook::Playbook,
    time::{self, Clock},
    trades::{build_trades, cash_flows},
};

use crate::zone::{self, Zone};

const DAY_MS: i64 = 86_400_000;

/// How far back the overview looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Range {
    Days7,
    #[default]
    Days30,
    Days90,
    YearToDate,
    All,
}

impl Range {
    pub(crate) const ALL: [Self; 5] = [Self::Days7, Self::Days30, Self::Days90, Self::YearToDate, Self::All];

    pub(crate) fn label(self) -> String {
        match self {
            Self::Days7 => t!("snapshot.range.days7"),
            Self::Days30 => t!("snapshot.range.days30"),
            Self::Days90 => t!("snapshot.range.days90"),
            Self::YearToDate => "YTD".to_owned(),
            Self::All => t!("common.all"),
        }
    }

    /// The summary line under the overview's title.
    pub(crate) fn describe(self) -> String {
        match self {
            Self::Days7 => t!("snapshot.range.describe.days7"),
            Self::Days30 => t!("snapshot.range.describe.days30"),
            Self::Days90 => t!("snapshot.range.describe.days90"),
            Self::YearToDate => t!("snapshot.range.describe.ytd"),
            Self::All => t!("snapshot.range.describe.all"),
        }
    }

    /// The server time (milliseconds) the range starts at, given the server's clock now;
    /// `None` for no limit.
    pub(crate) fn cutoff(self, now_server_ms: i64) -> Option<i64> {
        let days = |n: i64| Some(now_server_ms - n * DAY_MS);
        match self {
            Self::Days7 => days(7),
            Self::Days30 => days(30),
            Self::Days90 => days(90),
            Self::YearToDate => {
                let year = time::naive(now_server_ms).year();
                NaiveDate::from_ymd_opt(year, 1, 1).and_then(|d| d.and_hms_opt(0, 0, 0)).map(time::to_ms)
            }
            Self::All => None,
        }
    }
}

/// What to load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Query {
    /// One account, or every account when `None`.
    pub(crate) account: Option<i64>,
    pub(crate) range: Range,
    /// The real time now, milliseconds since 1970 UTC.
    pub(crate) now_utc_ms: i64,
    /// Minutes the trader is ahead of UTC.
    pub(crate) local_offset_min: i32,
}

impl Query {
    /// A query for the time on the machine's clock.
    pub(crate) fn now(account: Option<i64>, range: Range) -> Self {
        let now = chrono::Local::now();
        Self {
            account,
            range,
            now_utc_ms: now.timestamp_millis(),
            local_offset_min: now.offset().local_minus_utc() / 60,
        }
    }
}

/// One trade as the lists show it: the trade, its journal's summary and what the display
/// needs to place it in time.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TradeRow {
    pub(crate) trade: Trade,
    /// The title of the account the trade belongs to.
    pub(crate) account: String,
    pub(crate) currency: String,
    /// The result in R, from the journal's planned risk, the EA's risk or the stop.
    pub(crate) r: Option<f64>,
    /// The setup's name from the playbook, else the free-text setup; empty when none.
    pub(crate) setup: String,
    /// The trader wrote something about it.
    pub(crate) journaled: bool,
    /// The deals the position is made of, oldest first.
    pub(crate) deals: Vec<Deal>,
    /// How the trade came to be, from how its deals were written (not from the account it is filed
    /// under): the trader may change the facts of their own trades only.
    pub(crate) origin: Origin,
    /// The decimals the prices of the trade's symbol are shown with (see `price_digits`).
    pub(crate) digits: usize,
    clock: Clock,
    /// Minutes the account's server is ahead of UTC, whatever clock the times are shown on.
    server_offset_min: Option<i32>,
    /// Lowercase text the search box looks in.
    haystack: String,
}

impl TradeRow {
    pub(crate) fn key(&self) -> &TradeKey {
        &self.trade.key
    }

    /// A server time on the trader's clock.
    pub(crate) fn local(&self, server_ms: i64) -> NaiveDateTime {
        self.clock.local(server_ms)
    }

    /// When the position opened, on the trader's clock.
    pub(crate) fn opened(&self) -> NaiveDateTime {
        self.clock.local(self.trade.open_time)
    }

    /// When the position closed, on the trader's clock; `None` while it is open.
    pub(crate) fn closed(&self) -> Option<NaiveDateTime> {
        self.trade.close_time.map(|t| self.clock.local(t))
    }

    /// A server time on the UTC clock (for the trading sessions), whatever clock is shown.
    pub(crate) fn utc(&self, server_ms: i64) -> NaiveDateTime {
        Clock { server_offset_min: self.server_offset_min, local_offset_min: self.clock.local_offset_min }
            .utc(server_ms)
    }

    /// The time the trade is ordered by: its close, or its open while still open.
    pub(crate) fn sort_time(&self) -> i64 {
        self.trade.close_time.unwrap_or(self.trade.open_time)
    }

    /// Whether `needle` (already lowercase) occurs in the symbol, ticket, comment, setup or notes.
    pub(crate) fn contains(&self, needle: &str) -> bool {
        self.haystack.contains(needle)
    }
}

/// The trader's option lists the entry editor picks from.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Catalogs {
    pub(crate) tags: Vec<CatalogItem>,
    pub(crate) mistakes: Vec<CatalogItem>,
    pub(crate) emotions: Vec<CatalogItem>,
}

/// Everything the pages read, as of one moment.
#[derive(Debug, Clone, Default)]
pub(crate) struct Snapshot {
    /// The clock the times are shown on and "today" is counted on.
    pub(crate) zone: Zone,
    pub(crate) accounts: Vec<Account>,
    /// Every trade of the chosen accounts, newest first (open trades included).
    pub(crate) rows: Vec<TradeRow>,
    pub(crate) journals: HashMap<TradeKey, JournalEntry>,
    pub(crate) playbooks: Vec<Playbook>,
    pub(crate) catalogs: Catalogs,
    /// The statistics of the closed trades in the range.
    pub(crate) stats: Stats,
    /// The balance after each closed trade and cash movement in the range.
    pub(crate) curve: Vec<EquityPoint>,
    /// How many of the range's closed trades have money and how many only R: the money figures of
    /// `stats` and `curve` cover the first, and the screens show R when there is no money.
    pub(crate) basis: Basis,
    /// The server time the range starts at (`None`: no limit), for what lists the range's trades.
    pub(crate) range_from: Option<i64>,
    /// The balance the chosen accounts had before their first trade and cash movement.
    pub(crate) start_balance: f64,
    /// Every deposit and withdrawal of the chosen accounts, oldest first: `(server time, amount)`.
    pub(crate) cash: Vec<(i64, f64)>,
    /// `USD` and the like when every chosen account shares one, else empty.
    pub(crate) currency: String,
    pub(crate) symbols: Vec<String>,
    pub(crate) setups: Vec<String>,
}

impl Snapshot {
    pub(crate) fn row(&self, key: &TradeKey) -> Option<&TradeRow> {
        self.rows.iter().find(|row| row.key() == key)
    }

    /// The trades of the range (7N, 30N, …), newest first: those closed since it starts, or still open.
    pub(crate) fn rows_in_range(&self) -> impl Iterator<Item = &TradeRow> {
        self.rows.iter().filter(|row| self.range_from.is_none_or(|from| row.sort_time() >= from))
    }

    /// The unit the figures of the range are in: R when none of its trades has money.
    pub(crate) fn unit(&self) -> Unit {
        self.basis.unit()
    }

    /// The journal entry of a trade, empty when the trader wrote none.
    pub(crate) fn journal(&self, key: &TradeKey) -> JournalEntry {
        self.journals.get(key).cloned().unwrap_or_default()
    }
}

/// Reads what the pages show.
///
/// # Errors
///
/// [`DbError`] when a query fails or a stored entry does not parse.
pub(crate) fn load(db: &Db, query: &Query) -> Result<Snapshot, DbError> {
    let accounts = db.accounts()?;
    let display = zone::time_display(db)?;
    let journals: HashMap<TradeKey, JournalEntry> = db.journals(query.account)?.into_iter().collect();
    let playbooks = db.playbooks()?;
    let catalogs = Catalogs {
        tags: db.catalog(CatalogKind::Tag)?,
        mistakes: db.catalog(CatalogKind::Mistake)?,
        emotions: db.catalog(CatalogKind::Emotion)?,
    };

    let origins = db.trade_origins(query.account)?;
    let mut deals_by_account: BTreeMap<i64, Vec<Deal>> = BTreeMap::new();
    for (account_id, deal) in db.deals(query.account)? {
        deals_by_account.entry(account_id).or_default().push(deal);
    }
    let built: Vec<(i64, Vec<Trade>)> =
        deals_by_account.iter().map(|(account_id, deals)| (*account_id, build_trades(*account_id, deals))).collect();
    let digits = price_digits(built.iter().flat_map(|(_, trades)| trades));

    let mut rows: Vec<TradeRow> = Vec::new();
    let mut cash: Vec<(i64, f64)> = Vec::new();
    let mut start_total = 0.0;
    for (account_id, trades) in built {
        let deals = deals_by_account.get(&account_id).map_or(&[][..], Vec::as_slice);
        let account = accounts.iter().find(|a| a.id == account_id);
        let server_offset_min = account.and_then(|a| a.server_offset_min);
        let clock = display.clock(server_offset_min, query.local_offset_min);
        // Every deal has its account, so the fallback is only the safe answer. It is made on a blocking
        // thread, which cannot read the app's text: the core words it.
        let title = account.map_or_else(|| Account::unknown_title(account_id), Account::title);
        let currency = account.map_or_default(|a| a.currency.clone());
        let account_cash = cash_flows(deals);
        let account_rows: Vec<TradeRow> = trades
            .into_iter()
            .map(|trade| {
                let looks = Looks {
                    origin: origins
                        .get(&(account_id, trade.key.position_id))
                        .copied()
                        // Every trade has deals, so this is only the safe answer (not editable).
                        .unwrap_or(Origin::Mt5),
                    digits: digits.get(&trade.symbol).copied().unwrap_or(MIN_DIGITS),
                };
                make_row(trade, deals, &journals, &playbooks, (&title, &currency), (clock, server_offset_min), looks)
            })
            .collect();
        if let Some(account) = account {
            let samples = closed_samples(&account_rows);
            start_total += metrics::start_balance(account, &samples, &account_cash);
        }
        cash.extend(account_cash);
        rows.extend(account_rows);
    }
    cash.sort_by_key(|flow| flow.0);
    rows.sort_by_key(|row| std::cmp::Reverse((row.sort_time(), row.trade.key.position_id)));

    let in_view: Vec<&Account> = accounts.iter().filter(|a| query.account.is_none_or(|id| id == a.id)).collect();
    let zone = summary_zone(&in_view, display, query.local_offset_min);
    let now_server = zone::stored_now(zone.server_offset_min, query.local_offset_min, query.now_utc_ms);
    let cutoff = query.range.cutoff(now_server);
    let (stats, curve, basis) = summarise(&rows, &cash, start_total, cutoff, zone.clock());

    let currencies: BTreeSet<&str> = in_view.iter().map(|a| a.currency.as_str()).collect();
    let currency = match currencies.len() {
        1 => currencies.into_iter().next().unwrap_or_default().to_owned(),
        _ => String::new(),
    };
    let symbols: BTreeSet<String> = rows.iter().map(|r| r.trade.symbol.clone()).collect();
    let setups: BTreeSet<String> = rows.iter().filter(|r| !r.setup.is_empty()).map(|r| r.setup.clone()).collect();

    Ok(Snapshot {
        zone,
        accounts,
        rows,
        journals,
        playbooks,
        catalogs,
        stats,
        curve,
        basis,
        range_from: cutoff,
        start_balance: start_total,
        cash,
        currency,
        symbols: symbols.into_iter().collect(),
        setups: setups.into_iter().collect(),
    })
}

/// What a row knows beyond its own trade: where it came from and how its prices are shown.
#[derive(Clone, Copy)]
struct Looks {
    origin: Origin,
    digits: usize,
}

/// The fewest decimals a price is shown with.
pub(crate) const MIN_DIGITS: usize = 2;
/// The most decimals a price is shown with (5-digit FX).
const MAX_DIGITS: usize = 5;

/// The decimals each symbol's prices are shown with: the most any of its prices has, between
/// `MIN_DIGITS` and `MAX_DIGITS`, so that EURUSD reads 1.08500 and 1.08505 and XAUUSD 2648.20.
fn price_digits<'a>(trades: impl Iterator<Item = &'a Trade>) -> HashMap<String, usize> {
    let mut digits: HashMap<String, usize> = HashMap::new();
    for trade in trades.filter(|t| t.prices_known()) {
        let prices = [Some(trade.open_price), trade.close_price, Some(trade.sl), Some(trade.tp)];
        let seen = prices.into_iter().flatten().filter(|p| *p > 0.0).map(crate::format::decimals).max();
        let slot = digits.entry(trade.symbol.clone()).or_insert(MIN_DIGITS);
        *slot = (*slot).max(seen.unwrap_or(0)).min(MAX_DIGITS);
    }
    digits
}

fn make_row(
    trade: Trade,
    deals: &[Deal],
    journals: &HashMap<TradeKey, JournalEntry>,
    playbooks: &[Playbook],
    (account, currency): (&str, &str),
    (clock, server_offset_min): (Clock, Option<i32>),
    looks: Looks,
) -> TradeRow {
    let entry = journals.get(&trade.key);
    let setup = entry.map_or_else(String::new, |e| setup_name(e, playbooks));
    let r = trade.is_closed().then(|| trade.r_multiple(entry.and_then(|e| e.planned_risk))).flatten();
    let position = trade.key.position_id;
    let mut own: Vec<Deal> = deals
        .iter()
        .filter(|d| d.kind.is_trade() && (if d.position_id == 0 { d.ticket } else { d.position_id }) == position)
        .cloned()
        .collect();
    own.sort_by_key(|d| (d.time_msc, d.ticket));
    let notes = entry.map_or("", |e| e.notes.as_str());
    let tickets: Vec<String> = own.iter().map(|d| d.ticket.to_string()).collect();
    // Tags too: a tag pressed in Phân tích opens the trades list searched for it.
    let tags = entry.map_or_else(String::new, |e| e.tags.join(" "));
    let haystack =
        format!("{} {position} {} {} {setup} {notes} {tags}", trade.symbol, tickets.join(" "), trade.comment)
            .to_lowercase();
    TradeRow {
        journaled: entry.is_some_and(|e| !e.is_empty()),
        account: account.to_owned(),
        currency: currency.to_owned(),
        r,
        setup,
        deals: own,
        origin: looks.origin,
        digits: looks.digits,
        clock,
        server_offset_min,
        haystack,
        trade,
    }
}

/// The name of the setup an entry links to: the playbook's, else the free-text one.
fn setup_name(entry: &JournalEntry, playbooks: &[Playbook]) -> String {
    entry
        .playbook_id
        .and_then(|id| playbooks.iter().find(|p| p.id == id))
        .map_or_else(|| entry.setup.trim().to_owned(), |p| p.name.clone())
}

fn closed_samples(rows: &[TradeRow]) -> Vec<Sample<'_>> {
    rows.iter().filter(|row| row.trade.is_closed()).map(|row| Sample { trade: &row.trade, r: row.r }).collect()
}

/// The zone the statistics group days by: the first known offset of the accounts in view.
fn summary_zone(accounts: &[&Account], display: time::TimeDisplay, local_offset_min: i32) -> Zone {
    Zone { display, server_offset_min: accounts.iter().find_map(|a| a.server_offset_min), local_offset_min }
}

/// The statistics and the curve of the closed trades from `cutoff` on, the balance carried
/// over from everything before it.
fn summarise(
    rows: &[TradeRow],
    cash: &[(i64, f64)],
    start_total: f64,
    cutoff: Option<i64>,
    clock: Clock,
) -> (Stats, Vec<EquityPoint>, Basis) {
    let all = closed_samples(rows);
    let closed_at = |x: &Sample<'_>| x.trade.close_time.unwrap_or(x.trade.open_time);
    let (samples, flows): (Vec<Sample<'_>>, Vec<(i64, f64)>) = match cutoff {
        Some(from) => (
            all.iter().copied().filter(|x| closed_at(x) >= from).collect(),
            cash.iter().copied().filter(|flow| flow.0 >= from).collect(),
        ),
        None => (all.clone(), cash.to_vec()),
    };
    let start = match cutoff {
        Some(from) => {
            let before: f64 = all.iter().filter(|x| closed_at(x) < from).map(Sample::net).sum();
            let moved: f64 = cash.iter().filter(|flow| flow.0 < from).map(|flow| flow.1).sum();
            start_total + before + moved
        }
        None => start_total,
    };
    (
        metrics::stats(&samples, start, &flows, clock),
        metrics::equity_curve(&samples, start, &flows),
        Basis::of(&samples),
    )
}

#[cfg(test)]
mod tests {
    use tradelens_core::{
        DealBatch, DealEntry, DealKind,
        model::AccountInfo,
        time::{naive, parse_mt5_time},
    };

    use super::*;

    fn deal(ticket: i64, position: i64, at: &str, kind: DealKind, entry: DealEntry, profit: f64) -> Deal {
        Deal {
            ticket,
            order: ticket,
            position_id: position,
            time_msc: parse_mt5_time(at).expect("fixture times parse"),
            symbol: if kind.is_cash() { String::new() } else { "EURUSD".into() },
            kind,
            entry,
            volume: 0.1,
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

    fn seeded() -> Db {
        use DealEntry::{In, Out};
        use DealKind::{Balance, Buy, Sell};
        let mut db = Db::in_memory().expect("a memory database opens");
        let batch = DealBatch {
            account: AccountInfo {
                login: "1".into(),
                server: "S".into(),
                currency: "USD".into(),
                balance: Some(1100.0),
                ..AccountInfo::default()
            },
            deals: vec![
                deal(1, 0, "2026.11.01 00:00:00", Balance, DealEntry::None, 1000.0),
                deal(2, 10, "2026.11.10 09:00:00", Buy, In, 0.0),
                deal(3, 10, "2026.11.10 10:00:00", Sell, Out, 150.0),
                deal(4, 11, "2026.11.20 09:00:00", Sell, In, 0.0),
                deal(5, 11, "2026.11.20 10:00:00", Buy, Out, -50.0),
            ],
            positions: Vec::new(),
        };
        db.upsert_batch(&batch, "test").expect("the batch is stored");
        db
    }

    fn at(day: &str) -> i64 {
        parse_mt5_time(day).expect("a valid time")
    }

    fn query(range: Range) -> Query {
        Query { account: None, range, now_utc_ms: at("2026.11.21 12:00:00"), local_offset_min: 0 }
    }

    #[test]
    fn rows_are_newest_first_with_their_deals() {
        let snapshot = load(&seeded(), &query(Range::All)).expect("loads");
        let positions: Vec<i64> = snapshot.rows.iter().map(|r| r.trade.key.position_id).collect();
        assert_eq!(positions, [11, 10]);
        assert_eq!(snapshot.rows[0].deals.len(), 2);
        assert_eq!(snapshot.currency, "USD");
        assert_eq!(snapshot.symbols, ["EURUSD"]);
    }

    #[test]
    fn a_symbols_prices_are_shown_with_the_most_decimals_it_has_between_two_and_five() {
        let mut db = seeded();
        let account = db.accounts().expect("accounts").remove(0).id;
        let typed = |symbol: &str, open: f64, close: f64| tradelens_core::ManualTrade {
            symbol: symbol.into(),
            side: tradelens_core::Side::Long,
            volume: Some(0.1),
            open_time: at("2026.11.12 09:00:00"),
            open_price: Some(open),
            close_time: at("2026.11.12 10:00:00"),
            close_price: Some(close),
            date_only: false,
            sl: 0.0,
            tp: 0.0,
            profit: Some(1.0),
            commission: 0.0,
            swap: 0.0,
            result_r: None,
        };
        db.add_manual_trade(account, &typed("EURUSD", 1.0850, 1.08505)).expect("typed");
        db.add_manual_trade(account, &typed("XAUUSD", 2648.2, 2650.0)).expect("typed");
        let snapshot = load(&db, &query(Range::All)).expect("loads");
        let digits = |symbol: &str| snapshot.rows.iter().find(|r| r.trade.symbol == symbol).map(|r| r.digits);
        assert_eq!(digits("EURUSD"), Some(5), "1.08505 has five, so 1.0850 reads 1.08500");
        assert_eq!(digits("XAUUSD"), Some(2), "at least two");
        let eur =
            snapshot.rows.iter().find(|r| r.trade.symbol == "EURUSD" && r.origin == Origin::Manual).expect("the trade");
        assert_eq!(crate::format::price_to(eur.trade.open_price, eur.digits), "1.08500");
        assert_eq!(crate::format::price_to(eur.trade.close_price.expect("closed"), eur.digits), "1.08505");
    }

    #[test]
    fn a_trades_origin_is_how_its_deals_were_written_not_where_they_are_filed() {
        let mut db = seeded();
        let account = db.accounts().expect("accounts").remove(0).id;
        let key = db
            .add_manual_trade(
                account,
                &tradelens_core::ManualTrade {
                    symbol: "eurusd".into(),
                    side: tradelens_core::Side::Long,
                    volume: Some(0.1),
                    open_time: at("2026.11.12 09:00:00"),
                    open_price: Some(1.1),
                    close_time: at("2026.11.12 10:00:00"),
                    close_price: Some(1.2),
                    date_only: false,
                    sl: 0.0,
                    tp: 0.0,
                    profit: Some(1.0),
                    commission: 0.0,
                    swap: 0.0,
                    result_r: None,
                },
            )
            .expect("typed");
        let snapshot = load(&db, &query(Range::All)).expect("loads");
        let origin =
            |position: i64| snapshot.rows.iter().find(|r| r.trade.key.position_id == position).map(|r| r.origin);
        assert_eq!(origin(10), Some(Origin::Mt5));
        assert_eq!(origin(key.position_id), Some(Origin::Manual));
    }

    #[test]
    fn the_curve_starts_from_the_balance_the_terminal_reported() {
        let snapshot = load(&seeded(), &query(Range::All)).expect("loads");
        let last = snapshot.curve.last().expect("a curve has points");
        assert!((last.balance - 1100.0).abs() < 1e-9, "ends at the reported balance: {}", last.balance);
        assert_eq!(snapshot.stats.trades, 2);
        assert!((snapshot.stats.net - 100.0).abs() < 1e-9);
    }

    #[test]
    fn a_range_keeps_recent_trades_and_carries_the_balance_over() {
        let snapshot = load(&seeded(), &query(Range::Days7)).expect("loads");
        assert_eq!(snapshot.stats.trades, 1, "only the trade of the 20th is within 7 days");
        assert!((snapshot.stats.start_balance - 1150.0).abs() < 1e-9, "start {}", snapshot.stats.start_balance);
        assert_eq!(snapshot.rows.len(), 2, "the list keeps every trade");
    }

    #[test]
    fn range_cutoffs() {
        let now = at("2026.11.21 12:00:00");
        assert_eq!(Range::All.cutoff(now), None);
        assert_eq!(Range::Days7.cutoff(now), Some(now - 7 * DAY_MS));
        assert_eq!(Range::YearToDate.cutoff(now).map(naive).map(|d| d.date()), NaiveDate::from_ymd_opt(2026, 1, 1));
    }

    #[test]
    fn the_time_display_setting_moves_every_time_and_today_of_the_snapshot() {
        let mut db = seeded();
        let account = db.accounts().expect("accounts").remove(0);
        // The server runs two hours ahead of UTC, the trader seven.
        db.update_account(&Account { server_offset_min: Some(120), ..account }).expect("the offset is stored");
        let at_utc_plus_7 = Query { local_offset_min: 420, now_utc_ms: at("2026.11.20 20:00:00"), ..query(Range::All) };

        let local = load(&db, &at_utc_plus_7).expect("loads");
        let closed = |snapshot: &Snapshot| snapshot.rows[0].closed().expect("the trade is closed");
        assert_eq!(closed(&local).format("%d/%m %H:%M").to_string(), "20/11 15:00", "10:00 at UTC+2 is 15:00 at UTC+7");
        // 20:00 UTC is 22:00 at the server and 03:00 the next day at UTC+7.
        assert_eq!(local.zone.today(at_utc_plus_7.now_utc_ms), NaiveDate::from_ymd_opt(2026, 11, 21).expect("a date"));

        db.set_setting(zone::TIME_DISPLAY_KEY, "server").expect("the setting is stored");
        let server = load(&db, &at_utc_plus_7).expect("loads again");
        assert_eq!(closed(&server).format("%d/%m %H:%M").to_string(), "20/11 10:00", "the clock MT5 shows");
        assert_eq!(server.zone.today(at_utc_plus_7.now_utc_ms), NaiveDate::from_ymd_opt(2026, 11, 20).expect("a date"));
        // The trading sessions are on UTC whichever clock is shown.
        let utc_hour =
            |snapshot: &Snapshot| snapshot.rows[0].utc(snapshot.rows[0].trade.open_time).format("%H").to_string();
        assert_eq!(utc_hour(&local), utc_hour(&server));
        assert_eq!(utc_hour(&server), "07");
    }
    #[test]
    fn journal_summary_follows_the_entry() {
        let mut db = seeded();
        let first = load(&db, &query(Range::All)).expect("loads");
        assert!(first.rows.iter().all(|r| !r.journaled));
        let key = first.rows[0].key().clone();
        let entry = JournalEntry { setup: "Range".into(), notes: "đã vào sớm".into(), ..JournalEntry::default() };
        db.save_journal(&key, &entry).expect("saves");
        let after = load(&db, &query(Range::All)).expect("loads");
        let row = after.row(&key).expect("the row is there");
        assert!(row.journaled);
        assert_eq!(row.setup, "Range");
        assert!(row.contains("vào sớm"));
        assert_eq!(after.setups, ["Range"]);
        assert_eq!(after.rows.iter().filter(|r| !r.journaled).count(), 1);
    }

    #[test]
    fn an_empty_database_loads_empty() {
        let db = Db::in_memory().expect("opens");
        let snapshot = load(&db, &query(Range::All)).expect("loads");
        assert!(snapshot.rows.is_empty() && snapshot.accounts.is_empty());
        assert_eq!(snapshot.stats.trades, 0);
    }
}
