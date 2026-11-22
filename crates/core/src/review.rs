//! Weekly and monthly reviews written from a template the trader edits, and the queue
//! of trades that need a second look.

use chrono::{Datelike, Duration, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::{
    catalog::CustomValues,
    model::{JournalEntry, Trade, TradeKey},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Period {
    Week,
    Month,
}

impl Period {
    /// The name stored and referenced (`week`, `month`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Week => "week",
            Self::Month => "month",
        }
    }

    /// The period of a stored [`Period::name`].
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "week" => Some(Self::Week),
            "month" => Some(Self::Month),
            _ => None,
        }
    }

    /// `Tuần 47 · 16–22/11` or `Tháng 11/2026`: the period starting `start`, as titled to the trader.
    pub fn title(self, start: NaiveDate) -> String {
        let (first, last) = self.bounds(start);
        match self {
            Self::Week => {
                format!("Tuần {} · {}–{}", first.iso_week().week(), first.format("%d"), last.format("%d/%m"))
            }
            Self::Month => format!("Tháng {}", first.format("%m/%Y")),
        }
    }

    /// The first and last day of the period holding `day`; weeks start on Monday.
    pub fn bounds(self, day: NaiveDate) -> (NaiveDate, NaiveDate) {
        match self {
            Self::Week => {
                let start = day - Duration::days(i64::from(day.weekday().num_days_from_monday()));
                (start, start + Duration::days(6))
            }
            Self::Month => {
                let start = day.with_day(1).unwrap_or(day);
                let next = if start.month() == 12 {
                    NaiveDate::from_ymd_opt(start.year() + 1, 1, 1)
                } else {
                    NaiveDate::from_ymd_opt(start.year(), start.month() + 1, 1)
                };
                (start, next.map_or(day, |n| n - Duration::days(1)))
            }
        }
    }
}

/// The questions a review answers; the trader edits, adds and reorders them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewTemplate {
    pub period: Period,
    pub prompts: Vec<String>,
}

impl ReviewTemplate {
    pub fn default_for(period: Period) -> Self {
        let prompts: &[&str] = match period {
            Period::Week => &[
                "3 điều làm tốt tuần này",
                "3 lỗi lặp lại và cái giá của chúng",
                "Setup nào hiệu quả nhất, setup nào nên tạm dừng",
                "Mục tiêu và quy tắc cho tuần sau",
            ],
            Period::Month => &[
                "Kết quả so với mục tiêu tháng",
                "Thay đổi lớn nhất trong cách giao dịch",
                "Lỗi tốn kém nhất và kế hoạch loại bỏ",
                "Điều chỉnh playbook và quy tắc rủi ro",
                "Mục tiêu tháng sau",
            ],
        };
        Self { period, prompts: prompts.iter().map(|p| (*p).to_owned()).collect() }
    }
}

/// A written review of one week or month.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeriodReview {
    #[serde(skip)]
    pub id: i64,
    #[serde(default)]
    pub account_id: Option<i64>,
    pub period: Period,
    pub start: NaiveDate,
    /// `(prompt, answer)`: the prompts as they were when written, so editing the
    /// template later does not change old reviews.
    pub answers: Vec<(String, String)>,
    /// 1 to 5 for the period's discipline; 0 when not rated.
    #[serde(default)]
    pub rating: u8,
    #[serde(default)]
    pub custom: CustomValues,
}

impl PeriodReview {
    /// A blank review of the period holding `day`, with the template's prompts.
    pub fn blank(template: &ReviewTemplate, day: NaiveDate, account_id: Option<i64>) -> Self {
        Self {
            id: 0,
            account_id,
            period: template.period,
            start: template.period.bounds(day).0,
            answers: template.prompts.iter().map(|p| (p.clone(), String::new())).collect(),
            rating: 0,
            custom: CustomValues::new(),
        }
    }
}

/// Why a trade is in the review queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum QueueReason {
    /// The trader flagged it.
    Flagged,
    /// Lost more than the planned risk.
    LostMoreThanRisk,
    /// A closed trade with nothing written.
    NotJournalled,
}

impl QueueReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::Flagged => "Bạn đánh dấu xem lại",
            Self::LostMoreThanRisk => "Lỗ quá 1R",
            Self::NotJournalled => "Chưa ghi nhật ký",
        }
    }
}

/// The closed trades that need a look, with every reason that applies; flagged first.
pub fn queue<'t>(
    trades: impl IntoIterator<Item = (&'t Trade, Option<&'t JournalEntry>, Option<f64>)>,
) -> Vec<(TradeKey, Vec<QueueReason>)> {
    let mut queue: Vec<(TradeKey, Vec<QueueReason>)> = trades
        .into_iter()
        .filter(|(trade, _, _)| trade.is_closed())
        .filter_map(|(trade, journal, r)| {
            let mut reasons = Vec::new();
            if journal.is_some_and(|j| j.review_later) {
                reasons.push(QueueReason::Flagged);
            }
            if r.is_some_and(|r| r < -1.05) {
                reasons.push(QueueReason::LostMoreThanRisk);
            }
            if journal.is_none_or(JournalEntry::is_empty) {
                reasons.push(QueueReason::NotJournalled);
            }
            (!reasons.is_empty()).then(|| (trade.key.clone(), reasons))
        })
        .collect();
    queue.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    queue
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("a test date")
    }

    #[test]
    fn periods_start_on_monday_and_the_first_of_the_month() {
        assert_eq!(Period::Week.bounds(day("2026-11-20")), (day("2026-11-16"), day("2026-11-22")));
        assert_eq!(Period::Month.bounds(day("2026-12-31")), (day("2026-12-01"), day("2026-12-31")));
        assert_eq!(Period::Month.bounds(day("2028-02-10")), (day("2028-02-01"), day("2028-02-29")));
    }

    #[test]
    fn periods_are_named_for_the_trader_and_for_storage() {
        assert_eq!(Period::Week.title(day("2026-11-18")), "Tuần 47 · 16–22/11");
        assert_eq!(Period::Month.title(day("2026-11-18")), "Tháng 11/2026");
        assert_eq!(Period::parse(Period::Week.name()), Some(Period::Week));
        assert_eq!(Period::parse("year"), None);
    }

    #[test]
    fn the_queue_lists_flagged_oversized_and_blank_trades() {
        use crate::model::Side;
        let trade = |position_id| Trade {
            key: TradeKey { account_id: 1, position_id, segment: 0 },
            symbol: "EURUSD".into(),
            side: Side::Short,
            open_time: 0,
            close_time: Some(1),
            volume: 0.1,
            open_price: 1.0,
            close_price: Some(1.0),
            sl: 0.0,
            tp: 0.0,
            commission: 0.0,
            swap: 0.0,
            fee: 0.0,
            profit: 0.0,
            risk: None,
            magic: 0,
            comment: String::new(),
            manual: false,
        };
        let (a, b, c) = (trade(1), trade(2), trade(3));
        let flagged =
            JournalEntry { review_later: true, notes: "xem lại điểm vào".into(), ..JournalEntry::default() };
        let written = JournalEntry { notes: "đúng kế hoạch".into(), ..JournalEntry::default() };
        let got =
            queue([(&a, Some(&written), Some(-1.6)), (&b, Some(&flagged), Some(0.4)), (&c, Some(&written), Some(1.2))]);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], (b.key, vec![QueueReason::Flagged]));
        assert_eq!(got[1], (a.key, vec![QueueReason::LostMoreThanRisk]));
    }

    #[test]
    fn a_blank_review_carries_the_template() {
        let review = PeriodReview::blank(&ReviewTemplate::default_for(Period::Week), day("2026-11-20"), None);
        assert_eq!(review.start, day("2026-11-16"));
        assert_eq!(review.answers.len(), 4);
    }
}
