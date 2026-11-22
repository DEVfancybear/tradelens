//! What the calendar, the analytics and the overview's side panels read out of a
//! [`Snapshot`]: the closed trades of a range, broken down by symbol, hour, weekday, session,
//! setup, tag, emotion and mistake, the daily results, and how far today has gone toward the
//! account's rules. Pure functions over the snapshot, so they are tested without a window.

use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, NaiveDate};
use tradelens_core::{
    Account, Rules, Side, TradeKey,
    catalog::CatalogItem,
    metrics::{self, DayStat, Group, MistakeCost, RuleUsage, Sample},
    rules::{self, Violation},
    time::{Clock, Session, hour_of, weekday_index},
};

use crate::snapshot::{Range, Snapshot, TradeRow};

/// The short names of the weekdays, Monday first.
pub(crate) const WEEKDAYS: [&str; 7] = ["T2", "T3", "T4", "T5", "T6", "T7", "CN"];

/// The day a range starts on; `None` for no limit.
pub(crate) fn range_start(range: Range, today: NaiveDate) -> Option<NaiveDate> {
    let back = |days: u64| today.checked_sub_days(chrono::Days::new(days));
    match range {
        Range::Days7 => back(7),
        Range::Days30 => back(30),
        Range::Days90 => back(90),
        Range::YearToDate => NaiveDate::from_ymd_opt(today.year(), 1, 1),
        Range::All => None,
    }
}

/// The clock days are grouped by: the first known server offset of the trades shown.
pub(crate) fn summary_clock(snapshot: &Snapshot) -> Clock {
    let known = snapshot.rows.iter().map(TradeRow::clock).find(|clock| clock.server_offset_min.is_some());
    known.or_else(|| snapshot.rows.first().map(TradeRow::clock)).unwrap_or_default()
}

/// Today's date on the clock the trades' days use: the server's offset, when known, carries
/// "now" into the trader's day; without it the stored times are read as UTC, so is "now".
pub(crate) fn today(snapshot: &Snapshot, now_utc_ms: i64) -> NaiveDate {
    let clock = summary_clock(snapshot);
    let now_server = now_utc_ms + clock.server_offset_min.map_or(0, |minutes| i64::from(minutes) * 60_000);
    clock.day(now_server)
}

/// [`today`] by the machine's clock.
pub(crate) fn today_now(snapshot: &Snapshot) -> NaiveDate {
    today(snapshot, chrono::Utc::now().timestamp_millis())
}

fn closed_samples<'a>(rows: &[&'a TradeRow]) -> Vec<Sample<'a>> {
    rows.iter().map(|row| Sample { trade: &row.trade, r: row.r }).collect()
}

/// The closed trades that closed on or after `range` starts (a day on the summary clock).
fn closed_in_range(snapshot: &Snapshot, range: Range, today: NaiveDate) -> Vec<&TradeRow> {
    let clock = summary_clock(snapshot);
    let start = range_start(range, today);
    snapshot
        .rows
        .iter()
        .filter(|row| row.trade.is_closed())
        .filter(|row| start.is_none_or(|from| clock.day(row.sort_time()) >= from))
        .collect()
}

/// The result of every day any trade closed on, over the whole history of the chosen accounts.
pub(crate) fn daily_results(snapshot: &Snapshot) -> BTreeMap<NaiveDate, DayStat> {
    let rows: Vec<&TradeRow> = snapshot.rows.iter().filter(|row| row.trade.is_closed()).collect();
    metrics::daily(&closed_samples(&rows), summary_clock(snapshot))
}

/// The trades that closed on `day`, newest first.
pub(crate) fn trades_on(snapshot: &Snapshot, day: NaiveDate) -> Vec<&TradeRow> {
    let clock = summary_clock(snapshot);
    snapshot.rows.iter().filter(|row| row.trade.is_closed() && clock.day(row.sort_time()) == day).collect()
}

/// One cell of the entry hour by weekday heatmap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HeatCell {
    /// 0 for Monday.
    pub(crate) weekday: u32,
    pub(crate) hour: u32,
    pub(crate) net: f64,
    pub(crate) trades: usize,
}

/// The mistakes' cost over the range, and what it adds up to.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct MistakeReport {
    pub(crate) costs: Vec<MistakeCost>,
    /// Net of every trade that carries a mistake.
    pub(crate) net: f64,
    pub(crate) total_r: f64,
    /// How many trades carry at least one mistake.
    pub(crate) trades: usize,
}

/// Everything the analytics page shows, for one range.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Analysis {
    pub(crate) trades: usize,
    pub(crate) net: f64,
    /// Best result first.
    pub(crate) by_symbol: Vec<Group>,
    pub(crate) by_weekday: Vec<Group>,
    pub(crate) by_hour: Vec<Group>,
    pub(crate) by_session: Vec<Group>,
    pub(crate) by_setup: Vec<Group>,
    pub(crate) by_tag: Vec<Group>,
    pub(crate) by_emotion: Vec<Group>,
    pub(crate) by_side: Vec<Group>,
    pub(crate) heat: Vec<HeatCell>,
    pub(crate) mistakes: MistakeReport,
    pub(crate) r_buckets: Vec<(&'static str, usize)>,
    pub(crate) r_trades: usize,
    pub(crate) avg_r: Option<f64>,
    /// Trades that lost more than two R.
    pub(crate) beyond_two_r: usize,
}

fn label_of(items: &[CatalogItem], key: &str) -> String {
    items.iter().find(|item| item.key == key).map_or_else(|| key.to_owned(), |item| item.label.clone())
}

fn best_first(mut groups: Vec<Group>) -> Vec<Group> {
    groups.sort_by(|a, b| b.net.total_cmp(&a.net));
    groups
}

/// The trades closed in `range`, broken down every way the analytics page shows.
pub(crate) fn analyse(snapshot: &Snapshot, range: Range, today: NaiveDate) -> Analysis {
    let rows = closed_in_range(snapshot, range, today);
    let samples = closed_samples(&rows);
    let by_key: HashMap<&TradeKey, &TradeRow> = rows.iter().map(|row| (row.key(), *row)).collect();
    let row_of = |x: &Sample<'_>| by_key.get(&x.trade.key).copied();
    let journal = |x: &Sample<'_>| snapshot.journals.get(&x.trade.key);
    let catalogs = &snapshot.catalogs;

    let weekday_label = |index: &u32| WEEKDAYS.get(usize::try_from(*index).unwrap_or(usize::MAX)).unwrap_or(&"?");
    let opened_weekday = |x: &Sample<'_>| row_of(x).map(|row| weekday_index(row.opened().date()));
    let opened_hour = |x: &Sample<'_>| row_of(x).map(|row| hour_of(row.opened()));

    let by_symbol = metrics::breakdown(&samples, |x| vec![x.trade.symbol.clone()], String::clone);
    let by_weekday =
        metrics::breakdown(&samples, |x| opened_weekday(x).into_iter().collect(), |d| (*weekday_label(d)).to_owned());
    let by_hour = metrics::breakdown(&samples, |x| opened_hour(x).into_iter().collect(), |h| format!("{h:02}:00"));
    let by_session = metrics::breakdown(
        &samples,
        |x| {
            row_of(x)
                .map(|row| Session::of_utc_hour(hour_of(row.clock().utc(row.trade.open_time))))
                .into_iter()
                .collect()
        },
        |s: &Session| s.label().to_owned(),
    );
    let by_setup = metrics::breakdown(
        &samples,
        |x| row_of(x).filter(|row| !row.setup.is_empty()).map(|row| row.setup.clone()).into_iter().collect(),
        String::clone,
    );
    let by_tag = metrics::breakdown(
        &samples,
        |x| journal(x).map_or_else(Vec::new, |e| e.tags.clone()),
        |key: &String| label_of(&catalogs.tags, key),
    );
    let by_emotion = metrics::breakdown(
        &samples,
        |x| journal(x).filter(|e| !e.emotion_before.is_empty()).map(|e| e.emotion_before.clone()).into_iter().collect(),
        |key: &String| label_of(&catalogs.emotions, key),
    );
    let by_side = metrics::breakdown(
        &samples,
        |x| vec![u8::from(x.trade.side == Side::Short)],
        |short| (if *short == 1 { "Sell" } else { "Buy" }).to_owned(),
    );

    let mut heat: BTreeMap<(u32, u32), (f64, usize)> = BTreeMap::new();
    for x in &samples {
        if let (Some(weekday), Some(hour)) = (opened_weekday(x), opened_hour(x)) {
            let cell = heat.entry((weekday, hour)).or_default();
            cell.0 += x.net();
            cell.1 += 1;
        }
    }

    let costs = metrics::mistake_cost(
        &samples,
        |x| journal(x).map_or_else(Vec::new, |e| e.mistakes.clone()),
        |key| label_of(&catalogs.mistakes, key),
    );
    let with_mistake: Vec<&Sample<'_>> =
        samples.iter().filter(|x| journal(x).is_some_and(|e| !e.mistakes.is_empty())).collect();
    let mistakes = MistakeReport {
        costs,
        net: with_mistake.iter().map(|x| x.net()).sum(),
        total_r: with_mistake.iter().filter_map(|x| x.r).sum(),
        trades: with_mistake.len(),
    };

    let r_values: Vec<f64> = samples.iter().filter_map(|x| x.r).collect();
    let r_trades = r_values.len();
    let avg_r =
        (r_trades > 0).then(|| r_values.iter().sum::<f64>() / f64::from(u32::try_from(r_trades).unwrap_or(u32::MAX)));

    Analysis {
        trades: samples.len(),
        net: samples.iter().map(Sample::net).sum(),
        by_symbol: best_first(by_symbol),
        by_weekday,
        by_hour,
        by_session,
        by_setup: best_first(by_setup),
        by_tag: best_first(by_tag),
        by_emotion: best_first(by_emotion),
        by_side,
        heat: heat
            .into_iter()
            .map(|((weekday, hour), (net, trades))| HeatCell { weekday, hour, net, trades })
            .collect(),
        mistakes,
        r_buckets: metrics::r_distribution(&samples),
        r_trades,
        avg_r,
        beyond_two_r: r_values.iter().filter(|r| **r < -2.0).count(),
    }
}

/// The running total of R after each trade of `range` that has an R, as `(close time, total)`,
/// starting with a zero at the first of those trades' opening.
pub(crate) fn r_curve(snapshot: &Snapshot, range: Range, today: NaiveDate) -> Vec<(i64, f64)> {
    let mut rows: Vec<&TradeRow> =
        closed_in_range(snapshot, range, today).into_iter().filter(|row| row.r.is_some()).collect();
    rows.sort_by_key(|row| (row.sort_time(), row.trade.key.position_id));
    let Some(first) = rows.first() else { return Vec::new() };
    let mut curve = vec![(first.trade.open_time, 0.0)];
    let mut total = 0.0;
    for row in &rows {
        total += row.r.unwrap_or(0.0);
        curve.push((row.sort_time(), total));
    }
    curve
}

/// The best and the worst hour of entry by net result, as `(hour, net)`; `None` without trades.
pub(crate) fn hour_extremes(analysis: &Analysis) -> Option<((u32, f64), (u32, f64))> {
    let mut by_hour: BTreeMap<u32, f64> = BTreeMap::new();
    for cell in &analysis.heat {
        *by_hour.entry(cell.hour).or_default() += cell.net;
    }
    let best = by_hour.iter().max_by(|a, b| a.1.total_cmp(b.1)).map(|(h, n)| (*h, *n))?;
    let worst = by_hour.iter().min_by(|a, b| a.1.total_cmp(b.1)).map(|(h, n)| (*h, *n))?;
    Some((best, worst))
}

/// The rules of the first account (among `rules`, in the order given) that sets any limit.
pub(crate) fn rules_in_force(rules: &[(i64, Rules)]) -> Option<&(i64, Rules)> {
    rules.iter().find(|(_, r)| *r != Rules::default())
}

/// The balance the account has now, as far as the snapshot knows it.
fn current_balance(snapshot: &Snapshot) -> f64 {
    snapshot.curve.last().map_or(snapshot.stats.end_balance, |point| point.balance)
}

/// How far the trades closed `today` on `account` have gone toward each limit of `rules`.
pub(crate) fn discipline_today(snapshot: &Snapshot, account: i64, rules: &Rules, today: NaiveDate) -> Vec<RuleUsage> {
    let clock = summary_clock(snapshot);
    let rows: Vec<&TradeRow> = snapshot
        .rows
        .iter()
        .filter(|row| row.trade.key.account_id == account && row.trade.is_closed())
        .filter(|row| clock.day(row.sort_time()) == today)
        .collect();
    metrics::rule_usage(rules, &closed_samples(&rows), current_balance(snapshot))
}

/// The rules broken by the trades of the chosen accounts, oldest first. The balance a trade is
/// judged against is rebuilt from the account's results (deposits are not in the snapshot).
pub(crate) fn violations(snapshot: &Snapshot, rules: &[(i64, Rules)]) -> Vec<Violation> {
    let clock = summary_clock(snapshot);
    let mut all = Vec::new();
    for (account_id, account_rules) in rules {
        let rows: Vec<&TradeRow> = snapshot
            .rows
            .iter()
            .filter(|row| row.trade.key.account_id == *account_id && row.trade.is_closed())
            .collect();
        let samples = closed_samples(&rows);
        let start = snapshot
            .accounts
            .iter()
            .find(|a: &&Account| a.id == *account_id)
            .map_or(0.0, |account| metrics::start_balance(account, &samples, &[]));
        let curve = metrics::equity_curve(&samples, start, &[]);
        let balance_at = |ms: i64| curve.iter().take_while(|p| p.time_ms <= ms).last().map_or(start, |p| p.balance);
        all.extend(rules::evaluate(account_rules, &samples, balance_at, clock));
    }
    all.sort_by_key(|v| v.day);
    all
}

#[cfg(test)]
mod tests {
    use tradelens_core::{
        Db, DealBatch, DealEntry, DealKind, JournalEntry,
        model::{AccountInfo, Deal},
        time::parse_mt5_time,
    };

    use super::*;
    use crate::snapshot::{self, Query};

    fn deal(ticket: i64, position: i64, symbol: &str, at: &str, kind: DealKind, entry: DealEntry, profit: f64) -> Deal {
        Deal {
            ticket,
            order: ticket,
            position_id: position,
            time_msc: parse_mt5_time(at).expect("fixture times parse"),
            symbol: symbol.into(),
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

    /// A trade that opens at `open` and closes an hour later.
    fn trade(position: i64, symbol: &str, open: &str, close: &str, profit: f64) -> Vec<Deal> {
        vec![
            deal(position * 10, position, symbol, open, DealKind::Buy, DealEntry::In, 0.0),
            deal(position * 10 + 1, position, symbol, close, DealKind::Sell, DealEntry::Out, profit),
        ]
    }

    /// Four trades: EURUSD +100 (Mon 08:00), EURUSD -40 (Mon 22:00), XAUUSD +60 (Tue 14:00),
    /// GBPJPY -20 (a month earlier).
    fn seeded() -> (Db, Snapshot) {
        seeded_with(None)
    }

    fn seeded_with(server_offset_min: Option<i32>) -> (Db, Snapshot) {
        let mut db = Db::in_memory().expect("a memory database opens");
        let mut deals = trade(1, "EURUSD", "2026.11.16 08:00:00", "2026.11.16 09:00:00", 100.0);
        deals.extend(trade(2, "EURUSD", "2026.11.16 22:00:00", "2026.11.16 23:00:00", -40.0));
        deals.extend(trade(3, "XAUUSD", "2026.11.17 14:00:00", "2026.11.17 15:00:00", 60.0));
        deals.extend(trade(4, "GBPJPY", "2026.10.05 10:00:00", "2026.10.05 11:00:00", -20.0));
        let batch = DealBatch {
            account: AccountInfo {
                login: "1".into(),
                server: "S".into(),
                currency: "USD".into(),
                balance: Some(1000.0),
                server_offset_min,
                ..AccountInfo::default()
            },
            deals,
            positions: Vec::new(),
        };
        db.upsert_batch(&batch, "test").expect("the batch is stored");
        let keys: Vec<TradeKey> = db.journals(None).expect("journals read").into_iter().map(|(k, _)| k).collect();
        assert!(keys.is_empty(), "nothing is journalled yet");
        let snapshot = load(&db);
        (db, snapshot)
    }

    fn load(db: &Db) -> Snapshot {
        let now = parse_mt5_time("2026.11.20 12:00:00").expect("a valid time");
        let query = Query { account: None, range: Range::All, now_utc_ms: now, local_offset_min: 420 };
        snapshot::load(db, &query).expect("loads")
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 20).expect("a real date")
    }

    fn key_of(snapshot: &Snapshot, symbol: &str, net: f64) -> TradeKey {
        let row = snapshot
            .rows
            .iter()
            .find(|r| r.trade.symbol == symbol && (r.trade.net() - net).abs() < 1e-9)
            .expect("the trade is there");
        row.key().clone()
    }

    #[test]
    fn a_range_keeps_only_the_trades_that_closed_inside_it() {
        let (_db, snapshot) = seeded();
        assert_eq!(analyse(&snapshot, Range::All, today()).trades, 4);
        let recent = analyse(&snapshot, Range::Days30, today());
        assert_eq!(recent.trades, 3, "the trade of 5 October is 46 days back");
        assert!((recent.net - 120.0).abs() < 1e-9);
        assert_eq!(
            analyse(&snapshot, Range::Days7, today()).trades,
            3,
            "the 16th and 17th are within 7 days of the 20th"
        );
        let later = NaiveDate::from_ymd_opt(2026, 11, 30).expect("a real date");
        assert_eq!(analyse(&snapshot, Range::Days7, later).trades, 0);
        assert_eq!(analyse(&snapshot, Range::YearToDate, today()).trades, 4);
    }

    #[test]
    fn symbols_come_best_first() {
        let (_db, snapshot) = seeded();
        let a = analyse(&snapshot, Range::All, today());
        let names: Vec<&str> = a.by_symbol.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(names, ["EURUSD", "XAUUSD", "GBPJPY"]);
        assert_eq!(a.by_symbol[0].trades, 2);
        assert!((a.by_symbol[0].net - 60.0).abs() < 1e-9);
    }

    #[test]
    fn entries_group_by_local_weekday_hour_and_utc_session() {
        let (_db, snapshot) = seeded();
        let a = analyse(&snapshot, Range::Days30, today());
        let weekdays: Vec<(&str, usize)> = a.by_weekday.iter().map(|g| (g.label.as_str(), g.trades)).collect();
        assert_eq!(weekdays, [("T2", 2), ("T3", 1)]);
        assert!(a.heat.contains(&HeatCell { weekday: 0, hour: 8, net: 100.0, trades: 1 }));
        assert!(a.heat.contains(&HeatCell { weekday: 0, hour: 22, net: -40.0, trades: 1 }));
        let sessions: Vec<&str> = a.by_session.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(
            sessions,
            ["Phiên Âu (07–12 UTC)", "Âu–Mỹ giao thoa (12–16 UTC)", "Cuối ngày (21–24 UTC)"],
            "08:00, 14:00 and 22:00 fall in London, the overlap and the late hours"
        );
        let ((best, best_net), (worst, worst_net)) = hour_extremes(&a).expect("there are trades");
        assert_eq!((best, worst), (8, 22));
        assert!((best_net - 100.0).abs() < 1e-9 && (worst_net + 40.0).abs() < 1e-9);
    }

    #[test]
    fn setups_tags_and_mistakes_come_from_the_journal() {
        let (mut db, snapshot) = seeded();
        let losing = key_of(&snapshot, "EURUSD", -40.0);
        let winner = key_of(&snapshot, "EURUSD", 100.0);
        let entry = JournalEntry {
            setup: "Range".into(),
            tags: vec!["news".into()],
            mistakes: vec!["early_entry".into()],
            emotion_before: "fomo".into(),
            ..JournalEntry::default()
        };
        db.save_journal(&losing, &entry).expect("saves");
        db.save_journal(&winner, &JournalEntry { setup: "Range".into(), ..JournalEntry::default() }).expect("saves");
        let a = analyse(&load(&db), Range::All, today());

        assert_eq!(a.by_setup.len(), 1);
        assert_eq!((a.by_setup[0].label.as_str(), a.by_setup[0].trades), ("Range", 2));
        assert_eq!(a.by_tag.len(), 1);
        assert_eq!(a.by_tag[0].trades, 1);
        assert_eq!(a.by_emotion.len(), 1);
        assert_eq!(a.mistakes.trades, 1);
        assert!((a.mistakes.net + 40.0).abs() < 1e-9);
        assert_eq!(a.mistakes.costs.len(), 1);
        assert!((a.mistakes.costs[0].net + 40.0).abs() < 1e-9);
        assert!((a.mistakes.costs[0].share_of_losses - 40.0 / 60.0).abs() < 1e-9, "40 of the 60 lost");
    }

    #[test]
    fn days_group_on_the_close_and_list_their_trades() {
        let (_db, snapshot) = seeded();
        let days = daily_results(&snapshot);
        let monday = NaiveDate::from_ymd_opt(2026, 11, 16).expect("a real date");
        assert_eq!(days[&monday].trades, 2);
        assert!((days[&monday].net - 60.0).abs() < 1e-9);
        let listed: Vec<&str> = trades_on(&snapshot, monday).iter().map(|r| r.trade.symbol.as_str()).collect();
        assert_eq!(listed, ["EURUSD", "EURUSD"]);
        assert_eq!(trades_on(&snapshot, today()).len(), 0);
    }

    #[test]
    fn the_r_curve_adds_up_trades_that_have_an_r() {
        let (_db, snapshot) = seeded();
        // The fixture trades close at their entry price with no stop: none has an R.
        assert_eq!(r_curve(&snapshot, Range::All, today()).len(), 0);
    }

    #[test]
    fn today_follows_the_server_offset_when_it_is_known() {
        let (_db, snapshot) = seeded();
        let late = parse_mt5_time("2026.11.20 23:30:00").expect("a valid time");
        assert_eq!(super::today(&snapshot, late), NaiveDate::from_ymd_opt(2026, 11, 20).expect("a real date"));
        let (_db, known) = seeded_with(Some(120));
        // 23:30 UTC is 01:30 the next day at the server's UTC+2, and 06:30 at UTC+7.
        assert_eq!(super::today(&known, late), NaiveDate::from_ymd_opt(2026, 11, 21).expect("a real date"));
    }

    #[test]
    fn rules_in_force_skips_accounts_without_limits() {
        let set = Rules { max_daily_trades: Some(3), ..Rules::default() };
        let rules = [(1, Rules::default()), (2, set)];
        assert_eq!(rules_in_force(&rules).map(|(id, _)| *id), Some(2));
        assert!(rules_in_force(&[(1, Rules::default())]).is_none());
    }

    #[test]
    fn today_is_measured_against_the_rules_of_its_account() {
        let (_db, snapshot) = seeded();
        let account = snapshot.accounts[0].id;
        let rules = Rules { max_daily_trades: Some(4), max_daily_loss: Some(100.0), ..Rules::default() };
        let monday = NaiveDate::from_ymd_opt(2026, 11, 16).expect("a real date");
        let usage = discipline_today(&snapshot, account, &rules, monday);
        let trades = usage.iter().find(|u| u.kind == metrics::RuleKind::DailyTrades).expect("a limit on trades");
        assert!((trades.used - 2.0).abs() < 1e-9);
        let loss = usage.iter().find(|u| u.kind == metrics::RuleKind::DailyLoss).expect("a limit on loss");
        assert!(loss.used.abs() < 1e-9, "the day ended in profit");
        assert!(discipline_today(&snapshot, account + 1, &rules, monday).iter().all(|u| u.used.abs() < 1e-9));
    }
}
