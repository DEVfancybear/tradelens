//! Statistics over closed trades: the headline numbers, the balance curve and its
//! drawdown, and breakdowns by any key.

use std::collections::BTreeMap;

use chrono::{Datelike, Days, NaiveDate};

use crate::{
    model::{Account, Trade},
    time::Clock,
};

/// A closed trade with what the statistics need from its journal.
#[derive(Debug, Clone, Copy)]
pub struct Sample<'a> {
    pub trade: &'a Trade,
    pub r: Option<f64>,
}

impl Sample<'_> {
    pub fn net(&self) -> f64 {
        self.trade.net()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Stats {
    pub trades: usize,
    pub wins: usize,
    pub losses: usize,
    pub breakeven: usize,
    pub win_rate: f64,
    pub net: f64,
    pub gross_profit: f64,
    pub gross_loss: f64,
    pub profit_factor: Option<f64>,
    pub avg_win: f64,
    pub avg_loss: f64,
    pub payoff: Option<f64>,
    pub expectancy: f64,
    pub avg_r: Option<f64>,
    pub total_r: f64,
    pub r_trades: usize,
    pub largest_win: f64,
    pub largest_loss: f64,
    pub max_win_streak: usize,
    pub max_loss_streak: usize,
    pub max_drawdown: f64,
    pub max_drawdown_pct: Option<f64>,
    pub recovery_factor: Option<f64>,
    pub avg_hold_secs: i64,
    pub commission: f64,
    pub swap: f64,
    pub volume: f64,
    pub long_trades: usize,
    pub long_wins: usize,
    pub long_net: f64,
    pub short_trades: usize,
    pub short_wins: usize,
    pub short_net: f64,
    pub trading_days: usize,
    pub green_days: usize,
    pub best_day: Option<(NaiveDate, f64)>,
    pub worst_day: Option<(NaiveDate, f64)>,
    pub start_balance: f64,
    pub end_balance: f64,
    pub deposits: f64,
    pub withdrawals: f64,
}

const FLAT: f64 = 1e-6;

/// A count as a float; counts of trades stay far below `u32::MAX`.
fn real(n: usize) -> f64 {
    u32::try_from(n).map_or(f64::from(u32::MAX), f64::from)
}

/// The statistics of `samples` (closed trades, any order).
pub fn stats(samples: &[Sample<'_>], start_balance: f64, cash: &[(i64, f64)], clock: Clock) -> Stats {
    let mut s = Stats { trades: samples.len(), start_balance, ..Stats::default() };
    let mut ordered: Vec<&Sample<'_>> = samples.iter().collect();
    ordered.sort_by_key(|x| x.trade.close_time.unwrap_or(x.trade.open_time));

    let (mut win_streak, mut loss_streak) = (0, 0);
    let mut hold_total = 0_i64;
    for x in &ordered {
        let t = x.trade;
        let net = x.net();
        s.net += net;
        s.commission += t.commission + t.fee;
        s.swap += t.swap;
        s.volume += t.volume;
        hold_total += t.duration_secs().unwrap_or(0);
        if net > FLAT {
            s.wins += 1;
            s.gross_profit += net;
            s.largest_win = s.largest_win.max(net);
            win_streak += 1;
            loss_streak = 0;
        } else if net < -FLAT {
            s.losses += 1;
            s.gross_loss += net;
            s.largest_loss = s.largest_loss.min(net);
            loss_streak += 1;
            win_streak = 0;
        } else {
            s.breakeven += 1;
        }
        s.max_win_streak = s.max_win_streak.max(win_streak);
        s.max_loss_streak = s.max_loss_streak.max(loss_streak);
        if let Some(r) = x.r {
            s.total_r += r;
            s.r_trades += 1;
        }
        match t.side {
            crate::model::Side::Long => {
                s.long_trades += 1;
                s.long_net += net;
                s.long_wins += usize::from(net > FLAT);
            }
            crate::model::Side::Short => {
                s.short_trades += 1;
                s.short_net += net;
                s.short_wins += usize::from(net > FLAT);
            }
        }
    }
    if s.trades > 0 {
        s.win_rate = real(s.wins) / real(s.trades);
        s.expectancy = s.net / real(s.trades);
        s.avg_hold_secs = hold_total / i64::try_from(s.trades).unwrap_or(i64::MAX);
    }
    if s.wins > 0 {
        s.avg_win = s.gross_profit / real(s.wins);
    }
    if s.losses > 0 {
        s.avg_loss = s.gross_loss / real(s.losses);
    }
    if s.gross_loss < -FLAT {
        s.profit_factor = Some(s.gross_profit / -s.gross_loss);
    }
    if s.avg_loss < -FLAT && s.wins > 0 {
        s.payoff = Some(s.avg_win / -s.avg_loss);
    }
    if s.r_trades > 0 {
        s.avg_r = Some(s.total_r / real(s.r_trades));
    }

    let curve = equity_curve(samples, start_balance, cash);
    for p in &curve {
        if p.drawdown > s.max_drawdown {
            s.max_drawdown = p.drawdown;
            s.max_drawdown_pct = (p.peak > FLAT).then(|| p.drawdown / p.peak);
        }
    }
    s.end_balance = curve.last().map_or(start_balance, |p| p.balance);
    for &(_, amount) in cash {
        if amount >= 0.0 {
            s.deposits += amount;
        } else {
            s.withdrawals += amount;
        }
    }
    if s.max_drawdown > FLAT {
        s.recovery_factor = Some(s.net / s.max_drawdown);
    }

    let days = daily(samples, clock);
    s.trading_days = days.len();
    s.green_days = days.values().filter(|d| d.net > FLAT).count();
    s.best_day =
        days.iter().filter(|(_, d)| d.net > FLAT).max_by(|a, b| a.1.net.total_cmp(&b.1.net)).map(|(k, d)| (*k, d.net));
    s.worst_day =
        days.iter().filter(|(_, d)| d.net < -FLAT).min_by(|a, b| a.1.net.total_cmp(&b.1.net)).map(|(k, d)| (*k, d.net));
    s
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EquityPoint {
    pub time_ms: i64,
    pub balance: f64,
    pub peak: f64,
    pub drawdown: f64,
}

/// The balance after each closed trade and cash movement, with its drawdown from the
/// running peak. Deposits raise the peak with the balance, so they are not gains.
pub fn equity_curve(samples: &[Sample<'_>], start_balance: f64, cash: &[(i64, f64)]) -> Vec<EquityPoint> {
    enum Event {
        Cash(f64),
        Trade(f64),
    }
    let mut events: Vec<(i64, Event)> = cash.iter().map(|&(t, a)| (t, Event::Cash(a))).collect();
    events.extend(samples.iter().map(|x| (x.trade.close_time.unwrap_or(x.trade.open_time), Event::Trade(x.net()))));
    events.sort_by_key(|e| e.0);

    let first = events.first().map_or(0, |e| e.0);
    let mut balance = start_balance;
    let mut peak = start_balance;
    let mut out = vec![EquityPoint { time_ms: first, balance, peak, drawdown: 0.0 }];
    for (time_ms, event) in events {
        match event {
            Event::Cash(a) => {
                balance += a;
                peak = (peak + a).max(balance);
            }
            Event::Trade(n) => {
                balance += n;
                peak = peak.max(balance);
            }
        }
        out.push(EquityPoint { time_ms, balance, peak, drawdown: (peak - balance).max(0.0) });
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DayStat {
    pub net: f64,
    pub trades: usize,
    pub wins: usize,
    /// The sum of the R of the day's trades that have one.
    pub r: f64,
    /// How many of the day's trades have an R.
    pub r_trades: usize,
}

/// Net results by the local day the trades closed.
pub fn daily(samples: &[Sample<'_>], clock: Clock) -> BTreeMap<NaiveDate, DayStat> {
    let mut days: BTreeMap<NaiveDate, DayStat> = BTreeMap::new();
    for x in samples {
        let day = clock.day(x.trade.close_time.unwrap_or(x.trade.open_time));
        let d = days.entry(day).or_default();
        d.net += x.net();
        d.trades += 1;
        d.wins += usize::from(x.net() > FLAT);
        if let Some(r) = x.r {
            d.r += r;
            d.r_trades += 1;
        }
    }
    days
}

/// One day of a calendar month grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalendarDay {
    pub date: NaiveDate,
    /// The day belongs to the month shown, not to a neighbour that fills the week.
    pub in_month: bool,
    /// What the trader made that day; `None` when no trade closed.
    pub stat: Option<DayStat>,
}

/// One row of a month grid: seven days from Monday, and what the week made inside the month.
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarWeek {
    pub days: [CalendarDay; 7],
    /// Net of the week's days that belong to the month.
    pub net: f64,
    /// Trades of the week's days that belong to the month.
    pub trades: usize,
}

/// The weeks (Monday first) that cover `month` of `year`, each day with its result from `days`.
/// Empty for a month that does not exist.
pub fn month_weeks(year: i32, month: u32, days: &BTreeMap<NaiveDate, DayStat>) -> Vec<CalendarWeek> {
    let Some(first) = NaiveDate::from_ymd_opt(year, month, 1) else { return Vec::new() };
    let Some(month_end) =
        (if month == 12 { NaiveDate::from_ymd_opt(year + 1, 1, 1) } else { first.with_month(month + 1) })
    else {
        return Vec::new();
    };
    let length = usize::try_from((month_end - first).num_days()).unwrap_or(0);
    let lead = usize::try_from(first.weekday().num_days_from_monday()).unwrap_or(0);
    let Some(start) = first.checked_sub_days(Days::new(u64::try_from(lead).unwrap_or(0))) else {
        return Vec::new();
    };
    let cells: Vec<CalendarDay> = start
        .iter_days()
        .take((lead + length).div_ceil(7) * 7)
        .map(|date| CalendarDay { date, in_month: date >= first && date < month_end, stat: days.get(&date).copied() })
        .collect();
    cells
        .as_chunks::<7>()
        .0
        .iter()
        .map(|week| {
            let inside = week.iter().filter(|d| d.in_month).filter_map(|d| d.stat);
            let (net, trades) = inside.fold((0.0, 0), |(net, trades), s| (net + s.net, trades + s.trades));
            CalendarWeek { days: *week, net, trades }
        })
        .collect()
}

/// What a month of daily results adds up to.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MonthSummary {
    pub net: f64,
    pub trades: usize,
    pub green_days: usize,
    pub red_days: usize,
    pub best_day: Option<(NaiveDate, f64)>,
    pub worst_day: Option<(NaiveDate, f64)>,
}

/// The totals of `month` of `year` over `days`.
pub fn month_summary(year: i32, month: u32, days: &BTreeMap<NaiveDate, DayStat>) -> MonthSummary {
    let mut summary = MonthSummary::default();
    for (&date, stat) in days.iter().filter(|(d, _)| d.year() == year && d.month() == month) {
        summary.net += stat.net;
        summary.trades += stat.trades;
        if stat.net > FLAT {
            summary.green_days += 1;
            if summary.best_day.is_none_or(|(_, best)| stat.net > best) {
                summary.best_day = Some((date, stat.net));
            }
        } else if stat.net < -FLAT {
            summary.red_days += 1;
            if summary.worst_day.is_none_or(|(_, worst)| stat.net < worst) {
                summary.worst_day = Some((date, stat.net));
            }
        }
    }
    summary
}

/// Which limit of the account's rules a [`RuleUsage`] measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleKind {
    DailyLoss,
    DailyTrades,
    RiskPerTrade,
    LosingStreak,
}

/// How much of one rule's limit a day has used.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RuleUsage {
    pub kind: RuleKind,
    /// Money lost, trades taken, percent risked or losses in a row so far.
    pub used: f64,
    pub limit: f64,
}

impl RuleUsage {
    /// The share of the limit used, 0 when the limit is zero.
    pub fn ratio(&self) -> f64 {
        if self.limit > FLAT { self.used / self.limit } else { 0.0 }
    }
}

/// How far one day's trades (closed that day, any order) have gone toward each limit that
/// `rules` sets. `balance` is the account's balance, for the risk limit in percent.
pub fn rule_usage(rules: &crate::model::Rules, today: &[Sample<'_>], balance: f64) -> Vec<RuleUsage> {
    let mut ordered: Vec<&Sample<'_>> = today.iter().collect();
    ordered.sort_by_key(|x| x.trade.close_time.unwrap_or(x.trade.open_time));
    let mut usage = Vec::new();
    if let Some(limit) = rules.max_daily_loss {
        let net: f64 = ordered.iter().map(|x| x.net()).sum();
        usage.push(RuleUsage { kind: RuleKind::DailyLoss, used: (-net).max(0.0), limit });
    }
    if let Some(limit) = rules.max_daily_trades {
        usage.push(RuleUsage { kind: RuleKind::DailyTrades, used: real(ordered.len()), limit: f64::from(limit) });
    }
    if let Some(limit) = rules.max_risk_pct {
        let largest = ordered.iter().filter_map(|x| x.trade.risk).fold(0.0_f64, f64::max);
        let used = if balance > FLAT { largest / balance * 100.0 } else { 0.0 };
        usage.push(RuleUsage { kind: RuleKind::RiskPerTrade, used, limit });
    }
    if let Some(limit) = rules.max_consecutive_losses {
        let streak = ordered.iter().rev().take_while(|x| x.net() < -FLAT).count();
        usage.push(RuleUsage { kind: RuleKind::LosingStreak, used: real(streak), limit: f64::from(limit) });
    }
    usage
}

/// One row of a breakdown.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub label: String,
    pub trades: usize,
    pub wins: usize,
    pub net: f64,
    pub gross_profit: f64,
    pub gross_loss: f64,
    pub total_r: f64,
    pub r_trades: usize,
}

impl Group {
    fn new(label: String) -> Self {
        Self { label, trades: 0, wins: 0, net: 0.0, gross_profit: 0.0, gross_loss: 0.0, total_r: 0.0, r_trades: 0 }
    }

    pub fn win_rate(&self) -> f64 {
        if self.trades == 0 { 0.0 } else { real(self.wins) / real(self.trades) }
    }

    pub fn profit_factor(&self) -> Option<f64> {
        (self.gross_loss < -FLAT).then(|| self.gross_profit / -self.gross_loss)
    }

    pub fn avg_r(&self) -> Option<f64> {
        (self.r_trades > 0).then(|| self.total_r / real(self.r_trades))
    }

    pub fn expectancy(&self) -> f64 {
        if self.trades == 0 { 0.0 } else { self.net / real(self.trades) }
    }
}

/// Groups samples by keys: a sample may fall in several groups (its tags) or none.
/// Groups come back in the keys' order.
pub fn breakdown<K: Ord + Clone>(
    samples: &[Sample<'_>],
    keys: impl Fn(&Sample<'_>) -> Vec<K>,
    label: impl Fn(&K) -> String,
) -> Vec<Group> {
    let mut groups: BTreeMap<K, Group> = BTreeMap::new();
    for x in samples {
        for key in keys(x) {
            let g = groups.entry(key.clone()).or_insert_with(|| Group::new(label(&key)));
            let net = x.net();
            g.trades += 1;
            g.net += net;
            if net > FLAT {
                g.wins += 1;
                g.gross_profit += net;
            } else if net < -FLAT {
                g.gross_loss += net;
            }
            if let Some(r) = x.r {
                g.total_r += r;
                g.r_trades += 1;
            }
        }
    }
    groups.into_values().collect()
}

/// The R results in buckets: `(label, count)`.
pub fn r_distribution(samples: &[Sample<'_>]) -> Vec<(&'static str, usize)> {
    const BUCKETS: [(&str, f64, f64); 7] = [
        ("< -2R", f64::NEG_INFINITY, -2.0),
        ("-2…-1R", -2.0, -1.0),
        ("-1…0R", -1.0, 0.0),
        ("0…1R", 0.0, 1.0),
        ("1…2R", 1.0, 2.0),
        ("2…3R", 2.0, 3.0),
        ("≥ 3R", 3.0, f64::INFINITY),
    ];
    BUCKETS
        .iter()
        .map(|&(label, lo, hi)| (label, samples.iter().filter_map(|x| x.r).filter(|r| *r >= lo && *r < hi).count()))
        .collect()
}

/// What one mistake cost over a period.
#[derive(Debug, Clone, PartialEq)]
pub struct MistakeCost {
    pub key: String,
    pub label: String,
    pub trades: usize,
    /// Net of the trades with the mistake (usually negative).
    pub net: f64,
    pub total_r: f64,
    /// The mistake's share of all money lost, 0 to 1.
    pub share_of_losses: f64,
    /// Expectancy of the trades with the mistake minus that of trades with no mistake:
    /// what each occurrence costs against trading clean.
    pub expectancy_gap: f64,
}

/// The cost of each mistake, the most costly first. `mistakes_of` gives a trade's mistake
/// keys (from its journal), `label` a key's name in the trader's catalog.
pub fn mistake_cost(
    samples: &[Sample<'_>],
    mistakes_of: impl Fn(&Sample<'_>) -> Vec<String>,
    label: impl Fn(&str) -> String,
) -> Vec<MistakeCost> {
    let total_loss: f64 = samples.iter().map(Sample::net).filter(|n| *n < -FLAT).sum::<f64>().abs();
    let clean: Vec<f64> = samples.iter().filter(|x| mistakes_of(x).is_empty()).map(Sample::net).collect();
    let clean_expectancy = if clean.is_empty() { 0.0 } else { clean.iter().sum::<f64>() / real(clean.len()) };
    let groups = breakdown(samples, &mistakes_of, |key: &String| key.clone());
    let mut costs: Vec<MistakeCost> = groups
        .into_iter()
        .map(|g| MistakeCost {
            label: label(&g.label),
            share_of_losses: if total_loss > FLAT { (-g.net).max(0.0) / total_loss } else { 0.0 },
            expectancy_gap: g.expectancy() - clean_expectancy,
            trades: g.trades,
            net: g.net,
            total_r: g.total_r,
            key: g.label,
        })
        .collect();
    costs.sort_by(|a, b| a.net.total_cmp(&b.net));
    costs
}

/// The balance an account had before the first closed trade and cash movement of its
/// curve: the starting balance the trader set, else what makes the curve end at the last
/// balance the terminal reported (`last_balance` − results − cash), else zero.
pub fn start_balance(account: &Account, samples: &[Sample<'_>], cash: &[(i64, f64)]) -> f64 {
    if let Some(initial) = account.initial_balance {
        return initial;
    }
    account.last_balance.map_or(0.0, |last| {
        let results: f64 = samples.iter().map(Sample::net).sum();
        let moved: f64 = cash.iter().map(|&(_, amount)| amount).sum();
        last - results - moved
    })
}
