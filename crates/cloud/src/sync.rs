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
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

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

/// Images uploaded at the same time: enough to hide each request's round trip, few enough to leave
/// the (often slow) uplink to the bytes.
const UPLOAD_WORKERS: usize = 4;

/// Images downloaded at the same time: a restore of hundreds is bound by the round trip of each
/// request, not by the bytes, so the downlink takes more.
const DOWNLOAD_WORKERS: usize = 8;

/// Images read and hashed at the same time on a first sync.
const HASH_WORKERS: usize = 4;

/// After this many images failed in one sync no new one starts; the rest wait for the next sync.
const MAX_IMAGE_FAILURES: usize = 5;

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
    /// Images that could not be moved this time (the link failed, a file would not read); the
    /// next sync tries them again. The journal is synced all the same.
    pub attachments_failed: usize,
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

    /// Uploads this device's new changes, downloads and applies the others', uploads a new
    /// snapshot when the last is a week old, and only then moves the images: the ones this device
    /// has and the drive lacks go up, the ones the journal names and this device lacks come down.
    /// The journal is small and what another device needs first, so it never waits behind images.
    /// Safe to run at any time: an interrupted sync resumes where it stopped, and a change that
    /// arrives twice is applied once. An image another device has not uploaded yet is fetched by
    /// a later sync.
    ///
    /// # Errors
    ///
    /// [`CloudError`] when the provider, the network or the database fails, or the session is
    /// gone. Images that cannot be moved are not an error: they are counted in
    /// [`SyncReport::attachments_failed`] and the next sync tries them again.
    pub fn sync(&self, db: &mut Db) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        db.log_existing()?;
        // Hashing is local and logs the `attachment` entity, so it must come before the push.
        let images = self.record_attachments(db)?;
        self.push(db, &mut report)?;
        self.pull(db, &mut report)?;
        if self.snapshot_due()? {
            self.snapshot(db)?;
            report.snapshot_taken = true;
        }
        self.publish_device(db)?;
        db.mark_synced(&self.provider, (self.now)())?;
        let not_sent = self.upload_attachments(db, images, &mut report)?;
        let not_fetched = self.download_attachments(db, &mut report)?;
        report.attachments_failed = not_sent + not_fetched;
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
    //
    // Only what changed moves, like the change files: the database knows each image's hash (so a
    // file is read once, ever) and which images the drive holds (`Db::uploaded_images`), so a
    // sync with nothing new reads no image and lists no folder of them. The database is used on
    // this thread only; the workers read files, hash them and talk to the drive.

    fn remote_images(&self) -> Result<BTreeMap<String, RemoteFile>> {
        Ok(self.drive.list(ATTACHMENTS_DIR)?.into_iter().map(|file| (file.name.clone(), file)).collect())
    }

    /// Hashes the images the journal names that are on this machine and have no hash yet, and
    /// writes the `attachment` entity (path to hash) to the change log, so that it goes out with
    /// the next push. Returns the images on this machine, one per
    /// file name. No byte leaves the machine here.
    fn record_attachments(&self, db: &mut Db) -> Result<Vec<LocalImage>> {
        let Some(folder) = &self.attachments else { return Ok(Vec::new()) };
        let known = db.attachment_shas()?;
        let mut candidates = Vec::new();
        let mut unhashed = Vec::new();
        for path in db.attachment_paths()? {
            let Some(local) = inside(folder, &path) else { continue };
            if !local.is_file() {
                continue;
            }
            match known.get(&path) {
                Some(sha) => candidates.push(LocalImage { name: image_name(sha, &path), local }),
                None => unhashed.push((path, local)),
            }
        }
        // Reading a local file says nothing about the link, so no failure stops the others.
        let pooled = run_pool(HASH_WORKERS, usize::MAX, unhashed, |(path, local)| match std::fs::read(&local) {
            Ok(bytes) => Ok(Some((path, local, sha256_hex(&bytes)))),
            // Deleted since it was listed: nothing to hash.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        });
        let mut hashed = pooled.done;
        hashed.sort();
        // The hashes that were read are kept even when another file could not be read; that one
        // (locked by another program, say) is tried again by the next sync and holds nothing back.
        let rows: Vec<(String, String)> = hashed.iter().map(|(path, _, sha)| (path.clone(), sha.clone())).collect();
        db.record_attachments(&rows)?;
        if let Some(error) = pooled.error {
            log::warn!("sync: có ảnh không đọc được, bỏ qua lần này: {error}");
        }
        candidates
            .extend(hashed.into_iter().map(|(path, local, sha)| LocalImage { name: image_name(&sha, &path), local }));

        // Two journal entries may hold the same picture: it goes up once.
        let mut named = HashSet::new();
        candidates.retain(|image| named.insert(image.name.clone()));
        Ok(candidates)
    }

    /// Uploads the images the drive does not hold yet: one listing (only when there is an image
    /// the drive is not known to hold), then one request per image, several at a time. The images
    /// that went up stay up, and are remembered, when another fails. Returns how many could not
    /// be sent this time.
    fn upload_attachments(&self, db: &mut Db, images: Vec<LocalImage>, report: &mut SyncReport) -> Result<usize> {
        if images.is_empty() {
            return Ok(0);
        }
        let total = images.len();
        // What the database says is on the drive is a cache. It is corrected against a listing
        // whenever a snapshot is taken (the first sync, then weekly), so that a file lost from the
        // drive is sent again within a week.
        let mut listing = None;
        if report.snapshot_taken {
            match self.remote_images() {
                Ok(remote) => {
                    let names: Vec<String> = remote.keys().cloned().collect();
                    db.replace_uploaded(&self.provider, &names)?;
                    listing = Some(remote);
                }
                Err(error) => return carry_on(error, total),
            }
        }
        let known = db.uploaded_images(&self.provider)?;
        let images: Vec<LocalImage> = images.into_iter().filter(|image| !known.contains(&image.name)).collect();
        if images.is_empty() {
            return Ok(0);
        }
        let total = images.len();
        let remote = match listing.map_or_else(|| self.remote_images(), Ok) {
            Ok(remote) => remote,
            Err(error) => return carry_on(error, total),
        };
        let (held, missing): (Vec<LocalImage>, Vec<LocalImage>) =
            images.into_iter().partition(|image| remote.contains_key(&image.name));
        let to_send = missing.len();
        let pooled = run_pool(UPLOAD_WORKERS, MAX_IMAGE_FAILURES, missing, |image| {
            let bytes = match std::fs::read(&image.local) {
                Ok(bytes) => bytes,
                // Deleted since it was listed: nothing to send.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e.into()),
            };
            self.drive.upload_new(&image.name, &bytes)?;
            Ok(Some(image.name))
        });
        let unsent = to_send - pooled.done.len() - pooled.skipped;
        report.attachments_uploaded += pooled.done.len();
        let on_drive: Vec<String> = held.into_iter().map(|image| image.name).chain(pooled.done).collect();
        db.mark_uploaded(&self.provider, &on_drive)?;
        pooled.error.map_or(Ok(0), |error| carry_on(error, unsent))
    }

    /// Downloads the images the journal names that are not on this machine (the file's absence is
    /// all it looks at: nothing is hashed). One the drive does not hold yet (the other device has
    /// not uploaded it) is left for a later sync. Several at a time; a file appears whole or not at
    /// all. Returns how many could not be fetched this time.
    fn download_attachments(&self, db: &mut Db, report: &mut SyncReport) -> Result<usize> {
        let Some(folder) = &self.attachments else { return Ok(0) };
        let shas = db.attachment_shas()?;
        let mut wanted = Vec::new();
        for path in db.attachment_paths()? {
            let Some(local) = inside(folder, &path) else { continue };
            let Some(sha) = shas.get(&path) else { continue };
            if !local.exists() {
                wanted.push((path, local, sha.clone()));
            }
        }
        if wanted.is_empty() {
            return Ok(0);
        }
        let remote = match self.remote_images() {
            Ok(remote) => remote,
            Err(error) => return carry_on(error, wanted.len()),
        };
        let jobs: Vec<WantedImage> = wanted
            .into_iter()
            .filter_map(|(path, local, sha)| {
                let file = remote.get(&image_name(&sha, &path))?.clone();
                Some(WantedImage { path, local, sha, file })
            })
            .collect();
        // The drive holds these whether or not this sync manages to fetch them.
        let on_drive: Vec<String> = jobs.iter().map(|image| image.file.name.clone()).collect();
        let to_fetch = jobs.len();
        let pooled = run_pool(DOWNLOAD_WORKERS, MAX_IMAGE_FAILURES, jobs, |image| {
            let bytes = self.drive.download(&image.file)?;
            if sha256_hex(&bytes) != image.sha {
                return Err(CloudError::Corrupt(format!("ảnh {} tải về không khớp mã băm", image.path)));
            }
            if let Some(parent) = image.local.parent() {
                std::fs::create_dir_all(parent)?;
            }
            write_whole(&image.local, &bytes)?;
            Ok(Some(()))
        });
        let unfetched = to_fetch - pooled.done.len() - pooled.skipped;
        report.attachments_downloaded += pooled.done.len();
        db.mark_uploaded(&self.provider, &on_drive)?;
        pooled.error.map_or(Ok(0), |error| carry_on(error, unfetched))
    }
}

/// What an image error means for the sync. The journal is already on the drive when images move,
/// so a failure of the link, the service or a file only leaves `unmoved` images for the next sync
/// (logged, and counted by the caller). A lost session, a refused permission or a broken database
/// still ends the sync: the screen has to react to those.
fn carry_on(error: CloudError, unmoved: usize) -> Result<usize> {
    if ends_sync(&error) {
        return Err(error);
    }
    log::warn!("sync: {unmoved} ảnh chưa chuyển được, lần sync sau thử lại: {error}");
    Ok(unmoved)
}

/// Whether an image error is one the sync cannot carry on after.
fn ends_sync(error: &CloudError) -> bool {
    matches!(error, CloudError::SignedOut | CloudError::DriveNotGranted | CloudError::Config(_) | CloudError::Db(_))
}

/// Writes `bytes` to `target` through a temporary file next to it, so that a reader (or a sync
/// that dies half way) never sees half an image. The temporary file does not stay behind.
fn write_whole(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut part = target.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let written = std::fs::write(&part, bytes).and_then(|()| std::fs::rename(&part, target));
    written.inspect_err(|_| drop(std::fs::remove_file(&part)))
}

/// An image on this machine that the drive may lack.
struct LocalImage {
    local: PathBuf,
    /// `attachments/<sha256>.<ext>`.
    name: String,
}

/// An image the journal names, absent here, that the drive holds.
struct WantedImage {
    path: String,
    local: PathBuf,
    sha: String,
    file: RemoteFile,
}

/// What [`run_pool`] got done.
struct Pooled<R> {
    /// What the jobs that answered `Ok(Some(_))` produced.
    done: Vec<R>,
    /// Jobs that answered `Ok(None)`: nothing to do for them any more.
    skipped: usize,
    /// The first error.
    error: Option<CloudError>,
}

/// Runs `job` over `items` on up to `workers` threads. A failed item does not stop the others,
/// but once `max_failures` have failed no new item starts (the link is probably down).
fn run_pool<T: Send, R: Send>(
    workers: usize,
    max_failures: usize,
    items: Vec<T>,
    job: impl Fn(T) -> Result<Option<R>> + Sync,
) -> Pooled<R> {
    let workers = workers.min(items.len());
    let queue = Mutex::new(items.into_iter());
    let results = Mutex::new(Vec::new());
    let skipped = AtomicUsize::new(0);
    let failures = AtomicUsize::new(0);
    let first_error: Mutex<Option<CloudError>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                while failures.load(Ordering::Relaxed) < max_failures {
                    let next = queue.lock().expect("lock poisoned").next();
                    let Some(item) = next else { break };
                    match job(item) {
                        Ok(Some(result)) => results.lock().expect("lock poisoned").push(result),
                        Ok(None) => {
                            skipped.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(error) => {
                            failures.fetch_add(1, Ordering::Relaxed);
                            // An error that ends the sync is kept over one that does not.
                            let mut kept = first_error.lock().expect("lock poisoned");
                            if kept.as_ref().is_none_or(|old| !ends_sync(old) && ends_sync(&error)) {
                                *kept = Some(error);
                            }
                        }
                    }
                }
            });
        }
    });
    Pooled {
        done: results.into_inner().expect("lock poisoned"),
        skipped: skipped.into_inner(),
        error: first_error.into_inner().expect("lock poisoned"),
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
