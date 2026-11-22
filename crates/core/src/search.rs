//! Full-text search over everything the trader writes: trade notes and plans, day
//! notes, reviews, missed trades and the playbook. Accents are folded, so `lenh`
//! finds `lệnh`.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::{
    missed::MissedTrade,
    model::TradeKey,
    playbook::Playbook,
    review::{Period, PeriodReview},
};

/// What a hit points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocKind {
    Trade,
    Day,
    Review,
    Missed,
    Playbook,
}

impl DocKind {
    /// Every kind, in the order a result list groups them.
    pub const ALL: [Self; 5] = [Self::Trade, Self::Day, Self::Review, Self::Missed, Self::Playbook];

    /// The heading of the kind's results.
    pub fn label(self) -> &'static str {
        match self {
            Self::Trade => "Lệnh",
            Self::Day => "Nhật ký ngày",
            Self::Review => "Tổng kết",
            Self::Missed => "Lệnh bỏ lỡ",
            Self::Playbook => "Playbook",
        }
    }
}

/// A searchable text: `reference` is the thing's id as its screen opens it
/// (a [`crate::model::TradeKey::id`], a day `YYYY-MM-DD`, a row id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub kind: DocKind,
    pub reference: String,
    pub title: String,
    pub body: String,
}

/// One result, its matching words marked with `[` `]` in the snippet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub kind: DocKind,
    pub reference: String,
    pub title: String,
    pub snippet: String,
}

/// What a hit's `reference` names, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Trade(TradeKey),
    /// A day's note: `account_id` is `None` for the note of all accounts.
    Day {
        day: NaiveDate,
        account_id: Option<i64>,
    },
    /// A review of the period starting `start`; `account_id` is `None` for all accounts.
    Review {
        period: Period,
        start: NaiveDate,
        account_id: Option<i64>,
    },
    Missed(i64),
    Playbook(i64),
}

impl Hit {
    /// The thing the hit points at; `None` when its reference is not one this app wrote.
    pub fn target(&self) -> Option<Target> {
        let reference = self.reference.as_str();
        match self.kind {
            DocKind::Trade => TradeKey::parse(reference).map(Target::Trade),
            DocKind::Day => {
                let (day, account) = match reference.split_once(':') {
                    Some((day, account)) => (day, Some(account.parse().ok()?)),
                    None => (reference, None),
                };
                Some(Target::Day { day: day.parse().ok()?, account_id: account })
            }
            DocKind::Review => {
                let mut parts = reference.split(':');
                let period = Period::parse(parts.next()?)?;
                let start = parts.next()?.parse().ok()?;
                let account: i64 = parts.next()?.parse().ok()?;
                parts.next().is_none().then_some(Target::Review {
                    period,
                    start,
                    account_id: (account != 0).then_some(account),
                })
            }
            DocKind::Missed => reference.parse().ok().map(Target::Missed),
            DocKind::Playbook => reference.parse().ok().map(Target::Playbook),
        }
    }

    /// The snippet cut at its `[` `]` marks: `(text, is a match)`, in order, with no empty parts.
    pub fn segments(&self) -> Vec<(String, bool)> {
        marked(&self.snippet)
    }
}

/// `text` cut at `[` and `]` into `(text, marked)` parts.
pub fn marked(text: &str) -> Vec<(String, bool)> {
    let mut parts = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find(']').map(|at| open + at) else { break };
        if open > 0 {
            parts.push((rest[..open].to_owned(), false));
        }
        if close > open + 1 {
            parts.push((rest[open + 1..close].to_owned(), true));
        }
        rest = &rest[close + 1..];
    }
    if !rest.is_empty() {
        parts.push((rest.to_owned(), false));
    }
    parts
}

/// The index entry of a written review, referenced `period:start:account` (`0` for all accounts).
pub fn review_document(review: &PeriodReview) -> Document {
    let body: Vec<&str> =
        review.answers.iter().flat_map(|(prompt, answer)| [prompt.as_str(), answer.as_str()]).collect();
    Document {
        kind: DocKind::Review,
        reference: format!("{}:{}:{}", review.period.name(), review.start, review.account_id.unwrap_or(0)),
        title: review.period.title(review.start),
        body: body.join("\n"),
    }
}

/// The index entry of a missed trade, referenced by its row id (so it must be stored first).
pub fn missed_document(missed: &MissedTrade) -> Document {
    let seen = chrono::DateTime::from_timestamp_millis(missed.seen_at).map(|t| t.format("%d/%m").to_string());
    let mut parts: Vec<&str> = vec![missed.notes.as_str()];
    parts.extend(missed.reason.as_deref());
    parts.extend(missed.screenshots.iter().map(|s| s.caption.as_str()));
    parts.retain(|part| !part.trim().is_empty());
    Document {
        kind: DocKind::Missed,
        reference: missed.id.to_string(),
        title: format!("Bỏ lỡ {} {} {}", missed.symbol, missed.side.label(), seen.unwrap_or_default())
            .trim()
            .to_owned(),
        body: parts.join("\n"),
    }
}

/// The index entry of a setup, referenced by its row id (so it must be stored first).
pub fn playbook_document(playbook: &Playbook) -> Document {
    let mut parts: Vec<&str> = vec![playbook.description.as_str(), playbook.invalidation.as_str()];
    parts
        .extend(playbook.entry_rules.iter().chain(&playbook.exit_rules).chain(&playbook.checklist).map(String::as_str));
    parts.retain(|part| !part.trim().is_empty());
    Document {
        kind: DocKind::Playbook,
        reference: playbook.id.to_string(),
        title: playbook.name.clone(),
        body: parts.join("\n"),
    }
}

/// The FTS5 query of what the trader typed: each word must appear, as a prefix
/// (`breako` finds `breakout`); quotes and operators typed are taken literally.
pub fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(|word| {
            word.chars().filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')).collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .map(|word| format!("\"{word}\"*"))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Side;

    fn hit(kind: DocKind, reference: &str) -> Hit {
        Hit { kind, reference: reference.into(), title: String::new(), snippet: String::new() }
    }

    #[test]
    fn references_decode_to_what_they_name() {
        let key = TradeKey { account_id: 2, position_id: 77, segment: 0 };
        assert_eq!(hit(DocKind::Trade, &key.id()).target(), Some(Target::Trade(key)));
        let day = NaiveDate::from_ymd_opt(2026, 11, 20).expect("a date");
        assert_eq!(hit(DocKind::Day, "2026-11-20").target(), Some(Target::Day { day, account_id: None }));
        assert_eq!(hit(DocKind::Day, "2026-11-20:3").target(), Some(Target::Day { day, account_id: Some(3) }));
        assert_eq!(
            hit(DocKind::Review, "week:2026-11-16:0").target(),
            Some(Target::Review {
                period: Period::Week,
                start: NaiveDate::from_ymd_opt(2026, 11, 16).expect("a date"),
                account_id: None
            })
        );
        assert_eq!(hit(DocKind::Missed, "12").target(), Some(Target::Missed(12)));
        assert_eq!(hit(DocKind::Playbook, "4").target(), Some(Target::Playbook(4)));
        assert_eq!(hit(DocKind::Day, "nonsense").target(), None);
        assert_eq!(hit(DocKind::Review, "year:2026-01-01:0").target(), None);
    }

    #[test]
    fn a_snippet_is_cut_at_its_marks() {
        assert_eq!(
            marked("…[Vào sớm] trước khi nến [M15] đóng"),
            [
                ("…".to_owned(), false),
                ("Vào sớm".to_owned(), true),
                (" trước khi nến ".to_owned(), false),
                ("M15".to_owned(), true),
                (" đóng".to_owned(), false),
            ]
        );
        assert_eq!(marked("no marks"), [("no marks".to_owned(), false)]);
        assert_eq!(marked("open [only"), [("open [only".to_owned(), false)], "a mark never closed is text");
        assert_eq!(marked("").len(), 0, "no text, no parts");
    }

    #[test]
    fn written_things_become_documents_the_search_can_name() {
        let review = PeriodReview {
            id: 0,
            account_id: Some(2),
            period: Period::Week,
            start: NaiveDate::from_ymd_opt(2026, 11, 16).expect("a date"),
            answers: vec![("Lỗi lặp lại".into(), "Vào sớm".into())],
            rating: 0,
            custom: crate::catalog::CustomValues::new(),
        };
        let doc = review_document(&review);
        assert_eq!(doc.reference, "week:2026-11-16:2");
        assert_eq!(doc.title, "Tuần 47 · 16–22/11");
        assert!(doc.body.contains("Vào sớm"));
        let missed = MissedTrade {
            id: 9,
            account_id: None,
            symbol: "XAUUSD".into(),
            side: Side::Long,
            seen_at: 1_763_716_920_000,
            playbook_id: None,
            setup_grade: None,
            reason: Some("fear".into()),
            entry: None,
            stop_loss: None,
            take_profit: None,
            outcome_price: None,
            notes: "do dự".into(),
            screenshots: Vec::new(),
            custom: crate::catalog::CustomValues::new(),
        };
        let doc = missed_document(&missed);
        assert_eq!((doc.reference.as_str(), doc.title.as_str()), ("9", "Bỏ lỡ XAUUSD Buy 21/11"));
        let doc = playbook_document(&Playbook { id: 4, name: "Pullback".into(), ..Playbook::default() });
        assert_eq!((doc.reference.as_str(), doc.title.as_str()), ("4", "Pullback"));
    }

    #[test]
    fn typed_words_become_safe_prefix_terms() {
        assert_eq!(fts_query("dời SL"), Some("\"dời\"* \"SL\"*".into()));
        assert_eq!(fts_query("XAUUSD \"OR\" (break)"), Some("\"XAUUSD\"* \"OR\"* \"break\"*".into()));
        assert_eq!(fts_query("  ** "), None);
    }
}
