//! Changing a property of the trader's own across everything already written: when a
//! property gets another type, or one of its options is renamed, the values in every entry
//! of its form follow, as they do in Notion.

use rusqlite::{Connection, TransactionBehavior};

use crate::catalog::{CustomValues, FieldDef, FieldKind, FieldScope, FieldValue};
use crate::form::{self, options_from};
use crate::missed::MissedTrade;
use crate::model::{JournalEntry, TradeKey};
use crate::review::PeriodReview;

use super::{Db, Result, field_defs_on, save_field_def_on, save_journal_on, save_missed_on, save_review_on};

/// What a type change does to the values written under a property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KindImpact {
    /// Entries that hold a value for the property.
    pub written: usize,
    /// Of those, the entries whose value does not carry over to the new type and would be dropped.
    pub lost: usize,
}

/// What saving a definition did to the stored values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FieldApplied {
    /// The definition's id (new when it was added).
    pub id: i64,
    /// Entries whose value was converted to the new type.
    pub converted: usize,
    /// Entries whose value was dropped because it does not fit the new type.
    pub dropped: usize,
    /// Entries that follow a renamed option.
    pub renamed: usize,
}

/// One stored entry of a form that carries values of the trader's own.
enum Stored {
    Journal(TradeKey, Box<JournalEntry>),
    Missed(i64, Box<MissedTrade>),
    Review(i64, Box<PeriodReview>),
}

impl Stored {
    fn custom_mut(&mut self) -> &mut CustomValues {
        match self {
            Self::Journal(_, entry) => &mut entry.custom,
            Self::Missed(_, missed) => &mut missed.custom,
            Self::Review(_, review) => &mut review.custom,
        }
    }

    fn save(&self, conn: &Connection) -> Result<()> {
        match self {
            Self::Journal(key, entry) => save_journal_on(conn, key, entry),
            Self::Missed(id, missed) => save_missed_on(conn, &MissedTrade { id: *id, ..(**missed).clone() }).map(drop),
            Self::Review(id, review) => save_review_on(conn, &PeriodReview { id: *id, ..(**review).clone() }).map(drop),
        }
    }
}

/// Every stored entry of the form `scope`. The day's and the plan's forms keep no values of
/// the trader's own yet, so they have none.
fn stored_of(conn: &Connection, scope: FieldScope) -> Result<Vec<Stored>> {
    match scope {
        FieldScope::Trade => {
            let mut stmt = conn.prepare("SELECT account_id, position_id, segment, entry_json FROM journal")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    TradeKey { account_id: r.get(0)?, position_id: r.get(1)?, segment: r.get(2)? },
                    r.get::<_, String>(3)?,
                ))
            })?;
            rows.map(|row| {
                let (key, json) = row?;
                Ok(Stored::Journal(key, Box::new(serde_json::from_str(&json)?)))
            })
            .collect()
        }
        FieldScope::MissedTrade => {
            let mut stmt = conn.prepare("SELECT id, json FROM missed_trades")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
            rows.map(|row| {
                let (id, json) = row?;
                Ok(Stored::Missed(id, Box::new(serde_json::from_str(&json)?)))
            })
            .collect()
        }
        FieldScope::Review => {
            let mut stmt = conn.prepare("SELECT id, json FROM reviews")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
            rows.map(|row| {
                let (id, json) = row?;
                Ok(Stored::Review(id, Box::new(serde_json::from_str(&json)?)))
            })
            .collect()
        }
        FieldScope::EntryPlan | FieldScope::Day => Ok(Vec::new()),
    }
}

/// `value` as a scale of top `max` would hold it.
fn within_scale(value: FieldValue, max: u8) -> FieldValue {
    match value {
        FieldValue::Scale(n) => FieldValue::Scale(n.min(max)),
        other => other,
    }
}

/// The option `new` that replaced `old` in `before` → `after`, when the two lists differ by
/// exactly that one name (an option renamed in place, not added, removed or moved).
fn renamed_option<'a>(before: &'a [String], after: &'a [String]) -> Option<(&'a str, &'a str)> {
    if before.len() != after.len() {
        return None;
    }
    let gone: Vec<&String> = before.iter().filter(|o| !after.contains(o)).collect();
    let came: Vec<&String> = after.iter().filter(|o| !before.contains(o)).collect();
    match (gone.as_slice(), came.as_slice()) {
        ([old], [new]) => Some((old.as_str(), new.as_str())),
        _ => None,
    }
}

impl Db {
    /// How many entries of `scope` hold a value under `key` that would be dropped if the
    /// property became `to`, and how many hold one at all: what the trader is told before a
    /// type change.
    ///
    /// # Errors
    ///
    /// [`DbError`] when a read fails or a stored entry does not parse.
    pub fn kind_change_impact(&self, scope: FieldScope, key: &str, to: FieldKind) -> Result<KindImpact> {
        let mut impact = KindImpact::default();
        for mut stored in stored_of(&self.conn, scope)? {
            if let Some(value) = stored.custom_mut().get(key) {
                impact.written += 1;
                if form::convert(value, to).is_none() {
                    impact.lost += 1;
                }
            }
        }
        Ok(impact)
    }

    /// Saves a property of the trader's (`def.id` 0 adds it, or finds it again by its key) and
    /// brings the stored values along, in one transaction:
    ///
    /// - A new type converts the value of every entry of the form with [`form::convert`]; one
    ///   that does not carry over is dropped. A select's options grow with the values that
    ///   became choices, and a scale's value is held to its new top.
    /// - A select option renamed in place is renamed in every entry's value.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the write fails, in which case nothing is stored.
    pub fn apply_field_def(&mut self, def: &FieldDef) -> Result<FieldApplied> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let all = field_defs_on(&tx, def.scope)?;
        let before = all.iter().find(|d| (def.id != 0 && d.id == def.id) || (def.id == 0 && d.key == def.key));
        let mut next = def.clone();
        // A definition that is not stored yet is added, whatever id it came with.
        next.id = before.map_or(0, |b| b.id);
        let mut applied = FieldApplied::default();
        if let Some(before) = before {
            let converting = before.kind != next.kind;
            let rename = if converting { None } else { renamed_option(&before.options, &next.options) };
            if converting || rename.is_some() {
                let mut kept: Vec<FieldValue> = Vec::new();
                for mut stored in stored_of(&tx, next.scope)? {
                    let values = stored.custom_mut();
                    if converting {
                        let Some(value) = values.get(&next.key).cloned() else { continue };
                        if let Some(converted) = form::convert(&value, next.kind) {
                            let converted = within_scale(converted, next.scale_max());
                            kept.push(converted.clone());
                            values.insert(next.key.clone(), converted);
                            applied.converted += 1;
                        } else {
                            values.remove(&next.key);
                            applied.dropped += 1;
                        }
                    } else if let Some((old, new)) = rename {
                        let held = values.get(&next.key).cloned();
                        form::rename_option(values, &next.key, old, new);
                        if values.get(&next.key).cloned() == held {
                            continue;
                        }
                        applied.renamed += 1;
                    }
                    stored.save(&tx)?;
                }
                if matches!(next.kind, FieldKind::Select | FieldKind::MultiSelect) {
                    for option in options_from(&kept) {
                        if !next.options.contains(&option) {
                            next.options.push(option);
                        }
                    }
                }
            }
        }
        applied.id = save_field_def_on(&tx, &next)?;
        tx.commit()?;
        Ok(applied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Requirement;

    fn def(kind: FieldKind, options: &[&str]) -> FieldDef {
        FieldDef {
            id: 0,
            scope: FieldScope::Trade,
            key: "focus".into(),
            label: "Mức tập trung".into(),
            kind,
            options: options.iter().map(|o| (*o).to_owned()).collect(),
            max: Some(10),
            required: Requirement::Optional,
            analysed: false,
            position: 0,
            archived: false,
        }
    }

    fn account(db: &mut Db) -> i64 {
        db.upsert_account(&crate::AccountInfo {
            login: "1".into(),
            server: "S".into(),
            ..crate::AccountInfo::default()
        })
        .expect("account")
    }

    fn write(db: &mut Db, account_id: i64, position: i64, value: Option<FieldValue>) -> TradeKey {
        let key = TradeKey { account_id, position_id: position, segment: 0 };
        let mut entry = JournalEntry::default();
        if let Some(value) = value {
            entry.custom.insert("focus".into(), value);
        }
        db.save_journal(&key, &entry).expect("journal saved");
        key
    }

    #[test]
    fn a_new_type_converts_every_entry_and_drops_what_does_not_fit() {
        let mut db = Db::in_memory().expect("opens");
        let account = account(&mut db);
        let applied = db.apply_field_def(&def(FieldKind::Text, &[])).expect("the property is added");
        assert_ne!(applied.id, 0);
        let numeric = write(&mut db, account, 1, Some(FieldValue::Text("7".into())));
        let words = write(&mut db, account, 2, Some(FieldValue::Text("cao".into())));
        let blank = write(&mut db, account, 3, None);

        let impact = db.kind_change_impact(FieldScope::Trade, "focus", FieldKind::Number).expect("counts");
        assert_eq!(impact, KindImpact { written: 2, lost: 1 });

        let changed = FieldDef { id: applied.id, ..def(FieldKind::Number, &[]) };
        let applied = db.apply_field_def(&changed).expect("the type changes");
        assert_eq!((applied.converted, applied.dropped), (1, 1));
        let custom = |key: &TradeKey| db.journal(key).expect("reads").expect("an entry").custom;
        assert_eq!(custom(&numeric)["focus"], FieldValue::Number(7.0));
        assert!(!custom(&words).contains_key("focus"), "a value that does not carry over is dropped");
        assert!(custom(&blank).is_empty());
        assert_eq!(db.field_defs(FieldScope::Trade).expect("reads")[0].kind, FieldKind::Number);
    }

    #[test]
    fn text_becoming_a_select_makes_its_options_from_the_values() {
        let mut db = Db::in_memory().expect("opens");
        let account = account(&mut db);
        let id = db.apply_field_def(&def(FieldKind::Text, &[])).expect("added").id;
        let m5 = write(&mut db, account, 1, Some(FieldValue::Text("M5".into())));
        write(&mut db, account, 2, Some(FieldValue::Text("H1".into())));
        db.apply_field_def(&FieldDef { id, ..def(FieldKind::Select, &["M15"]) }).expect("converts");
        let stored = &db.field_defs(FieldScope::Trade).expect("reads")[0];
        assert_eq!(stored.options, ["M15", "M5", "H1"], "the trader's options first, then the values'");
        assert_eq!(db.journal(&m5).expect("reads").expect("an entry").custom["focus"], FieldValue::Choice("M5".into()));
    }

    #[test]
    fn a_renamed_option_follows_into_every_entry() {
        let mut db = Db::in_memory().expect("opens");
        let account = account(&mut db);
        let id = db.apply_field_def(&def(FieldKind::MultiSelect, &["M5", "H1"])).expect("added").id;
        let both = write(&mut db, account, 1, Some(FieldValue::Choices(vec!["M5".into(), "H1".into()])));
        let other = write(&mut db, account, 2, Some(FieldValue::Choices(vec!["H1".into()])));
        let applied =
            db.apply_field_def(&FieldDef { id, ..def(FieldKind::MultiSelect, &["5 phút", "H1"]) }).expect("renames");
        assert_eq!(applied.renamed, 1, "only the entry that held the option");
        assert_eq!(
            db.journal(&both).expect("reads").expect("an entry").custom["focus"],
            FieldValue::Choices(vec!["5 phút".into(), "H1".into()])
        );
        assert_eq!(
            db.journal(&other).expect("reads").expect("an entry").custom["focus"],
            FieldValue::Choices(vec!["H1".into()])
        );
    }

    #[test]
    fn adding_the_same_key_twice_finds_the_first() {
        let mut db = Db::in_memory().expect("opens");
        let first = db.apply_field_def(&def(FieldKind::Text, &[])).expect("added").id;
        let again = db.apply_field_def(&def(FieldKind::Text, &[])).expect("found again").id;
        assert_eq!(first, again);
        assert_eq!(db.field_defs(FieldScope::Trade).expect("reads").len(), 1);
    }

    #[test]
    fn a_scale_value_is_held_to_its_new_top() {
        let mut db = Db::in_memory().expect("opens");
        let account = account(&mut db);
        let id = db.apply_field_def(&def(FieldKind::Number, &[])).expect("added").id;
        let key = write(&mut db, account, 1, Some(FieldValue::Number(9.0)));
        db.apply_field_def(&FieldDef { id, max: Some(5), ..def(FieldKind::Scale, &[]) }).expect("converts");
        assert_eq!(db.journal(&key).expect("reads").expect("an entry").custom["focus"], FieldValue::Scale(5));
    }
}
