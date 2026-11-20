//! Missed trades: setups the trader saw and did not take, with what would have happened.
//! They show fear and hesitation that no executed trade can.

use serde::{Deserialize, Serialize};

use crate::{catalog::CustomValues, model::Side};

/// A setup seen and not traded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissedTrade {
    #[serde(skip)]
    pub id: i64,
    #[serde(default)]
    pub account_id: Option<i64>,
    pub symbol: String,
    pub side: Side,
    /// Unix milliseconds, local wall clock read as UTC (as the journal's days).
    pub seen_at: i64,
    #[serde(default)]
    pub playbook_id: Option<i64>,
    #[serde(default)]
    pub setup_grade: Option<String>,
    /// A key of the trader's missed-reason catalog.
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub entry: Option<f64>,
    #[serde(default)]
    pub stop_loss: Option<f64>,
    #[serde(default)]
    pub take_profit: Option<f64>,
    /// Where price went after the setup (the target hit, or the trader's estimate).
    #[serde(default)]
    pub outcome_price: Option<f64>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub screenshots: Vec<String>,
    #[serde(default)]
    pub custom: CustomValues,
}

impl MissedTrade {
    /// What the trade would have made, in R of its planned stop.
    pub fn would_be_r(&self) -> Option<f64> {
        let (entry, sl, out) = (self.entry?, self.stop_loss?, self.outcome_price?);
        let risk = (entry - sl).abs();
        if risk <= f64::EPSILON {
            return None;
        }
        let moved = match self.side {
            Side::Long => out - entry,
            Side::Short => entry - out,
        };
        Some(moved / risk)
    }
}

/// Missed trades, summed by reason.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MissedSummary {
    pub count: usize,
    /// R left on the table by the ones that would have won.
    pub r_missed: f64,
    /// R saved by the ones that would have lost.
    pub r_avoided: f64,
    /// `(reason key, count, net would-be R)`, most frequent first; no reason is `""`.
    pub by_reason: Vec<(String, usize, f64)>,
}

pub fn summarise(missed: &[MissedTrade]) -> MissedSummary {
    let mut summary = MissedSummary { count: missed.len(), ..MissedSummary::default() };
    let mut by_reason: std::collections::BTreeMap<String, (usize, f64)> = std::collections::BTreeMap::new();
    for m in missed {
        let r = m.would_be_r().unwrap_or(0.0);
        if r > 0.0 {
            summary.r_missed += r;
        } else {
            summary.r_avoided -= r;
        }
        let entry = by_reason.entry(m.reason.clone().unwrap_or_default()).or_default();
        entry.0 += 1;
        entry.1 += r;
    }
    summary.by_reason = by_reason.into_iter().map(|(k, (n, r))| (k, n, r)).collect();
    summary.by_reason.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    fn missed(side: Side, entry: f64, sl: f64, out: f64, reason: &str) -> MissedTrade {
        MissedTrade {
            id: 0,
            account_id: None,
            symbol: "XAUUSD".into(),
            side,
            seen_at: 0,
            playbook_id: None,
            setup_grade: None,
            reason: Some(reason.into()),
            entry: Some(entry),
            stop_loss: Some(sl),
            take_profit: None,
            outcome_price: Some(out),
            notes: String::new(),
            screenshots: Vec::new(),
            custom: CustomValues::new(),
        }
    }

    #[test]
    fn missed_trades_add_up_by_reason() {
        let list = [
            missed(Side::Long, 2650.0, 2645.0, 2665.0, "fear"), // +3R
            missed(Side::Short, 1.27, 1.272, 1.266, "fear"),    // +2R
            missed(Side::Long, 100.0, 99.0, 99.0, "unsure"),    // −1R
        ];
        let s = summarise(&list);
        assert_eq!(s.count, 3);
        assert!((s.r_missed - 5.0).abs() < 1e-6, "{}", s.r_missed);
        assert!((s.r_avoided - 1.0).abs() < 1e-9);
        assert_eq!(s.by_reason[0].0, "fear");
        assert_eq!(s.by_reason[0].1, 2);
    }
}
