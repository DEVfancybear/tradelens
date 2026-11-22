//! What the pages show, read from the database in one go on a background thread: the
//! accounts, every trade with its journal summary, the statistics and the balance curve of
//! the chosen accounts and range. A [`Snapshot`] is plain data (`Send`), swapped into the
//! store whole, so the UI thread never waits on SQLite.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{Datelike, NaiveDate, NaiveDateTime};
use tradelens_core::{
    Account, Db, DbError, Deal, JournalEntry, Trade, TradeKey,
    catalog::{CatalogItem, CatalogKind},
    metrics::{self, EquityPoint, Sample, Stats},
    playbook::Playbook,
    time::{self, Clock},
    trades::{build_trades, cash_flows},
};

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

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Days7 => "7N",
            Self::Days30 => "30N",
            Self::Days90 => "90N",
            Self::YearToDate => "YTD",
            Self::All => "Tất cả",
        }
    }

    /// The summary line under the overview's title.
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::Days7 => "7 ngày gần nhất",
            Self::Days30 => "30 ngày gần nhất",
            Self::Days90 => "90 ngày gần nhất",
            Self::YearToDate => "Từ đầu năm",
            Self::All => "Toàn bộ lịch sử",
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
    clock: Clock,
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

    /// How the account's server times become the trader's.
    pub(crate) fn clock(&self) -> Clock {
        self.clock
    }

    /// When the position opened, on the trader's clock.
    pub(crate) fn opened(&self) -> NaiveDateTime {
        self.clock.local(self.trade.open_time)
    }

    /// When the position closed, on the trader's clock; `None` while it is open.
    pub(crate) fn closed(&self) -> Option<NaiveDateTime> {
        self.trade.close_time.map(|t| self.clock.local(t))
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
    /// `USD` and the like when every chosen account shares one, else empty.
    pub(crate) currency: String,
    pub(crate) symbols: Vec<String>,
    pub(crate) setups: Vec<String>,
}

impl Snapshot {
    pub(crate) fn row(&self, key: &TradeKey) -> Option<&TradeRow> {
        self.rows.iter().find(|row| row.key() == key)
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
    let journals: HashMap<TradeKey, JournalEntry> = db.journals(query.account)?.into_iter().collect();
    let playbooks = db.playbooks()?;
    let catalogs = Catalogs {
        tags: db.catalog(CatalogKind::Tag)?,
        mistakes: db.catalog(CatalogKind::Mistake)?,
        emotions: db.catalog(CatalogKind::Emotion)?,
    };

    let mut deals_by_account: BTreeMap<i64, Vec<Deal>> = BTreeMap::new();
    for (account_id, deal) in db.deals(query.account)? {
        deals_by_account.entry(account_id).or_default().push(deal);
    }

    let mut rows: Vec<TradeRow> = Vec::new();
    let mut cash: Vec<(i64, f64)> = Vec::new();
    let mut start_total = 0.0;
    for (account_id, deals) in &deals_by_account {
        let account = accounts.iter().find(|a| a.id == *account_id);
        let clock = Clock {
            server_offset_min: account.and_then(|a| a.server_offset_min),
            local_offset_min: query.local_offset_min,
        };
        let title = account.map_or_else(|| format!("Tài khoản {account_id}"), Account::title);
        let currency = account.map_or_default(|a| a.currency.clone());
        let trades = build_trades(*account_id, deals);
        let account_cash = cash_flows(deals);
        let account_rows: Vec<TradeRow> = trades
            .into_iter()
            .map(|trade| make_row(trade, deals, &journals, &playbooks, &title, &currency, clock))
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
    let clock = summary_clock(&in_view, query.local_offset_min);
    let now_server = query.now_utc_ms + clock.server_offset_min.map_or(0, |m| i64::from(m) * 60_000);
    let cutoff = query.range.cutoff(now_server);
    let (stats, curve) = summarise(&rows, &cash, start_total, cutoff, clock);

    let currencies: BTreeSet<&str> = in_view.iter().map(|a| a.currency.as_str()).collect();
    let currency = match currencies.len() {
        1 => currencies.into_iter().next().unwrap_or_default().to_owned(),
        _ => String::new(),
    };
    let symbols: BTreeSet<String> = rows.iter().map(|r| r.trade.symbol.clone()).collect();
    let setups: BTreeSet<String> = rows.iter().filter(|r| !r.setup.is_empty()).map(|r| r.setup.clone()).collect();

    Ok(Snapshot {
        accounts,
        rows,
        journals,
        playbooks,
        catalogs,
        stats,
        curve,
        currency,
        symbols: symbols.into_iter().collect(),
        setups: setups.into_iter().collect(),
    })
}

fn make_row(
    trade: Trade,
    deals: &[Deal],
    journals: &HashMap<TradeKey, JournalEntry>,
    playbooks: &[Playbook],
    account: &str,
    currency: &str,
    clock: Clock,
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
    let haystack =
        format!("{} {position} {} {} {setup} {notes}", trade.symbol, tickets.join(" "), trade.comment).to_lowercase();
    TradeRow {
        journaled: entry.is_some_and(|e| !e.is_empty()),
        account: account.to_owned(),
        currency: currency.to_owned(),
        r,
        setup,
        deals: own,
        clock,
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

/// The clock the statistics group days by: the one account's, else the first known offset.
fn summary_clock(accounts: &[&Account], local_offset_min: i32) -> Clock {
    Clock { server_offset_min: accounts.iter().find_map(|a| a.server_offset_min), local_offset_min }
}

/// The statistics and the curve of the closed trades from `cutoff` on, the balance carried
/// over from everything before it.
fn summarise(
    rows: &[TradeRow],
    cash: &[(i64, f64)],
    start_total: f64,
    cutoff: Option<i64>,
    clock: Clock,
) -> (Stats, Vec<EquityPoint>) {
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
    (metrics::stats(&samples, start, &flows, clock), metrics::equity_curve(&samples, start, &flows))
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
