//! What leaves the app: a card of one trade to share as an image, and the monthly
//! report exported as PDF. This module assembles their content; the app draws the
//! card and the PDF writer lays out the report.

use chrono::NaiveDate;

use crate::{
    metrics::{Group, MistakeCost, Stats},
    missed::MissedSummary,
    model::{Side, Trade},
    review::PeriodReview,
    time::{self, Clock},
};

/// How much a shared card shows: traders often share R and hide money.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Privacy {
    /// Money and R.
    ShowMoney,
    /// R and percent of the account only.
    #[default]
    OnlyR,
    /// The chart, the setup and the outcome sign; no figures.
    NoFigures,
}

/// The content of a shared trade card.
#[derive(Debug, Clone, PartialEq)]
pub struct ShareCard {
    pub symbol: String,
    pub side: &'static str,
    /// `+2.10R`, or empty with [`Privacy::NoFigures`].
    pub result_r: String,
    /// `+$409.00`, shown only with [`Privacy::ShowMoney`].
    pub result_money: Option<String>,
    pub won: bool,
    pub setup: Option<String>,
    pub held: Option<String>,
    pub date: String,
    /// The chart image to put behind the figures (the exit screenshot first).
    pub chart: Option<String>,
}

impl ShareCard {
    pub fn new(
        trade: &Trade,
        r: Option<f64>,
        setup: Option<String>,
        chart: Option<String>,
        privacy: Privacy,
        clock: Clock,
    ) -> Self {
        let net = trade.net();
        let signed = |v: f64, unit: &str| format!("{}{:.2}{unit}", if v >= 0.0 { "+" } else { "−" }, v.abs());
        let figures = privacy != Privacy::NoFigures;
        Self {
            symbol: trade.symbol.clone(),
            side: match trade.side {
                Side::Long => "Buy",
                Side::Short => "Sell",
            },
            result_r: r.filter(|_| figures).map(|r| signed(r, "R")).unwrap_or_default(),
            result_money: (privacy == Privacy::ShowMoney).then(|| {
                let money = signed(net, "");
                format!(
                    "{}${}",
                    &money[..money.len() - money.trim_start_matches(['+', '−']).len()],
                    money.trim_start_matches(['+', '−'])
                )
            }),
            won: net > 0.0,
            setup,
            held: trade.duration_secs().map(time::format_duration),
            date: time::format_day(clock.day(trade.close_time.unwrap_or(trade.open_time))),
            chart,
        }
    }
}

/// The content of a monthly report.
#[derive(Debug, Clone, PartialEq)]
pub struct MonthlyReport {
    pub account: String,
    pub month: NaiveDate,
    pub stats: Stats,
    pub by_playbook: Vec<Group>,
    /// The three most costly mistakes.
    pub mistakes: Vec<MistakeCost>,
    pub missed: MissedSummary,
    /// The trader's monthly review, when written.
    pub review: Option<PeriodReview>,
}

impl MonthlyReport {
    /// Keeps the three most costly mistakes ([`crate::metrics::mistake_cost`] sorts them).
    pub fn new(
        account: String,
        month: NaiveDate,
        stats: Stats,
        by_playbook: Vec<Group>,
        mut mistakes: Vec<MistakeCost>,
        missed: MissedSummary,
        review: Option<PeriodReview>,
    ) -> Self {
        mistakes.truncate(3);
        Self { account, month, stats, by_playbook, mistakes, missed, review }
    }
}

/// Why a report could not be written.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    #[error("không ghi được file: {0}")]
    Io(#[from] std::io::Error),
    #[error("chưa làm: {0}")]
    NotImplemented(&'static str),
}

/// Writes `report` as an A4 PDF at `path`.
///
/// # Errors
///
/// [`ReportError::Io`] when the file cannot be written; [`ReportError::NotImplemented`] until v0.3.
pub fn write_pdf(_report: &MonthlyReport, _path: &std::path::Path) -> Result<(), ReportError> {
    // TODO(v0.3): cover (account, month, net, win rate, PF), balance curve, calendar,
    // playbook table, mistakes, missed trades, the review's answers; embedded Inter.
    Err(ReportError::NotImplemented("báo cáo PDF"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TradeKey;

    fn trade(profit: f64) -> Trade {
        Trade {
            key: TradeKey { account_id: 1, position_id: 1, segment: 0 },
            symbol: "XAUUSD".into(),
            side: Side::Long,
            open_time: 1_763_650_692_000,
            close_time: Some(1_763_653_350_000),
            volume: 0.5,
            open_price: 2648.2,
            close_price: Some(2656.45),
            sl: 2644.3,
            tp: 2660.0,
            commission: -3.5,
            swap: 0.0,
            fee: 0.0,
            profit,
            risk: Some(195.0),
            magic: 0,
            comment: String::new(),
            manual: false,
        }
    }

    #[test]
    fn privacy_decides_the_figures_on_a_card() {
        let t = trade(412.5);
        let card = |p| ShareCard::new(&t, Some(2.1), Some("Breakout".into()), None, p, Clock::default());
        assert_eq!(card(Privacy::OnlyR).result_r, "+2.10R");
        assert_eq!(card(Privacy::OnlyR).result_money, None);
        assert_eq!(card(Privacy::ShowMoney).result_money.as_deref(), Some("+$409.00"));
        assert_eq!(card(Privacy::NoFigures).result_r, "");
        assert!(card(Privacy::NoFigures).won);
        assert_eq!(card(Privacy::OnlyR).held.as_deref(), Some("44p"));
        let lost = ShareCard::new(&trade(-100.0), None, None, None, Privacy::ShowMoney, Clock::default());
        assert_eq!(lost.result_money.as_deref(), Some("−$103.50"));
    }
}
