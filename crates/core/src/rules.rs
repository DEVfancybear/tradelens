//! Discipline: an account's rules checked against its trades, day by day.

use chrono::NaiveDate;

use crate::{
    metrics::Sample,
    model::{Rules, TradeKey},
    time::Clock,
};

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

/// The violations of `rules` by `samples`, oldest first.
///
/// `balance_at` gives the balance before a trade, for the risk-per-trade rule.
pub fn evaluate(
    _rules: &Rules,
    _samples: &[Sample],
    _balance_at: impl Fn(i64) -> f64,
    _clock: Clock,
) -> Vec<Violation> {
    // TODO(v0.1): group by local day; check loss, count, risk %, streak.
    Vec::new()
}
