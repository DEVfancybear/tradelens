//! The journal around a trade: the plan written when it opened, and how the trade
//! that happened compares with it.

use serde::{Deserialize, Serialize};

use crate::model::{Side, Trade};

/// What the trader planned when the position opened (the EA's prompt, or by hand).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct EntryPlan {
    /// Why this trade: the setup seen, the confirmation waited for.
    #[serde(default)]
    pub thesis: String,
    #[serde(default)]
    pub entry: Option<f64>,
    #[serde(default)]
    pub stop_loss: Option<f64>,
    #[serde(default)]
    pub take_profit: Option<f64>,
    /// Money the trader meant to risk.
    #[serde(default)]
    pub risk: Option<f64>,
    /// The chart the decision was made on (`H1`, `M15`, …).
    #[serde(default)]
    pub timeframe: String,
    /// Unix milliseconds of writing; a plan written after the close is a review, not a plan.
    #[serde(default)]
    pub written_at: i64,
}

impl EntryPlan {
    /// Planned reward over planned risk, in price distance.
    pub fn reward_risk(&self) -> Option<f64> {
        let (entry, sl, tp) = (self.entry?, self.stop_loss?, self.take_profit?);
        let risk = (entry - sl).abs();
        (risk > f64::EPSILON).then(|| (tp - entry).abs() / risk)
    }
}

/// How a trade went against its plan.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PlanReview {
    /// The fill against the planned entry, in price; positive is worse for the trader.
    pub entry_slippage: Option<f64>,
    /// The stop at the close differs from the planned one.
    pub stop_moved: bool,
    /// The stop was moved away from the price (more risk), the classic mistake.
    pub stop_widened: bool,
    pub planned_reward_risk: Option<f64>,
    /// The trade's R measured on the planned stop.
    pub realised_r: Option<f64>,
    /// The plan was written before the trade closed.
    pub written_before_close: bool,
}

/// Compares `trade` with `plan`.
pub fn review(plan: &EntryPlan, trade: &Trade) -> PlanReview {
    let worse = |actual: f64, planned: f64| match trade.side {
        Side::Long => actual - planned,
        Side::Short => planned - actual,
    };
    let entry_slippage = plan.entry.map(|planned| worse(trade.open_price, planned));
    let (stop_moved, stop_widened) = match plan.stop_loss {
        Some(planned) if trade.sl > 0.0 && (trade.sl - planned).abs() > f64::EPSILON => {
            let widened = match trade.side {
                Side::Long => trade.sl < planned,
                Side::Short => trade.sl > planned,
            };
            (true, widened)
        }
        _ => (false, false),
    };
    let realised_r = match (plan.entry, plan.stop_loss, trade.close_price) {
        (Some(entry), Some(sl), Some(close)) if (entry - sl).abs() > f64::EPSILON => {
            let moved = match trade.side {
                Side::Long => close - trade.open_price,
                Side::Short => trade.open_price - close,
            };
            Some(moved / (entry - sl).abs())
        }
        _ => None,
    };
    PlanReview {
        entry_slippage,
        stop_moved,
        stop_widened,
        planned_reward_risk: plan.reward_risk(),
        realised_r,
        written_before_close: trade.close_time.is_none_or(|closed| plan.written_at > 0 && plan.written_at <= closed),
    }
}

/// The default exit-reason key of an MT5 `ENUM_DEAL_REASON` (`DEAL_REASON_SL` → `sl`).
pub fn exit_reason_of_deal(reason: &str) -> Option<&'static str> {
    let reason = reason.trim().to_ascii_lowercase();
    let reason = reason.strip_prefix("deal_reason_").unwrap_or(&reason);
    Some(match reason {
        "sl" | "4" => "sl",
        "tp" | "5" => "tp",
        "so" | "6" => "stop_out",
        "expert" | "3" => "ea",
        "client" | "mobile" | "web" | "0" | "1" | "2" => "manual",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TradeKey;

    fn trade(side: Side, open: f64, close: f64, sl: f64) -> Trade {
        Trade {
            key: TradeKey { account_id: 1, position_id: 1, segment: 0 },
            symbol: "XAUUSD".into(),
            side,
            open_time: 1_000,
            close_time: Some(5_000),
            volume: 0.5,
            open_price: open,
            close_price: Some(close),
            sl,
            tp: 0.0,
            commission: 0.0,
            swap: 0.0,
            fee: 0.0,
            profit: 0.0,
            risk: None,
            result_r: None,
            gaps: crate::model::Gaps::NONE,
            magic: 0,
            comment: String::new(),
            manual: false,
        }
    }

    #[test]
    fn a_long_whose_stop_was_widened() {
        let plan = EntryPlan {
            entry: Some(2648.0),
            stop_loss: Some(2644.0),
            take_profit: Some(2660.0),
            written_at: 900,
            ..EntryPlan::default()
        };
        let got = review(&plan, &trade(Side::Long, 2648.2, 2656.2, 2640.0));
        assert_eq!(got.planned_reward_risk, Some(3.0));
        assert!((got.entry_slippage.expect("planned entry") - 0.2).abs() < 1e-9);
        assert!(got.stop_moved && got.stop_widened);
        assert!((got.realised_r.expect("planned stop") - 2.0).abs() < 1e-9);
        assert!(got.written_before_close);
    }

    #[test]
    fn a_short_whose_stop_was_tightened() {
        let plan = EntryPlan { entry: Some(1.2684), stop_loss: Some(1.2705), ..EntryPlan::default() };
        let got = review(&plan, &trade(Side::Short, 1.2684, 1.2651, 1.2690));
        assert!(got.stop_moved && !got.stop_widened);
        assert!(!got.written_before_close, "a plan without a time is not a plan written before");
    }

    #[test]
    fn deal_reasons_map_to_exit_reasons() {
        assert_eq!(exit_reason_of_deal("DEAL_REASON_TP"), Some("tp"));
        assert_eq!(exit_reason_of_deal("4"), Some("sl"));
        assert_eq!(exit_reason_of_deal("mobile"), Some("manual"));
        assert_eq!(exit_reason_of_deal("rollover"), None);
    }
}
