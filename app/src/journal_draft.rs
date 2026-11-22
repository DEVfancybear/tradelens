//! The fields of a journal entry the trade page edits, as plain data that moves to the
//! blocking thread and is laid over the entry stored there. Whatever else the entry holds (the
//! plan, the screenshots the EA attached meanwhile) is left as the database has it.

use tradelens_core::{
    Deal, DealEntry, JournalEntry,
    catalog::{CustomValues, FieldValue},
    journal::exit_reason_of_deal,
};

/// Everything the trade page's form edits in one trade's journal entry.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Draft {
    pub(crate) playbook_id: Option<i64>,
    pub(crate) setup: String,
    pub(crate) setup_grade: Option<String>,
    pub(crate) exit_reason: Option<String>,
    pub(crate) market: Option<String>,
    pub(crate) tags: Vec<String>,
    pub(crate) mistakes: Vec<String>,
    pub(crate) emotion_before: String,
    pub(crate) emotion_after: String,
    pub(crate) rating: u8,
    pub(crate) followed_plan: Option<bool>,
    pub(crate) checklist: Vec<i64>,
    pub(crate) planned_risk: Option<f64>,
    pub(crate) review_later: bool,
    pub(crate) notes: String,
    /// The trader's own properties, by key.
    pub(crate) custom: CustomValues,
}

impl Draft {
    /// The fields of `entry` the page edits.
    pub(crate) fn of(entry: &JournalEntry) -> Self {
        Self {
            playbook_id: entry.playbook_id,
            setup: entry.setup.clone(),
            setup_grade: entry.setup_grade.clone(),
            exit_reason: entry.exit_reason.clone(),
            market: entry.market.clone(),
            tags: entry.tags.clone(),
            mistakes: entry.mistakes.clone(),
            emotion_before: entry.emotion_before.clone(),
            emotion_after: entry.emotion_after.clone(),
            rating: entry.rating,
            followed_plan: entry.followed_plan,
            checklist: entry.checklist.clone(),
            planned_risk: entry.planned_risk,
            review_later: entry.review_later,
            notes: entry.notes.clone(),
            custom: entry.custom.clone(),
        }
    }

    /// [`Draft::of`], with the exit reason the exit deal suggests when the trader has not
    /// chosen one. The suggestion is part of what the page shows as saved: it is written along
    /// with the next edit, and opening a trade never creates an entry by itself.
    pub(crate) fn of_trade(entry: &JournalEntry, deals: &[Deal]) -> Self {
        let mut draft = Self::of(entry);
        if draft.exit_reason.is_none() {
            draft.exit_reason = suggested_exit(deals).map(str::to_owned);
        }
        draft
    }

    /// Lays the draft over `entry`, leaving its other fields as they are.
    pub(crate) fn apply(&self, entry: &mut JournalEntry) {
        entry.playbook_id = self.playbook_id;
        entry.setup.clone_from(&self.setup);
        entry.setup_grade.clone_from(&self.setup_grade);
        entry.exit_reason.clone_from(&self.exit_reason);
        entry.market.clone_from(&self.market);
        entry.tags.clone_from(&self.tags);
        entry.mistakes.clone_from(&self.mistakes);
        entry.emotion_before.clone_from(&self.emotion_before);
        entry.emotion_after.clone_from(&self.emotion_after);
        entry.rating = self.rating;
        entry.followed_plan = self.followed_plan;
        entry.checklist.clone_from(&self.checklist);
        entry.planned_risk = self.planned_risk;
        entry.review_later = self.review_later;
        entry.notes.clone_from(&self.notes);
        entry.custom.clone_from(&self.custom);
    }
}

/// The exit reason the last deal that closed the position suggests (`DEAL_REASON_SL` → `sl`).
pub(crate) fn suggested_exit(deals: &[Deal]) -> Option<&'static str> {
    deals
        .iter()
        .rev()
        .find(|d| !matches!(d.entry, DealEntry::In | DealEntry::None))
        .and_then(|d| d.reason.as_deref())
        .and_then(exit_reason_of_deal)
}

/// `keys` without `key`.
pub(crate) fn removed(keys: &[String], key: &str) -> Vec<String> {
    keys.iter().filter(|k| *k != key).cloned().collect()
}

/// `keys` with `key` at the end, once: tags keep the order they were switched on in.
pub(crate) fn added(keys: &[String], key: &str) -> Vec<String> {
    let mut next = removed(keys, key);
    next.push(key.to_owned());
    next
}

/// `ids` without `id`.
pub(crate) fn unticked(ids: &[i64], id: i64) -> Vec<i64> {
    ids.iter().copied().filter(|i| *i != id).collect()
}

/// `ids` with `id` at the end, once.
pub(crate) fn ticked(ids: &[i64], id: i64) -> Vec<i64> {
    let mut next = unticked(ids, id);
    next.push(id);
    next
}

/// The custom values with `key` set, or cleared when `value` is `None`.
pub(crate) fn with_value(mut values: CustomValues, key: &str, value: Option<FieldValue>) -> CustomValues {
    match value {
        Some(value) => {
            values.insert(key.to_owned(), value);
        }
        None => {
            values.remove(key);
        }
    }
    values
}

/// `""` for no choice: a drop-down's value of an optional key.
pub(crate) fn key_or_empty(key: Option<&str>) -> String {
    key.unwrap_or_default().to_owned()
}

/// The optional key a drop-down's value stands for: nothing for the empty choice.
pub(crate) fn key_of_choice(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exit_deal(reason: Option<&str>) -> Deal {
        Deal {
            ticket: 2,
            order: 2,
            position_id: 1,
            time_msc: 0,
            symbol: "EURUSD".into(),
            kind: tradelens_core::DealKind::Sell,
            entry: DealEntry::Out,
            volume: 0.1,
            price: 1.0,
            commission: 0.0,
            swap: 0.0,
            fee: 0.0,
            profit: 0.0,
            magic: 0,
            comment: String::new(),
            sl: 0.0,
            tp: 0.0,
            risk: None,
            reason: reason.map(str::to_owned),
        }
    }

    #[test]
    fn a_draft_round_trips_through_an_entry() {
        let entry = JournalEntry {
            setup: "Range".into(),
            setup_grade: Some("a".into()),
            exit_reason: Some("tp".into()),
            tags: vec!["news".into()],
            rating: 4,
            followed_plan: Some(false),
            checklist: vec![2, 1],
            planned_risk: Some(50.0),
            review_later: true,
            notes: "ghi chú".into(),
            custom: CustomValues::from([("focus".to_owned(), FieldValue::Scale(8))]),
            ..JournalEntry::default()
        };
        let draft = Draft::of(&entry);
        let mut again = JournalEntry::default();
        draft.apply(&mut again);
        assert_eq!(again, entry);
    }

    #[test]
    fn applying_keeps_what_the_page_does_not_edit() {
        let mut stored = JournalEntry {
            plan: Some(tradelens_core::journal::EntryPlan { thesis: "phá đỉnh".into(), ..Default::default() }),
            ..JournalEntry::default()
        };
        Draft { rating: 3, ..Draft::default() }.apply(&mut stored);
        assert_eq!(stored.rating, 3);
        assert_eq!(stored.plan.as_ref().map(|p| p.thesis.as_str()), Some("phá đỉnh"), "the plan survives");
    }

    #[test]
    fn the_exit_deals_reason_fills_an_exit_reason_the_trader_left_empty() {
        let deals = [exit_deal(Some("DEAL_REASON_SL"))];
        assert_eq!(Draft::of_trade(&JournalEntry::default(), &deals).exit_reason.as_deref(), Some("sl"));
        let chosen = JournalEntry { exit_reason: Some("manual".into()), ..JournalEntry::default() };
        assert_eq!(Draft::of_trade(&chosen, &deals).exit_reason.as_deref(), Some("manual"), "a choice is kept");
        assert_eq!(Draft::of_trade(&JournalEntry::default(), &[exit_deal(None)]).exit_reason, None);
        assert_eq!(suggested_exit(&[exit_deal(Some("rollover"))]), None, "an unknown reason suggests nothing");
    }

    #[test]
    fn keys_are_added_once_and_removed() {
        let on = added(&[], "a");
        assert_eq!(on, ["a"]);
        assert_eq!(added(&on, "a"), ["a"]);
        assert_eq!(removed(&added(&on, "b"), "a"), ["b"]);
    }

    #[test]
    fn checklist_ticks_and_custom_values_edit_in_place() {
        assert_eq!(ticked(&[1, 2], 3), [1, 2, 3]);
        assert_eq!(unticked(&[1, 2, 3], 2), [1, 3]);
        assert_eq!(ticked(&[1], 1), [1], "ticking twice keeps one");
        let set = with_value(CustomValues::new(), "k", Some(FieldValue::Flag(true)));
        assert_eq!(set["k"], FieldValue::Flag(true));
        assert!(with_value(set, "k", None).is_empty());
        assert_eq!(key_of_choice(""), None);
        assert_eq!(key_of_choice("a"), Some("a".to_owned()));
        assert_eq!(key_or_empty(None), "");
    }
}
