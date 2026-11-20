//! Statistics over closed trades: the headline numbers, the balance curve and its
//! drawdown, and breakdowns by any key.

use std::collections::BTreeMap;

use chrono::NaiveDate;

use crate::{model::Trade, time::Clock};

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
    }
    days
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
