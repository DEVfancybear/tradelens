//! Statistics over closed trades: the headline numbers, the balance curve and its
//! drawdown, and breakdowns by any key.

use std::collections::BTreeMap;

use chrono::{Datelike, Days, NaiveDate};

use crate::{
    model::{Account, Trade, TradeKey},
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

    /// What decides whether the trade won: its money, or its R when it has no money.
    pub fn score(&self) -> f64 {
        self.trade.score()
    }
}

/// The unit the figures of a set of trades are shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Unit {
    /// Money (the account's currency).
    #[default]
    Money,
    /// R, for trades that have no money.
    R,
}

/// Which of a set of trades have money and which have only their R. A trade has money unless it was
/// imported with only its result in R ([`Trade::money_known`]); every money figure of a set (net,
/// profit factor, drawdown, calendar cells, breakdowns) counts the money trades alone, so a screen
/// shows R when there is no money, and says how many trades the money covers when there is a mix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Basis {
    pub money_trades: usize,
    pub r_only_trades: usize,
}

impl Basis {
    /// The basis of `samples`.
    pub fn of(samples: &[Sample<'_>]) -> Self {
        let money_trades = samples.iter().filter(|x| x.trade.money_known()).count();
        Self { money_trades, r_only_trades: samples.len() - money_trades }
    }

    /// R when the set has trades and none of them has money, else money.
    pub fn unit(self) -> Unit {
        if self.money_trades == 0 && self.r_only_trades > 0 { Unit::R } else { Unit::Money }
    }

    /// Some trades have money and some only R.
    pub fn is_mixed(self) -> bool {
        self.money_trades > 0 && self.r_only_trades > 0
    }

    /// What to say beside money figures that leave trades out; `None` when they cover every trade.
    pub fn money_note(self) -> Option<String> {
        self.is_mixed()
            .then(|| crate::tr!("core.metrics.money_note", money = self.money_trades, r_only = self.r_only_trades))
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
    /// R won over R lost: the profit factor of a journal kept in R.
    pub profit_factor_r: Option<f64>,
    /// The deepest fall of the running total of R from its peak, a positive number.
    pub max_drawdown_r: f64,
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

/// The results in R of trades in the order they closed: what the curve of R and its profit factor are made of.
#[derive(Debug, Default)]
struct RRun {
    total: f64,
    peak: f64,
    won: f64,
    lost: f64,
    /// The deepest fall of `total` from `peak`, a positive number.
    max_drawdown: f64,
}

impl RRun {
    fn add(&mut self, r: f64) {
        self.total += r;
        self.peak = self.peak.max(self.total);
        self.max_drawdown = self.max_drawdown.max(self.peak - self.total);
        if r > FLAT {
            self.won += r;
        } else if r < -FLAT {
            self.lost -= r;
        }
    }

    /// R won over R lost; `None` while nothing was lost.
    fn profit_factor(&self) -> Option<f64> {
        (self.lost > FLAT).then(|| self.won / self.lost)
    }
}

/// The statistics of `samples` (closed trades, any order).
pub fn stats(samples: &[Sample<'_>], start_balance: f64, cash: &[(i64, f64)], clock: Clock) -> Stats {
    let mut s = Stats { trades: samples.len(), start_balance, ..Stats::default() };
    let mut ordered: Vec<&Sample<'_>> = samples.iter().collect();
    ordered.sort_by_key(|x| x.trade.close_time.unwrap_or(x.trade.open_time));

    let (mut win_streak, mut loss_streak) = (0, 0);
    // Trades without money (imported in R) are won or lost by their R, but add nothing to money.
    let (mut money_trades, mut money_wins, mut money_losses) = (0_usize, 0_usize, 0_usize);
    let (mut hold_total, mut hold_trades) = (0_i64, 0_i64);
    // The running total of R, for the drawdown in R and the profit factor of R.
    let mut run = RRun::default();
    for x in &ordered {
        let t = x.trade;
        let net = x.net();
        let score = x.score();
        let money = t.money_known();
        money_trades += usize::from(money);
        s.net += net;
        s.commission += t.commission + t.fee;
        s.swap += t.swap;
        if t.volume_known() {
            s.volume += t.volume;
        }
        if let Some(held) = t.hold_secs() {
            hold_total += held;
            hold_trades += 1;
        }
        if score > FLAT {
            s.wins += 1;
            if money {
                money_wins += 1;
                s.gross_profit += net;
                s.largest_win = s.largest_win.max(net);
            }
            win_streak += 1;
            loss_streak = 0;
        } else if score < -FLAT {
            s.losses += 1;
            if money {
                money_losses += 1;
                s.gross_loss += net;
                s.largest_loss = s.largest_loss.min(net);
            }
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
            run.add(r);
        }
        match t.side {
            crate::model::Side::Long => {
                s.long_trades += 1;
                s.long_net += net;
                s.long_wins += usize::from(score > FLAT);
            }
            crate::model::Side::Short => {
                s.short_trades += 1;
                s.short_net += net;
                s.short_wins += usize::from(score > FLAT);
            }
        }
    }
    if s.trades > 0 {
        s.win_rate = real(s.wins) / real(s.trades);
        s.expectancy = s.net / real(money_trades.max(1));
        // Trades whose time of day is unknown (a date only) have no hold time to average.
        s.avg_hold_secs = hold_total / hold_trades.max(1);
    }
    s.max_drawdown_r = run.max_drawdown;
    s.profit_factor_r = run.profit_factor();
    if money_wins > 0 {
        s.avg_win = s.gross_profit / real(money_wins);
    }
    if money_losses > 0 {
        s.avg_loss = s.gross_loss / real(money_losses);
    }
    if s.gross_loss < -FLAT {
        s.profit_factor = Some(s.gross_profit / -s.gross_loss);
    }
    if s.avg_loss < -FLAT && money_wins > 0 {
        s.payoff = Some(s.avg_win / -s.avg_loss);
    }
    if s.r_trades > 0 {
        s.avg_r = Some(s.total_r / real(s.r_trades));
    }

    add_balance_figures(&mut s, samples, start_balance, cash);

    let days = daily(samples, clock);
    s.trading_days = days.len();
    s.green_days = days.values().filter(|d| d.net > FLAT).count();
    s.best_day =
        days.iter().filter(|(_, d)| d.net > FLAT).max_by(|a, b| a.1.net.total_cmp(&b.1.net)).map(|(k, d)| (*k, d.net));
    s.worst_day =
        days.iter().filter(|(_, d)| d.net < -FLAT).min_by(|a, b| a.1.net.total_cmp(&b.1.net)).map(|(k, d)| (*k, d.net));
    s
}

/// The figures that come from the balance curve and the cash movements: drawdown, end balance,
/// deposits and withdrawals, recovery factor.
fn add_balance_figures(s: &mut Stats, samples: &[Sample<'_>], start_balance: f64, cash: &[(i64, f64)]) {
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
///
/// An account that held nothing before its first cash movements (a new account funded inside
/// the period) starts at the balance they bring: the empty balance before them is no state of
/// the trading, and drawn as a point it would stretch a chart of a large balance down to zero.
pub fn equity_curve(samples: &[Sample<'_>], start_balance: f64, cash: &[(i64, f64)]) -> Vec<EquityPoint> {
    enum Event {
        Cash(f64),
        Trade(f64),
    }
    let mut events: Vec<(i64, Event)> = cash.iter().map(|&(t, a)| (t, Event::Cash(a))).collect();
    events.extend(samples.iter().map(|x| (x.trade.close_time.unwrap_or(x.trade.open_time), Event::Trade(x.net()))));
    // Stable: at the same instant, cash comes before the trades.
    events.sort_by_key(|e| e.0);

    let first = events.first().map_or(0, |e| e.0);
    let mut balance = start_balance;
    let mut peak = start_balance;
    let mut events = events.into_iter().peekable();
    if start_balance.abs() < FLAT {
        while let Some((_, Event::Cash(amount))) = events.next_if(|e| matches!(e.1, Event::Cash(_))) {
            balance += amount;
            peak = balance;
        }
    }
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
    /// How many of the day's trades have money (the others have only their R, and add 0 to `net`).
    pub money_trades: usize,
}

impl DayStat {
    /// The day's result in the unit it has: money when any of its trades has money, else R.
    pub fn result(&self) -> f64 {
        if self.money_trades > 0 { self.net } else { self.r }
    }

    /// The unit [`DayStat::result`] is in.
    pub fn unit(&self) -> Unit {
        if self.money_trades == 0 && self.trades > 0 { Unit::R } else { Unit::Money }
    }
}

/// The unit a month, a week or a whole calendar of days is shown in: R when no day has money.
pub fn unit_of_days<'a>(days: impl IntoIterator<Item = &'a DayStat>) -> Unit {
    let (mut money, mut any) = (false, false);
    for day in days {
        money |= day.money_trades > 0;
        any |= day.trades > 0;
    }
    if any && !money { Unit::R } else { Unit::Money }
}

/// Net results by the local day the trades closed.
pub fn daily(samples: &[Sample<'_>], clock: Clock) -> BTreeMap<NaiveDate, DayStat> {
    let mut days: BTreeMap<NaiveDate, DayStat> = BTreeMap::new();
    for x in samples {
        let day = clock.day(x.trade.close_time.unwrap_or(x.trade.open_time));
        let d = days.entry(day).or_default();
        d.net += x.net();
        d.money_trades += usize::from(x.trade.money_known());
        d.trades += 1;
        d.wins += usize::from(x.score() > FLAT);
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
    /// Net money of the week's days that belong to the month (trades without money add nothing).
    pub net: f64,
    /// R of the week's days that belong to the month.
    pub r: f64,
    /// Trades of the week's days that belong to the month.
    pub trades: usize,
    /// R when no day of the week has money, else money.
    pub unit: Unit,
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
            let inside: Vec<DayStat> = week.iter().filter(|d| d.in_month).filter_map(|d| d.stat).collect();
            CalendarWeek {
                days: *week,
                net: inside.iter().map(|s| s.net).sum(),
                r: inside.iter().map(|s| s.r).sum(),
                trades: inside.iter().map(|s| s.trades).sum(),
                unit: unit_of_days(&inside),
            }
        })
        .collect()
}

/// What a month of daily results adds up to, in one [`Unit`]: money when any day has money (days
/// with only R then count for their trades alone), else R.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MonthSummary {
    pub unit: Unit,
    /// The month's result in `unit`.
    pub result: f64,
    pub trades: usize,
    pub green_days: usize,
    pub red_days: usize,
    /// The best and the worst day, with their results in `unit`.
    pub best_day: Option<(NaiveDate, f64)>,
    pub worst_day: Option<(NaiveDate, f64)>,
}

/// The totals of `month` of `year` over `days`.
pub fn month_summary(year: i32, month: u32, days: &BTreeMap<NaiveDate, DayStat>) -> MonthSummary {
    let month_days: Vec<(NaiveDate, &DayStat)> =
        days.iter().filter(|(d, _)| d.year() == year && d.month() == month).map(|(d, s)| (*d, s)).collect();
    let unit = unit_of_days(month_days.iter().map(|(_, s)| *s));
    let mut summary = MonthSummary { unit, ..MonthSummary::default() };
    for (date, stat) in month_days {
        summary.trades += stat.trades;
        // In money, a day that has only R has no result to add up.
        let result = match unit {
            Unit::Money if stat.money_trades == 0 => continue,
            Unit::Money => stat.net,
            Unit::R => stat.r,
        };
        summary.result += result;
        if result > FLAT {
            summary.green_days += 1;
            if summary.best_day.is_none_or(|(_, best)| result > best) {
                summary.best_day = Some((date, result));
            }
        } else if result < -FLAT {
            summary.red_days += 1;
            if summary.worst_day.is_none_or(|(_, worst)| result < worst) {
                summary.worst_day = Some((date, result));
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
    /// R won and R lost (a negative sum) by the trades of the group: its profit factor in R.
    pub r_profit: f64,
    pub r_loss: f64,
    /// The trades of the group, in the order met: what a pressed row of the analytics opens.
    pub keys: Vec<TradeKey>,
}

impl Group {
    fn new(label: String) -> Self {
        Self {
            keys: Vec::new(),
            label,
            trades: 0,
            wins: 0,
            net: 0.0,
            gross_profit: 0.0,
            gross_loss: 0.0,
            total_r: 0.0,
            r_trades: 0,
            r_profit: 0.0,
            r_loss: 0.0,
        }
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

    /// R won over R lost, for a group of trades that have no money.
    pub fn profit_factor_r(&self) -> Option<f64> {
        (self.r_loss < -FLAT).then(|| self.r_profit / -self.r_loss)
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
            g.keys.push(x.trade.key.clone());
            g.trades += 1;
            g.net += net;
            if x.score() > FLAT {
                g.wins += 1;
            }
            if net > FLAT {
                g.gross_profit += net;
            } else if net < -FLAT {
                g.gross_loss += net;
            }
            if let Some(r) = x.r {
                g.total_r += r;
                g.r_trades += 1;
                if r > FLAT {
                    g.r_profit += r;
                } else if r < -FLAT {
                    g.r_loss += r;
                }
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
    /// The trades with the mistake: what a pressed row opens.
    pub keys: Vec<TradeKey>,
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
            keys: g.keys,
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
