//! What the share card and the monthly PDF read out of a [`Snapshot`]: the closed trades of one
//! week or month with their figures, and the month's report (statistics, curves, calendar,
//! playbooks, mistakes, missed trades, review). Pure over the snapshot and the few extra rows
//! the database holds (missed trades, the review), so it is tested without a window.

use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, NaiveDate};
use tradelens_core::{
    Db, DbError, TradeKey,
    catalog::{CatalogItem, CatalogKind},
    metrics::{self, Sample},
    missed::{self, MissedTrade},
    report::{MonthlyReport, PeriodFigures},
    review::{Period, PeriodReview},
    time,
};

use crate::{
    insight,
    snapshot::{Snapshot, TradeRow},
};

/// What the month's report needs besides the snapshot: rows the snapshot does not carry.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Extras {
    /// The missed trades seen in the month, for the account in view.
    pub(crate) missed: Vec<MissedTrade>,
    /// The trader's names for the missed-trade reasons.
    pub(crate) reasons: Vec<CatalogItem>,
    /// The monthly review, when written.
    pub(crate) review: Option<PeriodReview>,
}

/// Reads the [`Extras`] of the month holding `month`, for `account` (`None`: all accounts).
///
/// # Errors
///
/// [`DbError`] when a query fails or a stored row does not parse.
pub(crate) fn load_extras(db: &Db, account: Option<i64>, month: NaiveDate) -> Result<Extras, DbError> {
    let (first, last) = Period::Month.bounds(month);
    let day_start = |day: NaiveDate| day.and_hms_opt(0, 0, 0).map_or(0, time::to_ms);
    let (from, to) = (day_start(first), day_start(last + chrono::Duration::days(1)) - 1);
    let missed = db
        .missed_trades(from, to)?
        .into_iter()
        .filter(|m| account.is_none_or(|a| m.account_id.is_none_or(|own| own == a)))
        .collect();
    Ok(Extras {
        missed,
        reasons: db.catalog(CatalogKind::MissedReason)?,
        review: db.review(account, Period::Month, first)?,
    })
}

/// The name of the account in view, or `Tất cả tài khoản`.
pub(crate) fn account_name(snapshot: &Snapshot, account: Option<i64>) -> String {
    account
        .and_then(|id| snapshot.accounts.iter().find(|a| a.id == id))
        .map_or_else(|| "Tất cả tài khoản".to_owned(), tradelens_core::Account::title)
}

/// The closed trades that closed from `first` to `last` (days on the trader's clock), oldest first.
fn closed_between(snapshot: &Snapshot, first: NaiveDate, last: NaiveDate) -> Vec<&TradeRow> {
    let clock = insight::summary_clock(snapshot);
    let mut rows: Vec<&TradeRow> = snapshot
        .rows
        .iter()
        .filter(|row| row.trade.is_closed())
        .filter(|row| {
            let day = clock.day(row.sort_time());
            day >= first && day <= last
        })
        .collect();
    rows.sort_by_key(|row| (row.sort_time(), row.trade.key.position_id));
    rows
}

fn samples<'a>(rows: &[&'a TradeRow]) -> Vec<Sample<'a>> {
    rows.iter().map(|row| Sample { trade: &row.trade, r: row.r }).collect()
}

/// The cumulative R after each trade that has one, oldest first.
fn r_curve(rows: &[&TradeRow]) -> Vec<f64> {
    rows.iter()
        .filter_map(|row| row.r)
        .scan(0.0, |total, r| {
            *total += r;
            Some(*total)
        })
        .collect()
}

/// The playbook a trade belongs to; trades with none read as `Không playbook`.
fn setup_of(row: &TradeRow) -> String {
    if row.setup.is_empty() { "Không playbook".to_owned() } else { row.setup.clone() }
}

/// The figures a week's or month's share card is made of.
pub(crate) fn period_figures(snapshot: &Snapshot, period: Period, start: NaiveDate) -> PeriodFigures {
    let (first, last) = period.bounds(start);
    let rows = closed_between(snapshot, first, last);
    let all = samples(&rows);
    let net: f64 = all.iter().map(Sample::net).sum();
    let wins = all.iter().filter(|x| x.net() > 0.0).count();
    let gross_profit: f64 = all.iter().map(Sample::net).filter(|n| *n > 0.0).sum();
    let gross_loss: f64 = all.iter().map(Sample::net).filter(|n| *n < 0.0).sum();
    let mut by_setup: BTreeMap<&str, usize> = BTreeMap::new();
    for row in rows.iter().filter(|row| !row.setup.is_empty()) {
        *by_setup.entry(row.setup.as_str()).or_default() += 1;
    }
    let top_setup =
        by_setup.into_iter().max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0))).map(|(name, _)| name.to_owned());
    let measured: Vec<f64> = rows.iter().filter_map(|row| row.r).collect();
    PeriodFigures {
        trades: rows.len(),
        wins,
        net,
        total_r: (!measured.is_empty()).then(|| measured.iter().sum()),
        profit_factor: (gross_loss < -1e-6).then(|| gross_profit / -gross_loss),
        top_setup,
        r_curve: r_curve(&rows),
        currency: snapshot.currency.clone(),
    }
}

/// The monthly report of `month` for the accounts in the snapshot, `account` being the one in
/// view (`None` for all).
pub(crate) fn build(snapshot: &Snapshot, account: Option<i64>, month: NaiveDate, extras: &Extras) -> MonthlyReport {
    let (first, last) = Period::Month.bounds(month);
    let clock = insight::summary_clock(snapshot);
    let rows = closed_between(snapshot, first, last);
    let in_month = samples(&rows);

    // The balance the month starts from: what the accounts began with, plus everything before it.
    let before: f64 = snapshot
        .rows
        .iter()
        .filter(|row| row.trade.is_closed() && clock.day(row.sort_time()) < first)
        .map(|row| row.trade.net())
        .sum();
    let moved_before: f64 =
        snapshot.cash.iter().filter(|&&(at, _)| clock.day(at) < first).map(|&(_, amount)| amount).sum();
    let cash: Vec<(i64, f64)> =
        snapshot.cash.iter().copied().filter(|&(at, _)| (first..=last).contains(&clock.day(at))).collect();
    let start = snapshot.start_balance + before + moved_before;

    let stats = metrics::stats(&in_month, start, &cash, clock);
    let curve = metrics::equity_curve(&in_month, start, &cash);
    let days = metrics::daily(&in_month, clock);
    let weeks = metrics::month_weeks(month.year(), month.month(), &days);

    let setups: HashMap<&TradeKey, String> = rows.iter().map(|row| (row.key(), setup_of(row))).collect();
    let by_playbook = {
        let mut groups =
            metrics::breakdown(&in_month, |x| setups.get(&x.trade.key).cloned().into_iter().collect(), String::clone);
        groups.sort_by(|a, b| b.trades.cmp(&a.trades).then_with(|| a.label.cmp(&b.label)));
        groups
    };
    let mistake_label = |key: &str| {
        snapshot
            .catalogs
            .mistakes
            .iter()
            .find(|item| item.key == key)
            .map_or_else(|| key.to_owned(), |i| i.label.clone())
    };
    let mistakes = metrics::mistake_cost(
        &in_month,
        |x| snapshot.journals.get(&x.trade.key).map_or_else(Vec::new, |e| e.mistakes.clone()),
        mistake_label,
    );
    let reason_name = |key: &str| {
        extras.reasons.iter().find(|item| item.key == key).map_or_else(|| key.to_owned(), |i| i.label.clone())
    };
    MonthlyReport::new(account_name(snapshot, account), snapshot.currency.clone(), first, stats)
        .with_curves(curve, r_curve(&rows))
        .with_calendar(weeks)
        .with_playbooks(by_playbook)
        .with_mistakes(mistakes)
        .with_missed(missed::summarise(&extras.missed), reason_name)
        .with_review(extras.review.clone())
}

#[cfg(test)]
mod tests {
    use tradelens_core::{
        Deal, DealBatch, DealEntry, DealKind, JournalEntry, TradeKey,
        model::AccountInfo,
        report::{Privacy, pdf_bytes},
        time::parse_mt5_time,
    };

    use super::*;
    use crate::snapshot::{Query, Range, load};

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
            sl: 1.09,
            tp: 0.0,
            risk: Some(50.0),
            result_r: None,
            reason: None,
        }
    }

    /// October and November trades on one account, with one deposit in October.
    fn seeded() -> Db {
        use DealEntry::{In, Out};
        use DealKind::{Balance, Buy, Sell};
        let mut db = Db::in_memory().expect("a memory database opens");
        let batch = DealBatch {
            account: AccountInfo {
                login: "1".into(),
                server: "S".into(),
                currency: "USD".into(),
                balance: Some(1_300.0),
                ..AccountInfo::default()
            },
            deals: vec![
                deal(1, 0, "2026.10.01 00:00:00", Balance, DealEntry::None, 1000.0),
                deal(2, 9, "2026.10.20 09:00:00", Buy, In, 0.0),
                deal(3, 9, "2026.10.20 10:00:00", Sell, Out, 100.0),
                deal(4, 10, "2026.11.10 09:00:00", Buy, In, 0.0),
                deal(5, 10, "2026.11.10 10:00:00", Sell, Out, 150.0),
                deal(6, 11, "2026.11.20 09:00:00", Sell, In, 0.0),
                deal(7, 11, "2026.11.20 10:00:00", Buy, Out, -50.0),
                deal(8, 12, "2026.11.21 09:00:00", Sell, In, 0.0),
                deal(9, 12, "2026.11.21 10:00:00", Buy, Out, 100.0),
            ],
            positions: Vec::new(),
        };
        db.upsert_batch(&batch, "test").expect("the batch is stored");
        let flagged =
            JournalEntry { setup: "Breakout".into(), mistakes: vec!["move_sl".into()], ..JournalEntry::default() };
        db.save_journal(&TradeKey { account_id: 1, position_id: 11, segment: 0 }, &flagged).expect("stored");
        let other = JournalEntry { setup: "Breakout".into(), ..JournalEntry::default() };
        db.save_journal(&TradeKey { account_id: 1, position_id: 10, segment: 0 }, &other).expect("stored");
        db
    }

    fn snapshot() -> Snapshot {
        let query = Query { account: None, range: Range::All, now_utc_ms: 0, local_offset_min: 0 };
        load(&seeded(), &query).expect("the snapshot loads")
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("a test date")
    }

    #[test]
    fn the_months_figures_count_only_its_own_trades() {
        let snap = snapshot();
        let figures = period_figures(&snap, Period::Month, day("2026-11-15"));
        assert_eq!((figures.trades, figures.wins), (3, 2));
        assert!((figures.net - 200.0).abs() < 1e-6, "{}", figures.net);
        assert_eq!(figures.top_setup.as_deref(), Some("Breakout"));
        assert!((figures.profit_factor.expect("a loss exists") - 5.0).abs() < 1e-6);
        assert_eq!(figures.r_curve.len(), 3, "each trade has an R from its risk");
        let week = period_figures(&snap, Period::Week, day("2026-11-20"));
        assert_eq!(week.trades, 2, "the week of 16/11 holds the 20th and the 21st");
        let empty = period_figures(&snap, Period::Month, day("2026-08-01"));
        assert_eq!((empty.trades, empty.total_r, empty.profit_factor), (0, None, None));
    }

    #[test]
    fn the_report_starts_from_the_balance_before_the_month() {
        let snap = snapshot();
        let report = build(&snap, None, day("2026-11-15"), &Extras::default());
        assert_eq!(report.month, day("2026-11-01"));
        assert_eq!(report.stats.trades, 3);
        assert!((report.stats.start_balance - 1_100.0).abs() < 1e-6, "1000 deposit + 100 in October");
        assert!((report.stats.end_balance - 1_300.0).abs() < 1e-6, "{}", report.stats.end_balance);
        assert_eq!(report.curve.len(), 4, "the start and one point per trade");
        assert_eq!(report.weeks.len(), 6, "1 November 2026 is a Sunday");
        assert_eq!(report.by_playbook[0].label, "Breakout");
        assert_eq!(report.by_playbook[0].trades, 2);
        assert_eq!(report.by_playbook[1].label, "Không playbook");
        assert_eq!(report.mistakes.len(), 1);
        assert_eq!(report.mistakes[0].label, "move_sl");
        assert_eq!(report.account, "Tất cả tài khoản");
    }

    #[test]
    fn the_extras_come_from_the_database_by_month_and_account() {
        let mut db = seeded();
        let second = DealBatch {
            account: AccountInfo { login: "2".into(), server: "S".into(), ..AccountInfo::default() },
            deals: Vec::new(),
            positions: Vec::new(),
        };
        db.upsert_batch(&second, "test").expect("a second account is stored");
        let seen = |at: &str, account: Option<i64>| MissedTrade {
            id: 0,
            account_id: account,
            symbol: "XAUUSD".into(),
            side: tradelens_core::Side::Long,
            seen_at: parse_mt5_time(at).expect("a time"),
            playbook_id: None,
            setup_grade: None,
            reason: Some("fear".into()),
            entry: Some(2650.0),
            stop_loss: Some(2645.0),
            take_profit: None,
            outcome_price: Some(2665.0),
            notes: String::new(),
            screenshots: Vec::new(),
            custom: tradelens_core::catalog::CustomValues::new(),
        };
        for m in
            [seen("2026.11.05 10:00:00", None), seen("2026.10.31 23:59:00", None), seen("2026.11.30 23:59:00", Some(2))]
        {
            db.save_missed_trade(&m).expect("stored");
        }
        let extras = load_extras(&db, Some(1), day("2026-11-12")).expect("reads");
        assert_eq!(extras.missed.len(), 1, "one in November for account 1: the October one and account 2's are out");
        assert!(extras.review.is_none());
        let all = load_extras(&db, None, day("2026-11-12")).expect("reads");
        assert_eq!(all.missed.len(), 2, "with all accounts, account 2's counts too");
    }

    #[test]
    fn a_report_from_the_snapshot_makes_a_pdf_in_each_privacy_level() {
        let snap = snapshot();
        let report = build(&snap, None, day("2026-11-15"), &Extras::default());
        for privacy in [Privacy::ShowMoney, Privacy::OnlyR, Privacy::NoFigures] {
            let bytes = pdf_bytes(&report, privacy).expect("the PDF is made");
            assert!(bytes.starts_with(b"%PDF-"), "{privacy:?}");
        }
    }
}
