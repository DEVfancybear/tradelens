//! The month grid, its totals and the use of a day's rules, over hand-made trades.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use tradelens_core::{
    Rules, Side, Trade, TradeKey,
    metrics::{self, DayStat, RuleKind, Sample},
    time::{Clock, parse_mt5_time},
};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("a real date")
}

fn trade(position: i64, closed: &str, profit: f64, risk: Option<f64>) -> Trade {
    let close = parse_mt5_time(closed).expect("fixture times parse");
    Trade {
        key: TradeKey { account_id: 1, position_id: position, segment: 0 },
        symbol: "EURUSD".into(),
        side: Side::Long,
        open_time: close - 3_600_000,
        close_time: Some(close),
        volume: 0.1,
        open_price: 1.1,
        close_price: Some(1.1),
        sl: 0.0,
        tp: 0.0,
        commission: 0.0,
        swap: 0.0,
        fee: 0.0,
        profit,
        risk,
        result_r: None,
        magic: 0,
        comment: String::new(),
        manual: false,
    }
}

fn stat(net: f64, trades: usize) -> DayStat {
    DayStat { net, trades, wins: usize::from(net > 0.0), ..DayStat::default() }
}

#[test]
fn a_month_grid_starts_on_monday_and_pads_with_neighbours() {
    // November 2026 starts on a Sunday and ends on a Monday: six weeks.
    let weeks = metrics::month_weeks(2026, 11, &BTreeMap::new());
    assert_eq!(weeks.len(), 6);
    assert_eq!(weeks[0].days[0].date, day(2026, 10, 26));
    assert!(!weeks[0].days[0].in_month);
    assert_eq!(weeks[0].days[6].date, day(2026, 11, 1));
    assert!(weeks[0].days[6].in_month);
    assert_eq!(weeks[5].days[0].date, day(2026, 11, 30));
    assert!(weeks[5].days[0].in_month && !weeks[5].days[1].in_month);

    // February 2027 starts on a Monday and has 28 days: exactly four weeks.
    assert_eq!(metrics::month_weeks(2027, 2, &BTreeMap::new()).len(), 4);
    assert!(metrics::month_weeks(2026, 13, &BTreeMap::new()).is_empty(), "no such month");
}

#[test]
fn a_week_adds_up_only_the_days_inside_the_month() {
    let days = BTreeMap::from([
        (day(2026, 10, 30), stat(500.0, 2)), // a neighbour's day, not counted in November
        (day(2026, 11, 1), stat(-20.0, 1)),
        (day(2026, 11, 3), stat(70.0, 3)),
    ]);
    let weeks = metrics::month_weeks(2026, 11, &days);
    assert_eq!((weeks[0].net, weeks[0].trades), (-20.0, 1));
    assert_eq!((weeks[1].net, weeks[1].trades), (70.0, 3));
    assert_eq!(weeks[0].days[4].stat.map(|s| s.net), Some(500.0), "the neighbour's day still shows its result");
    assert_eq!(weeks[2].trades, 0);
}

#[test]
fn december_rolls_into_january() {
    let weeks = metrics::month_weeks(2026, 12, &BTreeMap::new());
    let last = weeks.last().expect("weeks").days;
    assert!(last.iter().any(|d| d.date == day(2026, 12, 31) && d.in_month));
}

#[test]
fn a_month_summary_picks_the_best_and_worst_day() {
    let days = BTreeMap::from([
        (day(2026, 10, 31), stat(999.0, 9)),
        (day(2026, 11, 2), stat(120.0, 2)),
        (day(2026, 11, 3), stat(-80.0, 1)),
        (day(2026, 11, 4), stat(40.0, 1)),
        (day(2026, 11, 5), stat(-10.0, 1)),
    ]);
    let s = metrics::month_summary(2026, 11, &days);
    assert!((s.net - 70.0).abs() < 1e-9);
    assert_eq!((s.trades, s.green_days, s.red_days), (5, 2, 2));
    assert_eq!(s.best_day, Some((day(2026, 11, 2), 120.0)));
    assert_eq!(s.worst_day, Some((day(2026, 11, 3), -80.0)));
    assert_eq!(metrics::month_summary(2026, 9, &days), metrics::MonthSummary::default());
}

#[test]
fn daily_keeps_the_sum_of_r() {
    let trades = [trade(1, "2026.11.20 10:00:00", 50.0, None), trade(2, "2026.11.20 12:00:00", -20.0, None)];
    let samples = [Sample { trade: &trades[0], r: Some(1.5) }, Sample { trade: &trades[1], r: None }];
    let days = metrics::daily(&samples, Clock::default());
    let d = days[&day(2026, 11, 20)];
    assert_eq!((d.trades, d.r_trades), (2, 1));
    assert!((d.r - 1.5).abs() < 1e-9);
}

#[test]
fn rule_usage_measures_the_day_against_each_limit() {
    let trades = [
        trade(1, "2026.11.20 09:00:00", 30.0, Some(10.0)),
        trade(2, "2026.11.20 10:00:00", -50.0, Some(120.0)),
        trade(3, "2026.11.20 11:00:00", -40.0, None),
    ];
    let samples: Vec<Sample<'_>> = trades.iter().map(|t| Sample { trade: t, r: None }).collect();
    let rules = Rules {
        max_daily_loss: Some(300.0),
        max_daily_trades: Some(5),
        max_risk_pct: Some(1.5),
        max_consecutive_losses: Some(3),
        daily_target: None,
    };
    let usage = metrics::rule_usage(&rules, &samples, 10_000.0);
    let of = |kind: RuleKind| usage.iter().find(|u| u.kind == kind).copied().expect("the rule is set");
    assert!((of(RuleKind::DailyLoss).used - 60.0).abs() < 1e-9);
    assert!((of(RuleKind::DailyLoss).ratio() - 0.2).abs() < 1e-9);
    assert!((of(RuleKind::DailyTrades).used - 3.0).abs() < 1e-9);
    assert!((of(RuleKind::RiskPerTrade).used - 1.2).abs() < 1e-9, "120 of 10,000 is 1.2 percent");
    assert!((of(RuleKind::LosingStreak).used - 2.0).abs() < 1e-9, "the last two trades lost");
    assert!(metrics::rule_usage(&Rules::default(), &samples, 10_000.0).is_empty(), "no rule, nothing to measure");
}

#[test]
fn a_profit_today_costs_nothing_against_the_loss_limit() {
    let trades = [trade(1, "2026.11.20 09:00:00", 80.0, None)];
    let samples = [Sample { trade: &trades[0], r: None }];
    let rules = Rules { max_daily_loss: Some(100.0), ..Rules::default() };
    let usage = metrics::rule_usage(&rules, &samples, 0.0);
    assert_eq!(usage.len(), 1);
    assert!(usage[0].used.abs() < 1e-12);
}
