//! Sync: every local change is a row of the `changes` table, numbered per device. A sync
//! uploads this device's new rows as files, downloads the other devices' new files and applies
//! them (the merge rules are the core's: `tradelens_core::sync`), moves the images, and keeps a
//! weekly snapshot of the whole database for restoring onto a new machine.
//!
//! The app folder holds (see `docs/cloud-sync.md`):
//!
//! ```text
//! changes/<device>/<seq_from>-<seq_to>.json   one device's changes, numbered
//! snapshots/<YYYY-MM-DD>.db                   the whole database, weekly
//! attachments/<sha256>.<ext>                  chart images, named by content
//! devices.json                                the devices that sync, and when they last did
//! ```

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tradelens_core::Db;

pub use tradelens_core::sync::Change;

use crate::drive::{CloudDrive, RemoteFile};
use crate::{CloudError, Result};

/// Changes per uploaded file: a first sync of years of deals is several files, not one huge one.
const CHANGES_PER_FILE: usize = 500;

/// A snapshot is due when the newest is this many days old.
const SNAPSHOT_EVERY_DAYS: i64 = 7;

const CHANGES_DIR: &str = "changes/";
const SNAPSHOTS_DIR: &str = "snapshots/";
const ATTACHMENTS_DIR: &str = "attachments/";
const DEVICES_FILE: &str = "devices.json";

/// What a sync did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SyncReport {
    /// Changes of this device uploaded.
    pub pushed: usize,
    /// Changes of other devices downloaded and applied.
    pub pulled: usize,
    /// Fields that two devices had written differently, which the newest write settled.
    pub conflicts_resolved: usize,
    /// Images uploaded.
    pub attachments_uploaded: usize,
    /// Images downloaded.
    pub attachments_downloaded: usize,
    /// Whether this sync also uploaded the weekly snapshot.
    pub snapshot_taken: bool,
}

/// A device that syncs, as `devices.json` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// The id the device's changes are filed under.
    pub id: String,
    /// What the trader called it.
    pub name: String,
    /// Unix milliseconds of its last sync.
    pub last_sync: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Devices {
    #[serde(default)]
    devices: Vec<DeviceInfo>,
}

/// Syncs one database with a cloud drive.
pub struct SyncEngine<'drive> {
    drive: &'drive dyn CloudDrive,
    provider: String,
    device_name: String,
    attachments: Option<PathBuf>,
    now: fn() -> i64,
}

impl fmt::Debug for SyncEngine<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyncEngine")
            .field("provider", &self.provider)
            .field("device_name", &self.device_name)
            .field("attachments", &self.attachments)
            .finish_non_exhaustive()
    }
}

fn system_now() -> i64 {
    Utc::now().timestamp_millis()
}

impl<'drive> SyncEngine<'drive> {
    /// An engine for `drive`. `provider` names the account the drive belongs to (`Google`,
    /// `Microsoft`): the progress of a sync is kept per provider, so that signing in elsewhere
    /// starts afresh. `device_name` is what `devices.json` shows for this machine.
    pub fn new(drive: &'drive dyn CloudDrive, provider: impl Into<String>, device_name: impl Into<String>) -> Self {
        Self { drive, provider: provider.into(), device_name: device_name.into(), attachments: None, now: system_now }
    }

    /// Syncs the images too: `folder` is the data folder the journal's image paths are relative to.
    #[must_use]
    pub fn with_attachments(mut self, folder: impl Into<PathBuf>) -> Self {
        self.attachments = Some(folder.into());
        self
    }

    /// Reads the time from `now` (Unix milliseconds) instead of the system, for tests.
    #[must_use]
    pub fn with_clock(mut self, now: fn() -> i64) -> Self {
        self.now = now;
        self
    }

    /// Uploads this device's new changes and images, downloads and applies the others', fetches
    /// the images that are missing and, when the last snapshot is a week old, uploads a new one.
    /// Safe to run at any time: an interrupted sync resumes where it stopped, and a change that
    /// arrives twice is applied once.
    ///
    /// # Errors
    ///
    /// [`CloudError`] when the provider, the network or the database fails.
    pub fn sync(&self, db: &mut Db) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        db.log_existing()?;
        self.upload_attachments(db, &mut report)?;
        self.push(db, &mut report)?;
        self.pull(db, &mut report)?;
        self.download_attachments(db, &mut report)?;
        if self.snapshot_due()? {
            self.snapshot(db)?;
            report.snapshot_taken = true;
        }
        self.publish_device(db)?;
        db.mark_synced(&self.provider, (self.now)())?;
        Ok(report)
    }

    /// Uploads a snapshot of the database as `snapshots/<today>.db`, replacing today's if any.
    ///
    /// # Errors
    ///
    /// [`CloudError`] when the provider, the network or the database fails.
    pub fn snapshot(&self, db: &Db) -> Result<()> {
        // A name no other snapshot in this process (or another) is being written to: the clock alone
        // does not tell two snapshots taken in the same tick apart.
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let number = COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary =
            std::env::temp_dir().join(format!("tradelens-snapshot-{}-{nanos}-{number}.db", std::process::id()));
        let copied = db.snapshot_to(&temporary);
        let bytes = copied.map_err(CloudError::from).and_then(|()| Ok(std::fs::read(&temporary)?));
        // Whatever happened, the copy does not stay behind.
        drop(std::fs::remove_file(&temporary));
        let name = format!("{SNAPSHOTS_DIR}{}.db", self.today().format("%Y-%m-%d"));
        self.drive.upload(&name, &bytes?)?;
        Ok(())
    }

    /// Restores the newest snapshot as the database at `db_path` (replacing what is there, which
    /// must not be open), makes it a device of its own (a new device id), and syncs: the changes
    /// made since the snapshot, and the images, come down. Returns the opened database.
    ///
    /// # Errors
    ///
    /// [`CloudError::NoSnapshot`] when the drive has none; otherwise [`CloudError`] when the
    /// provider, the network, the files or the database fail.
    pub fn restore(&self, db_path: &Path) -> Result<(Db, SyncReport)> {
        let newest = self
            .snapshots()?
            .into_iter()
            .max_by_key(|(date, _)| *date)
            .map(|(_, file)| file)
            .ok_or(CloudError::NoSnapshot)?;
        let bytes = self.drive.download(&newest)?;
        if let Some(folder) = db_path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        // Another database's leftovers (its write-ahead log) must not be replayed onto this one.
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = db_path.as_os_str().to_owned();
            sidecar.push(suffix);
            drop(std::fs::remove_file(PathBuf::from(sidecar)));
        }
        std::fs::write(db_path, &bytes)?;
        let mut db = Db::open(db_path)?;
        db.restored_as_new_device(&self.provider)?;
        let report = self.sync(&mut db)?;
        Ok((db, report))
    }

    /// The devices that sync with this drive, as `devices.json` lists them.
    ///
    /// # Errors
    ///
    /// [`CloudError`] when the provider or the network fails.
    pub fn devices(&self) -> Result<Vec<DeviceInfo>> {
        Ok(self.read_devices()?.devices)
    }

    // --- changes ---------------------------------------------------------------------

    fn push(&self, db: &mut Db, report: &mut SyncReport) -> Result<()> {
        let device = db.device_id()?;
        let mut sent = db.sync_state(&self.provider)?.pushed_seq;
        loop {
            let batch = db.pending_changes(sent, CHANGES_PER_FILE)?;
            let (Some(first), Some(last)) = (batch.first(), batch.last()) else { return Ok(()) };
            let (from, to) = (first.seq, last.seq);
            let bytes = serde_json::to_vec(&batch).map_err(|e| CloudError::Corrupt(e.to_string()))?;
            self.drive.upload(&format!("{CHANGES_DIR}{device}/{from}-{to}.json"), &bytes)?;
            db.mark_pushed(&self.provider, to)?;
            report.pushed += batch.len();
            sent = to;
        }
    }

    fn pull(&self, db: &mut Db, report: &mut SyncReport) -> Result<()> {
        let own = db.device_id()?;
        let state = db.sync_state(&self.provider)?;
        let mut files: BTreeMap<String, Vec<(u64, u64, RemoteFile)>> = BTreeMap::new();
        for file in self.drive.list(CHANGES_DIR)? {
            if let Some((device, from, to)) = parse_changes_name(&file.name)
                && device != own
            {
                files.entry(device).or_default().push((from, to, file));
            }
        }
        let mut changes = Vec::new();
        let mut cursors = BTreeMap::new();
        for (device, mut list) in files {
            list.sort_by_key(|(from, to, _)| (*from, *to));
            let first = state.pulled.get(&device).copied().unwrap_or(0) + 1;
            let mut next = first;
            'files: for (from, _, file) in list.iter().filter(|(_, to, _)| *to >= first) {
                if *from > next {
                    log::warn!("sync: thiếu thay đổi {next}..{from} của thiết bị {device}, dừng ở đó");
                    break;
                }
                let bytes = self.drive.download(file)?;
                let mut batch: Vec<Change> =
                    serde_json::from_slice(&bytes).map_err(|e| CloudError::Corrupt(format!("{}: {e}", file.name)))?;
                batch.sort_by_key(|change| change.seq);
                for change in batch {
                    if change.device_id != device || change.seq < next {
                        continue;
                    }
                    if change.seq > next {
                        log::warn!("sync: thiếu thay đổi {next} của thiết bị {device} trong {}", file.name);
                        break 'files;
                    }
                    next += 1;
                    changes.push(change);
                }
            }
            if next > first {
                cursors.insert(device, next - 1);
            }
        }
        if cursors.is_empty() {
            return Ok(());
        }
        let applied = db.apply_remote(&self.provider, &changes, &cursors)?;
        report.pulled += changes.len();
        report.conflicts_resolved += applied.conflicts;
        Ok(())
    }

    // --- snapshots and devices -----------------------------------------------------------

    fn today(&self) -> NaiveDate {
        DateTime::from_timestamp_millis((self.now)()).unwrap_or_default().date_naive()
    }

    /// The snapshots on the drive, with the day each was taken.
    fn snapshots(&self) -> Result<Vec<(NaiveDate, RemoteFile)>> {
        Ok(self
            .drive
            .list(SNAPSHOTS_DIR)?
            .into_iter()
            .filter_map(|file| {
                let day = file.name.strip_prefix(SNAPSHOTS_DIR)?.strip_suffix(".db")?;
                Some((NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?, file))
            })
            .collect())
    }

    fn snapshot_due(&self) -> Result<bool> {
        let newest = self.snapshots()?.into_iter().map(|(day, _)| day).max();
        Ok(newest.is_none_or(|day| self.today() - day >= Duration::days(SNAPSHOT_EVERY_DAYS)))
    }

    fn read_devices(&self) -> Result<Devices> {
        let Some(file) = self.drive.list(DEVICES_FILE)?.into_iter().find(|file| file.name == DEVICES_FILE) else {
            return Ok(Devices::default());
        };
        // A damaged list is rebuilt by the devices as they sync, so it is not worth failing over.
        Ok(serde_json::from_slice(&self.drive.download(&file)?).unwrap_or_default())
    }

    fn publish_device(&self, db: &Db) -> Result<()> {
        let id = db.device_id()?;
        let mut list = self.read_devices()?;
        list.devices.retain(|device| device.id != id);
        list.devices.push(DeviceInfo { id, name: self.device_name.clone(), last_sync: (self.now)() });
        list.devices.sort_by(|a, b| a.id.cmp(&b.id));
        let bytes = serde_json::to_vec_pretty(&list).map_err(|e| CloudError::Corrupt(e.to_string()))?;
        self.drive.upload(DEVICES_FILE, &bytes)?;
        Ok(())
    }

    // --- images -------------------------------------------------------------------------------

    fn remote_images(&self) -> Result<BTreeMap<String, RemoteFile>> {
        Ok(self.drive.list(ATTACHMENTS_DIR)?.into_iter().map(|file| (file.name.clone(), file)).collect())
    }

    fn upload_attachments(&self, db: &mut Db, report: &mut SyncReport) -> Result<()> {
        let Some(folder) = &self.attachments else { return Ok(()) };
        let mut remote: Option<HashSet<String>> = None;
        for path in db.attachment_paths()? {
            let Some(local) = inside(folder, &path) else { continue };
            if !local.is_file() {
                continue;
            }
            let known = db.attachment_sha(&path)?;
            let bytes = std::fs::read(&local)?;
            let sha = if let Some(sha) = known {
                sha
            } else {
                let sha = sha256_hex(&bytes);
                db.record_attachment(&path, &sha)?;
                sha
            };
            let name = image_name(&sha, &path);
            if remote.is_none() {
                remote = Some(self.remote_images()?.into_keys().collect());
            }
            if remote.as_ref().is_some_and(|names| !names.contains(&name)) {
                self.drive.upload(&name, &bytes)?;
                report.attachments_uploaded += 1;
            }
        }
        Ok(())
    }

    fn download_attachments(&self, db: &Db, report: &mut SyncReport) -> Result<()> {
        let Some(folder) = &self.attachments else { return Ok(()) };
        let mut remote: Option<BTreeMap<String, RemoteFile>> = None;
        for path in db.attachment_paths()? {
            let Some(local) = inside(folder, &path) else { continue };
            if local.exists() {
                continue;
            }
            let Some(sha) = db.attachment_sha(&path)? else { continue };
            if remote.is_none() {
                remote = Some(self.remote_images()?);
            }
            let Some(file) = remote.as_ref().and_then(|files| files.get(&image_name(&sha, &path))) else { continue };
            let bytes = self.drive.download(file)?;
            if sha256_hex(&bytes) != sha {
                log::warn!("sync: ảnh {path} tải về không khớp mã băm, bỏ qua");
                continue;
            }
            if let Some(parent) = local.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&local, &bytes)?;
            report.attachments_downloaded += 1;
        }
        Ok(())
    }
}

/// `(device, from, to)` of `changes/<device>/<from>-<to>.json`.
fn parse_changes_name(name: &str) -> Option<(String, u64, u64)> {
    let rest = name.strip_prefix(CHANGES_DIR)?;
    let (device, file) = rest.split_once('/')?;
    let (from, to) = file.strip_suffix(".json")?.split_once('-')?;
    (!device.is_empty()).then(|| Some((device.to_owned(), from.parse().ok()?, to.parse().ok()?)))?
}

/// The file `path` (as a journal entry names it) inside `folder`; `None` when it points outside
/// (an absolute path, or `..`), which only a damaged or hostile change would hold.
fn inside(folder: &Path, path: &str) -> Option<PathBuf> {
    let relative = Path::new(path);
    (!path.is_empty() && relative.components().all(|part| matches!(part, Component::Normal(_))))
        .then(|| folder.join(relative))
}

/// `attachments/<sha256>.<extension of the image>`.
fn image_name(sha256: &str, path: &str) -> String {
    let extension = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or_else(|| "bin".to_owned());
    format!("{ATTACHMENTS_DIR}{sha256}.{extension}")
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes).iter().fold(String::with_capacity(64), |mut hex, byte| {
        write!(hex, "{byte:02x}").expect("writing to a String cannot fail");
        hex
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_files_are_named_by_device_and_range() {
        assert_eq!(parse_changes_name("changes/abc/1-500.json"), Some(("abc".into(), 1, 500)));
        assert_eq!(parse_changes_name("changes/abc/x-5.json"), None);
        assert_eq!(parse_changes_name("changes/abc.json"), None);
        assert_eq!(parse_changes_name("snapshots/2026-11-23.db"), None);
    }

    #[test]
    fn image_paths_cannot_leave_the_data_folder() {
        let folder = Path::new("data");
        assert_eq!(inside(folder, "images/ab.png"), Some(folder.join("images/ab.png")));
        for hostile in ["../x.png", "/etc/passwd", "images/../../x", "", "C:\\x.png"] {
            let found = inside(folder, hostile);
            assert!(
                found.as_deref().is_none_or(|p| p.starts_with(folder) && !p.to_string_lossy().contains("..")),
                "{hostile}"
            );
        }
        assert_eq!(inside(folder, "../x.png"), None);
        assert_eq!(inside(folder, "/etc/passwd"), None);
    }

    #[test]
    fn images_are_named_by_content() {
        let sha = sha256_hex(b"abc");
        assert_eq!(sha, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(image_name(&sha, "images/ab.PNG"), format!("attachments/{sha}.png"));
        assert_eq!(image_name(&sha, "images/ab"), format!("attachments/{sha}.bin"));
    }
}
