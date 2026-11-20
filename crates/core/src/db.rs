//! SQLite storage. One file per user; deals are the source of truth and trades are
//! rebuilt from them. WAL mode lets the bridge thread and the UI write at once.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::catalog::{CatalogItem, CatalogKind, FieldDef, FieldScope};
use crate::missed::MissedTrade;
use crate::model::{
    Account, AccountInfo, ChecklistItem, DayNote, Deal, DealBatch, JournalEntry, ManualTrade, Rules, TradeKey,
};
use crate::playbook::Playbook;
use crate::review::{Period, PeriodReview, ReviewTemplate};
use crate::search::{self, DocKind, Document, Hit};

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("chưa làm: {0}")]
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
];

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
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
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
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
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
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
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
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
    pub fn schema_version(&self) -> Result<usize> {
        let version: i64 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        usize::try_from(version).map_err(|_| DbError::Invalid(format!("schema version {version}")))
    }

    // --- accounts ------------------------------------------------------------

    /// The account of `info` (by login and server), created when new; its balance,
    /// equity and server offset are refreshed.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
    pub fn upsert_account(&mut self, _info: &AccountInfo) -> Result<i64> {
        Err(DbError::NotImplemented("v0.1: INSERT .. ON CONFLICT(login, server) DO UPDATE"))
    }

    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
    pub fn accounts(&self) -> Result<Vec<Account>> {
        Err(DbError::NotImplemented("v0.1"))
    }

    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
    pub fn update_account(&mut self, _account: &Account) -> Result<()> {
        Err(DbError::NotImplemented("v0.1"))
    }

    // --- deals -----------------------------------------------------------------

    /// Writes a batch in one transaction; deals are keyed by (account, ticket).
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
    pub fn upsert_batch(&mut self, _batch: &DealBatch, _source: &str) -> Result<UpsertReport> {
        Err(DbError::NotImplemented("v0.1"))
    }

    /// The account's deals, or every account's when `None`.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
    pub fn deals(&self, _account_id: Option<i64>) -> Result<Vec<(i64, Deal)>> {
        Err(DbError::NotImplemented("v0.1"))
    }

    /// Stores a hand-entered trade as an entry and an exit deal with negative tickets.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
    pub fn add_manual_trade(&mut self, _account_id: i64, _trade: &ManualTrade) -> Result<TradeKey> {
        Err(DbError::NotImplemented("v0.1"))
    }

    // --- journal, rules, checklist, day notes -------------------------------

    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
    pub fn journal(&self, _key: &TradeKey) -> Result<Option<JournalEntry>> {
        Err(DbError::NotImplemented("v0.1"))
    }

    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
    pub fn journals(&self, _account_id: Option<i64>) -> Result<Vec<(TradeKey, JournalEntry)>> {
        Err(DbError::NotImplemented("v0.1"))
    }

    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
    pub fn save_journal(&mut self, _key: &TradeKey, _entry: &JournalEntry) -> Result<()> {
        Err(DbError::NotImplemented("v0.1: also append to the cloud changelog"))
    }

    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
    pub fn rules(&self, _account_id: i64) -> Result<Rules> {
        Err(DbError::NotImplemented("v0.1"))
    }

    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
    pub fn save_rules(&mut self, _account_id: i64, _rules: &Rules) -> Result<()> {
        Err(DbError::NotImplemented("v0.1"))
    }

    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
    pub fn checklist(&self, _account_id: Option<i64>) -> Result<Vec<ChecklistItem>> {
        Err(DbError::NotImplemented("v0.1"))
    }

    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
    pub fn save_checklist(&mut self, _account_id: Option<i64>, _items: &[ChecklistItem]) -> Result<()> {
        Err(DbError::NotImplemented("v0.1"))
    }

    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
    pub fn day_note(&self, _account_id: i64, _day: chrono::NaiveDate) -> Result<DayNote> {
        Err(DbError::NotImplemented("v0.1"))
    }

    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
    pub fn save_day_note(&mut self, _account_id: i64, _day: chrono::NaiveDate, _note: &DayNote) -> Result<()> {
        Err(DbError::NotImplemented("v0.1"))
    }

    // --- the trader's catalogs and fields -----------------------------------

    /// The options of `kind`, archived ones last, each group in the trader's order.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the query fails.
    pub fn catalog(&self, kind: CatalogKind) -> Result<Vec<CatalogItem>> {
        let mut stmt = self.conn.prepare(
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

    /// Adds an option (`item.id` 0) or changes one; returns its id. Keys are unique per kind.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the key is taken or the write fails.
    pub fn save_catalog_item(&mut self, item: &CatalogItem) -> Result<i64> {
        let kind = kind_name(item.kind)?;
        if item.id == 0 {
            self.conn.execute(
                "INSERT INTO catalog_items (kind, key, label, color, position, archived) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![kind, item.key, item.label, item.color, item.position, item.archived],
            )?;
            Ok(self.conn.last_insert_rowid())
        } else {
            self.conn.execute(
                "UPDATE catalog_items SET label = ?2, color = ?3, position = ?4, archived = ?5 WHERE id = ?1",
                rusqlite::params![item.id, item.label, item.color, item.position, item.archived],
            )?;
            Ok(item.id)
        }
    }

    /// The fields the trader defined for `scope`, in their order.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the query fails or a stored definition does not parse.
    pub fn field_defs(&self, scope: FieldScope) -> Result<Vec<FieldDef>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, def_json FROM field_defs WHERE scope = ?1 ORDER BY archived, position, id")?;
        let rows = stmt.query_map([scope_name(scope)?], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        rows.map(|row| {
            let (id, json) = row?;
            let mut def: FieldDef = serde_json::from_str(&json)?;
            def.id = id;
            Ok(def)
        })
        .collect()
    }

    /// Adds a field (`def.id` 0) or changes one; returns its id.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the key is taken in its scope or the write fails.
    pub fn save_field_def(&mut self, def: &FieldDef) -> Result<i64> {
        let json = serde_json::to_string(def)?;
        let scope = scope_name(def.scope)?;
        if def.id == 0 {
            self.conn.execute(
                "INSERT INTO field_defs (scope, key, def_json, position, archived) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![scope, def.key, json, def.position, def.archived],
            )?;
            Ok(self.conn.last_insert_rowid())
        } else {
            self.conn.execute(
                "UPDATE field_defs SET def_json = ?2, position = ?3, archived = ?4 WHERE id = ?1",
                rusqlite::params![def.id, json, def.position, def.archived],
            )?;
            Ok(def.id)
        }
    }

    // --- playbook -------------------------------------------------------------

    /// The setups, archived ones last.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the query fails or a stored setup does not parse.
    pub fn playbooks(&self) -> Result<Vec<Playbook>> {
        let mut stmt = self.conn.prepare("SELECT id, json FROM playbooks ORDER BY archived, position, id")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        rows.map(|row| {
            let (id, json) = row?;
            let mut playbook: Playbook = serde_json::from_str(&json)?;
            playbook.id = id;
            Ok(playbook)
        })
        .collect()
    }

    /// Adds a setup (`id` 0) or changes one; returns its id. Names are unique.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the name is taken or the write fails.
    pub fn save_playbook(&mut self, playbook: &Playbook, position: i32) -> Result<i64> {
        let json = serde_json::to_string(playbook)?;
        if playbook.id == 0 {
            self.conn.execute(
                "INSERT INTO playbooks (name, json, position, archived) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![playbook.name, json, position, playbook.archived],
            )?;
            Ok(self.conn.last_insert_rowid())
        } else {
            self.conn.execute(
                "UPDATE playbooks SET name = ?2, json = ?3, position = ?4, archived = ?5 WHERE id = ?1",
                rusqlite::params![playbook.id, playbook.name, json, position, playbook.archived],
            )?;
            Ok(playbook.id)
        }
    }

    // --- missed trades --------------------------------------------------------

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
        let json = serde_json::to_string(missed)?;
        if missed.id == 0 {
            self.conn.execute(
                "INSERT INTO missed_trades (account_id, seen_at, json) VALUES (?1, ?2, ?3)",
                rusqlite::params![missed.account_id, missed.seen_at, json],
            )?;
            Ok(self.conn.last_insert_rowid())
        } else {
            self.conn.execute(
                "UPDATE missed_trades SET account_id = ?2, seen_at = ?3, json = ?4 WHERE id = ?1",
                rusqlite::params![missed.id, missed.account_id, missed.seen_at, json],
            )?;
            Ok(missed.id)
        }
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
        use rusqlite::OptionalExtension;
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
        self.conn.execute(
            "INSERT INTO reviews (account_id, period, start, json) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(account_id, period, start) DO UPDATE SET json = excluded.json",
            rusqlite::params![
                review.account_id.unwrap_or(0),
                enum_name(&review.period)?,
                review.start.to_string(),
                serde_json::to_string(review)?
            ],
        )?;
        Ok(self.conn.query_row(
            "SELECT id FROM reviews WHERE account_id = ?1 AND period = ?2 AND start = ?3",
            rusqlite::params![review.account_id.unwrap_or(0), enum_name(&review.period)?, review.start.to_string()],
            |r| r.get(0),
        )?)
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

    /// Indexes `doc`, replacing what was indexed for the same thing. Called by every
    /// write of a journal, day note, review, missed trade or setup.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the write fails.
    pub fn index_document(&mut self, doc: &Document) -> Result<()> {
        let kind = enum_name(&doc.kind)?;
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM search_index WHERE kind = ?1 AND reference = ?2", [&kind, &doc.reference])?;
        if !doc.body.trim().is_empty() || !doc.title.trim().is_empty() {
            tx.execute(
                "INSERT INTO search_index (kind, reference, title, body) VALUES (?1, ?2, ?3, ?4)",
                [&kind, &doc.reference, &doc.title, &doc.body],
            )?;
        }
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
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        Ok(self.conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails; [`DbError::NotImplemented`] until v0.1.
    pub fn set_setting(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }
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
        assert!(db.field_defs(FieldScope::Day).unwrap().is_empty());
    }
}
