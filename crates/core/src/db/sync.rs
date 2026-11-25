//! Sync inside the database: logging what the trader writes, applying what other devices
//! wrote, and the bookkeeping between the two. The rules are in `docs/cloud-sync.md`.
//!
//! A local write goes through [`logged`]: the entity is read before and after, and the fields
//! that differ become one row of `changes`, stamped later than anything the entity has seen.
//! Applying a change ([`Db::apply_changes`]) merges it field by field into the tables with the
//! raw operations of [`views`], so it is never logged again.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde_json::Value;

use crate::model::{Deal, Origin};
use crate::sync::{
    Body, Change, FieldOp, Fields, Stamp, Winner, apply_op, diff, entity, is_synced_setting, merge_deal, new_uid,
};

use super::{DEVICE_ID_KEY, Db, DbError, Result, append_change, device_id, now_ms, set_setting_on, setting_on, views};

/// The setting that says the data that existed before sync was logged.
const EXPORTED_KEY: &str = "sync.exported";

/// What applying changes did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ApplyReport {
    /// Fields (or whole deals) whose value changed to the incoming one.
    pub applied: usize,
    /// Operations that lost to a newer write already held, or were already applied.
    pub stale: usize,
    /// Operations on a field that another device had written differently: concurrent edits
    /// the stamps resolved.
    pub conflicts: usize,
    /// Entities removed.
    pub deleted: usize,
    /// Deleted entities an edit newer than the deletion brought back.
    pub revived: usize,
}

impl ApplyReport {
    fn add(&mut self, other: Self) {
        self.applied += other.applied;
        self.stale += other.stale;
        self.conflicts += other.conflicts;
        self.deleted += other.deleted;
        self.revived += other.revived;
    }
}

/// How far a device has pushed and pulled, per cloud provider.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncState {
    /// The last of this device's changes that is in the cloud.
    pub pushed_seq: u64,
    /// Per other device, the last of its changes that has been applied here.
    pub pulled: BTreeMap<String, u64>,
    /// Unix milliseconds of the last complete sync.
    pub last_sync: Option<i64>,
}

/// The synced content of a database: entity kind, then key, then fields.
pub type PortableState = BTreeMap<String, BTreeMap<String, Fields>>;

fn to_u64(n: i64) -> Result<u64> {
    u64::try_from(n).map_err(|_| DbError::Invalid(format!("số thứ tự âm: {n}")))
}

fn to_i64(n: u64) -> Result<i64> {
    i64::try_from(n).map_err(|_| DbError::Invalid(format!("số thứ tự quá lớn: {n}")))
}

// --- clocks and tombstones ---------------------------------------------------------------

fn clock_of(conn: &Connection, kind: &str, key: &str, field: &str) -> Result<Option<Stamp>> {
    Ok(conn
        .prepare_cached("SELECT at, device_id FROM sync_clock WHERE entity = ?1 AND key = ?2 AND field = ?3")?
        .query_row([kind, key, field], |r| Ok(Stamp { at: r.get(0)?, device: r.get(1)? }))
        .optional()?)
}

fn set_clock(conn: &Connection, kind: &str, key: &str, field: &str, stamp: &Stamp) -> Result<()> {
    conn.prepare_cached(
        "INSERT OR REPLACE INTO sync_clock (entity, key, field, at, device_id) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?
    .execute(rusqlite::params![kind, key, field, stamp.at, stamp.device])?;
    Ok(())
}

/// The newest write any field of the entity has seen.
fn newest_field(conn: &Connection, kind: &str, key: &str) -> Result<Option<Stamp>> {
    Ok(conn
        .prepare_cached(
            "SELECT at, device_id FROM sync_clock WHERE entity = ?1 AND key = ?2
             ORDER BY at DESC, device_id DESC LIMIT 1",
        )?
        .query_row([kind, key], |r| Ok(Stamp { at: r.get(0)?, device: r.get(1)? }))
        .optional()?)
}

struct Tombstone {
    stamp: Stamp,
    data: Fields,
}

fn tombstone(conn: &Connection, kind: &str, key: &str) -> Result<Option<Tombstone>> {
    let row = conn
        .query_row("SELECT at, device_id, data FROM sync_tombstones WHERE entity = ?1 AND key = ?2", [kind, key], |r| {
            Ok((Stamp { at: r.get(0)?, device: r.get(1)? }, r.get::<_, String>(2)?))
        })
        .optional()?;
    row.map(|(stamp, data)| Ok(Tombstone { stamp, data: serde_json::from_str(&data)? })).transpose()
}

// --- logging local writes ----------------------------------------------------------------

/// The time a local write of the entity is stamped with: now, but later than anything the
/// entity has seen, so that it wins on every device even when this device's clock runs behind.
fn next_at(conn: &Connection, kind: &str, key: &str) -> Result<i64> {
    let newest = newest_field(conn, kind, key)?.map(|s| s.at);
    let deleted = tombstone(conn, kind, key)?.map(|t| t.stamp.at);
    let floor = newest.into_iter().chain(deleted).max().map_or(i64::MIN, |at| at.saturating_add(1));
    Ok(now_ms().max(floor))
}

/// Records `ops` on the entity at `at` (or at [`next_at`]): the clocks and the log row.
fn log_ops(conn: &Connection, kind: &str, key: &str, ops: &[FieldOp], at: Option<i64>) -> Result<()> {
    if ops.is_empty() {
        return Ok(());
    }
    let at = match at {
        Some(at) => at,
        None => next_at(conn, kind, key)?,
    };
    let stamp = Stamp { at, device: device_id(conn)? };
    for op in ops {
        set_clock(conn, kind, key, &op.field, &stamp)?;
    }
    append_change(conn, at, kind, key, Some(&serde_json::to_string(ops)?))
}

/// Records what a local write did to the entity: the fields that differ between `before` and
/// `after`, or its deletion when `after` is `None`.
fn record(conn: &Connection, kind: &str, key: &str, before: Option<&Fields>, after: Option<&Fields>) -> Result<()> {
    match (before, after) {
        (_, Some(after)) => log_ops(conn, kind, key, &diff(before.unwrap_or(&Fields::new()), after), None),
        (Some(before), None) => {
            let at = next_at(conn, kind, key)?;
            let device = device_id(conn)?;
            conn.execute(
                "INSERT OR REPLACE INTO sync_tombstones (entity, key, at, device_id, data) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![kind, key, at, device, serde_json::to_string(before)?],
            )?;
            append_change(conn, at, kind, key, None)
        }
        (None, None) => Ok(()),
    }
}

/// Runs a local write of one entity and logs it. `known` is the entity's key when it is known
/// before the write (so what it held is read first); `write` returns the key it wrote.
pub(super) fn logged<T>(
    conn: &Connection,
    kind: &str,
    known: Option<&str>,
    write: impl FnOnce() -> Result<(String, T)>,
) -> Result<T> {
    let before = known.map(|key| views::load(conn, kind, key)).transpose()?.flatten();
    let (key, out) = write()?;
    let after = views::load(conn, kind, &key)?;
    record(conn, kind, &key, before.as_ref(), after.as_ref())?;
    Ok(out)
}

/// Runs the deletion of one entity and logs it.
pub(super) fn logged_delete(
    conn: &Connection,
    kind: &str,
    key: &str,
    delete: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let before = views::load(conn, kind, key)?;
    delete()?;
    if before.is_some() && views::load(conn, kind, key)?.is_none() {
        record(conn, kind, key, before.as_ref(), None)?;
    }
    Ok(())
}

/// Logs a setting the trader changed, unless it belongs to this device.
pub(super) fn log_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    if !is_synced_setting(key) {
        return set_setting_on(conn, key, value);
    }
    logged(conn, entity::SETTING, Some(key), || {
        set_setting_on(conn, key, value)?;
        Ok((key.to_owned(), ()))
    })
}

// --- applying remote changes -----------------------------------------------------------

fn apply_on(conn: &Connection, changes: &[Change]) -> Result<ApplyReport> {
    let own = device_id(conn)?;
    let rank = |kind: &str| entity::APPLY_ORDER.iter().position(|known| *known == kind);
    let mut ordered: Vec<(usize, &Change)> = Vec::with_capacity(changes.len());
    let mut report = ApplyReport::default();
    for change in changes.iter().filter(|c| c.device_id != own) {
        if let Some(rank) = rank(&change.entity) {
            ordered.push((rank, change));
        } else {
            log::warn!("sync: bỏ qua loại thay đổi chưa biết {:?}", change.entity);
            report.stale += 1;
        }
    }
    ordered.sort_by(|(ra, a), (rb, b)| (ra, a.at, &a.device_id, a.seq).cmp(&(rb, b.at, &b.device_id, b.seq)));
    for (_, change) in ordered {
        // Each change is applied or not at all: one that cannot be stored (damaged or hostile data)
        // is skipped, and must not leave half of itself behind or stop the others.
        conn.execute_batch("SAVEPOINT sync_change")?;
        match apply_one(conn, change) {
            Ok(applied) => {
                conn.execute_batch("RELEASE sync_change")?;
                report.add(applied);
            }
            Err(error) if is_bad_data(&error) => {
                conn.execute_batch("ROLLBACK TO sync_change; RELEASE sync_change")?;
                log::warn!("sync: bỏ qua thay đổi {}/{} không lưu được: {error}", change.entity, change.key);
                report.stale += 1;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(report)
}

/// Whether `error` comes from what the change says (it does not parse, or breaks a constraint)
/// rather than from the database failing.
fn is_bad_data(error: &DbError) -> bool {
    match error {
        DbError::Json(_) | DbError::Invalid(_) => true,
        DbError::Sqlite(rusqlite::Error::SqliteFailure(failure, _)) => {
            failure.code == rusqlite::ErrorCode::ConstraintViolation
        }
        DbError::Sqlite(_) | DbError::NotImplemented(_) => false,
    }
}

fn apply_one(conn: &Connection, change: &Change) -> Result<ApplyReport> {
    let body = match change.body() {
        Ok(body) => body,
        Err(error) => {
            log::warn!("sync: thay đổi {}/{} hỏng: {error}", change.entity, change.key);
            return Ok(ApplyReport { stale: 1, ..ApplyReport::default() });
        }
    };
    if change.entity == entity::SETTING && !is_synced_setting(&change.key) {
        return Ok(ApplyReport { stale: 1, ..ApplyReport::default() });
    }
    match body {
        Body::Delete => apply_delete(conn, change),
        Body::Fields(ops) if change.entity == entity::DEAL => apply_deal(conn, change, &ops),
        Body::Fields(ops) => apply_fields(conn, change, &ops),
    }
}

/// Merges field operations into the entity: each wins when its stamp is later than the one the
/// field holds, and none wins against the deletion of the entity unless it is later too.
fn apply_fields(conn: &Connection, change: &Change, ops: &[FieldOp]) -> Result<ApplyReport> {
    let (kind, key, stamp) = (change.entity.as_str(), change.key.as_str(), change.stamp());
    let mut report = ApplyReport::default();
    let deleted = tombstone(conn, kind, key)?;
    let loaded = views::load(conn, kind, key)?;
    let mut working = match (&loaded, &deleted) {
        (Some(fields), _) => fields.clone(),
        (None, Some(gone)) => gone.data.clone(),
        (None, None) => Fields::new(),
    };
    let mut won = false;
    for op in ops {
        if deleted.as_ref().is_some_and(|gone| stamp <= gone.stamp) {
            report.stale += 1;
            continue;
        }
        let held = clock_of(conn, kind, key, &op.field)?;
        // Another device wrote the field differently: a concurrent edit, whoever the stamps favour.
        if held.as_ref().is_some_and(|held| held.device != stamp.device) && working.get(&op.field) != op.value.as_ref()
        {
            report.conflicts += 1;
        }
        if !stamp.beats(held.as_ref()) {
            report.stale += 1;
            continue;
        }
        apply_op(&mut working, op);
        set_clock(conn, kind, key, &op.field, &stamp)?;
        report.applied += 1;
        won = true;
    }
    if won {
        if loaded.is_none() && deleted.is_some() {
            conn.execute("DELETE FROM sync_tombstones WHERE entity = ?1 AND key = ?2", [kind, key])?;
            report.revived += 1;
        }
        if loaded.as_ref() != Some(&working) && !(loaded.is_none() && working.is_empty()) {
            views::store(conn, kind, key, &working)?;
        }
    }
    Ok(report)
}

/// Whether the deal record was written by the trader (typed, or imported from their journal), not
/// by the terminal or one of its reports.
fn is_traders_own(record: &Value) -> bool {
    record.get("source").and_then(Value::as_str).is_some_and(|source| Origin::of_source(source).is_editable())
}

/// A deal is one record: the later write wins whole, and the measured figures it lacks come
/// from the other version (see [`merge_deal`]).
fn apply_deal(conn: &Connection, change: &Change, ops: &[FieldOp]) -> Result<ApplyReport> {
    let (key, stamp) = (change.key.as_str(), change.stamp());
    let mut report = ApplyReport::default();
    for op in ops.iter().filter(|op| op.field == "deal") {
        let Some(record) = &op.value else { continue };
        // A deal that was deleted stays deleted against a write older than its deletion, and
        // comes back with a newer one (a report imported again, a trade edited elsewhere).
        let gone = tombstone(conn, entity::DEAL, key)?;
        if gone.as_ref().is_some_and(|gone| stamp <= gone.stamp) {
            report.stale += 1;
            continue;
        }
        let held = clock_of(conn, entity::DEAL, key, "deal")?;
        let wins = stamp.beats(held.as_ref());
        let stored = views::load(conn, entity::DEAL, key)?;
        let merged = match stored.as_ref().and_then(|fields| fields.get("deal")) {
            None => record.clone(),
            // What the trader typed or kept in a journal is whole as written: a stop they cleared is
            // not filled back from the older version of the deal.
            Some(stored_record) if is_traders_own(if wins { record } else { stored_record }) => {
                if wins {
                    record.clone()
                } else {
                    stored_record.clone()
                }
            }
            Some(stored_record) => {
                let (stored_deal, incoming_deal): (Deal, Deal) =
                    (serde_json::from_value(stored_record.clone())?, serde_json::from_value(record.clone())?);
                let mut merged = serde_json::to_value(merge_deal(
                    &stored_deal,
                    &incoming_deal,
                    if wins { Winner::Incoming } else { Winner::Stored },
                ))?;
                let source = if wins { record } else { stored_record }.get("source").cloned().unwrap_or(Value::Null);
                if let Some(object) = merged.as_object_mut() {
                    object.insert("source".to_owned(), source);
                }
                merged
            }
        };
        if stored.as_ref().and_then(|fields| fields.get("deal")) != Some(&merged) {
            views::store(conn, entity::DEAL, key, &Fields::from([("deal".to_owned(), merged)]))?;
        }
        if stored.is_none() && gone.is_some() {
            conn.execute("DELETE FROM sync_tombstones WHERE entity = ?1 AND key = ?2", [entity::DEAL, key])?;
            report.revived += 1;
        }
        if wins {
            let differs = stored.as_ref().and_then(|fields| fields.get("deal")).is_some_and(|stored| stored != record);
            if differs && held.as_ref().is_some_and(|held| held.device != stamp.device) {
                report.conflicts += 1;
            }
            set_clock(conn, entity::DEAL, key, "deal", &stamp)?;
            report.applied += 1;
        } else {
            report.stale += 1;
        }
    }
    Ok(report)
}

/// A deletion wins over every edit older than it and loses to a newer one. The content is kept
/// in a tombstone, so that an edit newer than the deletion that arrives later brings the entity
/// back as it was.
fn apply_delete(conn: &Connection, change: &Change) -> Result<ApplyReport> {
    let (kind, key, stamp) = (change.entity.as_str(), change.key.as_str(), change.stamp());
    let stale = Ok(ApplyReport { stale: 1, ..ApplyReport::default() });
    let held = tombstone(conn, kind, key)?;
    if held.as_ref().is_some_and(|gone| stamp <= gone.stamp) {
        return stale;
    }
    if newest_field(conn, kind, key)?.is_some_and(|edit| edit >= stamp) {
        return stale;
    }
    let data = views::load(conn, kind, key)?.or(held.map(|gone| gone.data)).unwrap_or_default();
    conn.execute(
        "INSERT OR REPLACE INTO sync_tombstones (entity, key, at, device_id, data) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![kind, key, stamp.at, stamp.device, serde_json::to_string(&data)?],
    )?;
    views::remove(conn, kind, key)?;
    Ok(ApplyReport { deleted: 1, ..ApplyReport::default() })
}

// --- state kept per provider -----------------------------------------------------------

fn read_state(conn: &Connection, provider: &str) -> Result<SyncState> {
    let row = conn
        .query_row("SELECT pushed_seq, pulled_json, last_sync FROM cloud_state WHERE provider = ?1", [provider], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<i64>>(2)?))
        })
        .optional()?;
    let Some((pushed, pulled, last_sync)) = row else { return Ok(SyncState::default()) };
    Ok(SyncState { pushed_seq: to_u64(pushed)?, pulled: serde_json::from_str(&pulled)?, last_sync })
}

/// Makes the row of `provider`, leaving what the sign-in stored there alone.
fn ensure_state(conn: &Connection, provider: &str) -> Result<()> {
    conn.execute("INSERT OR IGNORE INTO cloud_state (provider) VALUES (?1)", [provider])?;
    Ok(())
}

fn write_pulled(conn: &Connection, provider: &str, pulled: &BTreeMap<String, u64>) -> Result<()> {
    ensure_state(conn, provider)?;
    conn.execute(
        "UPDATE cloud_state SET pulled_json = ?2 WHERE provider = ?1",
        rusqlite::params![provider, serde_json::to_string(pulled)?],
    )?;
    Ok(())
}

impl Db {
    /// This installation's id in the change log, made on first use.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the database fails.
    pub fn device_id(&self) -> Result<String> {
        device_id(&self.conn)
    }

    /// Logs what was written before changes were logged, once: every field that has no stamp
    /// yet gets one, as old as the data (journal entries remember when they were saved; the rest
    /// count as older than any edit made on another device). Returns the number of entities
    /// logged. Sync calls this before its first push; it does nothing afterwards.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the database fails.
    pub fn log_existing(&mut self) -> Result<usize> {
        if setting_on(&self.conn, EXPORTED_KEY)?.is_some() {
            return Ok(0);
        }
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut logged = 0;
        for kind in entity::APPLY_ORDER {
            for key in views::keys(&tx, kind)? {
                let Some(fields) = views::load(&tx, kind, &key)? else { continue };
                let stamped: BTreeSet<String> = {
                    let mut stmt = tx.prepare_cached("SELECT field FROM sync_clock WHERE entity = ?1 AND key = ?2")?;
                    stmt.query_map([kind, key.as_str()], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
                };
                let unstamped: Fields = fields.into_iter().filter(|(field, _)| !stamped.contains(field)).collect();
                if unstamped.is_empty() {
                    continue;
                }
                let at = views::legacy_time(&tx, kind, &key)?;
                log_ops(&tx, kind, &key, &diff(&Fields::new(), &unstamped), Some(at))?;
                logged += 1;
            }
        }
        set_setting_on(&tx, EXPORTED_KEY, "1")?;
        tx.commit()?;
        Ok(logged)
    }

    /// This device's changes after `after_seq`, oldest first, at most `limit`.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the query fails.
    pub fn pending_changes(&self, after_seq: u64, limit: usize) -> Result<Vec<Change>> {
        let device = device_id(&self.conn)?;
        let mut stmt = self.conn.prepare(
            "SELECT seq, at, entity, key, value FROM changes WHERE device_id = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![device, to_i64(after_seq)?, i64::try_from(limit).unwrap_or(i64::MAX)],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            },
        )?;
        rows.map(|row| {
            let (seq, at, entity, key, value) = row?;
            Ok(Change {
                device_id: device.clone(),
                seq: to_u64(seq)?,
                at,
                entity,
                key,
                value: value.map(|json| serde_json::from_str(&json)).transpose()?,
            })
        })
        .collect()
    }

    /// Applies changes made on other devices, in one transaction, without logging them again.
    /// Applying the same changes twice, or in another order, gives the same data.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the database fails, in which case nothing is applied.
    pub fn apply_changes(&mut self, changes: &[Change]) -> Result<ApplyReport> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let report = apply_on(&tx, changes)?;
        tx.commit()?;
        Ok(report)
    }

    /// [`Db::apply_changes`], and in the same transaction the cursors that say how far each
    /// device's changes have been applied, so that a crash in between repeats nothing and skips
    /// nothing.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the database fails, in which case nothing is applied.
    pub fn apply_remote(
        &mut self,
        provider: &str,
        changes: &[Change],
        cursors: &BTreeMap<String, u64>,
    ) -> Result<ApplyReport> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let report = apply_on(&tx, changes)?;
        let mut pulled = read_state(&tx, provider)?.pulled;
        for (device, seq) in cursors {
            let slot = pulled.entry(device.clone()).or_insert(0);
            *slot = (*slot).max(*seq);
        }
        write_pulled(&tx, provider, &pulled)?;
        tx.commit()?;
        Ok(report)
    }

    /// How far this device has pushed and pulled with `provider`.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the query fails.
    pub fn sync_state(&self, provider: &str) -> Result<SyncState> {
        read_state(&self.conn, provider)
    }

    /// Notes that this device's changes up to `seq` are in the cloud.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the write fails.
    pub fn mark_pushed(&mut self, provider: &str, seq: u64) -> Result<()> {
        ensure_state(&self.conn, provider)?;
        self.conn.execute(
            "UPDATE cloud_state SET pushed_seq = MAX(pushed_seq, ?2) WHERE provider = ?1",
            rusqlite::params![provider, to_i64(seq)?],
        )?;
        Ok(())
    }

    /// Notes the time of a complete sync.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the write fails.
    pub fn mark_synced(&mut self, provider: &str, at: i64) -> Result<()> {
        ensure_state(&self.conn, provider)?;
        self.conn
            .execute("UPDATE cloud_state SET last_sync = ?2 WHERE provider = ?1", rusqlite::params![provider, at])?;
        Ok(())
    }

    /// Writes a consistent copy of the whole database to `path`, which must not exist yet.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the copy fails, for instance because `path` exists.
    pub fn snapshot_to(&self, path: &Path) -> Result<()> {
        let Some(target) = path.to_str() else {
            return Err(DbError::Invalid(format!("đường dẫn không phải UTF-8: {}", path.display())));
        };
        self.conn.execute("VACUUM INTO ?1", [target])?;
        Ok(())
    }

    /// Makes a database restored from another device's snapshot a device of its own: a new
    /// device id, an empty log (the snapshot's log is the old device's, which pushes it itself),
    /// and cursors that say the old device's changes up to the snapshot are already in.
    /// Returns the old device's id.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the database fails.
    pub fn restored_as_new_device(&mut self, provider: &str) -> Result<String> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old = device_id(&tx)?;
        let last: Option<i64> =
            tx.query_row("SELECT MAX(seq) FROM changes WHERE device_id = ?1", [&old], |r| r.get(0))?;
        tx.execute("DELETE FROM changes", [])?;
        tx.execute("DELETE FROM cloud_state WHERE provider <> ?1", [provider])?;
        tx.execute("DELETE FROM sync_uploaded WHERE provider <> ?1", [provider])?;
        ensure_state(&tx, provider)?;
        let mut pulled = read_state(&tx, provider)?.pulled;
        let slot = pulled.entry(old.clone()).or_insert(0);
        *slot = (*slot).max(to_u64(last.unwrap_or(0))?);
        write_pulled(&tx, provider, &pulled)?;
        tx.execute("UPDATE cloud_state SET pushed_seq = 0, last_sync = NULL WHERE provider = ?1", [provider])?;
        tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [DEVICE_ID_KEY, new_uid().as_str()],
        )?;
        tx.commit()?;
        Ok(old)
    }

    /// The image paths that journal entries, missed trades and setups refer to.
    ///
    /// # Errors
    ///
    /// [`DbError`] when a read fails or a stored entry does not parse.
    pub fn attachment_paths(&self) -> Result<Vec<String>> {
        views::attachment_paths(&self.conn)
    }

    /// The content hash recorded for the image at `path`, if sync has seen it.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the query fails.
    pub fn attachment_sha(&self, path: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT sha256 FROM sync_attachments WHERE path = ?1", [path], |r| r.get(0))
            .optional()?)
    }

    /// Every image hash recorded, by the path the journal refers to the image with: one query for
    /// all the images, not one per image.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the query fails.
    pub fn attachment_shas(&self) -> Result<BTreeMap<String, String>> {
        let mut stmt = self.conn.prepare("SELECT path, sha256 FROM sync_attachments")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Records the SHA-256 of the image at `path` (hex), which names it in the cloud, and logs it
    /// so that the other devices know what to download.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the write fails.
    pub fn record_attachment(&mut self, path: &str, sha256: &str) -> Result<()> {
        self.record_attachments(&[(path.to_owned(), sha256.to_owned())])
    }

    /// [`Db::record_attachment`] for many images, in one transaction.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the write fails; none of the hashes is then recorded.
    pub fn record_attachments(&mut self, hashes: &[(String, String)]) -> Result<()> {
        if hashes.is_empty() {
            return Ok(());
        }
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (path, sha256) in hashes {
            logged(&tx, entity::ATTACHMENT, Some(path), || {
                views::store(
                    &tx,
                    entity::ATTACHMENT,
                    path,
                    &Fields::from([("sha256".to_owned(), Value::from(sha256.as_str()))]),
                )?;
                Ok((path.clone(), ()))
            })?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The names (`attachments/<sha256>.<ext>`) of the images known to be on the drive of `provider`.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the query fails.
    pub fn uploaded_images(&self, provider: &str) -> Result<BTreeSet<String>> {
        let mut stmt = self.conn.prepare("SELECT name FROM sync_uploaded WHERE provider = ?1")?;
        let rows = stmt.query_map([provider], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Notes that the images `names` are on the drive of `provider`, in one transaction. Nothing of
    /// the change log: every device finds out for itself.
    ///
    /// # Errors
    ///
    /// [`DbError::Sqlite`] when the write fails.
    pub fn mark_uploaded(&mut self, provider: &str, names: &[String]) -> Result<()> {
        if names.is_empty() {
            return Ok(());
        }
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            let mut insert = tx.prepare("INSERT OR IGNORE INTO sync_uploaded (provider, name) VALUES (?1, ?2)")?;
            for name in names {
                insert.execute([provider, name])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Everything that syncs, as portable fields: what two devices that have synced fully hold
    /// identically, whatever their row ids. Also for diagnosing a sync that did not converge.
    ///
    /// # Errors
    ///
    /// [`DbError`] when a read fails or a stored entry does not parse.
    pub fn portable_state(&self) -> Result<PortableState> {
        let mut state = PortableState::new();
        for kind in entity::APPLY_ORDER {
            let mut entities = BTreeMap::new();
            for key in views::keys(&self.conn, kind)? {
                if let Some(fields) = views::load(&self.conn, kind, &key)? {
                    entities.insert(key, fields);
                }
            }
            state.insert(kind.to_owned(), entities);
        }
        Ok(state)
    }
}
