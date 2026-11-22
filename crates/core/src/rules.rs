//! Discipline: an account's rules checked against its trades, day by day.
//!
//! Days are the trader's local days ([`Clock::day`]). A day's loss counts on the day its
//! trades closed; the number of trades counts on the day they opened.

use std::collections::BTreeMap;

use chrono::NaiveDate;

use crate::{
    metrics::{EquityPoint, Sample},
    model::{Rules, TradeKey},
    time::Clock,
};

/// A net result closer to zero than this is a break-even, not a loss.
const FLAT: f64 = 1e-6;

/// One rule broken, on one day or by one trade.
#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    pub day: NaiveDate,
    pub kind: ViolationKind,
    /// The trade that broke it, when one trade did.
    pub trade: Option<TradeKey>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ViolationKind {
    /// The day's net loss passed the limit: (loss, limit).
    DailyLoss(f64, f64),
    /// More trades than allowed: (count, limit).
    DailyTrades(u32, u32),
    /// A trade risked more than allowed: (percent, limit).
    RiskPerTrade(f64, f64),
    /// A trade opened after the allowed losing streak: (streak, limit).
    AfterLosingStreak(u32, u32),
}

impl ViolationKind {
    pub fn label(&self) -> String {
        match self {
            Self::DailyLoss(loss, limit) => format!("Lỗ trong ngày {loss:.2} vượt giới hạn {limit:.2}"),
            Self::DailyTrades(n, limit) => format!("{n} lệnh trong ngày, giới hạn {limit}"),
            Self::RiskPerTrade(pct, limit) => format!("Rủi ro {pct:.2}% vượt {limit:.2}%/lệnh"),
            Self::AfterLosingStreak(n, limit) => format!("Vào lệnh sau {n} lệnh thua liên tiếp (giới hạn {limit})"),
        }
    }
}

/// What a day came to against the rules, for the status bars.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DayStatus {
    /// The net result of the trades that closed on the day.
    pub net: f64,
    /// The trades opened on the day.
    pub trades: u32,
    /// Losing trades in a row at the end of the day's closed trades.
    pub losing_streak: u32,
}

impl DayStatus {
    /// The loss so far as a positive amount; zero while the day is flat or ahead.
    pub fn loss(&self) -> f64 {
        (-self.net).max(0.0)
    }
}

/// A limit that is switched on: a rule set to zero or less is off as well.
fn limit(value: Option<f64>) -> Option<f64> {
    value.filter(|v| *v > 0.0)
}

/// The count of a rule that is switched on.
fn count_limit(value: Option<u32>) -> Option<u32> {
    value.filter(|v| *v > 0)
}

fn closed_at(sample: &Sample<'_>) -> i64 {
    sample.trade.close_time.unwrap_or(sample.trade.open_time)
}

/// Trailing losing trades of `nets`, which are in closing order.
fn trailing_losses(nets: impl DoubleEndedIterator<Item = f64>) -> u32 {
    let mut streak = 0_u32;
    for net in nets.rev() {
        if net < -FLAT {
            streak = streak.saturating_add(1);
        } else {
            break;
        }
    }
    streak
}

/// The status of `day` from the closed trades `samples` (any order).
pub fn day_status(samples: &[Sample<'_>], clock: Clock, day: NaiveDate) -> DayStatus {
    let mut closed: Vec<(i64, f64)> =
        samples.iter().filter(|s| clock.day(closed_at(s)) == day).map(|s| (closed_at(s), s.net())).collect();
    closed.sort_by_key(|c| c.0);
    let opened = samples.iter().filter(|s| clock.day(s.trade.open_time) == day).count();
    DayStatus {
        net: closed.iter().map(|c| c.1).sum(),
        trades: u32::try_from(opened).unwrap_or(u32::MAX),
        losing_streak: trailing_losses(closed.iter().map(|c| c.1)),
    }
}

/// The violations of `rules` by `samples` (closed trades, any order), oldest first.
///
/// `balance_at` gives the balance before a trade opened at a server time, for the
/// risk-per-trade rule. The losing streak is counted within one day: a trade violates when
/// at least `max_consecutive_losses` trades that closed earlier the same day, all losing,
/// came right before it.
pub fn evaluate(
    rules: &Rules,
    samples: &[Sample<'_>],
    balance_at: impl Fn(i64) -> f64,
    clock: Clock,
) -> Vec<Violation> {
    let mut found: Vec<Violation> = Vec::new();

    if let Some(max) = limit(rules.max_daily_loss) {
        let mut nets: BTreeMap<NaiveDate, f64> = BTreeMap::new();
        for sample in samples {
            *nets.entry(clock.day(closed_at(sample))).or_default() += sample.net();
        }
        for (day, net) in nets {
            if -net > max {
                found.push(Violation { day, kind: ViolationKind::DailyLoss(-net, max), trade: None });
            }
        }
    }

    if let Some(max) = count_limit(rules.max_daily_trades) {
        let mut opened: BTreeMap<NaiveDate, u32> = BTreeMap::new();
        for sample in samples {
            let n = opened.entry(clock.day(sample.trade.open_time)).or_default();
            *n = n.saturating_add(1);
        }
        for (day, n) in opened {
            if n > max {
                found.push(Violation { day, kind: ViolationKind::DailyTrades(n, max), trade: None });
            }
        }
    }

    if let Some(max) = limit(rules.max_risk_pct) {
        for sample in samples {
            let Some(risk) = sample.trade.risk.filter(|r| *r > 0.0) else { continue };
            let balance = balance_at(sample.trade.open_time);
            if balance <= 0.0 {
                continue;
            }
            let pct = risk / balance * 100.0;
            if pct > max {
                found.push(Violation {
                    day: clock.day(sample.trade.open_time),
                    kind: ViolationKind::RiskPerTrade(pct, max),
                    trade: Some(sample.trade.key.clone()),
                });
            }
        }
    }

    if let Some(max) = count_limit(rules.max_consecutive_losses) {
        let mut closed_by_day: BTreeMap<NaiveDate, Vec<(i64, f64)>> = BTreeMap::new();
        for sample in samples {
            closed_by_day.entry(clock.day(closed_at(sample))).or_default().push((closed_at(sample), sample.net()));
        }
        for list in closed_by_day.values_mut() {
            list.sort_by_key(|c| c.0);
        }
        for sample in samples {
            let day = clock.day(sample.trade.open_time);
            let Some(list) = closed_by_day.get(&day) else { continue };
            let before = list.partition_point(|c| c.0 <= sample.trade.open_time);
            let streak = trailing_losses(list[..before].iter().map(|c| c.1));
            if streak >= max {
                found.push(Violation {
                    day,
                    kind: ViolationKind::AfterLosingStreak(streak, max),
                    trade: Some(sample.trade.key.clone()),
                });
            }
        }
    }

    // Stable: within a day, the order the rules were checked in stays.
    found.sort_by_key(|v| v.day);
    found
}

/// The balance before a trade opened at `at_ms` (server time), read off the balance curve;
/// the curve's first balance when the trade is older than it.
pub fn balance_before(curve: &[EquityPoint], at_ms: i64) -> f64 {
    let after = curve.partition_point(|p| p.time_ms <= at_ms);
    after.checked_sub(1).and_then(|i| curve.get(i)).or(curve.first()).map_or(0.0, |p| p.balance)
}

/// The share of the days traded on which no rule was broken; `None` when no day was traded.
pub fn compliance_rate(violations: &[Violation], trading_days: usize) -> Option<f64> {
    if trading_days == 0 {
        return None;
    }
    let mut days: Vec<NaiveDate> = violations.iter().map(|v| v.day).collect();
    days.sort_unstable();
    days.dedup();
    let broken = days.len().min(trading_days);
    // Day counts stay far below `u32::MAX`.
    let count = |n: usize| f64::from(u32::try_from(n).unwrap_or(u32::MAX));
    Some(1.0 - count(broken) / count(trading_days))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::{Side, Trade},
        time::parse_mt5_time,
    };

    fn at(text: &str) -> i64 {
        parse_mt5_time(text).expect("fixture times parse")
    }

    fn trade(position: i64, opened: &str, closed: &str, net: f64, risk: Option<f64>) -> Trade {
        Trade {
            key: TradeKey { account_id: 1, position_id: position, segment: 0 },
            symbol: "EURUSD".into(),
            side: Side::Long,
            open_time: at(opened),
            close_time: Some(at(closed)),
            volume: 0.1,
            open_price: 1.1,
            close_price: Some(1.1),
            sl: 0.0,
            tp: 0.0,
            commission: 0.0,
            swap: 0.0,
            fee: 0.0,
            profit: net,
            risk,
            magic: 0,
            comment: String::new(),
            manual: false,
        }
    }

    fn samples(trades: &[Trade]) -> Vec<Sample<'_>> {
        trades.iter().map(|trade| Sample { trade, r: None }).collect()
    }

    fn clock() -> Clock {
        Clock { server_offset_min: Some(0), local_offset_min: 0 }
    }

    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("fixture days parse")
    }

    #[test]
    fn nothing_is_flagged_when_every_rule_is_off() {
        let trades = [trade(1, "2026.11.20 09:00", "2026.11.20 10:00", -500.0, Some(900.0))];
        assert_eq!(evaluate(&Rules::default(), &samples(&trades), |_| 1000.0, clock()), Vec::<Violation>::new());
    }

    #[test]
    fn a_day_past_the_loss_limit_is_flagged_once() {
        let trades = [
            trade(1, "2026.11.20 09:00", "2026.11.20 10:00", -200.0, None),
            trade(2, "2026.11.20 11:00", "2026.11.20 12:00", -150.0, None),
            trade(3, "2026.11.21 09:00", "2026.11.21 10:00", -100.0, None),
        ];
        let rules = Rules { max_daily_loss: Some(300.0), ..Rules::default() };
        let found = evaluate(&rules, &samples(&trades), |_| 1000.0, clock());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].day, day("2026-11-20"));
        assert_eq!(found[0].kind, ViolationKind::DailyLoss(350.0, 300.0));
        assert_eq!(found[0].trade, None);
    }

    #[test]
    fn a_loss_exactly_at_the_limit_is_not_a_violation() {
        let trades = [trade(1, "2026.11.20 09:00", "2026.11.20 10:00", -300.0, None)];
        let rules = Rules { max_daily_loss: Some(300.0), ..Rules::default() };
        assert_eq!(evaluate(&rules, &samples(&trades), |_| 1000.0, clock()), Vec::<Violation>::new());
    }

    #[test]
    fn too_many_trades_in_a_day_are_counted_by_opening_day() {
        let trades = [
            trade(1, "2026.11.20 09:00", "2026.11.20 10:00", 5.0, None),
            trade(2, "2026.11.20 11:00", "2026.11.20 12:00", 5.0, None),
            trade(3, "2026.11.20 23:00", "2026.11.21 01:00", 5.0, None),
            trade(4, "2026.11.21 02:00", "2026.11.21 03:00", 5.0, None),
        ];
        let rules = Rules { max_daily_trades: Some(2), ..Rules::default() };
        let found = evaluate(&rules, &samples(&trades), |_| 1000.0, clock());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, ViolationKind::DailyTrades(3, 2));
        assert_eq!(found[0].day, day("2026-11-20"));
    }

    #[test]
    fn risk_is_a_share_of_the_balance_before_the_trade() {
        let trades = [
            trade(1, "2026.11.20 09:00", "2026.11.20 10:00", 5.0, Some(20.0)),
            trade(2, "2026.11.20 11:00", "2026.11.20 12:00", 5.0, Some(10.0)),
            trade(3, "2026.11.20 13:00", "2026.11.20 14:00", 5.0, None),
        ];
        let rules = Rules { max_risk_pct: Some(1.5), ..Rules::default() };
        let found = evaluate(&rules, &samples(&trades), |_| 1000.0, clock());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, ViolationKind::RiskPerTrade(2.0, 1.5));
        assert_eq!(found[0].trade.as_ref().map(|k| k.position_id), Some(1));
    }

    #[test]
    fn a_trade_after_a_losing_streak_is_flagged_and_a_win_resets_it() {
        let trades = [
            trade(1, "2026.11.20 08:00", "2026.11.20 08:30", -10.0, None),
            trade(2, "2026.11.20 09:00", "2026.11.20 09:30", -10.0, None),
            trade(3, "2026.11.20 10:00", "2026.11.20 10:30", 20.0, None),
            trade(4, "2026.11.20 11:00", "2026.11.20 11:30", -10.0, None),
            trade(5, "2026.11.20 12:00", "2026.11.20 12:30", -10.0, None),
            trade(6, "2026.11.20 13:00", "2026.11.20 13:30", 1.0, None),
        ];
        let rules = Rules { max_consecutive_losses: Some(2), ..Rules::default() };
        let found = evaluate(&rules, &samples(&trades), |_| 1000.0, clock());
        let flagged: Vec<i64> = found.iter().filter_map(|v| v.trade.as_ref().map(|k| k.position_id)).collect();
        // Trade 3 follows two losses; trade 6 follows two losses after the win; nobody else does.
        assert_eq!(flagged, [3, 6]);
        assert_eq!(found[0].kind, ViolationKind::AfterLosingStreak(2, 2));
    }

    #[test]
    fn a_streak_does_not_carry_over_midnight() {
        let trades = [
            trade(1, "2026.11.19 21:00", "2026.11.19 22:00", -10.0, None),
            trade(2, "2026.11.19 22:30", "2026.11.19 23:00", -10.0, None),
            trade(3, "2026.11.20 09:00", "2026.11.20 10:00", 5.0, None),
        ];
        let rules = Rules { max_consecutive_losses: Some(2), ..Rules::default() };
        assert_eq!(evaluate(&rules, &samples(&trades), |_| 1000.0, clock()), Vec::<Violation>::new());
    }

    #[test]
    fn violations_come_oldest_first() {
        let trades = [
            trade(1, "2026.11.21 09:00", "2026.11.21 10:00", -400.0, None),
            trade(2, "2026.11.20 09:00", "2026.11.20 10:00", -400.0, None),
        ];
        let rules = Rules { max_daily_loss: Some(300.0), ..Rules::default() };
        let days: Vec<NaiveDate> =
            evaluate(&rules, &samples(&trades), |_| 1000.0, clock()).into_iter().map(|v| v.day).collect();
        assert_eq!(days, [day("2026-11-20"), day("2026-11-21")]);
    }

    #[test]
    fn a_day_reads_its_loss_trades_and_streak() {
        let trades = [
            trade(1, "2026.11.20 08:00", "2026.11.20 08:30", 30.0, None),
            trade(2, "2026.11.20 09:00", "2026.11.20 09:30", -10.0, None),
            trade(3, "2026.11.20 10:00", "2026.11.20 10:30", -25.0, None),
            trade(4, "2026.11.19 10:00", "2026.11.19 10:30", -99.0, None),
        ];
        let status = day_status(&samples(&trades), clock(), day("2026-11-20"));
        assert_eq!(status.trades, 3);
        assert_eq!(status.losing_streak, 2);
        assert!((status.net + 5.0).abs() < 1e-9);
        assert!((status.loss() - 5.0).abs() < 1e-9);
        assert_eq!(day_status(&samples(&trades), clock(), day("2026-11-18")), DayStatus::default());
    }

    #[test]
    fn the_balance_before_a_trade_comes_off_the_curve() {
        let point = |at_ms: i64, balance: f64| EquityPoint { time_ms: at_ms, balance, peak: balance, drawdown: 0.0 };
        let curve = [point(at("2026.11.20 00:00"), 1000.0), point(at("2026.11.20 10:00"), 900.0)];
        assert!((balance_before(&curve, at("2026.11.19 00:00")) - 1000.0).abs() < 1e-9);
        assert!((balance_before(&curve, at("2026.11.20 09:00")) - 1000.0).abs() < 1e-9);
        assert!((balance_before(&curve, at("2026.11.20 11:00")) - 900.0).abs() < 1e-9);
        assert!(balance_before(&[], 0).abs() < 1e-9);
    }

    #[test]
    fn compliance_is_the_share_of_clean_days() {
        let broken = |d: &str| Violation { day: day(d), kind: ViolationKind::DailyTrades(5, 3), trade: None };
        assert_eq!(compliance_rate(&[], 0), None);
        let rate = compliance_rate(&[broken("2026-11-20"), broken("2026-11-20"), broken("2026-11-21")], 10);
        assert!((rate.expect("a rate") - 0.8).abs() < 1e-9);
    }
}
