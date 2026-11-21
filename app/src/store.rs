//! The app's state: what the pages read and change, over `tradelens-core`.
//! A `Copy` handle of signals, provided once by the root.
//!
//! The database is only touched on NSH's blocking threads. A [`Snapshot`] read there is
//! swapped into a signal whole. Writers outside the UI (the bridge, later the cloud) never
//! touch a signal: they bump the process-wide [`Revision`], which a short timer compares
//! with the last one seen; a change reloads the snapshot.

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        mpsc::{self, Receiver},
    },
    time::Duration,
};

use nsh_core::prelude::*;
use tradelens_bridge::{Bridge, BridgeConfig, BridgeError, BridgeEvent, DEFAULT_PORT, Revision};
use tradelens_core::{Db, DbError, JournalEntry, TradeKey};

use crate::{
    journal_draft::Draft,
    snapshot::{self, Query, Range, Snapshot},
    trade_list::{Filter, Sort},
};

/// How often the revision is compared with the one the store last loaded for: the longest a
/// trade the bridge stored waits before the pages reload (a reload itself takes milliseconds).
const POLL: Duration = Duration::from_millis(400);

/// File name of the database in the app's data directory.
const DB_FILE: &str = "tradelens.db";

/// Settings keys of the bridge (`Db::setting`).
const SETTING_TOKEN: &str = "bridge_token";
const SETTING_PORT: &str = "bridge_port";

/// How many lines of the bridge's log the data sources page keeps.
const LOG_LINES: usize = 40;

/// The revision the bridge and every other writer of the database bump.
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

/// Whether the bridge listens for the EA and the Python script.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum BridgePhase {
    #[default]
    Starting,
    Listening {
        port: u16,
    },
    /// It could not start, or stopped; the reason is shown to the trader.
    Unavailable {
        reason: String,
    },
}

/// What the data sources page and the sidebar show of the bridge.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct BridgeInfo {
    pub(crate) phase: BridgePhase,
    /// Clients send it in `X-TradeLens-Token`.
    pub(crate) token: String,
    pub(crate) port: u16,
    /// What the bridge received, newest last.
    pub(crate) log: Vec<String>,
}

impl BridgeInfo {
    /// The line at the sidebar's foot.
    pub(crate) fn status_line(&self) -> String {
        match &self.phase {
            BridgePhase::Starting => "Bridge MT5: đang khởi động".to_owned(),
            BridgePhase::Listening { port } => format!("Bridge MT5: đang nghe cổng {port}"),
            BridgePhase::Unavailable { .. } => "Bridge MT5: chưa chạy".to_owned(),
        }
    }
}

type SharedDb = Arc<Mutex<Db>>;

#[derive(Clone, Copy)]
pub(crate) struct Store {
    pub(crate) accounts: Signal<AccountFilter>,
    /// How far back the overview looks.
    pub(crate) range: Signal<Range>,
    /// The revision the data was last asked for; moves when the bridge or the cloud stored
    /// something.
    pub(crate) revision: Signal<u64>,
    /// The trades page's filters and order, kept while a trade's page is open over it.
    pub(crate) trade_filter: Signal<Filter>,
    pub(crate) trade_sort: Signal<Option<Sort>>,
    snapshot: Signal<Arc<Snapshot>>,
    load: Signal<Load>,
    bridge: Signal<BridgeInfo>,
    db: Stored<Option<SharedDb>>,
    /// The newest read asked for: an older one finishing later is dropped.
    generation: Stored<u64>,
    bridge_handle: Stored<Option<Bridge>>,
    events: Stored<Option<Receiver<BridgeEvent>>>,
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
            bridge: signal(BridgeInfo::default()),
            db: stored(None),
            generation: stored(0),
            bridge_handle: stored(None),
            events: stored(None),
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

    pub(crate) fn bridge(&self) -> BridgeInfo {
        self.bridge.get()
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

    /// Writes the edited fields of a trade's journal entry over the entry the database has
    /// now, then reads the data again; `done` hears how the write went (in Vietnamese, for
    /// the trader).
    pub(crate) fn save_draft(self, key: TradeKey, draft: Draft, done: impl FnOnce(Result<(), String>) + 'static) {
        let Some(db) = self.db.get() else {
            return done(Err("Chưa mở được cơ sở dữ liệu.".to_owned()));
        };
        spawn(async move {
            let saved = blocking(move || {
                let mut db = db.lock().expect("database lock poisoned");
                // Read the entry again: the bridge may have attached screenshots since the page opened.
                let mut entry: JournalEntry = db.journal(&key)?.unwrap_or_default();
                draft.apply(&mut entry);
                db.save_journal(&key, &entry)
            })
            .await;
            let outcome = match saved {
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) => Err(format!("Không lưu được nhật ký: {error}")),
                Err(error) => Err(format!("Luồng ghi dữ liệu bị dừng: {error}")),
            };
            if outcome.is_ok() {
                self.reload();
            }
            done(outcome);
        });
    }

    /// Opens the database off the UI thread, then starts the bridge and the first read.
    fn open(self) {
        let path = data_dir().map(|dir| dir.join(DB_FILE));
        spawn(async move {
            let opened = blocking(move || open_database(path)).await;
            match opened {
                Ok(Ok(opened)) => {
                    self.db.replace(Some(opened.db));
                    self.reload();
                    self.start_bridge(opened.bridge);
                }
                Ok(Err(error)) => {
                    log::error!("database: {error}");
                    self.load.set(Load::Failed(format!("Không mở được cơ sở dữ liệu: {error}")));
                    self.set_unavailable("không có cơ sở dữ liệu".to_owned());
                }
                Err(error) => self.load.set(Load::Failed(format!("Luồng mở dữ liệu bị dừng: {error}"))),
            }
        });
    }

    fn set_unavailable(self, reason: String) {
        self.bridge.update(|info| info.phase = BridgePhase::Unavailable { reason });
    }

    /// Starts the bridge when the database lives in a file the bridge can open too.
    fn start_bridge(self, config: Option<BridgeConfig>) {
        let Some(config) = config else {
            return self.set_unavailable("dữ liệu chỉ nằm trong bộ nhớ".to_owned());
        };
        let (port, token) = (config.port, config.token.clone());
        self.bridge.update(|info| {
            info.port = port;
            info.token = token;
        });
        let (sender, receiver) = mpsc::channel();
        // The bridge reports from its own thread, so events go through a channel the timer drains.
        let started = Bridge::start(config, revision().clone(), move |event| {
            if sender.send(event).is_err() {
                log::debug!("the app stopped listening to the bridge");
            }
        });
        match started {
            Ok(bridge) => {
                // The bound port, which differs from the configured one when that is 0.
                let port = bridge.port();
                self.events.replace(Some(receiver));
                self.bridge_handle.replace(Some(bridge));
                self.bridge.update(|info| {
                    info.port = port;
                    info.phase = BridgePhase::Listening { port };
                });
            }
            Err(error) => {
                log::warn!("bridge: {error}");
                self.set_unavailable(bridge_reason(&error));
            }
        }
    }

    /// Wires the signals to the work: a change of accounts, range or revision reloads, and a
    /// timer brings the revision and the bridge's events over from other threads.
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
            self.drain_events();
        });
        self.open();
    }

    fn drain_events(self) {
        let events: Vec<BridgeEvent> = self.events.with(|rx| rx.as_ref().map_or_default(|rx| rx.try_iter().collect()));
        if events.is_empty() {
            return;
        }
        self.bridge.update(|info| {
            for event in events {
                apply_event(info, event);
            }
        });
    }
}

/// Folds one bridge event into what the data sources page shows.
fn apply_event(info: &mut BridgeInfo, event: BridgeEvent) {
    let line = match event {
        BridgeEvent::Listening { port } => {
            info.phase = BridgePhase::Listening { port };
            format!("Đang nghe cổng {port}")
        }
        BridgeEvent::Received { source, account, inserted, updated } => {
            format!("{source} · tài khoản {account}: {inserted} deal mới, {updated} cập nhật")
        }
        BridgeEvent::PositionOpened(position) => {
            format!("Lệnh mới mở: {} {} {}", position.symbol, position.side.label(), position.volume)
        }
        BridgeEvent::Rejected { reason } => format!("Từ chối: {reason}"),
        BridgeEvent::Stopped { error } => {
            let reason = error.unwrap_or_else(|| "đã dừng".to_owned());
            info.phase = BridgePhase::Unavailable { reason: reason.clone() };
            format!("Bridge dừng: {reason}")
        }
    };
    info.log.push(line);
    if info.log.len() > LOG_LINES {
        let excess = info.log.len() - LOG_LINES;
        info.log.drain(..excess);
    }
}

/// Why the bridge could not start, as the trader reads it.
fn bridge_reason(error: &BridgeError) -> String {
    match error {
        BridgeError::NotImplemented(_) => "bridge chưa có trong bản này".to_owned(),
        other => other.to_string(),
    }
}

/// What opening the database produced.
struct Opened {
    db: SharedDb,
    /// For the bridge: present when the database is a file.
    bridge: Option<BridgeConfig>,
}

fn open_database(path: Option<PathBuf>) -> Result<Opened, DbError> {
    let Some(path) = path else {
        return Ok(Opened { db: Arc::new(Mutex::new(Db::in_memory()?)), bridge: None });
    };
    let mut db = Db::open(&path)?;
    let token = match db.setting(SETTING_TOKEN)? {
        Some(token) if !token.is_empty() => token,
        _ => {
            let token = tradelens_bridge::new_token();
            db.set_setting(SETTING_TOKEN, &token)?;
            token
        }
    };
    let port = db.setting(SETTING_PORT)?.and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT);
    Ok(Opened { db: Arc::new(Mutex::new(db)), bridge: Some(BridgeConfig { db_path: path, port, token }) })
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
    fn events_fold_into_the_bridge_info() {
        let mut info = BridgeInfo::default();
        apply_event(&mut info, BridgeEvent::Listening { port: 7878 });
        assert_eq!(info.phase, BridgePhase::Listening { port: 7878 });
        assert_eq!(info.status_line(), "Bridge MT5: đang nghe cổng 7878");
        apply_event(
            &mut info,
            BridgeEvent::Received { source: "ea".into(), account: "52012345".into(), inserted: 2, updated: 0 },
        );
        assert_eq!(info.log.last().map(String::as_str), Some("ea · tài khoản 52012345: 2 deal mới, 0 cập nhật"));
        apply_event(&mut info, BridgeEvent::Stopped { error: Some("cổng bận".into()) });
        assert_eq!(info.status_line(), "Bridge MT5: chưa chạy");
    }

    #[test]
    fn the_log_keeps_its_newest_lines() {
        let mut info = BridgeInfo::default();
        for i in 0..(LOG_LINES + 5) {
            apply_event(&mut info, BridgeEvent::Rejected { reason: i.to_string() });
        }
        assert_eq!(info.log.len(), LOG_LINES);
        assert_eq!(info.log.last().map(String::as_str), Some(&*format!("Từ chối: {}", LOG_LINES + 4)));
    }

    #[test]
    fn the_account_filter_names_an_id_or_none() {
        assert_eq!(AccountFilter::All.id(), None);
        assert_eq!(AccountFilter::One(3).id(), Some(3));
    }

    #[test]
    fn a_missing_bridge_reads_as_not_running() {
        let reason = bridge_reason(&BridgeError::NotImplemented("bridge server"));
        assert_eq!(reason, "bridge chưa có trong bản này");
    }

    #[test]
    fn opening_without_a_directory_uses_memory_and_no_bridge() {
        let opened = open_database(None).expect("a memory database opens");
        assert!(opened.bridge.is_none());
    }

    #[test]
    fn opening_a_file_makes_a_token_once() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join(DB_FILE);
        let first = open_database(Some(path.clone())).expect("opens").bridge.expect("a file has a bridge");
        let second = open_database(Some(path)).expect("opens again").bridge.expect("a file has a bridge");
        assert_eq!(first.token.len(), 32);
        assert_eq!(first.token, second.token, "the token stays the same across runs");
        assert_eq!(first.port, DEFAULT_PORT);
    }
}
