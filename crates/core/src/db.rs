//! SQLite storage. One file per user; deals are the source of truth and trades are
//! rebuilt from them. WAL mode lets a background writer and the UI write at once.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior};

use crate::catalog::{CatalogItem, CatalogKind, CustomValues, FieldDef, FieldScope, FieldValue};
use crate::form::FormLayout;
use crate::missed::MissedTrade;
use crate::model::{
    Account, AccountInfo, ChecklistItem, DayNote, Deal, DealBatch, DealEntry, DealKind, Gaps, JournalEntry,
    PositionLevels, Rules, TradeKey,
};
use crate::playbook::Playbook;
use crate::review::{Period, PeriodReview, ReviewTemplate};
use crate::search::{self, DocKind, Document, Hit};
use crate::trades::build_trades;

mod fields;
pub use fields::{FieldApplied, KindImpact};
mod manual;
pub use manual::Deleted;
mod news;
pub use news::NewsFetch;
mod notion;
mod sync;
pub use sync::{ApplyReport, PortableState, SyncState};
mod views;

use crate::sync::{entity, join_key};

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("{}", crate::tr!("db.not_implemented", what = .0))]
    NotImplemented(&'static str),
}

pub type Result<T> = std::result::Result<T, DbError>;

/// The schema, in order. A migration is never edited once released.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_cloud_sync.sql"),
    include_str!("../migrations/0003_deal_reason.sql"),
    include_str!("../migrations/0004_catalogs_fields.sql"),
    include_str!("../migrations/0005_playbooks.sql"),
    include_str!("../migrations/0006_missed_trades.sql"),
    include_str!("../migrations/0007_reviews.sql"),
    include_str!("../migrations/0008_search.sql"),
    include_str!("../migrations/0009_day_note_custom.sql"),
    include_str!("../migrations/0010_sync_uids.sql"),
    include_str!("../migrations/0011_result_r.sql"),
    include_str!("../migrations/0012_deal_gaps.sql"),
    include_str!("../migrations/0013_removed.sql"),
    include_str!("../migrations/0014_sync_uploaded.sql"),
    include_str!("../migrations/0015_news.sql"),
];

/// One line of the import log: what an import or a sync stored, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportLogEntry {
    /// Unix milliseconds (UTC).
    pub at: i64,
    /// Where the data came from: `ea`, `python`, `html`, `xlsx`, `notion`, …
    pub source: String,
    pub account_id: Option<i64>,
    pub inserted: usize,
    pub updated: usize,
    pub message: String,
}

/// What an upsert of a batch did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UpsertReport {
    pub account_id: i64,
    pub inserted: usize,
    pub updated: usize,
}

#[derive(Debug)]
pub struct Db {
    conn: Connection,
    path: Option<PathBuf>,
}

impl Db {
    /// Opens (or creates) the database at `path` and brings its schema up to date.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| DbError::Invalid(e.to_string()))?;
        }
        let conn = Connection::open(&path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let mut db = Self { conn, path: Some(path) };
        db.prepare()?;
        Ok(db)
    }

    /// A database in memory, for tests.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails.
    pub fn in_memory() -> Result<Self> {
        let mut db = Self { conn: Connection::open_in_memory()?, path: None };
        db.prepare()?;
        Ok(db)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails.
    fn prepare(&mut self) -> Result<()> {
        self.conn.pragma_update(None, "foreign_keys", "ON")?;
        self.conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let version = self.schema_version()?;
        for (index, sql) in MIGRATIONS.iter().enumerate().skip(version) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            let next = i64::try_from(index + 1).expect("a few migrations fit in i64");
            tx.pragma_update(None, "user_version", next)?;
            tx.commit()?;
            log::info!("database migrated to v{}", index + 1);
        }
        self.seed_catalogs()?;
        Ok(())
    }

    /// Gives each empty catalog its default options, once.
    fn seed_catalogs(&mut self) -> Result<()> {
        let tx = self.conn.transaction()?;
        for kind in CatalogKind::ALL {
            let name = kind_name(kind)?;
            let count: i64 =
                tx.query_row("SELECT COUNT(*) FROM catalog_items WHERE kind = ?1", [&name], |r| r.get(0))?;
            if count > 0 {
                continue;
            }
            for (position, (key, label)) in (0_i32..).zip(kind.defaults()) {
                tx.execute(
                    "INSERT INTO catalog_items (kind, key, label, position) VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![name, key, label, position],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails.
    pub fn schema_version(&self) -> Result<usize> {
        let version: i64 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        usize::try_from(version).map_err(|_| DbError::Invalid(format!("schema version {version}")))
    }

    // --- accounts ------------------------------------------------------------

    /// The account of `info` (by login and server), created when new; its balance,
    /// equity and server offset are refreshed.
    ///
    /// Only what `info` actually carries is written: a report without a balance leaves
    /// the last known one alone, and a name the trader chose is never replaced.
    ///
    /// # Errors
    ///
    /// [`DbError::Invalid`] when `info` has no login; [`DbError::Sqlite`] when the database fails.
    pub fn upsert_account(&mut self, info: &AccountInfo) -> Result<i64> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = upsert_account_on(&tx, info)?;
        tx.commit()?;
        Ok(id)
    }

    /// Every account, in the order they were first seen.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the query fails.
    pub fn accounts(&self) -> Result<Vec<Account>> {
        let mut stmt = self.conn.prepare(&format!("SELECT {ACCOUNT_COLUMNS} FROM accounts ORDER BY id"))?;
        let rows = stmt.query_map([], account_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Saves what the trader edits of an account: broker, name, currency, starting
    /// balance and server offset. Login and server identify the account, and balance,
    /// equity and the last sync belong to the sync, so none of those change here.
    ///
    /// # Errors
    ///
    /// [`DbError::Invalid`] when the account does not exist; [`DbError::Sqlite`] when the write fails.
    pub fn update_account(&mut self, account: &Account) -> Result<()> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let key = match views::account_key(&tx, account.id) {
            Ok(key) => key,
            Err(DbError::Sqlite(rusqlite::Error::QueryReturnedNoRows)) => {
                return Err(DbError::Invalid(crate::tr!("core.db.no_account", id = account.id)));
            }
            Err(other) => return Err(other),
        };
        sync::logged(&tx, entity::ACCOUNT, Some(&key), || {
            tx.execute(
                "UPDATE accounts SET broker = ?2, name = ?3, currency = ?4, initial_balance = ?5,
                                     server_offset_min = ?6
                 WHERE id = ?1",
                rusqlite::params![
                    account.id,
                    account.broker,
                    account.name,
                    account.currency,
                    account.initial_balance,
                    account.server_offset_min
                ],
            )?;
            Ok((key.clone(), ()))
        })?;
        tx.commit()?;
        Ok(())
    }

    // --- deals -----------------------------------------------------------------

    /// Writes a batch in one transaction; deals are keyed by (account, ticket).
    ///
    /// The batch's account is upserted first. A deal already stored counts as
    /// `updated` only when something changed, so sending the same batch twice reports
    /// nothing the second time. What the EA measured and a report lacks (risk, reason,
    /// stop, target) is kept when the new deal has none; a position's stop and target from
    /// the batch's `positions` fill the deals that have none. Anything written is noted
    /// in `import_log`.
    ///
    /// # Errors
    ///
    /// [`DbError::Invalid`] when the batch has no account login; [`DbError::Sqlite`] when the
    /// database fails, in which case nothing of the batch is stored.
    pub fn upsert_batch(&mut self, batch: &DealBatch, source: &str) -> Result<UpsertReport> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let account_id = upsert_account_on(&tx, &batch.account)?;
        let levels: HashMap<i64, &PositionLevels> = batch.positions.iter().map(|p| (p.position_id, p)).collect();
        let mut report = UpsertReport { account_id, ..UpsertReport::default() };
        for incoming in &batch.deals {
            let mut deal = incoming.clone();
            if let Some(level) = levels.get(&deal.position_id).filter(|_| deal.position_id != 0) {
                if deal.sl <= 0.0 {
                    deal.sl = level.sl;
                }
                if deal.tp <= 0.0 {
                    deal.tp = level.tp;
                }
            }
            let stored = tx
                .prepare_cached(&format!("SELECT {DEAL_COLUMNS} FROM deals WHERE account_id = ?1 AND ticket = ?2"))?
                .query_row(rusqlite::params![account_id, deal.ticket], deal_from_row)
                .optional()?;
            match stored {
                None => {
                    write_deal(&tx, account_id, &deal, source)?;
                    report.inserted += 1;
                }
                Some(stored) => {
                    keep_measured(&mut deal, &stored);
                    if deal != stored {
                        write_deal(&tx, account_id, &deal, source)?;
                        report.updated += 1;
                    }
                }
            }
        }
        if report.inserted + report.updated > 0 {
            let message = format!("{} deal mới, {} deal cập nhật", report.inserted, report.updated);
            tx.execute(
                "INSERT INTO import_log (at, source, account_id, inserted, updated, message) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![now_ms(), source, account_id, count(report.inserted), count(report.updated), message],
            )?;
        }
        tx.commit()?;
        Ok(report)
    }

    /// The latest `limit` lines of the import log, newest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the query fails.
    pub fn import_log(&self, limit: u32) -> Result<Vec<ImportLogEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT at, source, account_id, inserted, updated, message FROM import_log ORDER BY at DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |r| {
            let (inserted, updated): (i64, i64) = (r.get(3)?, r.get(4)?);
            Ok(ImportLogEntry {
                at: r.get(0)?,
                source: r.get(1)?,
                account_id: r.get(2)?,
                inserted: usize::try_from(inserted).unwrap_or(0),
                updated: usize::try_from(updated).unwrap_or(0),
                message: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The account's deals, or every account's when `None`, as (account id, deal),
    /// oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the query fails.
    pub fn deals(&self, account_id: Option<i64>) -> Result<Vec<(i64, Deal)>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT account_id, {DEAL_COLUMNS} FROM deals WHERE (?1 IS NULL OR account_id = ?1)
             ORDER BY account_id, time_msc, ticket"
        ))?;
        let rows = stmt.query_map([account_id], |r| Ok((r.get(0)?, deal_from_row_at(r, 1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // --- journal, rules, checklist, day notes -------------------------------

    /// The journal entry of a trade, if the trader wrote one.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the query fails or the stored entry does not parse.
    pub fn journal(&self, key: &TradeKey) -> Result<Option<JournalEntry>> {
        journal_on(&self.conn, key)
    }

    /// Every journal entry of the account, or of all accounts when `None`.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the query fails or a stored entry does not parse.
    pub fn journals(&self, account_id: Option<i64>) -> Result<Vec<(TradeKey, JournalEntry)>> {
        let mut stmt = self.conn.prepare(
            "SELECT account_id, position_id, segment, entry_json FROM journal
             WHERE (?1 IS NULL OR account_id = ?1) ORDER BY account_id, position_id, segment",
        )?;
        let rows = stmt.query_map([account_id], |r| {
            let key = TradeKey { account_id: r.get(0)?, position_id: r.get(1)?, segment: r.get(2)? };
            Ok((key, r.get::<_, String>(3)?))
        })?;
        rows.map(|row| {
            let (key, json) = row?;
            Ok((key, serde_json::from_str(&json)?))
        })
        .collect()
    }

    /// Writes the entry of a trade, appends it to the cloud changelog and refreshes its
    /// full-text index, all in one transaction. Saving what is already stored does nothing.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the write fails, for instance because the account does not exist.
    pub fn save_journal(&mut self, key: &TradeKey, entry: &JournalEntry) -> Result<()> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        save_journal_on(&tx, key, entry)?;
        tx.commit()?;
        Ok(())
    }

    /// The account's discipline rules; all off when none were saved.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the query fails.
    pub fn rules(&self, account_id: i64) -> Result<Rules> {
        rules_on(&self.conn, account_id)
    }

    /// Replaces the account's rules.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the write fails, for instance because the account does not exist.
    pub fn save_rules(&mut self, account_id: i64, rules: &Rules) -> Result<()> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let key = views::account_key(&tx, account_id)?;
        sync::logged(&tx, entity::RULES, Some(&key), || {
            save_rules_raw(&tx, account_id, rules)?;
            Ok((key.clone(), ()))
        })?;
        tx.commit()?;
        Ok(())
    }

    /// The checklist the trader runs through before entering: the items for every
    /// account, then the account's own when `account_id` is given.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the query fails.
    pub fn checklist(&self, account_id: Option<i64>) -> Result<Vec<ChecklistItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, text FROM checklist WHERE account_id IS NULL OR account_id = ?1
             ORDER BY account_id IS NOT NULL, position, uid",
        )?;
        let rows = stmt.query_map([account_id], |r| Ok(ChecklistItem { id: r.get(0)?, text: r.get(1)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Replaces one checklist: the items for every account when `account_id` is `None`,
    /// else that account's own. An item with an id keeps it (journal entries refer to
    /// the ids they confirmed); an item with id 0 is new; stored items missing from
    /// `items` are removed. Items keep the order given.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the write fails, for instance because the account does not exist.
    pub fn save_checklist(&mut self, account_id: Option<i64>, items: &[ChecklistItem]) -> Result<()> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut kept = Vec::with_capacity(items.len());
        for (position, item) in (0_i64..).zip(items) {
            let known: Option<String> = tx
                .query_row(
                    "SELECT uid FROM checklist WHERE id = ?1 AND account_id IS ?2",
                    rusqlite::params![item.id, account_id],
                    |r| r.get(0),
                )
                .optional()?;
            let id = sync::logged(&tx, entity::CHECKLIST, known.as_deref(), || {
                let id = if known.is_some() {
                    tx.execute(
                        "UPDATE checklist SET text = ?2, position = ?3 WHERE id = ?1",
                        rusqlite::params![item.id, item.text, position],
                    )?;
                    item.id
                } else {
                    tx.execute(
                        "INSERT INTO checklist (account_id, text, position) VALUES (?1, ?2, ?3)",
                        rusqlite::params![account_id, item.text, position],
                    )?;
                    tx.last_insert_rowid()
                };
                let uid: String = tx.query_row("SELECT uid FROM checklist WHERE id = ?1", [id], |r| r.get(0))?;
                Ok((uid, id))
            })?;
            kept.push(id);
        }
        let stale: Vec<(i64, String)> = {
            let mut stmt = tx.prepare("SELECT id, uid FROM checklist WHERE account_id IS ?1")?;
            stmt.query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?
        };
        for (id, uid) in stale.into_iter().filter(|(id, _)| !kept.contains(id)) {
            sync::logged_delete(&tx, entity::CHECKLIST, &uid, || {
                tx.execute("DELETE FROM checklist WHERE id = ?1", [id])?;
                Ok(())
            })?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The note of a day (local date); `account_id` 0 is the note of all accounts.
    /// A day without a note reads as an empty one.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the query fails.
    pub fn day_note(&self, account_id: i64, day: chrono::NaiveDate) -> Result<DayNote> {
        let note = self
            .conn
            .query_row(
                "SELECT plan, review, mood FROM day_notes WHERE account_id = ?1 AND day = ?2",
                rusqlite::params![account_id, day.to_string()],
                |r| Ok(DayNote { plan: r.get(0)?, review: r.get(1)?, mood: r.get(2)? }),
            )
            .optional()?;
        Ok(note.unwrap_or_default())
    }

    /// The days that have a note (a plan, a review, a mood, or values of the trader's own day
    /// properties) for `account_id` (0: the note of all accounts), oldest first: what a date picker
    /// marks.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the query fails.
    pub fn days_with_notes(&self, account_id: i64) -> Result<Vec<chrono::NaiveDate>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT day FROM day_notes
             WHERE account_id = ?1 AND (plan <> '' OR review <> '' OR mood <> '' OR custom_json NOT IN ('', '{}'))
             ORDER BY day",
        )?;
        let days: Vec<String> = stmt.query_map([account_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(days.iter().filter_map(|day| day.parse().ok()).collect())
    }

    /// Writes the note of a day and refreshes its full-text index. The index reference
    /// is `YYYY-MM-DD` for the note of all accounts and `YYYY-MM-DD:<account id>` for an
    /// account's own, so the two never replace each other.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the write fails.
    pub fn save_day_note(&mut self, account_id: i64, day: chrono::NaiveDate, note: &DayNote) -> Result<()> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let write = || -> Result<()> {
            tx.execute(
                "INSERT INTO day_notes (account_id, day, plan, review, mood) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(account_id, day) DO UPDATE SET
                     plan = excluded.plan, review = excluded.review, mood = excluded.mood",
                rusqlite::params![account_id, day.to_string(), note.plan, note.review, note.mood],
            )?;
            let values = day_values_on(&tx, account_id, day)?;
            write_index(&tx, &day_document(account_id, day, note, &values))
        };
        // A note of an account this database does not know has no portable key and does not sync.
        match views::day_note_key(&tx, account_id, day).ok() {
            Some(key) => sync::logged(&tx, entity::DAY_NOTE, Some(&key), || write().map(|()| (key.clone(), ())))?,
            None => write()?,
        }
        tx.commit()?;
        Ok(())
    }

    /// The values of the trader's own fields (scope `day`) on a day; `account_id` 0 is all
    /// accounts. A day without a note has none.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the query fails or the stored values do not parse.
    pub fn day_values(&self, account_id: i64, day: chrono::NaiveDate) -> Result<CustomValues> {
        day_values_on(&self.conn, account_id, day)
    }

    /// Replaces the values of the trader's own fields on a day, keeping the plan, review and
    /// mood (the day's note is created when there is none) and refreshing the day's search
    /// document, whose reference is the same as [`Db::save_day_note`]'s.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the write fails.
    pub fn save_day_values(&mut self, account_id: i64, day: chrono::NaiveDate, values: &CustomValues) -> Result<()> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let write = || -> Result<()> {
            tx.execute(
                "INSERT INTO day_notes (account_id, day, custom_json) VALUES (?1, ?2, ?3)
                 ON CONFLICT(account_id, day) DO UPDATE SET custom_json = excluded.custom_json",
                rusqlite::params![account_id, day.to_string(), serde_json::to_string(values)?],
            )?;
            let note = tx.query_row(
                "SELECT plan, review, mood FROM day_notes WHERE account_id = ?1 AND day = ?2",
                rusqlite::params![account_id, day.to_string()],
                |r| Ok(DayNote { plan: r.get(0)?, review: r.get(1)?, mood: r.get(2)? }),
            )?;
            write_index(&tx, &day_document(account_id, day, &note, values))
        };
        match views::day_note_key(&tx, account_id, day).ok() {
            Some(key) => sync::logged(&tx, entity::DAY_NOTE, Some(&key), || write().map(|()| (key.clone(), ())))?,
            None => write()?,
        }
        tx.commit()?;
        Ok(())
    }

    // --- the trader's catalogs and fields -----------------------------------

    /// The options of `kind`, archived ones last, each group in the trader's order.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the query fails.
    pub fn catalog(&self, kind: CatalogKind) -> Result<Vec<CatalogItem>> {
        catalog_on(&self.conn, kind)
    }

    /// Adds an option (`item.id` 0) or changes one; returns its id. Keys are unique per kind.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the key is taken or the write fails.
    pub fn save_catalog_item(&mut self, item: &CatalogItem) -> Result<i64> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = save_catalog_item_on(&tx, item)?;
        tx.commit()?;
        Ok(id)
    }

    /// Adds an option named `label` (trimmed) to `kind` and returns its key. An option of `kind`
    /// already named so, in any case and archived ones included, is used instead: brought back when
    /// archived, so typing a removed timeframe again does not make a second one.
    ///
    /// # Errors
    ///
    /// [`DbError::Invalid`] when `label` is blank; [`DbError::Sqlite`] when the read or the write fails.
    pub fn add_catalog_label(&mut self, kind: CatalogKind, label: &str) -> Result<String> {
        let label = label.trim();
        if label.is_empty() {
            return Err(DbError::Invalid(crate::tr!("core.db.blank_option")));
        }
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let lowered = label.to_lowercase();
        let same = catalog_on(&tx, kind)?.into_iter().find(|item| item.label.to_lowercase() == lowered);
        let key = match same {
            Some(mut item) if item.archived => {
                item.archived = false;
                save_catalog_item_on(&tx, &item)?;
                item.key
            }
            Some(item) => item.key,
            None => {
                let key = format!("custom_{}", crate::sync::new_uid());
                let item = CatalogItem {
                    id: 0,
                    kind,
                    key: key.clone(),
                    label: label.to_owned(),
                    color: None,
                    position: i32::MAX,
                    archived: false,
                };
                save_catalog_item_on(&tx, &item)?;
                key
            }
        };
        tx.commit()?;
        Ok(key)
    }

    /// Archives the option `key` of `kind`: no longer offered to pick, while what stored it still
    /// reads its label. Nothing happens when there is no such option.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the read or the write fails.
    pub fn archive_catalog_item(&mut self, kind: CatalogKind, key: &str) -> Result<()> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(item) = catalog_on(&tx, kind)?.into_iter().find(|item| item.key == key && !item.archived) {
            save_catalog_item_on(&tx, &CatalogItem { archived: true, ..item })?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The fields the trader defined for `scope`, in their order.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the query fails or a stored definition does not parse.
    pub fn field_defs(&self, scope: FieldScope) -> Result<Vec<FieldDef>> {
        field_defs_on(&self.conn, scope)
    }

    /// Adds a field (`def.id` 0) or changes one; returns its id.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the key is taken in its scope or the write fails.
    pub fn save_field_def(&mut self, def: &FieldDef) -> Result<i64> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = save_field_def_on(&tx, def)?;
        tx.commit()?;
        Ok(id)
    }

    /// The trader's arrangement of a form, or the default one.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the setting cannot be read or does not parse.
    pub fn form_layout(&self, scope: FieldScope) -> Result<FormLayout> {
        form_layout_on(&self.conn, scope)
    }

    /// Saves the arrangement of a form.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the write fails.
    pub fn save_form_layout(&mut self, layout: &FormLayout) -> Result<()> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        save_form_layout_on(&tx, layout)?;
        tx.commit()?;
        Ok(())
    }

    // --- playbook -------------------------------------------------------------

    /// The setups, archived ones last.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the query fails or a stored setup does not parse.
    pub fn playbooks(&self) -> Result<Vec<Playbook>> {
        playbooks_on(&self.conn)
    }

    /// Adds a setup (`id` 0) or changes one; returns its id. Names are unique.
    ///
    /// # Errors
    ///
    /// [`DbError::Invalid`] when the name is taken; [`DbError::Sqlite`] when the write fails.
    pub fn save_playbook(&mut self, playbook: &Playbook, position: i32) -> Result<i64> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Saving by hand keeps names unique. Two devices may still each make a setup of the same
        // name; sync keeps both, and the trader renames one when they notice.
        let taken: i64 = tx.query_row(
            "SELECT COUNT(*) FROM playbooks WHERE name = ?1 AND id <> ?2",
            rusqlite::params![playbook.name, playbook.id],
            |r| r.get(0),
        )?;
        if taken > 0 {
            return Err(DbError::Invalid(crate::tr!("core.db.setup_name_taken", name = playbook.name)));
        }
        let known = if playbook.id == 0 { None } else { views::uid_of(&tx, "playbooks", playbook.id)? };
        let id = sync::logged(&tx, entity::PLAYBOOK, known.as_deref(), || {
            let json = serde_json::to_string(playbook)?;
            let id = if playbook.id == 0 {
                tx.execute(
                    "INSERT INTO playbooks (name, json, position, archived) VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![playbook.name, json, position, playbook.archived],
                )?;
                tx.last_insert_rowid()
            } else {
                tx.execute(
                    "UPDATE playbooks SET name = ?2, json = ?3, position = ?4, archived = ?5 WHERE id = ?1",
                    rusqlite::params![playbook.id, playbook.name, json, position, playbook.archived],
                )?;
                playbook.id
            };
            let uid: String = tx.query_row("SELECT uid FROM playbooks WHERE id = ?1", [id], |r| r.get(0))?;
            Ok((uid, id))
        })?;
        // The index entry goes in with the row: a reader that sees the setup finds it by search.
        write_index(&tx, &search::playbook_document(&Playbook { id, ..playbook.clone() }))?;
        tx.commit()?;
        Ok(id)
    }

    /// Deletes a setup and its index entry. Trades whose journal named it keep the link, which
    /// the statistics read as a deleted setup.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the write fails.
    pub fn delete_playbook(&mut self, id: i64) -> Result<()> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let delete = || -> Result<()> {
            tx.execute("DELETE FROM playbooks WHERE id = ?1", [id])?;
            let kind = enum_name(&DocKind::Playbook)?;
            tx.execute("DELETE FROM search_index WHERE kind = ?1 AND reference = ?2", [kind, id.to_string()])?;
            Ok(())
        };
        match views::uid_of(&tx, "playbooks", id)? {
            Some(uid) => sync::logged_delete(&tx, entity::PLAYBOOK, &uid, delete)?,
            None => delete()?,
        }
        tx.commit()?;
        Ok(())
    }

    // --- missed trades --------------------------------------------------------

    /// Deletes a missed trade and its index entry.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the write fails.
    pub fn delete_missed_trade(&mut self, id: i64) -> Result<()> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let delete = || -> Result<()> {
            tx.execute("DELETE FROM missed_trades WHERE id = ?1", [id])?;
            let kind = enum_name(&DocKind::Missed)?;
            tx.execute("DELETE FROM search_index WHERE kind = ?1 AND reference = ?2", [kind, id.to_string()])?;
            Ok(())
        };
        match views::uid_of(&tx, "missed_trades", id)? {
            Some(uid) => sync::logged_delete(&tx, entity::MISSED, &uid, delete)?,
            None => delete()?,
        }
        tx.commit()?;
        Ok(())
    }

    /// Missed trades seen between `from` and `to` (Unix milliseconds), newest first.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the query fails or a stored entry does not parse.
    pub fn missed_trades(&self, from: i64, to: i64) -> Result<Vec<MissedTrade>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, json FROM missed_trades WHERE seen_at BETWEEN ?1 AND ?2 ORDER BY seen_at DESC")?;
        let rows = stmt.query_map([from, to], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        rows.map(|row| {
            let (id, json) = row?;
            let mut missed: MissedTrade = serde_json::from_str(&json)?;
            missed.id = id;
            Ok(missed)
        })
        .collect()
    }

    /// Adds a missed trade (`id` 0) or changes one; returns its id.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the write fails.
    pub fn save_missed_trade(&mut self, missed: &MissedTrade) -> Result<i64> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = save_missed_on(&tx, missed)?;
        write_index(&tx, &search::missed_document(&MissedTrade { id, ..missed.clone() }))?;
        tx.commit()?;
        Ok(id)
    }

    // --- reviews ------------------------------------------------------------------

    /// The review of the period starting `start`, if written.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the query fails or the stored review does not parse.
    pub fn review(
        &self,
        account_id: Option<i64>,
        period: Period,
        start: chrono::NaiveDate,
    ) -> Result<Option<PeriodReview>> {
        let row = self
            .conn
            .query_row(
                "SELECT id, json FROM reviews WHERE account_id = ?1 AND period = ?2 AND start = ?3",
                rusqlite::params![account_id.unwrap_or(0), enum_name(&period)?, start.to_string()],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?;
        row.map(|(id, json)| {
            let mut review: PeriodReview = serde_json::from_str(&json)?;
            review.id = id;
            Ok(review)
        })
        .transpose()
    }

    /// Writes a review, replacing the one of the same account and period.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the write fails.
    pub fn save_review(&mut self, review: &PeriodReview) -> Result<i64> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = save_review_on(&tx, review)?;
        write_index(&tx, &search::review_document(review))?;
        tx.commit()?;
        Ok(id)
    }

    /// The trader's template for `period`, or the default.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the setting cannot be read or does not parse.
    pub fn review_template(&self, period: Period) -> Result<ReviewTemplate> {
        match self.setting(&format!("review.template.{}", enum_name(&period)?))? {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(ReviewTemplate::default_for(period)),
        }
    }

    /// Saves the trader's template.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the write fails.
    pub fn save_review_template(&mut self, template: &ReviewTemplate) -> Result<()> {
        let key = format!("review.template.{}", enum_name(&template.period)?);
        self.set_setting(&key, &serde_json::to_string(template)?)
    }

    // --- search ---------------------------------------------------------------------

    /// Indexes `doc`, replacing what was indexed for the same thing. Every write of a review,
    /// missed trade, setup, journal or day note indexes itself in the same transaction; this is
    /// for documents that have no such write.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the write fails.
    pub fn index_document(&mut self, doc: &Document) -> Result<()> {
        let tx = self.conn.transaction()?;
        write_index(&tx, doc)?;
        tx.commit()?;
        Ok(())
    }

    /// The best matches of what the trader typed, at most `limit`.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the query fails.
    pub fn search(&self, input: &str, limit: u32) -> Result<Vec<Hit>> {
        let Some(query) = search::fts_query(input) else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare(
            "SELECT kind, reference, title, snippet(search_index, 3, '[', ']', '…', 12)
             FROM search_index WHERE search_index MATCH ?1 ORDER BY bm25(search_index, 0, 0, 4.0, 1.0) LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![query, limit], |r| {
            Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        rows.map(|row| {
            let (kind, reference, title, snippet) = row?;
            let kind: DocKind = serde_json::from_value(serde_json::Value::String(kind))?;
            Ok(Hit { kind, reference, title, snippet })
        })
        .collect()
    }

    // --- settings -----------------------------------------------------------------

    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails.
    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        setting_on(&self.conn, key)
    }

    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails.
    pub fn set_setting(&mut self, key: &str, value: &str) -> Result<()> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        sync::log_setting(&tx, key, value)?;
        tx.commit()?;
        Ok(())
    }
}

/// The `source` of the deals of a hand-entered trade.
const MANUAL_SOURCE: &str = "manual";

// --- statements shared by the methods above and by multi-step transactions ----------

fn field_defs_on(conn: &Connection, scope: FieldScope) -> Result<Vec<FieldDef>> {
    let mut stmt =
        conn.prepare("SELECT id, def_json FROM field_defs WHERE scope = ?1 ORDER BY archived, position, id")?;
    let rows = stmt.query_map([scope_name(scope)?], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    rows.map(|row| {
        let (id, json) = row?;
        let mut def: FieldDef = serde_json::from_str(&json)?;
        def.id = id;
        Ok(def)
    })
    .collect()
}

/// Writes a field definition and logs it for sync.
fn save_field_def_on(conn: &Connection, def: &FieldDef) -> Result<i64> {
    let scope = scope_name(def.scope)?;
    let known = (def.id != 0).then(|| field_def_key(conn, def.id)).transpose()?.flatten();
    sync::logged(conn, entity::FIELD_DEF, known.as_deref(), || {
        let id = save_field_def_raw(conn, def)?;
        Ok((join_key(&[&scope, &def.key]), id))
    })
}

fn field_def_key(conn: &Connection, id: i64) -> Result<Option<String>> {
    let row: Option<(String, String)> = conn
        .query_row("SELECT scope, key FROM field_defs WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    Ok(row.map(|(scope, key)| join_key(&[&scope, &key])))
}

/// [`save_field_def_on`] without the log, for applying what another device wrote.
fn save_field_def_raw(conn: &Connection, def: &FieldDef) -> Result<i64> {
    let json = serde_json::to_string(def)?;
    let scope = scope_name(def.scope)?;
    if def.id == 0 {
        conn.execute(
            "INSERT INTO field_defs (scope, key, def_json, position, archived) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![scope, def.key, json, def.position, def.archived],
        )?;
        Ok(conn.last_insert_rowid())
    } else {
        conn.execute(
            "UPDATE field_defs SET def_json = ?2, position = ?3, archived = ?4 WHERE id = ?1",
            rusqlite::params![def.id, json, def.position, def.archived],
        )?;
        Ok(def.id)
    }
}

fn catalog_on(conn: &Connection, kind: CatalogKind) -> Result<Vec<CatalogItem>> {
    let mut stmt = conn.prepare(
        "SELECT id, key, label, color, position, archived FROM catalog_items
         WHERE kind = ?1 ORDER BY archived, position, id",
    )?;
    let rows = stmt.query_map([kind_name(kind)?], |r| {
        Ok(CatalogItem {
            id: r.get(0)?,
            kind,
            key: r.get(1)?,
            label: r.get(2)?,
            color: r.get(3)?,
            position: r.get(4)?,
            archived: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Writes a catalog option and logs it for sync.
fn save_catalog_item_on(conn: &Connection, item: &CatalogItem) -> Result<i64> {
    let kind = kind_name(item.kind)?;
    let known: Option<String> = if item.id == 0 {
        None
    } else {
        conn.query_row("SELECT kind, key FROM catalog_items WHERE id = ?1", [item.id], |r| {
            Ok(join_key(&[&r.get::<_, String>(0)?, &r.get::<_, String>(1)?]))
        })
        .optional()?
    };
    let written = known.clone().unwrap_or_else(|| join_key(&[&kind, &item.key]));
    sync::logged(conn, entity::CATALOG, known.as_deref(), || {
        Ok((written.clone(), write_catalog_item(conn, item, &kind)?))
    })
}

fn write_catalog_item(conn: &Connection, item: &CatalogItem, kind: &str) -> Result<i64> {
    if item.id == 0 {
        conn.execute(
            "INSERT INTO catalog_items (kind, key, label, color, position, archived) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![kind, item.key, item.label, item.color, item.position, item.archived],
        )?;
        Ok(conn.last_insert_rowid())
    } else {
        conn.execute(
            "UPDATE catalog_items SET label = ?2, color = ?3, position = ?4, archived = ?5 WHERE id = ?1",
            rusqlite::params![item.id, item.label, item.color, item.position, item.archived],
        )?;
        Ok(item.id)
    }
}

fn playbooks_on(conn: &Connection) -> Result<Vec<Playbook>> {
    let mut stmt = conn.prepare("SELECT id, json FROM playbooks ORDER BY archived, position, id")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    rows.map(|row| {
        let (id, json) = row?;
        let mut playbook: Playbook = serde_json::from_str(&json)?;
        playbook.id = id;
        Ok(playbook)
    })
    .collect()
}

fn journal_on(conn: &Connection, key: &TradeKey) -> Result<Option<JournalEntry>> {
    let json: Option<String> = conn
        .query_row(
            "SELECT entry_json FROM journal WHERE account_id = ?1 AND position_id = ?2 AND segment = ?3",
            rusqlite::params![key.account_id, key.position_id, key.segment],
            |r| r.get(0),
        )
        .optional()?;
    Ok(json.map(|json| serde_json::from_str(&json)).transpose()?)
}

/// Writes a journal entry, its changelog row and its index; saving what is stored does nothing.
fn save_journal_on(conn: &Connection, key: &TradeKey, entry: &JournalEntry) -> Result<()> {
    let json = serde_json::to_string(entry)?;
    let stored: Option<String> = conn
        .query_row(
            "SELECT entry_json FROM journal WHERE account_id = ?1 AND position_id = ?2 AND segment = ?3",
            rusqlite::params![key.account_id, key.position_id, key.segment],
            |r| r.get(0),
        )
        .optional()?;
    if stored.as_deref() == Some(json.as_str()) {
        return Ok(());
    }
    let portable = views::journal_key(conn, key)?;
    sync::logged(conn, entity::JOURNAL, Some(&portable), || {
        write_journal_raw(conn, key, entry)?;
        Ok((portable.clone(), ()))
    })
}

/// Writes a journal entry and refreshes its index, without logging it for sync.
fn write_journal_raw(conn: &Connection, key: &TradeKey, entry: &JournalEntry) -> Result<()> {
    conn.execute(
        "INSERT INTO journal (account_id, position_id, segment, entry_json, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(account_id, position_id, segment)
         DO UPDATE SET entry_json = excluded.entry_json, updated_at = excluded.updated_at",
        rusqlite::params![key.account_id, key.position_id, key.segment, serde_json::to_string(entry)?, now_ms()],
    )?;
    write_index(conn, &journal_document(conn, key, entry)?)
}

fn rules_on(conn: &Connection, account_id: i64) -> Result<Rules> {
    let rules = conn
        .query_row(
            "SELECT max_daily_loss, max_daily_trades, max_risk_pct, max_consecutive_losses, daily_target
             FROM rules WHERE account_id = ?1",
            [account_id],
            |r| {
                Ok(Rules {
                    max_daily_loss: r.get(0)?,
                    max_daily_trades: r.get(1)?,
                    max_risk_pct: r.get(2)?,
                    max_consecutive_losses: r.get(3)?,
                    daily_target: r.get(4)?,
                })
            },
        )
        .optional()?;
    Ok(rules.unwrap_or_default())
}

fn save_rules_raw(conn: &Connection, account_id: i64, rules: &Rules) -> Result<()> {
    conn.execute(
        "INSERT INTO rules (account_id, max_daily_loss, max_daily_trades, max_risk_pct, max_consecutive_losses,
                            daily_target)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(account_id) DO UPDATE SET
             max_daily_loss = excluded.max_daily_loss,
             max_daily_trades = excluded.max_daily_trades,
             max_risk_pct = excluded.max_risk_pct,
             max_consecutive_losses = excluded.max_consecutive_losses,
             daily_target = excluded.daily_target",
        rusqlite::params![
            account_id,
            rules.max_daily_loss,
            rules.max_daily_trades,
            rules.max_risk_pct,
            rules.max_consecutive_losses,
            rules.daily_target
        ],
    )?;
    Ok(())
}

/// Adds a missed trade (`id` 0) or changes one, and logs it for sync.
fn save_missed_on(conn: &Connection, missed: &MissedTrade) -> Result<i64> {
    let known = if missed.id == 0 { None } else { views::uid_of(conn, "missed_trades", missed.id)? };
    let json = serde_json::to_string(missed)?;
    sync::logged(conn, entity::MISSED, known.as_deref(), || {
        let id = if missed.id == 0 {
            conn.execute(
                "INSERT INTO missed_trades (account_id, seen_at, json) VALUES (?1, ?2, ?3)",
                rusqlite::params![missed.account_id, missed.seen_at, json],
            )?;
            conn.last_insert_rowid()
        } else {
            conn.execute(
                "UPDATE missed_trades SET account_id = ?2, seen_at = ?3, json = ?4 WHERE id = ?1",
                rusqlite::params![missed.id, missed.account_id, missed.seen_at, json],
            )?;
            missed.id
        };
        let uid: String = conn.query_row("SELECT uid FROM missed_trades WHERE id = ?1", [id], |r| r.get(0))?;
        Ok((uid, id))
    })
}

/// Writes a review, replacing the one of the same account and period, and logs it for sync.
fn save_review_on(conn: &Connection, review: &PeriodReview) -> Result<i64> {
    let key = views::review_key(conn, review)?;
    sync::logged(conn, entity::REVIEW, Some(&key), || Ok((key.clone(), save_review_raw(conn, review)?)))
}

fn save_review_raw(conn: &Connection, review: &PeriodReview) -> Result<i64> {
    conn.execute(
        "INSERT INTO reviews (account_id, period, start, json) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(account_id, period, start) DO UPDATE SET json = excluded.json",
        rusqlite::params![
            review.account_id.unwrap_or(0),
            enum_name(&review.period)?,
            review.start.to_string(),
            serde_json::to_string(review)?
        ],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM reviews WHERE account_id = ?1 AND period = ?2 AND start = ?3",
        rusqlite::params![review.account_id.unwrap_or(0), enum_name(&review.period)?, review.start.to_string()],
        |r| r.get(0),
    )?)
}

fn setting_on(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()?)
}

fn set_setting_on(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

fn day_values_on(conn: &Connection, account_id: i64, day: chrono::NaiveDate) -> Result<CustomValues> {
    let json: Option<String> = conn
        .query_row(
            "SELECT custom_json FROM day_notes WHERE account_id = ?1 AND day = ?2",
            rusqlite::params![account_id, day.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    match json {
        Some(json) => Ok(serde_json::from_str(&json)?),
        None => Ok(CustomValues::new()),
    }
}

/// What the trader wrote on a day (plan, review, mood and the text values of their own
/// fields) as one searchable text. The reference is `YYYY-MM-DD` for the note of all accounts
/// and `YYYY-MM-DD:<account id>` for an account's own.
fn day_document(account_id: i64, day: chrono::NaiveDate, note: &DayNote, values: &CustomValues) -> Document {
    let mut parts: Vec<&str> = vec![note.plan.as_str(), note.review.as_str(), note.mood.as_str()];
    parts.extend(values.values().filter_map(|value| match value {
        FieldValue::Text(text) => Some(text.as_str()),
        _ => None,
    }));
    Document {
        kind: DocKind::Day,
        reference: if account_id == 0 { day.to_string() } else { format!("{day}:{account_id}") },
        title: format!("Nhật ký ngày {}", day.format("%d/%m/%Y")),
        body: parts.join("\n"),
    }
}

fn form_layout_on(conn: &Connection, scope: FieldScope) -> Result<FormLayout> {
    match setting_on(conn, &format!("form.layout.{}", scope_name(scope)?))? {
        Some(json) => Ok(serde_json::from_str(&json)?),
        None => Ok(FormLayout::default_for(scope)),
    }
}

/// Saves the arrangement of a form and logs it for sync.
fn save_form_layout_on(conn: &Connection, layout: &FormLayout) -> Result<()> {
    let scope = scope_name(layout.scope)?;
    let key = format!("form.layout.{scope}");
    // A form never arranged has no stored layout: what it held before is the default one.
    let before = setting_on(conn, &key)?.is_some().then_some(scope.as_str());
    sync::logged(conn, entity::FORM_LAYOUT, before, || {
        set_setting_on(conn, &key, &serde_json::to_string(layout)?)?;
        Ok((scope.clone(), ()))
    })
}

/// The setting that holds this installation's id in the changelog.
const DEVICE_ID_KEY: &str = "sync.device_id";

const ACCOUNT_COLUMNS: &str = "id, login, server, broker, name, currency, initial_balance, last_balance, last_equity,
     server_offset_min, last_sync";

const DEAL_COLUMNS: &str = "ticket, order_id, position_id, time_msc, symbol, kind, entry, volume, price, commission,
     swap, fee, profit, magic, comment, sl, tp, risk, reason, result_r, gaps";

/// A row count as SQLite stores it.
fn count(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// Unix milliseconds now.
fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn account_from_row(r: &Row<'_>) -> rusqlite::Result<Account> {
    Ok(Account {
        id: r.get(0)?,
        login: r.get(1)?,
        server: r.get(2)?,
        broker: r.get(3)?,
        name: r.get(4)?,
        currency: r.get(5)?,
        initial_balance: r.get(6)?,
        last_balance: r.get(7)?,
        last_equity: r.get(8)?,
        server_offset_min: r.get(9)?,
        last_sync: r.get(10)?,
    })
}

fn deal_from_row(r: &Row<'_>) -> rusqlite::Result<Deal> {
    deal_from_row_at(r, 0)
}

/// A deal read from the columns of [`DEAL_COLUMNS`] starting at column `first`.
fn deal_from_row_at(r: &Row<'_>, first: usize) -> rusqlite::Result<Deal> {
    Ok(Deal {
        ticket: r.get(first)?,
        order: r.get(first + 1)?,
        position_id: r.get(first + 2)?,
        time_msc: r.get(first + 3)?,
        symbol: r.get(first + 4)?,
        kind: DealKind::parse(&r.get::<_, String>(first + 5)?),
        entry: DealEntry::parse(&r.get::<_, String>(first + 6)?),
        volume: r.get(first + 7)?,
        price: r.get(first + 8)?,
        commission: r.get(first + 9)?,
        swap: r.get(first + 10)?,
        fee: r.get(first + 11)?,
        profit: r.get(first + 12)?,
        magic: r.get(first + 13)?,
        comment: r.get(first + 14)?,
        sl: r.get(first + 15)?,
        tp: r.get(first + 16)?,
        risk: r.get(first + 17)?,
        reason: r.get(first + 18)?,
        result_r: r.get(first + 19)?,
        gaps: Gaps::from_bits(r.get(first + 20)?),
    })
}

/// Inserts a deal or replaces the stored one with the same (account, ticket), and logs it for sync.
fn write_deal(conn: &Connection, account_id: i64, deal: &Deal, source: &str) -> Result<()> {
    let key = views::deal_key(conn, account_id, deal.ticket)?;
    sync::logged(conn, entity::DEAL, Some(&key), || {
        write_deal_raw(conn, account_id, deal, source)?;
        Ok((key.clone(), ()))
    })
}

/// [`write_deal`] without the log, for applying what another device wrote.
fn write_deal_raw(conn: &Connection, account_id: i64, deal: &Deal, source: &str) -> Result<()> {
    conn.prepare_cached(
        "INSERT OR REPLACE INTO deals (account_id, ticket, order_id, position_id, time_msc, symbol, kind, entry,
             volume, price, commission, swap, fee, profit, magic, comment, sl, tp, risk, reason, result_r, gaps, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
    )?
    .execute(rusqlite::params![
        account_id,
        deal.ticket,
        deal.order,
        deal.position_id,
        deal.time_msc,
        deal.symbol,
        deal.kind.as_str(),
        deal.entry.as_str(),
        deal.volume,
        deal.price,
        deal.commission,
        deal.swap,
        deal.fee,
        deal.profit,
        deal.magic,
        deal.comment,
        deal.sl,
        deal.tp,
        deal.risk,
        deal.reason,
        deal.result_r,
        deal.gaps.bits(),
        source
    ])?;
    Ok(())
}

/// What the terminal measured and a report cannot tell (risk, reason, stop, target) is not
/// lost when a report is imported over the terminal's deals. The risk is the one measured first,
/// at the entry: a later poll measures it again with the tick value of that moment, which moves
/// with price, and must not make an unchanged deal look updated.
fn keep_measured(deal: &mut Deal, stored: &Deal) {
    if stored.risk.is_some() || deal.risk.is_none() {
        deal.risk = stored.risk;
    }
    if deal.reason.is_none() {
        deal.reason.clone_from(&stored.reason);
    }
    if deal.sl <= 0.0 {
        deal.sl = stored.sl;
    }
    if deal.tp <= 0.0 {
        deal.tp = stored.tp;
    }
}

/// [`Db::upsert_account`] on any connection or transaction.
fn upsert_account_on(conn: &Connection, info: &AccountInfo) -> Result<i64> {
    let login = info.login.trim();
    if login.is_empty() {
        return Err(DbError::Invalid(crate::tr!("core.db.no_login")));
    }
    let key = join_key(&[login, info.server.trim()]);
    let id = sync::logged(conn, entity::ACCOUNT, Some(&key), || {
        let id = upsert_account_row(conn, info, login)?;
        Ok((key.clone(), id))
    })?;
    adopt_shared_rules(conn, id, &key)?;
    Ok(id)
}

/// Gives the account `id` the discipline rules the trader already keeps, when it never had rules
/// of its own: the rules are the trader's, and an account added later (a new MT5 login, an
/// import) is judged by them at once. An account whose rules were saved, even all off, keeps them.
/// The rules come from the first account (by id) with some rule on, as the dashboard picks them.
fn adopt_shared_rules(conn: &Connection, id: i64, key: &str) -> Result<()> {
    let has_own: bool =
        conn.query_row("SELECT EXISTS (SELECT 1 FROM rules WHERE account_id = ?1)", [id], |r| r.get(0))?;
    if has_own {
        return Ok(());
    }
    let shared: Option<i64> = conn
        .query_row(
            "SELECT account_id FROM rules
             WHERE account_id <> ?1 AND (max_daily_loss IS NOT NULL OR max_daily_trades IS NOT NULL
                   OR max_risk_pct IS NOT NULL OR max_consecutive_losses IS NOT NULL OR daily_target IS NOT NULL)
             ORDER BY account_id LIMIT 1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(source) = shared else { return Ok(()) };
    let rules = rules_on(conn, source)?;
    sync::logged(conn, entity::RULES, Some(key), || {
        save_rules_raw(conn, id, &rules)?;
        Ok((key.to_owned(), ()))
    })
}

fn upsert_account_row(conn: &Connection, info: &AccountInfo, login: &str) -> Result<i64> {
    // Empty text means "not reported": it neither blanks a stored value nor beats the default.
    let id = conn.query_row(
        "INSERT INTO accounts (login, server, broker, name, currency, last_balance, last_equity, server_offset_min,
                               last_sync)
         VALUES (?1, ?2, ?3, ?4, COALESCE(NULLIF(?5, ''), 'USD'), ?6, ?7, ?8, ?9)
         ON CONFLICT(login, server) DO UPDATE SET
             broker = COALESCE(NULLIF(?3, ''), broker),
             name = COALESCE(NULLIF(name, ''), ?4),
             currency = COALESCE(NULLIF(?5, ''), currency),
             last_balance = COALESCE(?6, last_balance),
             last_equity = COALESCE(?7, last_equity),
             server_offset_min = COALESCE(?8, server_offset_min),
             last_sync = ?9
         RETURNING id",
        rusqlite::params![
            login,
            info.server.trim(),
            info.broker.trim(),
            info.name.trim(),
            info.currency.trim(),
            info.balance,
            info.equity,
            info.server_offset_min,
            now_ms()
        ],
        |r| r.get(0),
    )?;
    Ok(id)
}

/// Replaces what is indexed for `doc` (see [`Db::index_document`]); the caller owns the transaction.
fn write_index(conn: &Connection, doc: &Document) -> Result<()> {
    let kind = enum_name(&doc.kind)?;
    conn.execute("DELETE FROM search_index WHERE kind = ?1 AND reference = ?2", [&kind, &doc.reference])?;
    if !doc.body.trim().is_empty() || !doc.title.trim().is_empty() {
        conn.execute(
            "INSERT INTO search_index (kind, reference, title, body) VALUES (?1, ?2, ?3, ?4)",
            [&kind, &doc.reference, &doc.title, &doc.body],
        )?;
    }
    Ok(())
}

/// What the trader wrote on a trade, as one searchable text titled like the trade
/// (`XAUUSD Buy 20/11`, from the trade's deals; a bare reference when they are gone).
pub(super) fn journal_document(conn: &Connection, key: &TradeKey, entry: &JournalEntry) -> Result<Document> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {DEAL_COLUMNS} FROM deals
         WHERE account_id = ?1 AND (position_id = ?2 OR (position_id = 0 AND ticket = ?2))"
    ))?;
    let deals: Vec<Deal> = stmt
        .query_map(rusqlite::params![key.account_id, key.position_id], deal_from_row)?
        .collect::<rusqlite::Result<_>>()?;
    let title = build_trades(key.account_id, &deals)
        .into_iter()
        .find(|t| t.key == *key)
        .and_then(|t| {
            let opened = chrono::DateTime::from_timestamp_millis(t.open_time)?;
            Some(format!("{} {} {}", t.symbol, t.side.label(), opened.format("%d/%m")))
        })
        .unwrap_or_else(|| format!("{}{}", search::TRADE_PREFIX, key.position_id));

    let mut parts: Vec<&str> = vec![&entry.setup];
    parts.extend(entry.tags.iter().chain(&entry.mistakes).map(String::as_str));
    parts.extend([entry.emotion_before.as_str(), entry.emotion_after.as_str(), entry.notes.as_str()]);
    if let Some(plan) = &entry.plan {
        parts.push(&plan.thesis);
    }
    parts.extend(entry.screenshots.iter().map(|s| s.caption.as_str()));
    parts.extend(entry.custom.values().filter_map(|value| match value {
        FieldValue::Text(text) => Some(text.as_str()),
        _ => None,
    }));
    parts.retain(|part| !part.trim().is_empty());
    Ok(Document { kind: DocKind::Trade, reference: key.id(), title, body: parts.join("\n") })
}

/// Appends a row to the cloud changelog (`value` `None` deletes), numbered after this
/// device's last one.
fn append_change(conn: &Connection, at: i64, entity: &str, key: &str, value: Option<&str>) -> Result<()> {
    let device_id = device_id(conn)?;
    conn.execute(
        "INSERT INTO changes (device_id, seq, at, entity, key, value)
         VALUES (?1, (SELECT COALESCE(MAX(seq), 0) + 1 FROM changes WHERE device_id = ?1), ?2, ?3, ?4, ?5)",
        rusqlite::params![device_id, at, entity, key, value],
    )?;
    Ok(())
}

/// This installation's id in the changelog, made on first use and kept in the settings.
fn device_id(conn: &Connection) -> Result<String> {
    let stored: Option<String> =
        conn.query_row("SELECT value FROM settings WHERE key = ?1", [DEVICE_ID_KEY], |r| r.get(0)).optional()?;
    if let Some(id) = stored {
        return Ok(id);
    }
    // OR IGNORE: if another connection made one first, theirs wins and is read back.
    conn.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2)",
        [DEVICE_ID_KEY, new_device_id().as_str()],
    )?;
    Ok(conn.query_row("SELECT value FROM settings WHERE key = ?1", [DEVICE_ID_KEY], |r| r.get(0))?)
}

/// Random enough to tell this machine's changelog from another's; not a secret.
fn new_device_id() -> String {
    crate::sync::new_uid()
}

/// The stored name of a serde enum unit variant (`CatalogKind::ExitReason` → `exit_reason`).
fn kind_name(kind: CatalogKind) -> Result<String> {
    enum_name(&kind)
}

fn scope_name(scope: FieldScope) -> Result<String> {
    enum_name(&scope)
}

fn enum_name(value: &impl serde::Serialize) -> Result<String> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(name) => Ok(name),
        other => Err(DbError::Invalid(format!("not a unit variant: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_and_keeps_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tradelens.db");
        {
            let mut db = Db::open(&path).unwrap();
            assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len());
            db.set_setting("bridge.port", "7878").unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(db.setting("bridge.port").unwrap().as_deref(), Some("7878"));
        assert_eq!(db.setting("missing").unwrap(), None);
    }

    #[test]
    fn catalogs_start_with_defaults_and_keep_the_traders_changes() {
        use crate::catalog::{CatalogItem, CatalogKind};
        let mut db = Db::in_memory().unwrap();
        let reasons = db.catalog(CatalogKind::ExitReason).unwrap();
        assert_eq!(reasons.first().map(|r| r.key.as_str()), Some("tp"));

        let mut mistake = CatalogItem {
            id: 0,
            kind: CatalogKind::Mistake,
            key: "no_journal".into(),
            label: "Không ghi lý do".into(),
            color: Some("#f59e0b".into()),
            position: 99,
            archived: false,
        };
        mistake.id = db.save_catalog_item(&mistake).unwrap();
        mistake.label = "Quên ghi lý do vào lệnh".into();
        db.save_catalog_item(&mistake).unwrap();
        let mistakes = db.catalog(CatalogKind::Mistake).unwrap();
        assert_eq!(mistakes.last().map(|m| m.label.as_str()), Some("Quên ghi lý do vào lệnh"));
        assert!(db.save_catalog_item(&CatalogItem { id: 0, ..mistake }).is_err(), "keys are unique per kind");
    }

    #[test]
    fn playbooks_round_trip() {
        use crate::playbook::Playbook;
        let mut db = Db::in_memory().unwrap();
        let setup = Playbook {
            name: "Breakout phiên Á".into(),
            entry_rules: vec!["Phá đỉnh phiên Á".into(), "Retest giữ được".into()],
            risk_pct: Some(1.0),
            ..Playbook::default()
        };
        let id = db.save_playbook(&setup, 0).unwrap();
        assert_eq!(db.playbooks().unwrap(), vec![Playbook { id, ..setup }]);
    }

    #[test]
    fn deleting_a_setup_or_a_missed_trade_removes_its_index_entry() {
        use crate::{
            missed::MissedTrade,
            model::Side,
            playbook::Playbook,
            search::{missed_document, playbook_document},
        };
        let mut db = Db::in_memory().unwrap();
        let mut setup = Playbook { name: "Pullback EMA20".into(), ..Playbook::default() };
        setup.id = db.save_playbook(&setup, 0).unwrap();
        db.index_document(&playbook_document(&setup)).unwrap();
        let mut missed = MissedTrade {
            id: 0,
            account_id: None,
            symbol: "EURUSD".into(),
            side: Side::Short,
            seen_at: 1_763_716_920_000,
            playbook_id: None,
            setup_grade: None,
            reason: None,
            entry: None,
            stop_loss: None,
            take_profit: None,
            outcome_price: None,
            notes: "không tin vào setup".into(),
            screenshots: Vec::new(),
            custom: std::collections::BTreeMap::default(),
        };
        missed.id = db.save_missed_trade(&missed).unwrap();
        db.index_document(&missed_document(&missed)).unwrap();
        assert_eq!(db.search("pullback", 10).unwrap().len(), 1);
        assert_eq!(db.search("khong tin", 10).unwrap().len(), 1);

        db.delete_playbook(setup.id).unwrap();
        db.delete_missed_trade(missed.id).unwrap();
        assert_eq!(db.playbooks().unwrap().len(), 0, "the setup is gone");
        assert_eq!(db.missed_trades(0, i64::MAX).unwrap().len(), 0, "the missed trade is gone");
        assert_eq!(db.search("pullback", 10).unwrap().len(), 0, "its index entry is gone");
        assert_eq!(db.search("khong tin", 10).unwrap().len(), 0, "its index entry is gone");
    }

    #[test]
    fn a_saved_setup_review_or_missed_trade_is_searchable_as_soon_as_it_is_stored() {
        use crate::review::{Period, PeriodReview};
        let mut db = Db::in_memory().unwrap();
        db.save_playbook(&Playbook { name: "Pullback EMA20".into(), ..Playbook::default() }, 0).unwrap();
        assert_eq!(db.search("pullback", 10).unwrap().len(), 1, "the setup is indexed by the write that stores it");
        let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        let template = db.review_template(Period::Week).unwrap();
        let mut review = PeriodReview::blank(&template, day, None);
        review.answers[0].1 = "Giữ đúng ba lệnh mỗi ngày".into();
        db.save_review(&review).unwrap();
        assert_eq!(db.search("ba lenh", 10).unwrap().len(), 1, "the review is indexed by its write");
        let renamed = Playbook { id: db.playbooks().unwrap()[0].id, name: "Breakout".into(), ..Playbook::default() };
        db.save_playbook(&renamed, 0).unwrap();
        assert_eq!(db.search("pullback", 10).unwrap().len(), 0, "a rename replaces the index entry");
        assert_eq!(db.search("breakout", 10).unwrap().len(), 1);
    }

    #[test]
    fn reviews_replace_their_period_and_templates_default() {
        use crate::review::{Period, PeriodReview, ReviewTemplate};
        let mut db = Db::in_memory().unwrap();
        let template = db.review_template(Period::Week).unwrap();
        assert_eq!(template, ReviewTemplate::default_for(Period::Week));
        let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        let mut review = PeriodReview::blank(&template, day, None);
        review.answers[0].1 = "Giữ đúng 3 lệnh/ngày".into();
        let id = db.save_review(&review).unwrap();
        review.rating = 4;
        assert_eq!(db.save_review(&review).unwrap(), id, "same period, same review");
        let read = db.review(None, Period::Week, review.start).unwrap().unwrap();
        assert_eq!(read.rating, 4);
        assert_eq!(read.answers[0].1, "Giữ đúng 3 lệnh/ngày");
    }

    #[test]
    fn search_folds_accents_and_replaces_documents() {
        use crate::search::{DocKind, Document};
        let mut db = Db::in_memory().unwrap();
        let doc = |body: &str| Document {
            kind: DocKind::Trade,
            reference: "1:61234567:0".into(),
            title: "XAUUSD Buy 20/11".into(),
            body: body.into(),
        };
        db.index_document(&doc("Dời SL lần nữa, vào lệnh sớm trước tin")).unwrap();
        let hits = db.search("doi sl", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.contains("[Dời]"), "{}", hits[0].snippet);
        db.index_document(&doc("Đúng kế hoạch")).unwrap();
        assert!(db.search("doi", 10).unwrap().is_empty(), "the old text is gone");
        assert_eq!(db.search("ke hoach", 10).unwrap().len(), 1);
    }

    #[test]
    fn form_layouts_default_and_persist() {
        use crate::catalog::FieldScope;
        use crate::form::{BuiltinField, FieldRef, FormLayout};
        let mut db = Db::in_memory().unwrap();
        let mut form = db.form_layout(FieldScope::Trade).unwrap();
        assert_eq!(form, FormLayout::default_for(FieldScope::Trade));
        form.rename_builtin(BuiltinField::Mistakes, "Lỗi kỷ luật").unwrap();
        form.hide(&FieldRef::Builtin(BuiltinField::Market)).unwrap();
        db.save_form_layout(&form).unwrap();
        assert_eq!(db.form_layout(FieldScope::Trade).unwrap(), form);
    }

    #[test]
    fn field_definitions_round_trip() {
        use crate::catalog::{FieldDef, FieldKind, FieldScope, Requirement};
        let mut db = Db::in_memory().unwrap();
        let def = FieldDef {
            id: 0,
            scope: FieldScope::Trade,
            key: "focus".into(),
            label: "Mức tập trung".into(),
            kind: FieldKind::Scale,
            options: Vec::new(),
            max: Some(10),
            required: Requirement::Expected,
            analysed: true,
            position: 0,
            archived: false,
        };
        let id = db.save_field_def(&def).unwrap();
        let read = db.field_defs(FieldScope::Trade).unwrap();
        assert_eq!(read, vec![FieldDef { id, ..def }]);
        assert_eq!(db.field_defs(FieldScope::Day).unwrap(), Vec::<FieldDef>::new());
    }
}
