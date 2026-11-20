//! Full-text search over everything the trader writes: trade notes and plans, day
//! notes, reviews, missed trades and the playbook. Accents are folded, so `lenh`
//! finds `lệnh`.

use serde::{Deserialize, Serialize};

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
    use super::fts_query;

    #[test]
    fn typed_words_become_safe_prefix_terms() {
        assert_eq!(fts_query("dời SL"), Some("\"dời\"* \"SL\"*".into()));
        assert_eq!(fts_query("XAUUSD \"OR\" (break)"), Some("\"XAUUSD\"* \"OR\"* \"break\"*".into()));
        assert_eq!(fts_query("  ** "), None);
    }
}
