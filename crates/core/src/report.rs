//! What leaves the app: a card of a trade or a period to share as an image, and the monthly
//! report exported as PDF. This module assembles their content and decides what each
//! [`Privacy`] level lets through; [`render_png`] draws the card and [`write_pdf`] lays out the
//! report, both with the Inter and `JetBrains Mono` files the app bundles.
//!
//! The image is rasterized here, with `resvg` over an SVG built from the card, rather than by an
//! NSH canvas snapshot: the platform's offscreen render is not available to headless runs, and
//! doing it in `core` lets the gate test the PNG (size, decoding, that it differs by privacy).
//! The PDF is written with `krilla` (real, selectable text; fonts subset and embedded).

use chrono::NaiveDate;

use crate::{
    metrics::{CalendarWeek, EquityPoint, Group, MistakeCost, Stats},
    missed::MissedSummary,
    model::{Side, Trade},
    review::PeriodReview,
    time::{self, Clock},
};

mod card;
mod fonts;
mod pdf;

pub use card::render_png;
pub use pdf::{pdf_bytes, write_pdf};

/// How much a shared card or report shows: traders often share R and hide money.
///
/// On the monthly report, [`Privacy::NoFigures`] hides money and R but keeps counts and ratios
/// (trades, win rate, profit factor, drawdown percent); the trader's own review text is printed
/// as written.
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

impl Privacy {
    /// Whether money amounts may be shown.
    pub fn shows_money(self) -> bool {
        self == Self::ShowMoney
    }

    /// Whether results in R may be shown.
    pub fn shows_r(self) -> bool {
        self != Self::NoFigures
    }
}

/// Which way a result went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Win,
    Loss,
    Flat,
}

impl Outcome {
    /// Under half a cent is flat.
    const FLAT: f64 = 0.005;

    /// The outcome of a net result.
    pub fn of(net: f64) -> Self {
        if net > Self::FLAT {
            Self::Win
        } else if net < -Self::FLAT {
            Self::Loss
        } else {
            Self::Flat
        }
    }

    /// The word the card shows when it hides the figures.
    pub fn label(self) -> &'static str {
        match self {
            Self::Win => "Thắng",
            Self::Loss => "Thua",
            Self::Flat => "Hòa vốn",
        }
    }
}

/// `1,234.50` with `decimals` places: commas between the thousands, a dot for decimals.
fn grouped(value: f64, decimals: usize) -> String {
    let fixed = format!("{:.*}", decimals, value.abs());
    let (whole, fraction) = fixed.split_once('.').unwrap_or((fixed.as_str(), ""));
    let mut out = String::with_capacity(fixed.len() + whole.len() / 3);
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    if !fraction.is_empty() {
        out.push('.');
        out.push_str(fraction);
    }
    out
}

/// Whether `value` rounds to a non-zero amount at `decimals` places.
fn is_nonzero(value: f64, decimals: usize) -> bool {
    format!("{:.*}", decimals, value.abs()).chars().any(|c| matches!(c, '1'..='9'))
}

/// The sign to print before `value` at `decimals` places: `+`, a real minus, or nothing at zero.
fn sign_of(value: f64, decimals: usize) -> &'static str {
    match (is_nonzero(value, decimals), value < 0.0) {
        (false, _) => "",
        (true, true) => "\u{2212}",
        (true, false) => "+",
    }
}

/// A signed amount of money: `+$409.00`, `−$103.50`, `+1,200.00 EUR`. USD, or an unknown
/// currency, is a `$` before the digits; any other currency follows as its code.
pub(crate) fn money(value: f64, currency: &str) -> String {
    let (sign, digits) = (sign_of(value, 2), grouped(value, 2));
    match currency {
        "" | "USD" => format!("{sign}${digits}"),
        code => format!("{sign}{digits} {code}"),
    }
}

/// [`money`] rounded to whole units, for the small cells of a calendar: `+$330`, `−$1,249`.
pub(crate) fn money_whole(value: f64, currency: &str) -> String {
    let (sign, digits) = (sign_of(value, 0), grouped(value, 0));
    match currency {
        "" | "USD" => format!("{sign}${digits}"),
        code => format!("{sign}{digits} {code}"),
    }
}

/// A signed result in R at `decimals` places: `+6.4R`, `−1.25R`.
pub(crate) fn r_text(r: f64, decimals: usize) -> String {
    format!("{}{:.*}R", sign_of(r, decimals), decimals, r.abs())
}

/// A share between 0 and 1 as a whole percent: `62%`.
pub(crate) fn percent(share: f64) -> String {
    format!("{:.0}%", share * 100.0)
}

/// One labelled figure at the foot of a card: `("Lệnh", "21")`.
pub type Figure = (String, String);

/// The content of a shared card: one trade, or one week or month.
#[derive(Debug, Clone, PartialEq)]
pub struct ShareCard {
    /// `XAUUSD · Buy`, or `Tuần 47 · 16–22/11/2026`.
    pub title: String,
    /// The day of a trade; empty for a period.
    pub date: String,
    /// The account's name, when the trader chose to show it.
    pub account: Option<String>,
    pub privacy: Privacy,
    /// `+2.10R`, or empty with [`Privacy::NoFigures`] or when the trade has no R.
    pub result_r: String,
    /// `+$409.00`, shown only with [`Privacy::ShowMoney`].
    pub result_money: Option<String>,
    pub outcome: Outcome,
    /// Labelled figures along the foot: side, time held and setup; or trades, win rate, profit
    /// factor and main setup. Figures are left out when the privacy hides them.
    pub stats: Vec<Figure>,
    /// A line to draw beside the result (cumulative R of a period); a shape, never labelled.
    pub curve: Vec<f64>,
    /// The chart image to put beside the figures (the exit screenshot first): the path of a file.
    pub chart: Option<String>,
}

/// What the big result of a card says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Headline {
    pub main: String,
    /// A smaller second figure: the R under the money.
    pub secondary: Option<String>,
    /// The line under the result, saying what is and is not shown.
    pub caption: &'static str,
}

impl ShareCard {
    /// The card of a closed trade. `currency` is the account's (USD shows as `$`).
    pub fn new(
        trade: &Trade,
        r: Option<f64>,
        setup: Option<String>,
        chart: Option<String>,
        currency: &str,
        privacy: Privacy,
        clock: Clock,
    ) -> Self {
        let net = trade.net();
        let side = match trade.side {
            Side::Long => "Buy",
            Side::Short => "Sell",
        };
        let mut stats = vec![("Chiều".to_owned(), side.to_owned())];
        if let Some(held) = trade.duration_secs() {
            stats.push(("Thời gian giữ".to_owned(), time::format_duration(held)));
        }
        stats.extend(setup.filter(|s| !s.trim().is_empty()).map(|s| ("Setup".to_owned(), s)));
        Self {
            title: format!("{} · {side}", trade.symbol),
            date: time::format_day(clock.day(trade.close_time.unwrap_or(trade.open_time))),
            account: None,
            privacy,
            result_r: r.filter(|_| privacy.shows_r()).map_or_default(|r| r_text(r, 2)),
            result_money: privacy.shows_money().then(|| money(net, currency)),
            outcome: Outcome::of(net),
            stats,
            curve: Vec::new(),
            chart,
        }
    }

    /// The card of a week or a month.
    pub fn period(title: String, figures: &PeriodFigures, privacy: Privacy) -> Self {
        let mut stats = Vec::new();
        if privacy.shows_r() {
            stats.push(("Lệnh".to_owned(), figures.trades.to_string()));
            if figures.trades > 0 {
                let rate = f64::from(u32::try_from(figures.wins).unwrap_or(u32::MAX))
                    / f64::from(u32::try_from(figures.trades).unwrap_or(u32::MAX));
                stats.push(("Thắng".to_owned(), percent(rate)));
            }
            if let Some(factor) = figures.profit_factor {
                stats.push(("Profit factor".to_owned(), format!("{factor:.2}")));
            }
        }
        stats.extend(figures.top_setup.clone().filter(|s| !s.trim().is_empty()).map(|s| ("Setup chính".to_owned(), s)));
        Self {
            title,
            date: String::new(),
            account: None,
            privacy,
            result_r: figures.total_r.filter(|_| privacy.shows_r()).map_or_default(|r| r_text(r, 1)),
            result_money: privacy.shows_money().then(|| money(figures.net, &figures.currency)),
            outcome: Outcome::of(figures.net),
            stats,
            curve: figures.r_curve.clone(),
            chart: None,
        }
    }

    /// The same card naming the account (and broker) it came from.
    #[must_use]
    pub fn with_account(mut self, account: Option<String>) -> Self {
        self.account = account.filter(|a| !a.trim().is_empty());
        self
    }

    /// What the big result says: the money, the R, or only whether it won.
    pub fn headline(&self) -> Headline {
        let r = (!self.result_r.is_empty()).then(|| self.result_r.clone());
        match (self.privacy, &self.result_money, r) {
            (Privacy::ShowMoney, Some(money), r) => {
                Headline { main: money.clone(), secondary: r, caption: "kết quả theo tiền và R" }
            }
            (Privacy::NoFigures, _, _) => {
                Headline {
                    main: self.outcome.label().to_owned(), secondary: None, caption: "không hiện số liệu"
                }
            }
            (_, _, Some(r)) => {
                Headline { main: r, secondary: None, caption: "kết quả theo R, không hiện số tiền" }
            }
            (_, _, None) => Headline {
                main: self.outcome.label().to_owned(),
                secondary: None,
                caption: "chưa đo được R, không hiện số tiền",
            },
        }
    }
}

/// What the card of a week or a month is made of.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PeriodFigures {
    pub trades: usize,
    pub wins: usize,
    pub net: f64,
    /// The period's R; `None` when no trade has one.
    pub total_r: Option<f64>,
    pub profit_factor: Option<f64>,
    /// The setup with the most trades.
    pub top_setup: Option<String>,
    /// The cumulative R after each trade, oldest first.
    pub r_curve: Vec<f64>,
    pub currency: String,
}

/// The content of a monthly report.
#[derive(Debug, Clone, PartialEq)]
pub struct MonthlyReport {
    /// The account's title, or `Tất cả tài khoản`.
    pub account: String,
    pub currency: String,
    /// Any day of the month.
    pub month: NaiveDate,
    pub stats: Stats,
    /// The balance after each closed trade and cash movement of the month.
    pub curve: Vec<EquityPoint>,
    /// The cumulative R after each trade of the month, oldest first.
    pub r_curve: Vec<f64>,
    /// The month's calendar, Monday first.
    pub weeks: Vec<CalendarWeek>,
    pub by_playbook: Vec<Group>,
    /// The three most costly mistakes.
    pub mistakes: Vec<MistakeCost>,
    pub missed: MissedSummary,
    /// `(reason's name, count, net would-be R)`, from [`MissedSummary::by_reason`] with the
    /// trader's names for the reasons.
    pub missed_reasons: Vec<(String, usize, f64)>,
    /// The trader's monthly review, when written.
    pub review: Option<PeriodReview>,
}

impl MonthlyReport {
    /// A report with its headline numbers; the rest is added with the `with_*` methods.
    pub fn new(account: String, currency: String, month: NaiveDate, stats: Stats) -> Self {
        Self {
            account,
            currency,
            month,
            stats,
            curve: Vec::new(),
            r_curve: Vec::new(),
            weeks: Vec::new(),
            by_playbook: Vec::new(),
            mistakes: Vec::new(),
            missed: MissedSummary::default(),
            missed_reasons: Vec::new(),
            review: None,
        }
    }

    /// The balance curve and the cumulative R curve of the month.
    #[must_use]
    pub fn with_curves(mut self, balance: Vec<EquityPoint>, r: Vec<f64>) -> Self {
        self.curve = balance;
        self.r_curve = r;
        self
    }

    /// The month's calendar ([`crate::metrics::month_weeks`]).
    #[must_use]
    pub fn with_calendar(mut self, weeks: Vec<CalendarWeek>) -> Self {
        self.weeks = weeks;
        self
    }

    /// The breakdown by playbook.
    #[must_use]
    pub fn with_playbooks(mut self, groups: Vec<Group>) -> Self {
        self.by_playbook = groups;
        self
    }

    /// Keeps the three most costly mistakes ([`crate::metrics::mistake_cost`] sorts them).
    #[must_use]
    pub fn with_mistakes(mut self, mut mistakes: Vec<MistakeCost>) -> Self {
        mistakes.truncate(3);
        self.mistakes = mistakes;
        self
    }

    /// The missed trades of the month; `reason_name` gives the trader's name for a reason's key.
    #[must_use]
    pub fn with_missed(mut self, summary: MissedSummary, reason_name: impl Fn(&str) -> String) -> Self {
        self.missed_reasons = summary
            .by_reason
            .iter()
            .map(|(key, count, r)| {
                let name = if key.is_empty() { "Không ghi lý do".to_owned() } else { reason_name(key) };
                (name, *count, *r)
            })
            .collect();
        self.missed = summary;
        self
    }

    /// The month's review.
    #[must_use]
    pub fn with_review(mut self, review: Option<PeriodReview>) -> Self {
        self.review = review;
        self
    }
}

/// Why a report could not be written.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    #[error("không ghi được file: {0}")]
    Io(#[from] std::io::Error),
    #[error("không dựng được báo cáo: {0}")]
    Render(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TradeKey;

    pub(super) fn trade(profit: f64) -> Trade {
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

    fn card(p: Privacy) -> ShareCard {
        ShareCard::new(&trade(412.5), Some(2.1), Some("Breakout".into()), None, "USD", p, Clock::default())
    }

    #[test]
    fn privacy_decides_the_figures_on_a_card() {
        assert_eq!(card(Privacy::OnlyR).result_r, "+2.10R");
        assert_eq!(card(Privacy::OnlyR).result_money, None);
        assert_eq!(card(Privacy::ShowMoney).result_money.as_deref(), Some("+$409.00"));
        assert_eq!(card(Privacy::NoFigures).result_r, "");
        assert_eq!(card(Privacy::NoFigures).result_money, None);
        assert_eq!(card(Privacy::NoFigures).outcome, Outcome::Win);
        let lost = ShareCard::new(&trade(-100.0), None, None, None, "USD", Privacy::ShowMoney, Clock::default());
        assert_eq!(lost.result_money.as_deref(), Some("−$103.50"));
        assert_eq!(lost.outcome, Outcome::Loss);
    }

    #[test]
    fn a_trade_card_lists_side_time_held_and_setup() {
        let stats = card(Privacy::OnlyR).stats;
        assert_eq!(stats[0], ("Chiều".to_owned(), "Buy".to_owned()));
        assert_eq!(stats[1], ("Thời gian giữ".to_owned(), "44p".to_owned()));
        assert_eq!(stats[2], ("Setup".to_owned(), "Breakout".to_owned()));
        let without =
            ShareCard::new(&trade(1.0), None, Some("  ".into()), None, "USD", Privacy::OnlyR, Clock::default());
        assert_eq!(without.stats.len(), 2, "a blank setup is not shown");
        assert_eq!(card(Privacy::OnlyR).title, "XAUUSD · Buy");
    }

    #[test]
    fn the_headline_says_what_the_privacy_allows() {
        let money = card(Privacy::ShowMoney).headline();
        assert_eq!((money.main.as_str(), money.secondary.as_deref()), ("+$409.00", Some("+2.10R")));
        let only_r = card(Privacy::OnlyR).headline();
        assert_eq!(only_r.main, "+2.10R");
        assert!(only_r.caption.contains("không hiện số tiền"));
        let hidden = card(Privacy::NoFigures).headline();
        assert_eq!((hidden.main.as_str(), hidden.secondary), ("Thắng", None));
        let unmeasured = ShareCard::new(&trade(50.0), None, None, None, "USD", Privacy::OnlyR, Clock::default());
        assert_eq!(unmeasured.headline().main, "Thắng", "no R to show, so only the outcome");
    }

    #[test]
    fn a_period_card_keeps_counts_unless_the_figures_are_hidden() {
        let figures = PeriodFigures {
            trades: 21,
            wins: 13,
            net: 1248.6,
            total_r: Some(6.4),
            profit_factor: Some(2.314),
            top_setup: Some("Breakout".into()),
            r_curve: vec![0.5, 1.0, 0.4],
            currency: "USD".into(),
        };
        let shown = ShareCard::period("Tuần 47 · 16–22/11/2026".into(), &figures, Privacy::OnlyR);
        assert_eq!(shown.result_r, "+6.4R");
        let labels: Vec<&str> = shown.stats.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(labels, ["Lệnh", "Thắng", "Profit factor", "Setup chính"]);
        assert_eq!(shown.stats[1].1, "62%");
        assert_eq!(shown.curve.len(), 3);
        let money = ShareCard::period("Tháng 11/2026".into(), &figures, Privacy::ShowMoney);
        assert_eq!(money.result_money.as_deref(), Some("+$1,248.60"));
        let hidden = ShareCard::period("Tháng 11/2026".into(), &figures, Privacy::NoFigures);
        assert_eq!(hidden.result_r, "");
        assert_eq!(hidden.stats, [("Setup chính".to_owned(), "Breakout".to_owned())]);
        assert_eq!(hidden.headline().main, "Thắng");
    }

    #[test]
    fn money_and_r_read_like_the_app_shows_them() {
        assert_eq!(money(1126.4, "USD"), "+$1,126.40");
        assert_eq!(money(-116.35, ""), "−$116.35");
        assert_eq!(money(2000.0, "EUR"), "+2,000.00 EUR");
        assert_eq!(money(0.001, "USD"), "$0.00");
        assert_eq!(r_text(-1.25, 2), "−1.25R");
        assert_eq!(r_text(0.0, 1), "0.0R");
    }

    #[test]
    fn the_report_keeps_three_mistakes_and_names_the_missed_reasons() {
        let cost = |key: &str, net: f64| MistakeCost {
            key: key.into(),
            label: key.into(),
            trades: 1,
            net,
            total_r: net / 100.0,
            share_of_losses: 0.1,
            expectancy_gap: 0.0,
        };
        let missed = MissedSummary {
            count: 3,
            r_missed: 4.0,
            r_avoided: 1.0,
            by_reason: vec![("fear".into(), 2, 3.0), (String::new(), 1, -1.0)],
        };
        let report = MonthlyReport::new(
            "Demo".into(),
            "USD".into(),
            NaiveDate::from_ymd_opt(2026, 11, 1).expect("a date"),
            Stats::default(),
        )
        .with_mistakes(vec![cost("a", -9.0), cost("b", -8.0), cost("c", -7.0), cost("d", -1.0)])
        .with_missed(missed, |key| format!("Lý do {key}"));
        assert_eq!(report.mistakes.len(), 3);
        assert_eq!(report.missed_reasons[0].0, "Lý do fear");
        assert_eq!(report.missed_reasons[1].0, "Không ghi lý do");
    }
}
