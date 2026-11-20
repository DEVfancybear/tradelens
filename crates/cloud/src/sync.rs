//! Sync: every local change is a row of the `changes` table, numbered per device.
//! A sync pushes this device's new rows as one file, pulls the other devices' new
//! files and applies them. Deals merge by ticket, so they never conflict; journal
//! entries and settings take the newest value per field. A snapshot of the whole
//! database is kept weekly, for restoring onto a new machine.

use serde::{Deserialize, Serialize};

use crate::drive::CloudDrive;

/// One change, as stored in `changes` and in the pushed files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub device_id: String,
    pub seq: u64,
    /// Unix milliseconds.
    pub at: i64,
    /// `account`, `deal`, `journal`, `rules`, `checklist`, `day_note`, `setting`.
    pub entity: String,
    pub key: String,
    /// The new value; `None` deletes.
    pub value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SyncReport {
    pub pushed: usize,
    pub pulled: usize,
    pub conflicts_resolved: usize,
}

pub struct SyncEngine<'drive> {
    pub drive: &'drive dyn CloudDrive,
    pub device_id: String,
}

impl std::fmt::Debug for SyncEngine<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncEngine").field("device_id", &self.device_id).finish_non_exhaustive()
    }
}

impl SyncEngine<'_> {
    /// Push, then pull. Safe to run at any time; an interrupted sync resumes.
    ///
    /// # Errors
    ///
    /// [`crate::CloudError`] when the provider, the network or the store fails.
    pub fn sync(&self, _db: &mut tradelens_core::Db) -> crate::Result<SyncReport> {
        Err(crate::CloudError::NotImplemented("v0.1"))
    }

    /// Uploads a snapshot of the database.
    ///
    /// # Errors
    ///
    /// [`crate::CloudError`] when the provider, the network or the store fails.
    pub fn snapshot(&self, _db: &tradelens_core::Db) -> crate::Result<()> {
        Err(crate::CloudError::NotImplemented("v0.2"))
    }

    /// Replaces the local database with the newest snapshot plus later changes.
    ///
    /// # Errors
    ///
    /// [`crate::CloudError`] when the provider, the network or the store fails.
    pub fn restore(&self, _db_path: &std::path::Path) -> crate::Result<SyncReport> {
        Err(crate::CloudError::NotImplemented("v0.1"))
    }
}
