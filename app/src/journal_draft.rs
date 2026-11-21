//! The fields of a journal entry the trade page edits in v0.1, as plain data that moves to
//! the blocking thread and is laid over the entry stored there. Whatever else the entry
//! holds (the plan, the screenshots the EA attached meanwhile, the trader's own fields) is
//! left as the database has it.

use tradelens_core::JournalEntry;

/// Setup, tags, mistakes, emotions, grade and notes of one trade.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Draft {
    pub(crate) playbook_id: Option<i64>,
    pub(crate) setup: String,
    pub(crate) tags: Vec<String>,
    pub(crate) mistakes: Vec<String>,
    pub(crate) emotion_before: String,
    pub(crate) emotion_after: String,
    pub(crate) rating: u8,
    pub(crate) notes: String,
}

impl Draft {
    /// The fields of `entry` the page edits.
    pub(crate) fn of(entry: &JournalEntry) -> Self {
        Self {
            playbook_id: entry.playbook_id,
            setup: entry.setup.clone(),
            tags: entry.tags.clone(),
            mistakes: entry.mistakes.clone(),
            emotion_before: entry.emotion_before.clone(),
            emotion_after: entry.emotion_after.clone(),
            rating: entry.rating,
            notes: entry.notes.clone(),
        }
    }

    /// Lays the draft over `entry`, leaving its other fields as they are.
    pub(crate) fn apply(&self, entry: &mut JournalEntry) {
        entry.playbook_id = self.playbook_id;
        entry.setup.clone_from(&self.setup);
        entry.tags.clone_from(&self.tags);
        entry.mistakes.clone_from(&self.mistakes);
        entry.emotion_before.clone_from(&self.emotion_before);
        entry.emotion_after.clone_from(&self.emotion_after);
        entry.rating = self.rating;
        entry.notes.clone_from(&self.notes);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draft_round_trips_through_an_entry() {
        let entry = JournalEntry {
            setup: "Range".into(),
            tags: vec!["news".into()],
            rating: 4,
            notes: "ghi chú".into(),
            ..JournalEntry::default()
        };
        let draft = Draft::of(&entry);
        let mut again = JournalEntry::default();
        draft.apply(&mut again);
        assert_eq!(again, entry);
    }

    #[test]
    fn applying_keeps_what_the_page_does_not_edit() {
        let mut stored = JournalEntry { review_later: true, followed_plan: Some(true), ..JournalEntry::default() };
        Draft { rating: 3, ..Draft::default() }.apply(&mut stored);
        assert_eq!(stored.rating, 3);
        assert!(stored.review_later, "the queue flag survives");
        assert_eq!(stored.followed_plan, Some(true));
    }

    #[test]
    fn keys_are_added_once_and_removed() {
        let on = added(&[], "a");
        assert_eq!(on, ["a"]);
        assert_eq!(added(&on, "a"), ["a"]);
        assert_eq!(removed(&added(&on, "b"), "a"), ["b"]);
    }
}
