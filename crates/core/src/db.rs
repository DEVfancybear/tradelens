//! SQLite storage. One file per user; deals are the source of truth and trades are
//! rebuilt from them. WAL mode lets the bridge thread and the UI write at once.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::model::{
    Account, AccountInfo, ChecklistItem, DayNote, Deal, DealBatch, JournalEntry, ManualTrade, Rules, TradeKey,
};

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
}
