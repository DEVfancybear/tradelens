//! Storing a Notion import: trades, journals, the properties of the trade form and the
//! option lists they use, all in one transaction.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use crate::catalog::{CatalogItem, CatalogKind, FieldDef, FieldKind, FieldScope, FieldValue, Requirement};
use crate::form;
use crate::import::notion::{
    NotionImportReport, NotionPlan, PlannedJournal, PlannedProperty, PlannedRow, PropertyCreated, RowIssue, fold_name,
    key_of,
};
use crate::model::{Deal, JournalEntry, TradeKey};
use crate::playbook::Playbook;

use super::{
    DEAL_COLUMNS, Db, Result, catalog_on, count, deal_from_row, field_defs_on, form_layout_on, journal_on, now_ms,
    playbooks_on, save_catalog_item_on, save_field_def_on, save_form_layout_on, save_journal_on, upsert_account_on,
    write_deal,
};

/// The `source` of the deals a Notion import wrote.
const NOTION_SOURCE: &str = "notion";

/// How far apart two opens may be for a Notion trade and an MT5 trade to be the same trade.
const SAME_TRADE_WINDOW_MS: i64 = 2 * 60 * 1000;

impl Db {
    /// Stores what [`crate::import::notion::infer`] understood of a Notion file, in one
    /// transaction.
    ///
    /// - Each row becomes a manual-style trade (entry and exit deal, negative tickets) with its
    ///   journal. A row found again keeps its tickets, so importing the same file twice stores
    ///   nothing new. A row that is the same trade as one of the account's MT5 trades (same
    ///   symbol and side, opened within two minutes) is not stored as a trade: its journal is
    ///   written on the MT5 trade.
    /// - A journal field the trader already filled is left as it is; only empty ones are filled.
    /// - Each other column becomes a property of the trade form (appended to its layout), unless
    ///   a property with the same name, ignoring case and diacritics, exists: that one takes the
    ///   values, and its options grow with the file's.
    /// - Tags, mistakes, emotions, setup grades, exit reasons and market states are matched with
    ///   the option lists by label or key, and added to them when new.
    ///
    /// # Errors
    ///
    /// [`crate::DbError::Sqlite`] when the database fails, in which case nothing is stored. Rows
    /// that cannot be stored (a ticket held by another kind of deal) are skipped and reported.
    pub fn import_notion(&mut self, plan: &NotionPlan) -> Result<NotionImportReport> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let account_id = upsert_account_on(&tx, &plan.account)?;
        let mut report = NotionImportReport {
            account_id,
            skipped: plan.issues.clone(),
            warnings: plan.warnings.clone(),
            ..NotionImportReport::default()
        };
        let mut form = TradeForm::load(&tx)?;
        let resolved = form.resolve(&tx, &plan.properties, &mut report)?;
        let mut options = OptionLists::default();
        let playbooks = playbooks_on(&tx)?;
        for row in &plan.rows {
            match store_row(&tx, account_id, row, &resolved, &mut form, &mut options, &playbooks)? {
                Stored::Skipped(reason) => report.skipped.push(RowIssue { line: row.line, reason }),
                Stored::Done(outcome) => outcome.add_to(&mut report),
            }
        }
        form.save(&tx)?;
        report.options_added = options.added;
        report.skipped.sort_by_key(|issue| issue.line);
        if report.created + report.updated > 0 {
            let message = format!("{} lệnh mới, {} lệnh cập nhật từ Notion", report.created, report.updated);
            tx.execute(
                "INSERT INTO import_log (at, source, account_id, inserted, updated, message) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    now_ms(),
                    NOTION_SOURCE,
                    account_id,
                    count(report.created),
                    count(report.updated),
                    message
                ],
            )?;
        }
        tx.commit()?;
        Ok(report)
    }
}

/// The trade form's properties while an import runs.
struct TradeForm {
    defs: Vec<FieldDef>,
    /// Indexes of `defs` whose options or archived state changed.
    dirty: Vec<usize>,
    layout: crate::form::FormLayout,
    layout_changed: bool,
}

/// Where a planned property's values go.
struct Target {
    /// The planned property's own kind.
    planned: FieldKind,
    /// Index into [`TradeForm::defs`].
    def: usize,
}

impl TradeForm {
    fn load(conn: &Connection) -> Result<Self> {
        Ok(Self {
            defs: field_defs_on(conn, FieldScope::Trade)?,
            dirty: Vec::new(),
            layout: form_layout_on(conn, FieldScope::Trade)?,
            layout_changed: false,
        })
    }

    /// Finds or creates the property of each planned one, by planned key.
    fn resolve(
        &mut self,
        conn: &Connection,
        planned: &[PlannedProperty],
        report: &mut NotionImportReport,
    ) -> Result<HashMap<String, Target>> {
        let mut targets = HashMap::new();
        for property in planned {
            let name = fold_name(&property.label);
            let found = self
                .defs
                .iter()
                .position(|d| !d.archived && fold_name(&d.label) == name)
                .or_else(|| self.defs.iter().position(|d| fold_name(&d.label) == name));
            let index = if let Some(index) = found {
                if self.defs[index].archived {
                    self.defs[index].archived = false;
                    self.mark_dirty(index);
                }
                report.properties_reused.push(self.defs[index].label.clone());
                index
            } else {
                let mut key = key_of(&property.label);
                let base = key.clone();
                for n in 2.. {
                    if self.defs.iter().all(|d| d.key != key) {
                        break;
                    }
                    key = format!("{base}_{n}");
                }
                let mut def = FieldDef {
                    id: 0,
                    scope: FieldScope::Trade,
                    key,
                    label: property.label.clone(),
                    kind: property.kind,
                    options: property.options.clone(),
                    max: None,
                    required: Requirement::Optional,
                    analysed: false,
                    position: self.defs.iter().map(|d| d.position).max().map_or(0, |p| p + 1),
                    archived: false,
                };
                def.id = save_field_def_on(conn, &def)?;
                self.layout.add_custom(&def.key);
                self.layout_changed = true;
                report.properties_created.push(PropertyCreated { label: def.label.clone(), kind: def.kind });
                self.defs.push(def);
                self.defs.len() - 1
            };
            targets.insert(property.key.clone(), Target { planned: property.kind, def: index });
        }
        Ok(targets)
    }

    fn mark_dirty(&mut self, index: usize) {
        if !self.dirty.contains(&index) {
            self.dirty.push(index);
        }
    }

    /// Puts a planned value on the property it belongs to, converted to the property's kind
    /// and with the options it needs; `None` when it does not fit.
    fn value_for(&mut self, target: &Target, value: &FieldValue) -> Option<(String, FieldValue)> {
        let kind = self.defs[target.def].kind;
        let converted = if kind == target.planned { value.clone() } else { form::convert(value, kind)? };
        let wanted: Vec<&String> = match &converted {
            FieldValue::Choice(c) => vec![c],
            FieldValue::Choices(cs) => cs.iter().collect(),
            _ => Vec::new(),
        };
        if matches!(kind, FieldKind::Select | FieldKind::MultiSelect) {
            let missing: Vec<String> =
                wanted.into_iter().filter(|w| !self.defs[target.def].options.contains(w)).cloned().collect();
            if !missing.is_empty() {
                self.defs[target.def].options.extend(missing);
                self.mark_dirty(target.def);
            }
        }
        let def = &self.defs[target.def];
        def.validate(&converted).ok().map(|()| (def.key.clone(), converted))
    }

    fn save(&mut self, conn: &Connection) -> Result<()> {
        for &index in &self.dirty {
            save_field_def_on(conn, &self.defs[index])?;
        }
        if self.layout_changed {
            save_form_layout_on(conn, &self.layout)?;
        }
        Ok(())
    }
}

/// The option lists (tags, mistakes, …) while an import runs.
#[derive(Default)]
struct OptionLists {
    items: HashMap<CatalogKind, Vec<CatalogItem>>,
    added: usize,
}

impl OptionLists {
    /// The key of the option named `label` (matched by key or label, ignoring case, then
    /// diacritics), added when the list has none; `None` for an empty label.
    fn key_for(&mut self, conn: &Connection, kind: CatalogKind, label: &str) -> Result<Option<String>> {
        let label = label.trim();
        if label.is_empty() {
            return Ok(None);
        }
        let items = match self.items.entry(kind) {
            Entry::Occupied(known) => known.into_mut(),
            Entry::Vacant(unread) => unread.insert(catalog_on(conn, kind)?),
        };
        let lower = label.to_lowercase();
        let folded = fold_name(label);
        let found = items
            .iter()
            .find(|i| i.label.to_lowercase() == lower || i.key.to_lowercase() == lower)
            .or_else(|| items.iter().find(|i| fold_name(&i.label) == folded || fold_name(&i.key) == folded));
        if let Some(item) = found {
            return Ok(Some(item.key.clone()));
        }
        let base = key_of(label);
        let mut key = base.clone();
        for n in 2.. {
            if items.iter().all(|i| i.key != key) {
                break;
            }
            key = format!("{base}_{n}");
        }
        let mut item = CatalogItem {
            id: 0,
            kind,
            key: key.clone(),
            label: label.to_owned(),
            color: None,
            position: items.iter().map(|i| i.position).max().map_or(0, |p| p + 1),
            archived: false,
        };
        item.id = save_catalog_item_on(conn, &item)?;
        items.push(item);
        self.added += 1;
        Ok(Some(key))
    }

    fn keys_for(&mut self, conn: &Connection, kind: CatalogKind, labels: &[String]) -> Result<Vec<String>> {
        let mut keys: Vec<String> = Vec::new();
        for label in labels {
            if let Some(key) = self.key_for(conn, kind, label)?
                && !keys.contains(&key)
            {
                keys.push(key);
            }
        }
        Ok(keys)
    }
}

/// What storing one row did.
enum Stored {
    Done(Outcome),
    Skipped(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Created,
    Updated,
    Unchanged,
    MatchedUpdated,
    MatchedUnchanged,
}

impl Outcome {
    fn add_to(self, report: &mut NotionImportReport) {
        match self {
            Self::Created => report.created += 1,
            Self::Updated => report.updated += 1,
            Self::Unchanged => report.unchanged += 1,
            Self::MatchedUpdated => {
                report.matched_mt5 += 1;
                report.updated += 1;
            }
            Self::MatchedUnchanged => {
                report.matched_mt5 += 1;
                report.unchanged += 1;
            }
        }
    }
}

fn store_row(
    conn: &Connection,
    account_id: i64,
    row: &PlannedRow,
    targets: &HashMap<String, Target>,
    form: &mut TradeForm,
    options: &mut OptionLists,
    playbooks: &[Playbook],
) -> Result<Stored> {
    let sources: Vec<String> = {
        let mut stmt = conn.prepare_cached("SELECT source FROM deals WHERE account_id = ?1 AND ticket IN (?2, ?3)")?;
        let rows = stmt.query_map(rusqlite::params![account_id, row.entry.ticket, row.exit.ticket], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    if sources.iter().any(|source| source != NOTION_SOURCE) {
        return Ok(Stored::Skipped("trùng mã lệnh nội bộ với một lệnh khác trong tài khoản".to_owned()));
    }
    let matched = if sources.is_empty() { find_mt5_trade(conn, account_id, row)? } else { None };
    let key = TradeKey { account_id, position_id: matched.unwrap_or(row.position_id), segment: 0 };

    let mut created = false;
    let mut deals_changed = false;
    if matched.is_none() {
        for deal in [&row.entry, &row.exit] {
            match stored_deal(conn, account_id, deal.ticket)? {
                None => {
                    write_deal(conn, account_id, deal, NOTION_SOURCE)?;
                    created = true;
                }
                Some(stored) if stored != *deal => {
                    write_deal(conn, account_id, deal, NOTION_SOURCE)?;
                    deals_changed = true;
                }
                Some(_) => {}
            }
        }
    }

    let before = journal_on(conn, &key)?;
    let mut entry = before.clone().unwrap_or_default();
    let planned = journal_values(conn, &row.journal, targets, form, options, playbooks)?;
    fill_empty(&mut entry, planned);
    let journal_changed = if before.is_none() { !entry.is_empty() } else { before.as_ref() != Some(&entry) };
    if journal_changed {
        save_journal_on(conn, &key, &entry)?;
    }
    let changed = deals_changed || journal_changed;
    Ok(Stored::Done(match (matched.is_some(), created, changed) {
        (true, _, true) => Outcome::MatchedUpdated,
        (true, _, false) => Outcome::MatchedUnchanged,
        (false, true, _) => Outcome::Created,
        (false, false, true) => Outcome::Updated,
        (false, false, false) => Outcome::Unchanged,
    }))
}

fn stored_deal(conn: &Connection, account_id: i64, ticket: i64) -> Result<Option<Deal>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {DEAL_COLUMNS} FROM deals WHERE account_id = ?1 AND ticket = ?2"))?
        .query_row(rusqlite::params![account_id, ticket], deal_from_row)
        .optional()?)
}

/// The position of an MT5 trade of the account that `row` describes: same symbol and side,
/// opened within [`SAME_TRADE_WINDOW_MS`], the closest first.
fn find_mt5_trade(conn: &Connection, account_id: i64, row: &PlannedRow) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT position_id FROM deals
             WHERE account_id = ?1 AND ticket > 0 AND entry IN ('in', 'inout') AND kind = ?2
               AND upper(symbol) = upper(?3) AND abs(time_msc - ?4) < ?5
             ORDER BY abs(time_msc - ?4), ticket LIMIT 1",
            rusqlite::params![
                account_id,
                row.entry.kind.as_str(),
                row.entry.symbol,
                row.entry.time_msc,
                SAME_TRADE_WINDOW_MS
            ],
            |r| r.get(0),
        )
        .optional()?)
}

/// The journal a row describes, with names turned into the option lists' keys.
fn journal_values(
    conn: &Connection,
    planned: &PlannedJournal,
    targets: &HashMap<String, Target>,
    form: &mut TradeForm,
    options: &mut OptionLists,
    playbooks: &[Playbook],
) -> Result<JournalEntry> {
    let setup_name = fold_name(&planned.setup);
    let playbook_id = (!setup_name.is_empty())
        .then(|| playbooks.iter().find(|p| fold_name(&p.name) == setup_name).map(|p| p.id))
        .flatten();
    let mut notes = planned.notes.clone();
    if !planned.screenshots.is_empty() {
        if !notes.is_empty() {
            notes.push_str("\n\n");
        }
        notes.push_str("Ảnh: ");
        notes.push_str(&planned.screenshots.join("\n"));
    }
    let mut custom = crate::catalog::CustomValues::new();
    for (planned_key, value) in &planned.custom {
        if let Some(target) = targets.get(planned_key)
            && let Some((key, value)) = form.value_for(target, value)
        {
            custom.insert(key, value);
        }
    }
    Ok(JournalEntry {
        playbook_id,
        setup: planned.setup.clone(),
        tags: options.keys_for(conn, CatalogKind::Tag, &planned.tags)?,
        mistakes: options.keys_for(conn, CatalogKind::Mistake, &planned.mistakes)?,
        emotion_before: options.key_for(conn, CatalogKind::Emotion, &planned.emotion_before)?.unwrap_or_default(),
        emotion_after: options.key_for(conn, CatalogKind::Emotion, &planned.emotion_after)?.unwrap_or_default(),
        rating: planned.rating,
        setup_grade: options.key_for(conn, CatalogKind::SetupGrade, &planned.setup_grade)?,
        exit_reason: options.key_for(conn, CatalogKind::ExitReason, &planned.exit_reason)?,
        market: options.key_for(conn, CatalogKind::MarketCondition, &planned.market)?,
        review_later: planned.review_later == Some(true),
        custom,
        followed_plan: planned.followed_plan,
        notes,
        ..JournalEntry::default()
    })
}

/// Copies into `entry` what the trader has not written yet.
fn fill_empty(entry: &mut JournalEntry, planned: JournalEntry) {
    if entry.playbook_id.is_none() {
        entry.playbook_id = planned.playbook_id;
    }
    if entry.setup.is_empty() {
        entry.setup = planned.setup;
    }
    if entry.tags.is_empty() {
        entry.tags = planned.tags;
    }
    if entry.mistakes.is_empty() {
        entry.mistakes = planned.mistakes;
    }
    if entry.emotion_before.is_empty() {
        entry.emotion_before = planned.emotion_before;
    }
    if entry.emotion_after.is_empty() {
        entry.emotion_after = planned.emotion_after;
    }
    if entry.rating == 0 {
        entry.rating = planned.rating;
    }
    entry.setup_grade = entry.setup_grade.take().or(planned.setup_grade);
    entry.exit_reason = entry.exit_reason.take().or(planned.exit_reason);
    entry.market = entry.market.take().or(planned.market);
    entry.followed_plan = entry.followed_plan.or(planned.followed_plan);
    entry.review_later |= planned.review_later;
    if entry.notes.is_empty() {
        entry.notes = planned.notes;
    }
    for (key, value) in planned.custom {
        entry.custom.entry(key).or_insert(value);
    }
}
