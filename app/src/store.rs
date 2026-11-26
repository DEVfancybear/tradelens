//! The app's state: what the pages read and change, over `tradelens-core`.
//! A `Copy` handle of signals, provided once by the root.
//!
//! The database is only touched on NSH's blocking threads. A [`Snapshot`] read there is
//! swapped into a signal whole. Writers outside the UI (an import, later the MT5 connection and
//! the cloud) never touch a signal: they bump the process-wide [`Revision`], which a short
//! timer compares with the last one seen; a change reloads the snapshot.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use nsh_core::prelude::*;
use tradelens_core::{Db, DbError, legacy};

use crate::{
    snapshot::{self, Query, Range, Snapshot},
    trade_list::{Filter, Sort},
};

/// How often the revision is compared with the one the store last loaded for: the longest a
/// change written off the UI thread waits before the pages reload (a reload itself takes
/// milliseconds).
const POLL: Duration = Duration::from_millis(400);

/// File name of the database in the app's data directory.
const DB_FILE: &str = "tradelens.db";

/// A counter every writer of the database bumps, shared across threads.
#[derive(Debug, Default)]
struct Revision(AtomicU64);

impl Revision {
    fn get(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }

    fn bump(&self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
}

/// The revision every writer of the database bumps.
fn revision() -> &'static Revision {
    static REVISION: OnceLock<Revision> = OnceLock::new();
    REVISION.get_or_init(Revision::default)
}

/// Tells the app that the database changed behind its back (an import, a sync, a second
/// writer): the pages reload within a poll.
pub fn data_changed() {
    revision().bump();
}

/// Which accounts the pages show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum AccountFilter {
    #[default]
    All,
    One(i64),
}

impl AccountFilter {
    /// The account id, or `None` for all accounts.
    pub(crate) fn id(self) -> Option<i64> {
        match self {
            Self::All => None,
            Self::One(id) => Some(id),
        }
    }
}

/// Where reading the data stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum Load {
    /// Nothing read yet.
    #[default]
    Loading,
    Ready,
    /// The last read failed; the previous snapshot stays on screen.
    Failed(String),
}

pub(crate) type SharedDb = Arc<Mutex<Db>>;

/// Work that waits for the database to open; it is given the database, or `None` when opening failed.
type WhenOpen = Box<dyn FnOnce(Option<SharedDb>)>;

/// What a read or a write says when the database could not be opened.
const NOT_OPEN: &str = "Chưa mở được cơ sở dữ liệu.";

#[derive(Clone, Copy)]
pub(crate) struct Store {
    pub(crate) accounts: Signal<AccountFilter>,
    /// How far back the overview looks.
    pub(crate) range: Signal<Range>,
    /// The revision the data was last asked for; moves when something stored data off the
    /// UI thread.
    pub(crate) revision: Signal<u64>,
    /// The trades page's filters and order, kept while a trade's page is open over it.
    pub(crate) trade_filter: Signal<Filter>,
    pub(crate) trade_sort: Signal<Option<Sort>>,
    snapshot: Signal<Arc<Snapshot>>,
    load: Signal<Load>,
    /// The database is open: reads that wait for it (`query`) start.
    ready: Signal<bool>,
    db: Stored<Option<SharedDb>>,
    /// The newest read asked for: an older one finishing later is dropped.
    generation: Stored<u64>,
    /// What was asked of the database before it was open: each runs once it is, or once opening has
    /// failed (`None`).
    waiting: Stored<Vec<WhenOpen>>,
}

impl Store {
    fn new() -> Self {
        Self {
            accounts: signal(AccountFilter::All),
            range: signal(Range::default()),
            revision: signal(revision().get()),
            trade_filter: signal(Filter::default()),
            trade_sort: signal(None),
            snapshot: signal_with(Arc::new(Snapshot::default()), SignalEquals::Never),
            load: signal(Load::Loading),
            ready: signal(false),
            db: stored(None),
            generation: stored(0),
            waiting: stored(Vec::new()),
        }
    }

    /// The data the pages show (tracked where it is read).
    pub(crate) fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot.get()
    }

    /// The data as it is now, for a listener (nothing subscribes).
    pub(crate) fn snapshot_now(&self) -> Arc<Snapshot> {
        self.snapshot.get_untracked()
    }

    pub(crate) fn load_state(&self) -> Load {
        self.load.get()
    }

    /// Reads the data again for the chosen accounts and range; the pages follow when it
    /// arrives. Does nothing until the database is open.
    pub(crate) fn reload(self) {
        let Some(db) = self.db.get() else { return };
        let query = Query::now(self.accounts.get_untracked().id(), self.range.get_untracked());
        let generation = self.generation.with(|g| {
            *g += 1;
            *g
        });
        spawn(async move {
            let read = blocking(move || {
                let db = db.lock().expect("database lock poisoned");
                snapshot::load(&db, &query)
            })
            .await;
            if self.generation.get() != generation {
                return;
            }
            match read {
                Ok(Ok(next)) => batch(|| {
                    self.snapshot.set(Arc::new(next));
                    self.load.set(Load::Ready);
                }),
                Ok(Err(error)) => self.load.set(Load::Failed(format!("Không đọc được dữ liệu: {error}"))),
                Err(error) => self.load.set(Load::Failed(format!("Luồng đọc dữ liệu bị dừng: {error}"))),
            }
        });
    }

    /// The open database, for a writer off the UI thread (the MT5 connection) that bumps the
    /// revision itself ([`data_changed`]); `None` until it is open.
    pub(crate) fn database(&self) -> Option<SharedDb> {
        self.db.get()
    }

    /// Whether the database is open: what waits for it (`query`, [`Store::read`]) starts then (tracked).
    pub(crate) fn is_ready(&self) -> bool {
        self.ready.get()
    }

    /// The database file, `None` when the app has no data directory (it then keeps the data in
    /// memory and nothing else can open it).
    pub(crate) fn database_path() -> Option<PathBuf> {
        data_dir().map(|dir| dir.join(DB_FILE))
    }

    /// Reads the database once on a blocking thread, and tells the UI thread the result (the
    /// failure in Vietnamese). Unlike a [`Reading`] it does not follow the revision.
    ///
    /// # Panics
    ///
    /// On the reading thread, when another thread panicked while holding the database lock.
    pub(crate) fn read<T: Send + 'static>(
        self,
        op: impl FnOnce(&Db) -> Result<T, DbError> + Send + 'static,
        done: impl FnOnce(Result<T, String>) + 'static,
    ) {
        self.when_open(move |db| {
            let Some(db) = db else {
                return done(Err(NOT_OPEN.to_owned()));
            };
            spawn(async move {
                let read = blocking(move || {
                    let db = db.lock().expect("database lock poisoned");
                    op(&db)
                })
                .await;
                done(match read {
                    Ok(Ok(value)) => Ok(value),
                    Ok(Err(error)) => Err(format!("Không đọc được dữ liệu: {error}")),
                    Err(error) => Err(format!("Luồng đọc dữ liệu bị dừng: {error}")),
                });
            });
        });
    }

    /// Runs `job` with the database: at once when it is open, else as soon as it is (or, when opening
    /// failed, at once with `None`). Nothing is polled: [`Store::open`] hands the database over.
    fn when_open(self, job: impl FnOnce(Option<SharedDb>) + 'static) {
        if let Some(db) = self.db.get() {
            return job(Some(db));
        }
        if matches!(self.load.get_untracked(), Load::Failed(_)) {
            return job(None);
        }
        let mut jobs = self.waiting.replace(Vec::new());
        jobs.push(Box::new(job));
        self.waiting.replace(jobs);
    }

    /// Hands the database (`None`: opening failed) to what waited for it.
    fn flush_waiting(self, db: Option<&SharedDb>) {
        for job in self.waiting.replace(Vec::new()) {
            job(db.map(Arc::clone));
        }
    }

    /// Replaces the database file with another one, for restoring a backup. The open database is
    /// first copied to `backup` (a file that must not exist yet), then closed: every other reader
    /// and writer waits on the lock meanwhile. `replace` gets the file's path, which nothing has
    /// open, and returns the database that now lives there, or fails; either way the file is
    /// opened again afterwards, so the app goes on with a database. The pages read again when it
    /// is done.
    ///
    /// # Panics
    ///
    /// On the working thread, when another thread panicked while holding the database lock.
    pub(crate) fn replace_database<T: Send + 'static>(
        self,
        backup: PathBuf,
        replace: impl FnOnce(&Path) -> Result<(Db, T), String> + Send + 'static,
        done: impl FnOnce(Result<T, String>) + 'static,
    ) {
        let (Some(db), Some(path)) = (self.db.get(), Self::database_path()) else {
            return done(Err("Chưa mở được tệp dữ liệu trên máy này.".to_owned()));
        };
        spawn(async move {
            let worked = blocking(move || {
                let mut guard = db.lock().expect("database lock poisoned");
                guard.snapshot_to(&backup).map_err(|error| format!("Không sao lưu được dữ liệu hiện tại: {error}"))?;
                let stand_in = Db::in_memory().map_err(|error| format!("Không đóng được dữ liệu hiện tại: {error}"))?;
                // Closing the file: nothing may hold it while it is replaced.
                drop(std::mem::replace(&mut *guard, stand_in));
                match replace(&path) {
                    Ok((fresh, value)) => {
                        *guard = fresh;
                        Ok(value)
                    }
                    Err(message) => {
                        match Db::open(&path) {
                            Ok(reopened) => *guard = reopened,
                            Err(error) => log::error!("database: cannot be reopened after a restore: {error}"),
                        }
                        Err(message)
                    }
                }
            })
            .await;
            let outcome = worked.unwrap_or_else(|error| Err(format!("Luồng khôi phục bị dừng: {error}")));
            if outcome.is_ok() {
                data_changed();
                self.revision.set(revision().get());
            }
            done(outcome);
        });
    }

    /// Opens the database off the UI thread, then makes the first read.
    fn open(self) {
        let dir = data_dir();
        let path = dir.as_ref().map(|dir| dir.join(DB_FILE));
        spawn(async move {
            let opened = blocking(move || {
                if let Some(dir) = &dir {
                    adopt_earlier_data(dir)?;
                }
                open_database(path).map_err(|error| error.to_string())
            })
            .await;
            match opened {
                Ok(Ok(db)) => {
                    self.db.replace(Some(Arc::clone(&db)));
                    self.ready.set(true);
                    self.flush_waiting(Some(&db));
                    self.reload();
                }
                Ok(Err(error)) => {
                    log::error!("database: {error}");
                    self.load.set(Load::Failed(format!("Không mở được cơ sở dữ liệu: {error}")));
                    self.flush_waiting(None);
                }
                Err(error) => {
                    self.load.set(Load::Failed(format!("Luồng mở dữ liệu bị dừng: {error}")));
                    self.flush_waiting(None);
                }
            }
        });
    }

    /// Wires the signals to the work: a change of accounts, range or revision reloads, and a
    /// timer brings the revision over from other threads.
    fn start(self) {
        effect(move || {
            let _ = self.accounts.get();
            let _ = self.range.get();
            let _ = self.revision.get();
            untracked(|| self.reload());
        });
        let seen = stored(self.revision.get_untracked());
        set_interval(POLL, move || {
            let now = revision().get();
            if now != seen.get() {
                seen.replace(now);
                self.revision.set(now);
            }
        });
        self.open();
    }
}

/// The folder NSH gave every app that had no id, which is where this app's data was until it got one.
const SHARED_FOLDER: &str = "default";

/// Copies the database and the images an earlier version kept in the folder shared by apps without
/// an id (`default`, next to the app's own folder) into `dir`, when `dir` has no database yet. The
/// old folder is not touched. Only the app's own folder qualifies: a data folder given by a test or
/// a command line has no earlier data to take.
fn adopt_earlier_data(dir: &Path) -> Result<(), String> {
    let own = dir.file_name().is_some_and(|name| name == crate::APP_ID);
    let Some(shared) = dir.parent().map(|parent| parent.join(SHARED_FOLDER)).filter(|_| own) else { return Ok(()) };
    match legacy::adopt(dir, &shared) {
        Ok(Some(adopted)) => {
            log::info!(
                "data: copied the database and {} images from {} to {}; the old folder is left as it was",
                adopted.images,
                adopted.from.display(),
                dir.display()
            );
            Ok(())
        }
        Ok(None) => Ok(()),
        Err(error) => {
            log::error!("data: the earlier data could not be copied: {error}");
            Err(format!("Không chép được dữ liệu từ thư mục cũ ({}): {error}", shared.display()))
        }
    }
}

/// The database file in `path`, or one in memory when there is no data directory.
fn open_database(path: Option<PathBuf>) -> Result<SharedDb, DbError> {
    let db = match path {
        Some(path) => Db::open(&path)?,
        None => Db::in_memory()?,
    };
    Ok(Arc::new(Mutex::new(db)))
}

/// What a [`Reading`] holds: the last value that was read, and how the next read stands.
struct ReadState<T> {
    value: Option<Arc<T>>,
    error: Option<String>,
    loading: bool,
}

impl<T> Clone for ReadState<T> {
    fn clone(&self) -> Self {
        Self { value: self.value.clone(), error: self.error.clone(), loading: self.loading }
    }
}

impl<T> Default for ReadState<T> {
    fn default() -> Self {
        Self { value: None, error: None, loading: false }
    }
}

/// A read of the database that follows the app: it runs on a blocking thread, and again
/// whenever the store's revision (an import, a [`Writer`] stored something), the
/// account filter or the query's own input changes. The last good value stays while the next
/// is read and when a read fails. A `Copy` handle, made with [`query`] or [`query_with`].
pub struct Reading<T: 'static> {
    state: Signal<ReadState<T>>,
    again: Signal<u32>,
}

impl<T> Clone for Reading<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Reading<T> {}

impl<T> std::fmt::Debug for Reading<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reading").finish_non_exhaustive()
    }
}

impl<T> Reading<T> {
    /// The last value read, `None` until the first read arrives (tracked where it is read).
    pub fn value(&self) -> Option<Arc<T>> {
        self.state.with(|s| s.value.clone())
    }

    /// Why the last read failed, in Vietnamese; `None` when it went through (tracked).
    pub fn error(&self) -> Option<String> {
        self.state.with(|s| s.error.clone())
    }

    /// A read is under way (tracked); [`Reading::value`] still holds the previous one.
    pub fn loading(&self) -> bool {
        self.state.with(|s| s.loading)
    }

    /// Reads again now, for data that changed where the store cannot see it.
    pub fn refresh(&self) {
        self.again.update(|n| *n = n.wrapping_add(1));
    }
}

/// Reads the database for the chosen accounts: `load` gets the database and the account filter
/// (`None` for all accounts) and runs on a blocking thread. Call it while building a component
/// (it makes an effect that the component owns).
pub fn query<T: Send + 'static>(
    load: impl Fn(&Db, Option<i64>) -> Result<T, DbError> + Send + Sync + 'static,
) -> Reading<T> {
    query_with(|| (), move |db, account, ()| load(db, account))
}

/// [`query`] with an input that is read where the query is made and moves with it: when what
/// `input` reads changes (a chosen month, the open trade), `load` runs again with the new value.
///
/// # Panics
///
/// On the reading thread, when another thread panicked while holding the database lock.
pub fn query_with<I: Send + 'static, T: Send + 'static>(
    input: impl Fn() -> I + 'static,
    load: impl Fn(&Db, Option<i64>, &I) -> Result<T, DbError> + Send + Sync + 'static,
) -> Reading<T> {
    let store = store();
    let load = Arc::new(load);
    let state = signal_with(ReadState::<T>::default(), SignalEquals::Never);
    let again = signal(0_u32);
    let generation = stored(0_u64);
    effect(move || {
        let ready = store.ready.get();
        let _ = (store.revision.get(), again.get());
        let account = store.accounts.get().id();
        let input = input();
        if !ready {
            // The database never opened: say why instead of waiting for it.
            if let Load::Failed(error) = store.load_state() {
                state.update(|s| {
                    s.error = Some(error);
                    s.loading = false;
                });
            }
            return;
        }
        untracked(|| {
            let Some(db) = store.db.get() else { return };
            let this_read = generation.with(|g| {
                *g += 1;
                *g
            });
            state.update(|s| s.loading = true);
            let load = Arc::clone(&load);
            spawn(async move {
                let read = blocking(move || {
                    let db = db.lock().expect("database lock poisoned");
                    load(&db, account, &input)
                })
                .await;
                if generation.get() != this_read {
                    return;
                }
                state.update(|s| {
                    s.loading = false;
                    match read {
                        Ok(Ok(value)) => {
                            s.value = Some(Arc::new(value));
                            s.error = None;
                        }
                        Ok(Err(error)) => s.error = Some(format!("Không đọc được dữ liệu: {error}")),
                        Err(error) => s.error = Some(format!("Luồng đọc dữ liệu bị dừng: {error}")),
                    }
                });
            });
        });
    });
    Reading { state, again }
}

/// Writes to the database from a screen. Made with [`writer`] while building, then used from
/// listeners: a `Copy` handle.
#[derive(Clone, Copy)]
pub struct Writer {
    store: Store,
}

impl std::fmt::Debug for Writer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Writer").finish_non_exhaustive()
    }
}

/// The handle to write with; call it while building a component, not in a listener.
pub fn writer() -> Writer {
    Writer { store: store() }
}

impl Writer {
    /// Runs `op` on a blocking thread with the database; when it has stored something the
    /// revision moves, so every [`Reading`] reads again. `done` then hears the result on the UI
    /// thread, with the failure in Vietnamese for the trader.
    ///
    /// # Panics
    ///
    /// On the writing thread, when another thread panicked while holding the database lock.
    pub fn run<T: Send + 'static>(
        self,
        op: impl FnOnce(&mut Db) -> Result<T, DbError> + Send + 'static,
        done: impl FnOnce(Result<T, String>) + 'static,
    ) {
        let store = self.store;
        // A write asked for while the database is still opening (a file dropped right after the
        // launch) waits for it; it fails only when opening did.
        store.when_open(move |db| {
            let Some(db) = db else {
                return done(Err(NOT_OPEN.to_owned()));
            };
            spawn(async move {
                let written = blocking(move || {
                    let mut db = db.lock().expect("database lock poisoned");
                    let stored = op(&mut db);
                    if stored.is_ok() {
                        // Bumped on the writing thread, not after the await: the screen that asked
                        // may be left while this runs, which ends its task, and the pages must still
                        // read the change (the store's poll sees the revision).
                        data_changed();
                    }
                    stored
                })
                .await;
                let outcome = match written {
                    Ok(Ok(value)) => Ok(value),
                    Ok(Err(error)) => Err(format!("Không ghi được dữ liệu: {error}")),
                    Err(error) => Err(format!("Luồng ghi dữ liệu bị dừng: {error}")),
                };
                if outcome.is_ok() {
                    store.revision.set(revision().get());
                }
                done(outcome);
            });
        });
    }
}

/// Provides the store to everything the root makes after it, and starts its work.
pub(crate) fn provide() {
    let store = Store::new();
    provide_context(store);
    store.start();
}

/// The store; read while building, never in a listener.
pub(crate) fn store() -> Store {
    use_context::<Store>().expect("the root provides the store")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_filter_names_an_id_or_none() {
        assert_eq!(AccountFilter::All.id(), None);
        assert_eq!(AccountFilter::One(3).id(), Some(3));
    }

    #[test]
    fn the_apps_own_folder_takes_a_copy_of_the_data_the_shared_folder_had() {
        let root = tempfile::tempdir().expect("a temp dir");
        let shared = root.path().join(SHARED_FOLDER);
        let mut old = Db::open(shared.join(DB_FILE)).expect("the old database opens");
        old.set_setting("marker", "old data").expect("writes");
        drop(old);
        let own = root.path().join(crate::APP_ID);

        adopt_earlier_data(&own).expect("copies");
        let copy = Db::open(own.join(DB_FILE)).expect("the copy opens");
        assert_eq!(copy.setting("marker").expect("reads").as_deref(), Some("old data"));
        assert!(shared.join(DB_FILE).exists(), "the old database stays where it was");
    }

    #[test]
    fn a_folder_that_is_not_the_apps_own_has_no_earlier_data_to_take() {
        let root = tempfile::tempdir().expect("a temp dir");
        Db::open(root.path().join(SHARED_FOLDER).join(DB_FILE)).expect("the old database opens");
        let other = root.path().join("a test's folder");
        adopt_earlier_data(&other).expect("nothing to do");
        assert!(!other.exists(), "nothing was created");
    }

    #[test]
    fn opening_without_a_directory_uses_memory() {
        let db = open_database(None).expect("a memory database opens");
        assert_eq!(db.lock().expect("unlocked").accounts().expect("reads"), Vec::new());
    }

    #[test]
    fn a_change_written_elsewhere_moves_the_revision() {
        let before = revision().get();
        data_changed();
        assert!(revision().get() > before, "the revision moved from {before}");
    }

    mod helpers {
        use std::time::Duration;

        use nsh_core::Size;
        use nsh_engine::{EngineConfig, Root};
        use nsh_testing::Headless;

        use super::*;
        use crate::ui::controls::{Variant, action};

        /// Shows what a query reads and has buttons that write and change the query's input.
        #[component]
        fn Harness() -> impl IntoElement {
            crate::ui::theme::install();
            provide();
            let key = signal("a");
            let reads = query_with(move || key.get(), |db, _, key| db.setting(key));
            let write = writer();
            view()
                .gap_2()
                .child(text(move || match reads.value() {
                    None => "nothing yet".to_owned(),
                    Some(value) => format!("got {value:?}"),
                }))
                .child(text(move || format!("loading {}", reads.loading())))
                .child(action("Ghi a", Variant::Secondary, move || {
                    write.run(|db| db.set_setting("a", "one"), |_| {});
                }))
                .child(action("Ghi a hai", Variant::Secondary, move || {
                    write.run(|db| db.set_setting("a", "two"), |_| {});
                }))
                .child(action("Ghi b", Variant::Secondary, move || {
                    write.run(|db| db.set_setting("b", "bee"), |_| {});
                }))
                .child(action("Đọc b", Variant::Secondary, move || key.set("b")))
        }

        fn start() -> (tempfile::TempDir, Headless) {
            let dir = tempfile::tempdir().expect("a temp dir");
            Db::open(dir.path().join(DB_FILE)).expect("the database opens");
            let config = EngineConfig {
                viewport: Size::new(nsh_core::prelude::px(600.0), nsh_core::prelude::px(500.0)),
                data_dir: Some(dir.path().to_path_buf()),
                ..EngineConfig::new(Root::of::<Harness>())
            };
            (dir, Headless::start(config).expect("the engine starts"))
        }

        #[test]
        fn a_query_waits_for_the_database_then_follows_every_write() {
            let (_dir, mut app) = start();
            app.wait_for_within("got None", Duration::from_secs(5)).expect("the first read finds no value");
            app.press("Ghi a").expect("the write button");
            app.wait_for_within("got Some(\"one\")", Duration::from_secs(5))
                .expect("the query read again after the write");
            app.press("Ghi a hai").expect("the second write");
            app.wait_for_within("got Some(\"two\")", Duration::from_secs(5)).expect("and again after the next");
            app.wait_for("loading false").expect("a read that is done is not loading");
        }

        /// Asks for a write and a read while it is being built, before the database has had the time
        /// to open on its own thread, and says how each went.
        #[component]
        fn EarlyAsks() -> impl IntoElement {
            crate::ui::theme::install();
            provide();
            let wrote = signal(String::from("write waiting"));
            let read = signal(String::from("read waiting"));
            writer().run(
                |db| db.set_setting("early", "yes"),
                move |result| wrote.set(format!("write {}", result.map_or_else(|error| error, |()| "ok".to_owned()))),
            );
            store().read(
                |db| db.setting("early"),
                move |result| {
                    read.set(format!("read {}", result.map_or_else(|error| error, |value| format!("{value:?}"))));
                },
            );
            view().child(text(move || wrote.get())).child(text(move || read.get()))
        }

        fn start_early(database: impl FnOnce(&std::path::Path)) -> (tempfile::TempDir, Headless) {
            let dir = tempfile::tempdir().expect("a temp dir");
            database(&dir.path().join(DB_FILE));
            let config = EngineConfig {
                viewport: Size::new(nsh_core::prelude::px(600.0), nsh_core::prelude::px(500.0)),
                data_dir: Some(dir.path().to_path_buf()),
                ..EngineConfig::new(Root::of::<EarlyAsks>())
            };
            (dir, Headless::start(config).expect("the engine starts"))
        }

        #[test]
        fn a_write_asked_for_before_the_database_opens_is_applied_once_it_does() {
            let (dir, mut app) = start_early(|path| {
                Db::open(path).expect("the database opens");
            });
            app.wait_for_within("write ok", Duration::from_secs(30)).expect("the early write was applied");
            let stored = Db::open(dir.path().join(DB_FILE)).expect("opens").setting("early").expect("reads");
            assert_eq!(stored.as_deref(), Some("yes"), "the write reached the database");
            // The read asked for at the same time ran after the database opened too (order of the two is
            // theirs: it saw the value or not, but it did not fail).
            app.wait_for_within("text:/^read (None|Some)/", Duration::from_secs(30))
                .expect("the early read was served");
        }

        #[test]
        fn a_write_asked_for_before_the_database_opens_fails_once_opening_has() {
            // A folder where the database file should be: opening it fails.
            let (_dir, mut app) = start_early(|path| std::fs::create_dir(path).expect("a folder in the way"));
            app.wait_for_within("write Chưa mở được cơ sở dữ liệu.", Duration::from_secs(30))
                .expect("the write is told the database is not open");
            app.wait_for_within("read Chưa mở được cơ sở dữ liệu.", Duration::from_secs(30))
                .expect("and so is the read");
        }

        #[test]
        fn a_query_reads_again_when_its_input_changes() {
            let (_dir, mut app) = start();
            // The database opens in the background: the first read says it is ready to write to.
            app.wait_for_within("got None", Duration::from_secs(5)).expect("input a has no value");
            app.press("Ghi b").expect("the write button");
            app.press("Đọc b").expect("changes the input");
            app.wait_for_within("got Some(\"bee\")", Duration::from_secs(5)).expect("input b is read");
        }
    }
}
