//! The trades of each account on the trader's clock, for the pages that group trades by day
//! (accounts and discipline, the daily journal). Pure over a [`Db`], so it runs on a reading
//! thread and is tested without a window.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use tradelens_core::{
    Account, Db, DbError, Deal, Trade,
    metrics::{self, EquityPoint, Sample},
    time::{Clock, TimeDisplay},
    trades::{build_trades, cash_flows},
};

/// The setting that holds which clock times are shown on (`local` or `server`).
pub(crate) const TIME_DISPLAY_KEY: &str = "time_display";

/// The clock choice the trader made in the settings.
///
/// # Errors
///
/// [`DbError`] when the setting cannot be read.
pub(crate) fn time_display(db: &Db) -> Result<TimeDisplay, DbError> {
    Ok(db.setting(TIME_DISPLAY_KEY)?.map_or_else(TimeDisplay::default, |text| TimeDisplay::parse(&text)))
}

/// Minutes the machine's zone is ahead of UTC.
pub(crate) fn local_offset_min() -> i32 {
    chrono::Local::now().offset().local_minus_utc() / 60
}

/// Today on the machine's calendar.
pub(crate) fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

/// One account with its trades, grouped by day on `clock`.
#[derive(Debug, Clone)]
pub(crate) struct AccountTrades {
    pub(crate) account: Account,
    pub(crate) clock: Clock,
    /// Every trade, oldest first (open ones included).
    pub(crate) trades: Vec<Trade>,
    /// Deposits and withdrawals: (server time, amount).
    pub(crate) cash: Vec<(i64, f64)>,
}

impl AccountTrades {
    /// The closed trades, for statistics and rules.
    pub(crate) fn samples(&self) -> Vec<Sample<'_>> {
        self.trades.iter().filter(|t| t.is_closed()).map(|trade| Sample { trade, r: None }).collect()
    }

    /// The balance after each closed trade and cash movement.
    pub(crate) fn curve(&self) -> Vec<EquityPoint> {
        let samples = self.samples();
        let start = metrics::start_balance(&self.account, &samples, &self.cash);
        metrics::equity_curve(&samples, start, &self.cash)
    }
}

/// Reads the trades of the chosen account, or of every account when `account` is `None`.
///
/// # Errors
///
/// [`DbError`] when a query fails.
pub(crate) fn load(db: &Db, account: Option<i64>) -> Result<Vec<AccountTrades>, DbError> {
    let display = time_display(db)?;
    let local = local_offset_min();
    let mut deals: BTreeMap<i64, Vec<Deal>> = BTreeMap::new();
    for (id, deal) in db.deals(account)? {
        deals.entry(id).or_default().push(deal);
    }
    Ok(db
        .accounts()?
        .into_iter()
        .filter(|a| account.is_none_or(|id| id == a.id))
        .map(|account| {
            let own = deals.remove(&account.id).unwrap_or_default();
            AccountTrades {
                clock: display.clock(account.server_offset_min, local),
                trades: build_trades(account.id, &own),
                cash: cash_flows(&own),
                account,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use tradelens_core::{DealBatch, DealEntry, DealKind, model::AccountInfo, time::parse_mt5_time};

    use super::*;

    fn deal(ticket: i64, at: &str, kind: DealKind, entry: DealEntry, profit: f64) -> Deal {
        Deal {
            ticket,
            order: ticket,
            position_id: 7,
            time_msc: parse_mt5_time(at).expect("fixture times parse"),
            symbol: "EURUSD".into(),
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
        let mut db = Db::in_memory().expect("a memory database opens");
        let batch = DealBatch {
            account: AccountInfo {
                login: "1".into(),
                server: "S".into(),
                balance: Some(1_050.0),
                server_offset_min: Some(120),
                ..AccountInfo::default()
            },
            deals: vec![
                deal(1, "2026.11.20 22:30:00", DealKind::Buy, DealEntry::In, 0.0),
                deal(2, "2026.11.20 23:30:00", DealKind::Sell, DealEntry::Out, 50.0),
            ],
            positions: Vec::new(),
        };
        db.upsert_batch(&batch, "test").expect("the batch is stored");
        db
    }

    #[test]
    fn trades_group_by_the_local_day_unless_the_server_clock_is_chosen() {
        let mut db = seeded();
        let loaded = load(&db, None).expect("loads");
        let own = &loaded[0];
        let opened = own.trades[0].open_time;
        // 22:30 at UTC+2 is already the 21st at UTC+7 (local offset of the machine aside).
        assert_eq!(own.clock.server_offset_min, Some(120));
        assert_eq!(own.samples().len(), 1);
        db.set_setting(TIME_DISPLAY_KEY, "server").expect("the setting is stored");
        let server = load(&db, None).expect("loads again");
        assert_eq!(server[0].clock.server_offset_min, None, "server times stay as MT5 shows them");
        assert_eq!(server[0].clock.day(opened), NaiveDate::from_ymd_opt(2026, 11, 20).expect("a date"));
    }

    #[test]
    fn one_account_can_be_chosen() {
        let db = seeded();
        assert_eq!(load(&db, Some(999)).expect("loads").len(), 0);
        assert_eq!(load(&db, Some(1)).expect("loads").len(), 1);
    }

    #[test]
    fn the_curve_ends_at_the_balance_the_terminal_reported() {
        let db = seeded();
        let loaded = load(&db, None).expect("loads");
        let curve = loaded[0].curve();
        assert!((curve.last().expect("a curve has points").balance - 1_050.0).abs() < 1e-9);
    }
}
