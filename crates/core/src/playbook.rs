//! The playbook: the trader's setups as rules, examples and statistics of their own.
//! A journal entry links its trade to one setup of it.

use serde::{Deserialize, Serialize};

use crate::{
    catalog::CustomValues,
    metrics::{self, Group, Sample},
};

/// One setup the trader trades.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Playbook {
    #[serde(skip)]
    pub id: i64,
    pub name: String,
    /// What the setup is, in a sentence or two.
    #[serde(default)]
    pub description: String,
    /// Symbols it applies to; empty for any.
    #[serde(default)]
    pub symbols: Vec<String>,
    /// Keys of the trader's timeframe catalog.
    #[serde(default)]
    pub timeframes: Vec<String>,
    #[serde(default)]
    pub entry_rules: Vec<String>,
    #[serde(default)]
    pub exit_rules: Vec<String>,
    /// When the idea is wrong and the trade is not taken.
    #[serde(default)]
    pub invalidation: String,
    /// Usual risk, in percent of the balance.
    #[serde(default)]
    pub risk_pct: Option<f64>,
    /// Screenshots of textbook examples.
    #[serde(default)]
    pub examples: Vec<crate::attachments::Attachment>,
    /// Items to confirm before entering with this setup, on top of the account's checklist.
    #[serde(default)]
    pub checklist: Vec<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub custom: CustomValues,
}

/// The statistics of each setup, in the order of `playbooks`; trades with no setup
/// come last as "Chưa gắn setup".
pub fn by_playbook(
    samples: &[Sample<'_>],
    playbook_of: impl Fn(&Sample<'_>) -> Option<i64>,
    playbooks: &[Playbook],
) -> Vec<Group> {
    /// Ranks of trades whose setup was deleted, and of trades with none: after the playbook.
    const DELETED: usize = usize::MAX - 1;
    const NONE: usize = usize::MAX;
    let rank = |x: &Sample<'_>| match playbook_of(x) {
        Some(id) => playbooks.iter().position(|p| p.id == id).unwrap_or(DELETED),
        None => NONE,
    };
    // breakdown returns its groups in key order: the playbook's.
    metrics::breakdown(
        samples,
        |x| vec![rank(x)],
        |&rank| match rank {
            NONE => "Chưa gắn setup".to_owned(),
            DELETED => "Setup đã xoá".to_owned(),
            index => playbooks.get(index).map_or_else(String::new, |p| p.name.clone()),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Side, Trade, TradeKey};

    fn trade(position_id: i64, profit: f64) -> Trade {
        Trade {
            key: TradeKey { account_id: 1, position_id, segment: 0 },
            symbol: "XAUUSD".into(),
            side: Side::Long,
            open_time: position_id,
            close_time: Some(position_id + 1),
            volume: 0.1,
            open_price: 1.0,
            close_price: Some(1.0),
            sl: 0.0,
            tp: 0.0,
            commission: 0.0,
            swap: 0.0,
            fee: 0.0,
            profit,
            risk: None,
            result_r: None,
            magic: 0,
            comment: String::new(),
            manual: false,
        }
    }

    #[test]
    fn setups_get_their_own_statistics_in_playbook_order() {
        let playbooks = [
            Playbook { id: 7, name: "Breakout phiên Á".into(), ..Playbook::default() },
            Playbook { id: 3, name: "Pullback EMA".into(), ..Playbook::default() },
        ];
        let trades = [trade(1, 100.0), trade(2, -50.0), trade(3, 80.0), trade(4, -20.0)];
        let samples: Vec<Sample<'_>> = trades.iter().map(|t| Sample { trade: t, r: t.result_r }).collect();
        let link = |x: &Sample<'_>| match x.trade.key.position_id {
            1 | 2 => Some(7),
            3 => Some(3),
            _ => None,
        };
        let groups = by_playbook(&samples, link, &playbooks);
        let names: Vec<&str> = groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(names, ["Breakout phiên Á", "Pullback EMA", "Chưa gắn setup"]);
        assert_eq!(groups[0].trades, 2);
        assert!((groups[0].net - 50.0).abs() < 1e-9);
        assert!((groups[0].win_rate() - 0.5).abs() < 1e-9);
    }
}
